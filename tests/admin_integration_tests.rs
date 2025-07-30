// tests/admin_integration_tests.rs

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use gemini_proxy_key_rotation_rust::{
    admin::{AddKeysRequest, CsrfTokenResponse, DeleteKeysRequest},
    config::{AppConfig, KeyGroup},
    run,
};
use http_body_util::BodyExt;
use std::sync::Once;
use tempfile::TempDir;
use tower::util::ServiceExt;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

static TRACING_INIT: Once = Once::new();

/// Initializes the tracing subscriber for tests, ensuring it only runs once.
fn ensure_tracing_initialized() {
    TRACING_INIT.call_once(|| {
        let env_filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
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
    _temp_dir: TempDir, // Keep temp dir alive
    auth_cookie: Option<String>,
    csrf_token: Option<String>,
    csrf_cookie: Option<String>,
}

impl TestApp {
    async fn new(config: AppConfig) -> Self {
        ensure_tracing_initialized();
        let temp_dir = tempfile::tempdir().unwrap();
        let config_file_path = temp_dir.path().join("config.yaml");

        let config_str = serde_yaml::to_string(&config).unwrap();
        tokio::fs::write(&config_file_path, &config_str)
            .await
            .unwrap();

        let (router, _app_state) = run(Some(config_file_path))
            .await
            .expect("Failed to create test router");

        TestApp {
            router,
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
                    .body(Body::from(format!(r#"{{"token": "{token}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK, "Login failed");
        let cookie = response
            .headers()
            .get("set-cookie")
            .expect("set-cookie header missing")
            .to_str()
            .unwrap();
        self.auth_cookie = Some(cookie.to_string());
    }

    /// Gets a CSRF token and stores it.
    async fn get_csrf_token(&mut self) {
        let auth_cookie = self.auth_cookie.as_ref().expect("Must be logged in to get CSRF token");
        let response = self
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/admin/csrf-token")
                    .header(header::COOKIE, auth_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK, "Failed to get CSRF token");
        let csrf_cookie = response
            .headers()
            .get("set-cookie")
            .expect("CSRF set-cookie header missing")
            .to_str()
            .unwrap();
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
        let auth_cookie = self.auth_cookie.as_ref().expect("Not logged in");
        let csrf_cookie = self.csrf_cookie.as_ref().expect("CSRF cookie not set");
        let csrf_token = self.csrf_token.as_ref().expect("CSRF token not set");

        self.router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("Content-Type", "application/json")
                    .header(header::COOKIE, format!("{auth_cookie}; {csrf_cookie}"))
                    .header("x-csrf-token", csrf_token)
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
    config.server.test_mode = true; // Enable test mode
    config.groups = vec![KeyGroup {
        name: "default".to_string(),
        api_keys: vec!["key1".to_string()],
        ..Default::default()
    }];
    // For integration tests, disable Redis to avoid connection issues
    config.redis_url = None;
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

    app.login("secret_admin_token").await;
    assert!(app.auth_cookie.is_some());

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

    assert_eq!(
        received_config.server.admin_token,
        config.server.admin_token
    );
}

#[tokio::test]
async fn test_add_keys_unauthorized() {
    let config = get_default_config();
    let app = TestApp::new(config).await;

    let body = Body::from(
        serde_json::to_string(&AddKeysRequest {
            group_name: "default".to_string(),
            api_keys: vec!["new_key".to_string()],
        })
        .unwrap(),
    );

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

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_add_keys_no_csrf() {
    let config = get_default_config();
    let mut app = TestApp::new(config).await;
    app.login("secret_admin_token").await;

    let body = Body::from(
        serde_json::to_string(&AddKeysRequest {
            group_name: "default".to_string(),
            api_keys: vec!["new_key".to_string()],
        })
        .unwrap(),
    );

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

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_add_keys_success() {
    let config = get_default_config();
    let mut app = TestApp::new(config).await;

    app.login("secret_admin_token").await;
    app.get_csrf_token().await;

    let body = Body::from(
        serde_json::to_string(&AddKeysRequest {
            group_name: "default".to_string(),
            api_keys: vec!["new_key_1".to_string(), "new_key_2".to_string()],
        })
        .unwrap(),
    );

    let response = app.authed_request(Method::POST, "/admin/keys", body).await;
    assert_eq!(response.status(), StatusCode::OK);

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

    let default_group = updated_config
        .groups
        .iter()
        .find(|g| g.name == "default")
        .unwrap();
    assert_eq!(default_group.api_keys.len(), 3);
    assert!(default_group.api_keys.contains(&"key1".to_string()));
    assert!(default_group.api_keys.contains(&"new_key_1".to_string()));
}

#[tokio::test]
async fn test_delete_keys_success() {
    let mut config = get_default_config();
    config.groups[0].api_keys.push("key_to_delete".to_string());
    let mut app = TestApp::new(config).await;
    app.login("secret_admin_token").await;
    app.get_csrf_token().await;

    let body = Body::from(
        serde_json::to_string(&DeleteKeysRequest {
            group_name: "default".to_string(),
            api_keys: vec!["key_to_delete".to_string()],
        })
        .unwrap(),
    );

    let response = app
        .authed_request(Method::DELETE, "/admin/keys", body)
        .await;
    assert_eq!(response.status(), StatusCode::OK);

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

    let default_group = updated_config
        .groups
        .iter()
        .find(|g| g.name == "default")
        .unwrap();
    assert_eq!(default_group.api_keys.len(), 1);
    assert!(default_group.api_keys.contains(&"key1".to_string()));
    assert!(!default_group.api_keys.contains(&"key_to_delete".to_string()));
}

#[tokio::test]
async fn test_update_config_success() {
    let initial_config = get_default_config();
    let mut app = TestApp::new(initial_config).await;
    app.login("secret_admin_token").await;
    app.get_csrf_token().await;

    let mut new_config = get_default_config();
    new_config.server.port = 9999;
    new_config.groups[0].name = "renamed_group".to_string();
    new_config.groups[0].api_keys.push("new_key_in_updated_config".to_string());

    let body = Body::from(serde_json::to_string(&new_config).unwrap());
    let response = app.authed_request(Method::PUT, "/admin/config", body).await;
    assert_eq!(response.status(), StatusCode::OK);

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

    assert_eq!(updated_config.server.port, 9999);
    assert_eq!(updated_config.groups.len(), 1);
    assert_eq!(updated_config.groups[0].name, "renamed_group");
    assert_eq!(updated_config.groups[0].api_keys.len(), 2);
    assert!(updated_config.groups[0].api_keys.contains(&"new_key_in_updated_config".to_string()));
}
