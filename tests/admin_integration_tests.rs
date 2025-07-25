// tests/admin_integration_tests.rs

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use gemini_proxy_key_rotation_rust::{
    config::AppConfig,
    state::AppState,
    run,
};
use std::sync::Arc;
use tempfile::TempDir;
use tower::util::ServiceExt;

// Helper structure to manage the test application state
struct TestApp {
    router: axum::Router,
    _state: Arc<AppState>, // Keep state alive
    _temp_dir: TempDir,   // Keep temp dir alive
}

impl TestApp {
    async fn new(mut config: AppConfig) -> Self {
        let temp_dir = tempfile::tempdir().unwrap();
        let config_path = temp_dir.path().to_path_buf();

        // Save the provided config to a temporary file
        let config_str = serde_yaml::to_string(&config).unwrap();
        tokio::fs::write(config_path.join("config.yaml"), config_str)
            .await
            .unwrap();

        let state = Arc::new(AppState::new(&mut config, &config_path).await.unwrap());
        let router = run(state.clone()).await;

        TestApp {
            router,
            _state: state,
            _temp_dir: temp_dir,
        }
    }
}

#[tokio::test]
async fn test_detailed_health_ok() {
    let config = AppConfig::default();
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
async fn test_login_success() {
    let mut config = AppConfig::default();
    config.server.admin_token = Some("secret_admin_token".to_string());
    let app = TestApp::new(config).await;

    let response = app
        .router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/login")
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"token": "secret_admin_token"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response.headers().get("set-cookie").unwrap().to_str().unwrap();
    assert!(cookie.contains("admin_token=secret_admin_token"));
    assert!(cookie.contains("HttpOnly"));
}

#[tokio::test]
async fn test_login_failure() {
    let mut config = AppConfig::default();
    config.server.admin_token = Some("secret_admin_token".to_string());
    let app = TestApp::new(config).await;

    let response = app
        .router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/login")
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"token": "wrong_token"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}