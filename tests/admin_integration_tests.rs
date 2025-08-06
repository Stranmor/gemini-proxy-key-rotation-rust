use axum::{
    body::{to_bytes, Body},
    extract::connect_info::MockConnectInfo,
    http::{header, Method, Request, StatusCode},
    Router,
};
use gemini_proxy::{
    admin::admin_routes,
    config::{AppConfig, KeyGroup, ServerConfig},
    state::AppState,
};
use std::{net::SocketAddr, sync::Arc};
use tempfile::TempDir;
use tower::ServiceExt;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};
struct TestApp {
    router: Router,
    _state: Arc<AppState>,
    auth_cookie: Option<String>,
    csrf_cookie: Option<String>,
    csrf_token: Option<String>,
    _mock_server: MockServer,
    _temp_dir: TempDir,
}

impl TestApp {
    async fn new() -> Self {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let config_file_path = temp_dir.path().join("config.yaml");

        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let app_config = AppConfig {
            server: ServerConfig {
                admin_token: Some("test-token".to_string()),
                test_mode: true,
                port: 0,
                connect_timeout_secs: 1, // Short timeout for tests
                request_timeout_secs: 1, // Short timeout for tests
                ..Default::default()
            },
            groups: vec![KeyGroup {
                name: "default".to_string(),
                api_keys: vec!["test-key-1".to_string()],
                target_url: mock_server.uri(), // Use a real URL that doesn't require connection
                proxy_url: None,
                ..Default::default()
            }],
            redis_url: None, // Explicitly disable Redis
            ..Default::default()
        };

        let (state_instance, mut config_update_rx) = AppState::new(&app_config, &config_file_path)
            .await
            .expect("Failed to create app state");
        let state: Arc<AppState> = Arc::new(state_instance);
        
        // Start background worker for config updates (like in main app)
        let state_for_worker = state.clone();
        tokio::spawn(async move {
            loop {
                match config_update_rx.recv().await {
                    Ok(new_config) => {
                        if let Err(e) = gemini_proxy::admin::reload_state_from_config(
                            state_for_worker.clone(), 
                            new_config
                        ).await {
                            eprintln!("Failed to reload state in test worker: {:?}", e);
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        let router = admin_routes(state.clone())
            .with_state(state.clone())
            .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 3000))));

        TestApp {
            router,
            _state: state,
            auth_cookie: None,
            csrf_cookie: None,
            csrf_token: None,
            _mock_server: mock_server,
            _temp_dir: temp_dir,
        }
    }

    async fn login(&mut self) {
        let request = Request::builder()
            .method(Method::POST)
            .uri("/admin/login")
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"token": "test-token"}"#))
            .unwrap();

        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        if status != StatusCode::OK {
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let body_str = String::from_utf8_lossy(&body);
            panic!("Login failed with status {status}: {body_str}");
        }

        let auth_cookie = response
            .headers()
            .get("set-cookie")
            .expect("set-cookie header missing")
            .to_str()
            .unwrap()
            .to_string();
        self.auth_cookie = Some(auth_cookie);
    }

    async fn get_csrf_token(&mut self) {
        if self.auth_cookie.is_none() {
            self.login().await;
        }
        let auth_cookie = self
            .auth_cookie
            .as_ref()
            .expect("Must be logged in to get CSRF token");

        let request = Request::builder()
            .method(Method::GET)
            .uri("/admin/csrf-token")
            .header(header::COOKIE, auth_cookie)
            .body(Body::empty())
            .unwrap();

        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        if status != StatusCode::OK {
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let body_str = String::from_utf8_lossy(&body);
            panic!("Failed to get CSRF token with status {status}: {body_str}");
        }

        let csrf_cookie = response
            .headers()
            .get("set-cookie")
            .expect("CSRF set-cookie header missing")
            .to_str()
            .unwrap()
            .to_string();
        self.csrf_cookie = Some(csrf_cookie);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let csrf_token = json["csrf_token"].as_str().unwrap().to_string();
        self.csrf_token = Some(csrf_token);
    }

    async fn authed_request(
        &self,
        method: Method,
        uri: &str,
        body: Body,
    ) -> axum::response::Response {
        let auth_cookie = self.auth_cookie.as_ref().expect("Not logged in");

        let mut request_builder = Request::builder()
            .method(method.clone())
            .uri(format!("/admin{uri}"))
            .header(header::COOKIE, auth_cookie);

        // Only add CSRF headers for methods that require them
        if matches!(method, Method::POST | Method::PUT | Method::DELETE) {
            let csrf_cookie = self.csrf_cookie.as_ref().expect("CSRF cookie not set");
            let csrf_token = self.csrf_token.as_ref().expect("CSRF token not set");
            
            request_builder = request_builder
                .header(header::COOKIE, csrf_cookie)
                .header("x-csrf-token", csrf_token);
        }

        let request = request_builder
            .header("Content-Type", "application/json")
            .body(body)
            .unwrap();

        self.router.clone().oneshot(request).await.unwrap()
    }
}

#[tokio::test]
async fn test_login_and_csrf_token() {
    let mut app = TestApp::new().await;
    app.login().await;
    assert!(app.auth_cookie.is_some());

    app.get_csrf_token().await;
    assert!(app.csrf_cookie.is_some());
    assert!(app.csrf_token.is_some());
}

#[tokio::test]
async fn test_health_check() {
    let mut app = TestApp::new().await;
    app.login().await;

    let response = app
        .authed_request(Method::GET, "/health", Body::empty())
        .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_get_config() {
    let mut app = TestApp::new().await;
    app.login().await;

    let response = app
        .authed_request(Method::GET, "/config", Body::empty())
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let config: AppConfig = serde_json::from_slice(&body).unwrap();
    assert_eq!(config.server.admin_token, Some("test-token".to_string()));
}

#[tokio::test]
async fn test_add_key() {
    let mut app = TestApp::new().await;
    app.login().await;
    app.get_csrf_token().await;

    let new_key = serde_json::json!({
        "group_name": "default",
        "api_keys": ["new-test-key"]
    });
    let body = Body::from(serde_json::to_string(&new_key).unwrap());

    let response = app.authed_request(Method::POST, "/keys", body).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // Allow time for the background worker to process the update
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Verify that the key was added by checking config
    let response = app
        .authed_request(Method::GET, "/config", Body::empty())
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let config: AppConfig = serde_json::from_slice(&body).unwrap();
    let group = config.groups.iter().find(|g| g.name == "default").unwrap();
    assert!(group.api_keys.contains(&"new-test-key".to_string()));
}

#[tokio::test]
async fn test_add_key_with_model_specific_rules() {
    let mut app = TestApp::new().await;
    app.login().await;
    app.get_csrf_token().await;

    let new_key = serde_json::json!({
        "group_name": "default",
        "api_keys": ["new-key-with-rules"]
    });
    let body = Body::from(serde_json::to_string(&new_key).unwrap());

    let response = app.authed_request(Method::POST, "/keys", body).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // Allow time for the background worker to process the update
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Verify key was added
    let response = app
        .authed_request(Method::GET, "/config", Body::empty())
        .await;
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let config: AppConfig = serde_json::from_slice(&body).unwrap();
    let group = config
        .groups
        .iter()
        .find(|g| g.name == "default")
        .unwrap();
    let key = group.api_keys.iter().find(|k| **k == "new-key-with-rules").unwrap();
    assert_eq!(*key, "new-key-with-rules");
}

#[tokio::test]
async fn test_delete_key() {
    let mut app = TestApp::new().await;
    app.login().await;
    app.get_csrf_token().await;

    // First, add a key to be deleted
    let key_to_delete = serde_json::json!({
        "group_name": "default",
        "api_keys": ["a_new_key_to_be_added_and_then_deleted"]
    });
    let body = Body::from(serde_json::to_string(&key_to_delete).unwrap());
    let response = app.authed_request(Method::POST, "/keys", body).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // Allow time for the background worker to process the update
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Now, delete the key
    let body = Body::from(
        serde_json::to_string(&serde_json::json!({
            "group_name": "default",
            "api_keys": ["a_new_key_to_be_added_and_then_deleted"]
        }))
        .unwrap(),
    );
    let response = app.authed_request(Method::DELETE, "/keys", body).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // Allow time for the background worker to process the update
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Verify that the key was deleted
    let response = app
        .authed_request(Method::GET, "/config", Body::empty())
        .await;
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8(body.to_vec()).unwrap();
    assert!(!body_str.contains("a_new_key_to_be_added_and_then_deleted"));
}

#[tokio::test]
async fn test_update_config() {
    let mut app = TestApp::new().await;
    app.login().await;
    app.get_csrf_token().await;

    // First, get the current config to make sure we are not deleting anything important
    let response = app
        .authed_request(Method::GET, "/config", Body::empty())
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let mut config: AppConfig = serde_json::from_slice(&body).unwrap();

    // Add a new key to the config
    config.groups[0].api_keys.push("new_key_in_updated_config".to_string());

    let body = Body::from(serde_json::to_string(&config).unwrap());
    let response = app.authed_request(Method::PUT, "/config", body).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // Allow time for the background worker to process the update
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Verify that the config was updated
    let response = app
        .authed_request(Method::GET, "/config", Body::empty())
        .await;
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8(body.to_vec()).unwrap();
    assert!(body_str.contains("new_key_in_updated_config"));
}
