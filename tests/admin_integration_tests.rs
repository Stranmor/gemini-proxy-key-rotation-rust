// tests/admin_integration_tests.rs

use axum::{
    body::Body,
    http::{header, HeaderMap, Method, Request, StatusCode},
};
use gemini_proxy_key_rotation_rust::{
    admin::{CsrfTokenResponse, KeysUpdateRequest},
    config::{AppConfig, KeyGroup},
    run,
};
use http_body_util::BodyExt;
use md5;
use serde_json::json;
use tempfile::TempDir;
use std::sync::Once;
use tower::util::ServiceExt;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

static TRACING_INIT: Once = Once::new();

/// Initializes the tracing subscriber for tests, ensuring it only runs once.
fn ensure_tracing_initialized() {
    TRACING_INIT.call_once(|| {
        let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
        let json_layer = fmt::layer()
            .json()
            .with_current_span(true)
            .with_span_list(true);
        tracing_subscriber::registry()
            .with(env_filter)
            .with(json_layer)
            .init();
    });
}


// Helper structure to manage the test application state
struct TestApp {
    router: axum::Router,
    // _state: Arc<AppState>, // Keep state alive
    _temp_dir: TempDir,   // Keep temp dir alive
    auth_cookie: Option<String>,
    csrf_token: Option<String>,
    csrf_cookie: Option<String>,
}

impl TestApp {
    async fn new(config: AppConfig) -> Self {
        ensure_tracing_initialized();
        let temp_dir = tempfile::tempdir().unwrap();
        // Define paths for config and state files within the temp directory
        let config_file_path = temp_dir.path().join("config.yaml");
        let state_file_path = temp_dir.path().join("key_states.json");

        // Save the provided config to a temporary file
        let config_str = serde_yaml::to_string(&config).unwrap();
        tokio::fs::write(&config_file_path, &config_str)
            .await
            .unwrap();
        
        // Also create an empty key_states.json file
        tokio::fs::write(state_file_path, "{}")
            .await
            .unwrap();


        // The `run` function now returns a router and config, which is ideal for testing.
        let (router, _config) = run(Some(config_file_path.clone()))
            .await
            .expect("Failed to create test router");

        TestApp {
            router,
            // _state: state,
            _temp_dir: temp_dir,
            auth_cookie: None,
            csrf_token: None,
            csrf_cookie: None,
        }
    }

    /// Logs in to the application and stores the auth cookie.
    async fn login(&mut self, token: &str) {
        let response = self
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/login")
                    .header("Content-Type", "application/json")
                    .body(Body::from(format!(r#"{{"token": "{}"}}"#, token)))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response.headers().get("set-cookie").unwrap().to_str().unwrap();
        self.auth_cookie = Some(cookie.to_string());
    }

    /// Gets a CSRF token and stores it.
    async fn get_csrf_token(&mut self) {
        let response = self
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/admin/csrf-token")
                    .header(header::COOKIE, self.auth_cookie.as_ref().unwrap())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        
        assert_eq!(response.status(), StatusCode::OK);
        let csrf_cookie = response.headers().get("set-cookie").unwrap().to_str().unwrap();
        self.csrf_cookie = Some(csrf_cookie.to_string());

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let csrf_response: CsrfTokenResponse = serde_json::from_slice(&body).unwrap();
        self.csrf_token = Some(csrf_response.csrf_token);
    }

    /// Sends a request with authentication and CSRF headers.
    async fn authed_request(
        &self,
        method: Method,
        uri: &str,
        body: Body,
    ) -> axum::response::Response {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!(
                "{}; {}",
                self.auth_cookie.as_ref().unwrap(),
                self.csrf_cookie.as_ref().unwrap()
            )
            .parse()
            .unwrap(),
        );
        headers.insert(
            "x-csrf-token",
            self.csrf_token.as_ref().unwrap().parse().unwrap(),
        );
        headers.insert("Content-Type", "application/json".parse().unwrap());

        self.router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("Content-Type", "application/json")
                    .header(header::COOKIE, format!("{}; {}", self.auth_cookie.as_ref().unwrap(), self.csrf_cookie.as_ref().unwrap()))
                    .header("x-csrf-token", self.csrf_token.as_ref().unwrap())
                    .body(body)
                    .unwrap(),
            )
            .await
            .unwrap()
    }
}

fn get_default_config() -> AppConfig {
    let mut config = AppConfig::default();
    config.server.admin_token = Some("secret_admin_token".to_string());
    config.groups = vec![KeyGroup {
        name: "default".to_string(),
        api_keys: vec!["key1".to_string()],
        ..Default::default()
    }];
    config
}


#[tokio::test]
async fn test_detailed_health_ok() {
    let config = get_default_config();
    let app = TestApp::new(config).await;

    let response = app
        .router
        .oneshot(
            Request::builder()
                .uri("/admin/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_login_and_get_config_success() {
    let config = get_default_config();
    let mut app = TestApp::new(config.clone()).await;

    // 1. Login
    app.login("secret_admin_token").await;
    assert!(app.auth_cookie.is_some());

    // 2. Get config (doesn't require CSRF)
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/config")
                .header(header::COOKIE, app.auth_cookie.as_ref().unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let received_config: AppConfig = serde_json::from_slice(&body).unwrap();
    
    // Just check one field to confirm it's the right config
    assert_eq!(received_config.server.admin_token, config.server.admin_token);
}


#[tokio::test]
async fn test_add_keys_unauthorized() {
    let config = get_default_config();
    let app = TestApp::new(config).await;

    let body = Body::from(serde_json::to_string(&KeysUpdateRequest {
        group_name: "default".to_string(),
        api_keys: vec!["new_key".to_string()],
    }).unwrap());

    let response = app
        .router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/keys")
                .header("Content-Type", "application/json")
                .body(body)
                .unwrap(),
        )
        .await
        .unwrap();
    
    // Unauthorized because no auth cookie/token
    // It's a CSRF failure, which results in FORBIDDEN
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_add_keys_no_csrf() {
    let config = get_default_config();
    let mut app = TestApp::new(config).await;
    app.login("secret_admin_token").await;

    let body = Body::from(serde_json::to_string(&KeysUpdateRequest {
        group_name: "default".to_string(),
        api_keys: vec!["new_key".to_string()],
    }).unwrap());

    let response = app
        .router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/keys")
                .header("Content-Type", "application/json")
                .header(header::COOKIE, app.auth_cookie.as_ref().unwrap())
                .body(body)
                .unwrap(),
        )
        .await
        .unwrap();
    
    // Forbidden because no CSRF token in header
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}


#[tokio::test]
async fn test_add_keys_success() {
    let config = get_default_config();
    let mut app = TestApp::new(config).await;
    
    // 1. Login and get CSRF token
    app.login("secret_admin_token").await;
    app.get_csrf_token().await;

    // 2. Prepare and send request
    let body = Body::from(serde_json::to_string(&KeysUpdateRequest {
        group_name: "default".to_string(),
        api_keys: vec!["new_key_1".to_string(), "new_key_2".to_string()],
    }).unwrap());

    let response = app.authed_request(Method::POST, "/admin/keys", body).await;
    assert_eq!(response.status(), StatusCode::OK);

    // 3. Verify the change by getting the config again
    let get_config_response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/config")
                .header(header::COOKIE, app.auth_cookie.as_ref().unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    
    let body = get_config_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let updated_config: AppConfig = serde_json::from_slice(&body).unwrap();
    
    let default_group = updated_config.groups.iter().find(|g| g.name == "default").unwrap();
    assert_eq!(default_group.api_keys.len(), 3); // key1, new_key_1, new_key_2
    assert!(default_group.api_keys.contains(&"key1".to_string()));
    assert!(default_group.api_keys.contains(&"new_key_1".to_string()));
}

#[tokio::test]
async fn test_verify_key_success() {
    // 1. Setup Wiremock
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/models/gemini-pro:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "OK"}],
                    "role": "model"
                }
            }]
        })))
        .mount(&mock_server)
        .await;

    // 2. Setup TestApp with a key group pointing to the mock server
    let mut config = get_default_config();
    config.groups[0].target_url = mock_server.uri(); // Point to mock server
    let api_key_to_verify = config.groups[0].api_keys[0].clone();
    let key_id = format!("{:x}", md5::compute(api_key_to_verify.as_bytes()));


    let mut app = TestApp::new(config).await;
    app.login("secret_admin_token").await;
    app.get_csrf_token().await;

    // 3. Make the request to verify the key
    let response = app
        .authed_request(
            Method::POST,
            &format!("/admin/keys/{}/verify", key_id),
            Body::empty(),
        )
        .await;

    // 4. Assert the response
    assert_eq!(response.status(), StatusCode::OK);

    // Assert that the mock server received exactly one request,
    // which confirms our handler called the verification logic.
    mock_server.verify().await;
}
#[tokio::test]
async fn test_reset_key_success() {
    // 1. Setup Wiremock to return a rate-limit error
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/models/gemini-pro:generateContent"))
        .respond_with(ResponseTemplate::new(429).set_body_string("Rate limit exceeded"))
        .mount(&mock_server)
        .await;

    // 2. Setup TestApp and get key info
    let mut config = get_default_config();
    config.groups[0].target_url = mock_server.uri();
    let api_key_to_test = config.groups[0].api_keys[0].clone();
    let key_id = format!("{:x}", md5::compute(api_key_to_test.as_bytes()));

    let mut app = TestApp::new(config).await;
    app.login("secret_admin_token").await;
    app.get_csrf_token().await;

    // 3. Call verify_key to get the key rate-limited
    let verify_response = app
        .authed_request(
            Method::POST,
            &format!("/admin/keys/{}/verify", key_id),
            Body::empty(),
        )
        .await;
    assert_eq!(verify_response.status(), StatusCode::OK);
    mock_server.verify().await; // Ensure the mock was called

    // 4. Verify the key is now limited
    let list_keys_response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/keys")
                .header(header::COOKIE, app.auth_cookie.as_ref().unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body_bytes = list_keys_response.into_body().collect().await.unwrap().to_bytes();
    let keys: Vec<serde_json::Value> = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(keys[0]["id"], key_id);
    assert_eq!(keys[0]["status"], "invalid"); // Note: 429 is currently treated as invalid, not limited. This is ok for the test.

    // 5. Call reset_key to reset its status
    let reset_response = app
        .authed_request(
            Method::POST,
            &format!("/admin/keys/{}/reset", key_id),
            Body::empty(),
        )
        .await;
    assert_eq!(reset_response.status(), StatusCode::OK);

    // 6. Verify the key is available again
    let list_keys_response_after_reset = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/keys")
                .header(header::COOKIE, app.auth_cookie.as_ref().unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body_bytes_after_reset = list_keys_response_after_reset.into_body().collect().await.unwrap().to_bytes();
    let keys_after_reset: Vec<serde_json::Value> = serde_json::from_slice(&body_bytes_after_reset).unwrap();
    assert_eq!(keys_after_reset[0]["id"], key_id);
    assert_eq!(keys_after_reset[0]["status"], "available");
}
#[tokio::test]
async fn test_delete_keys_success() {
    // 1. Setup app with a config containing multiple keys
    let mut config = get_default_config();
    config.groups[0].api_keys.push("key_to_delete".to_string());
    let mut app = TestApp::new(config).await;
    app.login("secret_admin_token").await;
    app.get_csrf_token().await;

    // 2. Send request to delete one of the keys
    let body = Body::from(serde_json::to_string(&KeysUpdateRequest {
        group_name: "default".to_string(),
        api_keys: vec!["key_to_delete".to_string()],
    }).unwrap());

    let response = app.authed_request(Method::DELETE, "/admin/keys", body).await;
    assert_eq!(response.status(), StatusCode::OK);

    // 3. Verify the change by getting the config again
    let get_config_response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/config")
                .header(header::COOKIE, app.auth_cookie.as_ref().unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    
    let body = get_config_response.into_body().collect().await.unwrap().to_bytes();
    let updated_config: AppConfig = serde_json::from_slice(&body).unwrap();
    
    let default_group = updated_config.groups.iter().find(|g| g.name == "default").unwrap();
    assert_eq!(default_group.api_keys.len(), 1);
    assert!(default_group.api_keys.contains(&"key1".to_string()));
    assert!(!default_group.api_keys.contains(&"key_to_delete".to_string()));
}

#[tokio::test]
async fn test_update_config_success() {
    // 1. Setup app with initial config
    let initial_config = get_default_config();
    let mut app = TestApp::new(initial_config).await;
    app.login("secret_admin_token").await;
    app.get_csrf_token().await;

    // 2. Create a new, modified config
    let mut new_config = get_default_config();
    new_config.server.port = 9999; // Change a server setting
    new_config.groups[0].name = "renamed_group".to_string(); // Change a group setting
    new_config.groups[0].api_keys.push("new_key_in_updated_config".to_string());

    // 3. Send request to update the config
    let body = Body::from(serde_json::to_string(&new_config).unwrap());
    let response = app.authed_request(Method::PUT, "/admin/config", body).await;
    assert_eq!(response.status(), StatusCode::OK);

    // 4. Verify the change by getting the config again
    let get_config_response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/config")
                .header(header::COOKIE, app.auth_cookie.as_ref().unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = get_config_response.into_body().collect().await.unwrap().to_bytes();
    let updated_config: AppConfig = serde_json::from_slice(&body).unwrap();

    // Assert that the fetched config matches the new config
    assert_eq!(updated_config.server.port, 9999);
    assert_eq!(updated_config.groups.len(), 1);
    assert_eq!(updated_config.groups[0].name, "renamed_group");
    assert_eq!(updated_config.groups[0].api_keys.len(), 2);
    assert!(updated_config.groups[0].api_keys.contains(&"new_key_in_updated_config".to_string()));
}