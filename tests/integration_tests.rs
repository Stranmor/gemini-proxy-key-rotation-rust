// tests/integration_tests.rs

use axum::{
    // body::Bytes as _, // Removed unused import placeholder
    extract::{Request, State},
    http::{/* header, */ Method, StatusCode, Uri}, // Removed unused header
    response::Response,
};
use gemini_proxy_key_rotation_rust::{
    config::{AppConfig, KeyGroup, ServerConfig},
    handler, // Import the handler module
    // key_manager::FlattenedKeyInfo, // Removed unused import
    // proxy,
    state::AppState,
};
use std::{fs::File, path::PathBuf, sync::Arc};
use tempfile::tempdir;
use wiremock::{
    matchers::{method, path, path_regex, query_param}, // Use path and query_param
    Mock,
    MockServer,
    ResponseTemplate,
};

// Helper function to create a basic AppConfig for testing
fn create_test_config(groups: Vec<KeyGroup>, server_port: u16) -> AppConfig {
    AppConfig {
        server: ServerConfig {
            port: server_port,
            top_p: None,
        },
        groups,
        rate_limit_behavior: Default::default(),
    }
}

// Helper to create a dummy config file path within a temp dir
fn create_dummy_config_path_for_test(dir: &tempfile::TempDir) -> PathBuf {
    let file_path = dir.path().join("dummy_config_for_test.yaml");
    // Create the file, but content doesn't strictly matter for these handler tests
    File::create(&file_path).expect("Failed to create dummy config file");
    file_path
}

// Helper to make a request to the proxy handler
async fn call_proxy_handler(
    state: Arc<AppState>,
    method: Method,
    path: &str,
    body: axum::body::Body,
) -> Response {
    let uri: Uri = format!("http://test-proxy.com{}", path) // Base URL doesn't matter here
        .parse()
        .expect("Failed to parse test URI for handler");
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .body(body) // Use empty body for GET/POST tests for simplicity
        .unwrap();

    // Call the actual handler function
    handler::proxy_handler(State(state), request)
        .await
        .expect("Proxy handler returned an error") // Unwrap the Result<Response, AppError>
}

#[tokio::test]
async fn test_forward_request_openai_compat_success_no_proxy() {
    // This test now implicitly tests the handler as well for the success path.
    // We can keep it or refactor it slightly to use call_proxy_handler.
    // Let's keep it for now as it tests proxy::forward_request logic well.

    // 1. Setup Mock Server
    let server = MockServer::start().await;
    let test_api_key = "test-key-123";
    let test_path = "/v1/models";
    let expected_path = "/v1beta/openai/models";
    // let _expected_bearer = format!("Bearer {}", test_api_key); // Unused now

    Mock::given(method("GET"))
        .and(path(expected_path))
        .and(query_param("key", test_api_key)) // Match key in query param
        // Removed header matchers
        .respond_with(
            ResponseTemplate::new(200).set_body_string("{\"object\": \"list\", \"data\": []}"),
        )
        .mount(&server)
        .await;

    // 2. Setup Config and State
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let dummy_config_path = create_dummy_config_path_for_test(&temp_dir);
    let test_group = KeyGroup {
        name: "test-group".to_string(),
        api_keys: vec![test_api_key.to_string()],
        target_url: server.uri(),
        proxy_url: None,
        top_p: None,
    };
    let config = create_test_config(vec![test_group], 9999);
    let app_state = Arc::new(
        AppState::new(&config, &dummy_config_path)
            .await
            .expect("AppState failed"),
    ); // Wrap in Arc

    // 3. Call handler directly
    let response =
        call_proxy_handler(app_state, Method::GET, test_path, axum::body::Body::empty()).await;

    // 4. Assertions
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Expected status OK (200)"
    );
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read response body");
    let body_str = String::from_utf8(body_bytes.to_vec()).expect("Body not UTF-8");
    assert!(body_str.contains("list"), "Response body mismatch");
}

#[tokio::test]
async fn test_handler_retries_on_429_and_succeeds() {
    // 1. Setup Mock Server
    let server = MockServer::start().await;
    let key1 = "key-limited";
    let key2 = "key-working";
    let test_path = "/v1/generateContent";
    let expected_path = "/v1beta/openai/generateContent";

    // Mock for the first key (key1) - returns 429
    Mock::given(method("POST")) // Assuming POST for generateContent
        .and(path(expected_path))
        .and(query_param("key", key1)) // Match key in query param
        // Removed header matcher
        .respond_with(ResponseTemplate::new(429).set_body_string("Rate limit exceeded"))
        .mount(&server)
        .await;

    // Mock for the second key (key2) - returns 200
    Mock::given(method("POST"))
        .and(path(expected_path))
        .and(query_param("key", key2)) // Match key in query param
        // Removed header matcher
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"candidates\": []}")) // Example success response
        .mount(&server)
        .await;

    // 2. Setup Config and State
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let dummy_config_path = create_dummy_config_path_for_test(&temp_dir);
    // Key order matters for the round-robin
    let test_group = KeyGroup {
        name: "retry-group".to_string(),
        api_keys: vec![key1.to_string(), key2.to_string()], // key1 will be tried first
        target_url: server.uri(),
        proxy_url: None,
        top_p: None,
    };
    let config = create_test_config(vec![test_group], 9998); // Different port just in case
    let app_state = Arc::new(
        AppState::new(&config, &dummy_config_path)
            .await
            .expect("AppState failed"),
    );

    // 3. Call handler
    // We expect the handler to try key1, get 429, mark it, try key2, get 200, and return 200.
    let response = call_proxy_handler(
        app_state,
        Method::POST,
        test_path,
        axum::body::Body::empty(),
    )
    .await;

    // 4. Assertions
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Expected status OK (200) after retry"
    );
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read success response body");
    let body_str = String::from_utf8(body_bytes.to_vec()).expect("Success body not UTF-8");
    assert!(
        body_str.contains("candidates"),
        "Success response body mismatch"
    );

    // TODO: Optionally, check the persisted state file to ensure key1 is marked as limited.
    // This requires reading the state file (`key_states.json`) from the temp_dir.
}

#[tokio::test]
async fn test_handler_returns_last_429_on_exhaustion() {
    // 1. Setup Mock Server
    let server = MockServer::start().await;
    let key1 = "key-exhausted-1";
    let key2 = "key-exhausted-2";
    let test_path = "/v1/models"; // Using GET for simplicity here
    let expected_path = "/v1beta/openai/models";

    // Mock for the first key (key1) - returns 429
    Mock::given(method("GET"))
        .and(path(expected_path))
        .and(query_param("key", key1)) // Match key in query param
        // Removed header matcher
        .respond_with(ResponseTemplate::new(429).set_body_string("Rate limit 1"))
        .mount(&server)
        .await;

    // Mock for the second key (key2) - also returns 429
    Mock::given(method("GET"))
        .and(path(expected_path))
        .and(query_param("key", key2)) // Match key in query param
        // Removed header matcher
        .respond_with(ResponseTemplate::new(429).set_body_string("Rate limit 2")) // Different body to check which 429 is returned
        .mount(&server)
        .await;

    // 2. Setup Config and State
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let dummy_config_path = create_dummy_config_path_for_test(&temp_dir);
    let test_group = KeyGroup {
        name: "exhaust-group".to_string(),
        api_keys: vec![key1.to_string(), key2.to_string()],
        target_url: server.uri(),
        proxy_url: None,
        top_p: None,
    };
    let config = create_test_config(vec![test_group], 9997);
    let app_state = Arc::new(
        AppState::new(&config, &dummy_config_path)
            .await
            .expect("AppState failed"),
    );

    // 3. Call handler
    // We expect the handler to try key1 (429), try key2 (429), run out of keys, and return the *last* 429 response.
    let response =
        call_proxy_handler(app_state, Method::GET, test_path, axum::body::Body::empty()).await;

    // 4. Assertions
    assert_eq!(
        response.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "Expected status 429 when all keys are exhausted"
    );
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read 429 response body");
    let body_str = String::from_utf8(body_bytes.to_vec()).expect("429 body not UTF-8");
    // Check it returned the body from the *second* 429 response
    assert!(
        body_str.contains("Rate limit 2"),
        "Expected body from the last 429 response, got: {}",
        body_str
    );
}



#[tokio::test]
async fn test_handler_group_round_robin() {
    // 1. Setup Mock Server
    let server = MockServer::start().await;
    let test_path = "/v1/models";
    let expected_path = "/v1beta/openai/models";

    let g1_key1 = "g1-key-1";
    let g1_key2 = "g1-key-2";
    let g2_key1 = "g2-key-1"; // Single key in this group
    let g3_key1 = "g3-key-1";

    // Mock successful responses for all keys initially
    for key in [g1_key1, g1_key2, g2_key1, g3_key1] {
        Mock::given(method("GET"))
            .and(path_regex(format!("^{}.*", expected_path))) // Match any path starting with test_path
            .and(query_param("key", key))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!("{{\"key_used\": \"{}\"}}", key)))
            .mount(&server)
            .await;
    }

    // 2. Setup Config and State
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let dummy_config_path = create_dummy_config_path_for_test(&temp_dir);
    let groups = vec![
        KeyGroup {
            name: "group1".to_string(),
            api_keys: vec![g1_key1.to_string(), g1_key2.to_string()],
            target_url: server.uri(),
            proxy_url: None,
            top_p: None,
        },
        KeyGroup {
            name: "group2".to_string(),
            api_keys: vec![g2_key1.to_string()],
            target_url: server.uri(),
            proxy_url: None,
            top_p: None,
        },
        KeyGroup {
            name: "group3".to_string(),
            api_keys: vec![g3_key1.to_string()],
            target_url: server.uri(),
            proxy_url: None,
            top_p: None,
        },
    ];
    let config = create_test_config(groups, 9996);
    let app_state = Arc::new(
        AppState::new(&config, &dummy_config_path)
            .await
            .expect("AppState failed"),
    );

    // Helper to extract key from response body
    async fn get_key_from_response(response: Response) -> String {
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("Failed to read response body");
        let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).expect("Invalid JSON");
        body_json["key_used"].as_str().unwrap().to_string()
    }

    // 3. Call handler multiple times and check key rotation
    // Expected sequence: g1k1, g2k1, g3k1, g1k2, g2k1, g3k1, g1k1, ...

    let res1 = call_proxy_handler(Arc::clone(&app_state), Method::GET, &format!("{}?req=1", test_path), axum::body::Body::empty()).await;
    assert_eq!(res1.status(), StatusCode::OK);
    assert_eq!(get_key_from_response(res1).await, g1_key1);

    let res2 = call_proxy_handler(Arc::clone(&app_state), Method::GET, &format!("{}?req=2", test_path), axum::body::Body::empty()).await;
    assert_eq!(res2.status(), StatusCode::OK);
    assert_eq!(get_key_from_response(res2).await, g2_key1);

    let res3 = call_proxy_handler(Arc::clone(&app_state), Method::GET, &format!("{}?req=3", test_path), axum::body::Body::empty()).await;
    assert_eq!(res3.status(), StatusCode::OK);
    assert_eq!(get_key_from_response(res3).await, g3_key1);

    let res4 = call_proxy_handler(Arc::clone(&app_state), Method::GET, &format!("{}?req=4", test_path), axum::body::Body::empty()).await;
    assert_eq!(res4.status(), StatusCode::OK);
    assert_eq!(get_key_from_response(res4).await, g1_key2); // Next key in group1

    let res5 = call_proxy_handler(Arc::clone(&app_state), Method::GET, &format!("{}?req=5", test_path), axum::body::Body::empty()).await;
    assert_eq!(res5.status(), StatusCode::OK);
    assert_eq!(get_key_from_response(res5).await, g2_key1); // Back to group2 (only one key)

    let res6 = call_proxy_handler(Arc::clone(&app_state), Method::GET, &format!("{}?req=6", test_path), axum::body::Body::empty()).await;
    assert_eq!(res6.status(), StatusCode::OK);
    assert_eq!(get_key_from_response(res6).await, g3_key1); // Back to group3 (only one key)

    let res7 = call_proxy_handler(Arc::clone(&app_state), Method::GET, &format!("{}?req=7", test_path), axum::body::Body::empty()).await;
    assert_eq!(res7.status(), StatusCode::OK);
    assert_eq!(get_key_from_response(res7).await, g1_key1); // Back to start of group1

    // 4. Test skipping a rate-limited group
    // Reset mocks and set g2_key1 to return 429, others to 200
    server.reset().await;
    Mock::given(method("GET"))
        .and(path(expected_path))
        .and(query_param("key", g2_key1))
        .respond_with(ResponseTemplate::new(429))
        .mount(&server)
        .await;
    // Remount mocks for other keys to return 200
    for key in [g1_key1, g1_key2, g3_key1] { // Exclude g2_key1
        Mock::given(method("GET"))
            .and(path(expected_path))
            .and(query_param("key", key))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!("{{\"key_used\": \"{}\"}}", key)))
            .mount(&server)
            .await;
    }

    // Make a request - should hit g2k1, get 429, mark key, retry
    // Expected sequence now: g3k1 (skips g2), g1k2 (skips g2), ...

    // Current state: next should be group2 (index 1) according to previous calls
    // Try g2k1 -> 429 -> mark g2k1 limited -> continue search
    // Try group3 (index 2) -> g3k1 -> OK
    let res_skip1 = call_proxy_handler(Arc::clone(&app_state), Method::GET, &format!("{}?req=8", test_path), axum::body::Body::empty()).await;
    assert_eq!(res_skip1.status(), StatusCode::OK);
    assert_eq!(get_key_from_response(res_skip1).await, g3_key1); // Expect g3_key1 because g2 is skipped

    // Current state: next should be group0 (index 0)
    // Try g1k2 -> OK
    let res_skip2 = call_proxy_handler(Arc::clone(&app_state), Method::GET, &format!("{}?req=9", test_path), axum::body::Body::empty()).await;
    assert_eq!(res_skip2.status(), StatusCode::OK);
    assert_eq!(get_key_from_response(res_skip2).await, g1_key2);

    // Current state: next should be group1 (index 1)
    // Try g2k1 -> still 429 -> continue search
    // Try group3 (index 2) -> g3k1 -> OK
    let res_skip3 = call_proxy_handler(Arc::clone(&app_state), Method::GET, &format!("{}?req=10", test_path), axum::body::Body::empty()).await;
    assert_eq!(res_skip3.status(), StatusCode::OK);
    assert_eq!(get_key_from_response(res_skip3).await, g3_key1);
}

// TODO: Add more tests from the plan:
// - Test with POST /v1/chat/completions and body forwarding (similar structure, just change method and add body to request/mocks)
// - Test error scenarios (e.g., mock server returning 500) -> Handler should return the corresponding error response immediately
// - Test persistence logic explicitly by reading/writing state file.
// - Test SOCKS5 proxy scenario (more complex setup needed)

#[tokio::test]
async fn test_openai_top_p_injection_correctly() {
    // Goal: Verify that top_p is injected at the top level for OpenAI compatibility.
    // 1. Setup Mock Server
    let server = MockServer::start().await;
    let test_api_key = "openai-top-p-key";
    let test_path = "/v1/chat/completions";
    let expected_path = "/v1beta/openai/chat/completions";
    let top_p_value = 0.88f32;

    // Mock to verify the body modification
    Mock::given(method("POST"))
        .and(path(expected_path))
        .and(query_param("key", test_api_key))
        .and(move |req: &wiremock::Request| {
            // Custom matcher to inspect the body for a top-level "top_p"
            if let Ok(body_json) = serde_json::from_slice::<serde_json::Value>(&req.body) {
                if let Some(top_p) = body_json.get("top_p") {
                    return top_p.as_f64().map_or(false, |v| (v as f32 - top_p_value).abs() < f32::EPSILON);
                }
            }
            false
        })
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": "chatcmpl-123", "object": "chat.completion" })))
        .mount(&server)
        .await;

    // 2. Setup Config and State
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let dummy_config_path = create_dummy_config_path_for_test(&temp_dir);
    
    // Create a new AppConfig with top_p at the server level for this test
    let mut config = create_test_config(vec![
        KeyGroup {
            name: "openai-top-p-group".to_string(),
            api_keys: vec![test_api_key.to_string()],
            target_url: server.uri(),
            proxy_url: None,
            top_p: None, // Group level top_p is not used for this path
        }
    ], 9993);
    config.server.top_p = Some(top_p_value); // Set top_p at the server level

    let app_state = Arc::new(
        AppState::new(&config, &dummy_config_path)
            .await
            .expect("AppState failed"),
    );

    // 3. Call handler with a standard OpenAI body
    let original_body = serde_json::json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "Hello!"}]
    });
    let body_bytes = serde_json::to_vec(&original_body).unwrap();
    let response = call_proxy_handler(
        app_state,
        Method::POST,
        test_path,
        axum::body::Body::from(body_bytes),
    )
    .await;

    // 4. Assertions
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Expected status OK (200) with top_p injected for openai compat"
    );
}

#[tokio::test]
async fn test_health_detailed_maps_to_models_endpoint() {
    // Goal: Verify that /health/detailed calls the upstream /v1beta/models endpoint.
    // 1. Setup Mock Server
    let server = MockServer::start().await;
    let test_api_key = "health-check-key";
    let models_path = "/v1beta/models";
    let mock_response_body = serde_json::json!({ "data": ["model1"] });

    // Mock for the upstream models endpoint
    Mock::given(method("GET"))
        .and(path(models_path))
        .and(query_param("key", test_api_key))
        .respond_with(ResponseTemplate::new(200).set_body_json(&mock_response_body))
        .mount(&server)
        .await;

    // 2. Setup Config and State
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let dummy_config_path = create_dummy_config_path_for_test(&temp_dir);
    let test_group = KeyGroup {
        name: "health-group".to_string(),
        api_keys: vec![test_api_key.to_string()],
        target_url: server.uri(),
        proxy_url: None,
        top_p: None,
    };
    let config = create_test_config(vec![test_group], 9992);
    let app_state = Arc::new(
        AppState::new(&config, &dummy_config_path)
            .await
            .expect("AppState failed"),
    );

    // 3. Call handler for the /health/detailed path
    let response = call_proxy_handler(
        app_state,
        Method::GET,
        "/health/detailed",
        axum::body::Body::empty(),
    ).await;

    // 4. Assertions
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Expected status OK (200) for /health/detailed"
    );
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read response body");
    let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).expect("Invalid JSON response");
    assert_eq!(body_json, mock_response_body, "Response body from /health/detailed did not match expected models list");
}

#[tokio::test]
async fn test_content_length_is_updated_after_top_p_injection() {
    // Goal: Verify Content-Length is recalculated after body modification.
    // 1. Setup Mock Server
    let server = MockServer::start().await;
    let test_api_key = "content-length-key";
    let test_path = "/v1/chat/completions";
    let expected_path = "/v1beta/openai/chat/completions";
    let top_p_value = 0.88f32;

    // The original body without top_p
    let original_body = serde_json::json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "Hello!"}]
    });

    // The expected body *after* injection
    let mut expected_body_json = original_body.clone();
    expected_body_json["top_p"] = serde_json::json!(top_p_value);
    let expected_body_bytes = serde_json::to_vec(&expected_body_json).unwrap();
    let expected_content_length = expected_body_bytes.len().to_string();

    // Mock to verify the body and the Content-Length header
    Mock::given(method("POST"))
        .and(path(expected_path))
        .and(query_param("key", test_api_key))
        .and(move |req: &wiremock::Request| {
            // Check Content-Length header first
            let has_correct_content_length = req
                .headers
                .get("Content-Length")
                .map_or(false, |val| val.to_str().unwrap() == expected_content_length);

            if !has_correct_content_length {
                return false;
            }

            // Check body content
            if let Ok(body_json) = serde_json::from_slice::<serde_json::Value>(&req.body) {
                if let Some(top_p) = body_json.get("top_p") {
                    return top_p.as_f64().map_or(false, |v| (v as f32 - top_p_value).abs() < f32::EPSILON);
                }
            }
            false
        })
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": "chatcmpl-456", "object": "chat.completion" })))
        .mount(&server)
        .await;

    // 2. Setup Config and State
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let dummy_config_path = create_dummy_config_path_for_test(&temp_dir);
    
    let mut config = create_test_config(vec![
        KeyGroup {
            name: "content-length-group".to_string(),
            api_keys: vec![test_api_key.to_string()],
            target_url: server.uri(),
            proxy_url: None,
            top_p: None,
        }
    ], 9991);
    config.server.top_p = Some(top_p_value);

    let app_state = Arc::new(
        AppState::new(&config, &dummy_config_path)
            .await
            .expect("AppState failed"),
    );

    // 3. Call handler
    let body_bytes = serde_json::to_vec(&original_body).unwrap();
    let response = call_proxy_handler(
        app_state,
        Method::POST,
        test_path,
        axum::body::Body::from(body_bytes),
    )
    .await;

    // 4. Assertions
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Expected status OK (200) with correct content-length"
    );
}

#[tokio::test]
async fn test_top_p_client_precedence() {
    // 1. Setup Mock Server
    let server = MockServer::start().await;
    let test_api_key = "client-precedence-key";
    let test_path = "/v1/models:generateContent";
    let expected_path = "/v1beta/openai/models:generateContent";
    let server_top_p = 0.5; // Server-side config
    let client_top_p = 0.99; // Client-side value, should win

    // Mock to verify that the client's top_p is what arrives
    Mock::given(method("POST"))
        .and(path(expected_path))
        .and(query_param("key", test_api_key))
        .and(move |req: &wiremock::Request| {
            // Custom matcher to inspect the body
            if let Ok(body_json) = serde_json::from_slice::<serde_json::Value>(&req.body) {
                if let Some(config) = body_json.get("generationConfig") {
                    if let Some(top_p) = config.get("topP") {
                        // Check that the value is the one from the client
                        return top_p.as_f64().map_or(false, |v| (v - client_top_p as f64).abs() < f64::EPSILON);
                    }
                }
            }
            false
        })
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "candidates": [] })))
        .mount(&server)
        .await;

    // 2. Setup Config and State
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let dummy_config_path = create_dummy_config_path_for_test(&temp_dir);
    let test_group = KeyGroup {
        name: "client-precedence-group".to_string(),
        api_keys: vec![test_api_key.to_string()],
        target_url: server.uri(),
        proxy_url: None,
        top_p: Some(server_top_p), // Set a server-side value
    };
    let config = create_test_config(vec![test_group], 9994);
    let app_state = Arc::new(
        AppState::new(&config, &dummy_config_path)
            .await
            .expect("AppState failed"),
    );

    // 3. Call handler with a body that already contains topP
    let original_body = serde_json::json!({
        "contents": [{"role": "user", "parts": [{"text": "Hello"}]}],
        "generationConfig": {
            "temperature": 0.7,
            "topP": client_top_p // Client provides topP
        }
    });
    let body_bytes = serde_json::to_vec(&original_body).unwrap();
    let response = call_proxy_handler(
        app_state,
        Method::POST,
        test_path,
        axum::body::Body::from(body_bytes),
    )
    .await;

    // 4. Assertions
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Expected status OK (200) when client top_p takes precedence"
    );
}

#[tokio::test]
async fn test_url_translation_for_v1_path() {
    // Goal: Verify that a request to a `/v1/...` path is translated to `/v1beta/openai/...`
    // 1. Setup Mock Server
    let server = MockServer::start().await;
    let test_api_key = "translation-key-v1";
    let incoming_path = "/v1/chat/completions";
    let expected_translated_path = "/v1beta/openai/chat/completions";

    // Mock to expect the *translated* path
    Mock::given(method("POST"))
        .and(path(expected_translated_path))
        .and(query_param("key", test_api_key))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "status": "ok" })))
        .mount(&server)
        .await;

    // 2. Setup Config and State
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let dummy_config_path = create_dummy_config_path_for_test(&temp_dir);
    let test_group = KeyGroup {
        name: "translation-group".to_string(),
        api_keys: vec![test_api_key.to_string()],
        target_url: server.uri(),
        proxy_url: None,
        top_p: None,
    };
    let config = create_test_config(vec![test_group], 9990);
    let app_state = Arc::new(
        AppState::new(&config, &dummy_config_path)
            .await
            .expect("AppState failed"),
    );

    // 3. Call handler with the original `/v1/...` path
    let response = call_proxy_handler(
        app_state,
        Method::POST,
        incoming_path,
        axum::body::Body::empty(),
    )
    .await;

    // 4. Assertions
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Expected status OK (200) for translated v1 path"
    );
    // The mock server implicitly verifies that the path was translated correctly.
    // If the request had gone to the original path, the mock would not have matched,
    // and wiremock would have returned a 404, failing the test.
}

#[tokio::test]
async fn test_url_translation_for_non_v1_path() {
    // Goal: Verify that a request to a path NOT starting with `/v1/` is NOT translated.
    // 1. Setup Mock Server
    let server = MockServer::start().await;
    let test_api_key = "translation-key-non-v1";
    let incoming_path = "/health"; // A non-v1 path

    // Mock to expect the *original* path, unchanged
    Mock::given(method("GET"))
        .and(path(incoming_path))
        .and(query_param("key", test_api_key))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "status": "healthy" })))
        .mount(&server)
        .await;

    // 2. Setup Config and State
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let dummy_config_path = create_dummy_config_path_for_test(&temp_dir);
    let test_group = KeyGroup {
        name: "non-translation-group".to_string(),
        api_keys: vec![test_api_key.to_string()],
        target_url: server.uri(),
        proxy_url: None,
        top_p: None,
    };
    let config = create_test_config(vec![test_group], 9989);
    let app_state = Arc::new(
        AppState::new(&config, &dummy_config_path)
            .await
            .expect("AppState failed"),
    );

    // 3. Call handler with the non-v1 path
    let response = call_proxy_handler(
        app_state,
        Method::GET,
        incoming_path,
        axum::body::Body::empty(),
    )
    .await;

    // 4. Assertions
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Expected status OK (200) for non-v1 path"
    );
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains("healthy"));
}
