// tests/integration_tests.rs

use axum::http::{header, HeaderMap, Method, StatusCode, Uri}; // Added header
use axum::body::Bytes;
use gemini_proxy_key_rotation_rust::{
    config::{AppConfig, KeyGroup, ServerConfig},
    key_manager::FlattenedKeyInfo,
    proxy,
    state::AppState,
};
use wiremock::{MockServer, Mock, ResponseTemplate, matchers::{method, path, header}};

// Helper function to create a basic AppConfig for testing
fn create_test_config(groups: Vec<KeyGroup>, server_host: &str, server_port: u16) -> AppConfig {
    AppConfig {
        server: ServerConfig {
            host: server_host.to_string(),
            port: server_port,
        },
        groups,
    }
}

#[tokio::test]
async fn test_forward_request_openai_compat_success_no_proxy() { // Renamed for clarity
    // 1. Setup Mock Server (Target API - OpenAI Compatibility Layer)
    let server = MockServer::start().await;
    let test_api_key = "test-key-123";
    // Use an OpenAI-compatible path
    let test_path = "/v1/models";

    // Expect both headers required by the OpenAI compatibility layer
    let expected_bearer = format!("Bearer {}", test_api_key);

    Mock::given(method("GET"))
        .and(path("/models")) // Expect path after prefix stripping
        .and(header("x-goog-api-key", test_api_key)) // Check for x-goog-api-key
        .and(header(header::AUTHORIZATION.as_str(), expected_bearer.as_str())) // Check for Authorization: Bearer
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"object\": \"list\", \"data\": []}")) // Example OpenAI-like response
        .mount(&server)
        .await;

    // 2. Setup Test Configuration and State
    let test_group = KeyGroup {
        name: "test-group".to_string(),
        api_keys: vec![test_api_key.to_string()],
        // Target URL should be the BASE URL of the OpenAI compatibility endpoint
        target_url: server.uri(), // Point target URL to the mock server's root
        proxy_url: None, // Explicitly no proxy for this test
    };
    // Use a dummy server host/port for AppConfig as it's not directly used in this test logic
    let config = create_test_config(vec![test_group], "127.0.0.1", 9999);
    let app_state = AppState::new(&config).expect("Failed to create AppState for test");
    let client = app_state.client(); // Get the shared reqwest client

    // Prepare key info for the request
    let key_info = FlattenedKeyInfo {
        key: test_api_key.to_string(),
        group_name: "test-group".to_string(),
        target_url: server.uri(), // Use the same base URL
        proxy_url: None,
    };

    // 3. Prepare Request Components
    let method = Method::GET;
    // The URI passed to forward_request represents the *original* request made to the proxy
    // It should contain the path that the client intended to access.
    let uri: Uri = format!("http://proxy-address-doesnt-matter:8081{}", test_path)
                     .parse()
                     .expect("Failed to parse test URI");
    let headers = HeaderMap::new(); // Start with empty headers, proxy adds auth
    let body_bytes = Bytes::new();

    // 4. Call the function under test
    let result = proxy::forward_request(
        client,
        &key_info,
        method,
        uri,
        headers,
        body_bytes,
    ).await;

    // 5. Assertions
    assert!(result.is_ok(), "forward_request failed: {:?}", result.err());
    let response = result.unwrap();
    assert_eq!(response.status(), StatusCode::OK, "Expected status OK (200)");

    // Optional: Check response body if needed
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                        .await
                        .expect("Failed to read response body");
    let body_str = String::from_utf8(body_bytes.to_vec()).expect("Body not UTF-8");
    assert!(body_str.contains("list"), "Response body mismatch"); // Check for OpenAI-like field
}

// TODO: Add more tests:
// - Test with POST /v1/chat/completions and body forwarding
// - Test rate limit handling (429 response from mock server) -> Need KeyManager interaction
// - Test error scenarios (e.g., mock server returning 500, connection error)
// - Test SOCKS5 proxy scenario (requires more complex setup, potentially external process or dockerized proxy)