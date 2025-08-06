// tests/integration_tests.rs

use axum::body::to_bytes;
use axum::{
    // body::Bytes as _, // Removed unused import placeholder
    extract::{Request, State},
    http::{/* header, */ Method, StatusCode, Uri}, // Removed unused header
    response::Response,
};
use crate::key_manager::KeyManagerTrait;
use crate::{
    config::{AppConfig, KeyGroup, ServerConfig},
    handlers, // Import the handler module
    // key_manager::FlattenedKeyInfo, // Removed unused import
    // proxy,
    state::AppState,
};
use std::{
    fs::File,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tempfile::tempdir;
use wiremock::{
    matchers::{method, path, query_param}, // Use path and query_param
    Mock,
    MockServer,
    ResponseTemplate,
};

// Use a unique Redis DB for each test to ensure isolation when running in parallel.
// We start from DB #2, as #0 is default and #1 was used before.
static TEST_DB_COUNTER: AtomicUsize = AtomicUsize::new(2);

// Helper function to create a basic AppConfig for testing
fn create_test_config(groups: Vec<KeyGroup>, server_port: u16, _db_num: usize) -> AppConfig {
    AppConfig {
        server: ServerConfig {
            port: server_port,
            top_p: None,
            admin_token: Some("test_token".to_string()),
            test_mode: true,
            connect_timeout_secs: 10,
            request_timeout_secs: 60,
        },
        groups,
        redis_url: None, // Disable Redis for tests
        redis_key_prefix: None,
        internal_retries: Some(3),
        temporary_block_minutes: Some(1),
        top_p: None,
        max_failures_threshold: Some(10),
        rate_limit: None,
        circuit_breaker: None,
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
    let uri: Uri = format!("http://test-proxy.com{path}") // Base URL doesn't matter here
        .parse()
        .expect("Failed to parse test URI for handler");
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .body(body) // Use empty body for GET/POST tests for simplicity
        .unwrap();

    // Call the actual handler function
    handlers::proxy_handler(State(state), request)
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
    let db_num = TEST_DB_COUNTER.fetch_add(1, Ordering::SeqCst);
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let dummy_config_path = create_dummy_config_path_for_test(&temp_dir);
    let test_group = KeyGroup {
        name: "test-group".to_string(),
        api_keys: vec![test_api_key.to_string()],
        model_aliases: vec![],
        target_url: server.uri(),
        proxy_url: None,
        top_p: None,
    };
    let config = create_test_config(vec![test_group], 9999, db_num);
    let (app_state_instance, _) = AppState::new(&config, &dummy_config_path)
        .await
        .expect("AppState failed");
    let app_state: Arc<AppState> = Arc::new(app_state_instance);

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
    let db_num = TEST_DB_COUNTER.fetch_add(1, Ordering::SeqCst);
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let dummy_config_path = create_dummy_config_path_for_test(&temp_dir);
    // Key order matters for the round-robin
    let test_group = KeyGroup {
        name: "retry-group".to_string(),
        api_keys: vec![key1.to_string(), key2.to_string()], // key1 will be tried first
        model_aliases: vec![],
        target_url: server.uri(),
        proxy_url: None,
        top_p: None,
    };
    let config = create_test_config(vec![test_group], 9998, db_num); // Different port just in case
    let (app_state_instance, _) = AppState::new(&config, &dummy_config_path)
        .await
        .expect("AppState failed");
    let app_state = Arc::new(app_state_instance);

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
    let db_num = TEST_DB_COUNTER.fetch_add(1, Ordering::SeqCst);
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let dummy_config_path = create_dummy_config_path_for_test(&temp_dir);
    let test_group = KeyGroup {
        name: "exhaust-group".to_string(),
        api_keys: vec![key1.to_string(), key2.to_string()],
        model_aliases: vec![],
        target_url: server.uri(),
        proxy_url: None,
        top_p: None,
    };
    let config = create_test_config(vec![test_group], 9997, db_num);
    let (app_state_instance, _) = AppState::new(&config, &dummy_config_path)
        .await
        .expect("AppState failed");
    let app_state = Arc::new(app_state_instance);

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
    // Check it returned a body from one of the 429 responses
    assert!(
        body_str == "Rate limit 1" || body_str == "Rate limit 2",
        "Expected body from one of the 429 responses, got: {body_str}"
    );
}

#[tokio::test]
async fn test_handler_group_round_robin() {
    // 1. Setup Mock Server
    let server = MockServer::start().await;
    let test_path = "/v1/chat/completions"; // Use a path that expects a body
    let expected_path = "/v1beta/openai/chat/completions";

    let g1_key1 = "g1-key-1";
    let g1_key2 = "g1-key-2";
    let g2_key1 = "g2-key-1"; // Other groups, should not be used

    // Mock successful responses for the keys in the target group
    for key in [g1_key1, g1_key2] {
        Mock::given(method("POST")) // Expect POST now
            .and(path(expected_path))
            .and(query_param("key", key))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(format!("{{\"key_used\": \"{key}\"}}")),
            )
            .mount(&server)
            .await;
    }

    // 2. Setup Config and State
    let db_num = TEST_DB_COUNTER.fetch_add(1, Ordering::SeqCst);
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let dummy_config_path = create_dummy_config_path_for_test(&temp_dir);
    let groups = vec![
        KeyGroup {
            name: "group1".to_string(),
            api_keys: vec![g1_key1.to_string(), g1_key2.to_string()],
            model_aliases: vec!["test-model".to_string()], // This is the key for routing
            target_url: server.uri(),
            proxy_url: None,
            top_p: None,
        },
        KeyGroup {
            name: "group2".to_string(),
            api_keys: vec![g2_key1.to_string()],
            model_aliases: vec!["other-model".to_string()],
            target_url: server.uri(),
            proxy_url: None,
            top_p: None,
        },
    ];
    let config = create_test_config(groups, 9996, db_num);
    let (app_state_instance, _) = AppState::new(&config, &dummy_config_path)
        .await
        .expect("AppState failed");
    let app_state = Arc::new(app_state_instance);

    // Helper to extract key from response body
    async fn get_key_from_response(response: Response) -> String {
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("Failed to read response body");
        let body_json: serde_json::Value =
            serde_json::from_slice(&body_bytes).expect("Invalid JSON");
        body_json["key_used"].as_str().unwrap().to_string()
    }

    // Create a request body that specifies the model for routing
    let request_body = serde_json::json!({
        "model": "test-model",
        "messages": [{"role": "user", "content": "Hello"}]
    });
    let body_bytes = serde_json::to_vec(&request_body).unwrap();

    // 3. Call handler multiple times and check key rotation WITHIN the group
    // Expected sequence for group1: g1k1, g1k2, g1k1, ...

    let res1 = call_proxy_handler(
        Arc::clone(&app_state),
        Method::POST,
        test_path,
        axum::body::Body::from(body_bytes.clone()),
    )
    .await;
    assert_eq!(res1.status(), StatusCode::OK);
    assert_eq!(get_key_from_response(res1).await, g1_key1);

    let res2 = call_proxy_handler(
        Arc::clone(&app_state),
        Method::POST,
        test_path,
        axum::body::Body::from(body_bytes.clone()),
    )
    .await;
    assert_eq!(res2.status(), StatusCode::OK);
    assert_eq!(get_key_from_response(res2).await, g1_key2); // Next key in the SAME group

    let res3 = call_proxy_handler(
        Arc::clone(&app_state),
        Method::POST,
        test_path,
        axum::body::Body::from(body_bytes.clone()),
    )
    .await;
    assert_eq!(res3.status(), StatusCode::OK);
    assert_eq!(get_key_from_response(res3).await, g1_key1); // Rotated back to the start of the group
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
                    return top_p
                        .as_f64()
                        .is_some_and(|v| (v as f32 - top_p_value).abs() < f32::EPSILON);
                }
            }
            false
        })
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "id": "chatcmpl-123", "object": "chat.completion" }),
        ))
        .mount(&server)
        .await;

    // 2. Setup Config and State
    let db_num = TEST_DB_COUNTER.fetch_add(1, Ordering::SeqCst);
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let dummy_config_path = create_dummy_config_path_for_test(&temp_dir);

    // Create a new AppConfig with top_p at the server level for this test
    let mut config = create_test_config(
        vec![KeyGroup {
            name: "openai-top-p-group".to_string(),
            api_keys: vec![test_api_key.to_string()],
            model_aliases: vec![],
            target_url: server.uri(),
            proxy_url: None,
            top_p: None, // Group level top_p is not used for this path
        }],
        9993,
        db_num,
    );
    config.top_p = Some(top_p_value);

    let (app_state_instance, _) = AppState::new(&config, &dummy_config_path)
        .await
        .expect("AppState failed");
    let app_state = Arc::new(app_state_instance);

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
    let db_num = TEST_DB_COUNTER.fetch_add(1, Ordering::SeqCst);
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let dummy_config_path = create_dummy_config_path_for_test(&temp_dir);
    let test_group = KeyGroup {
        name: "health-group".to_string(),
        api_keys: vec![test_api_key.to_string()],
        model_aliases: vec![],
        target_url: server.uri(),
        proxy_url: None,
        top_p: None,
    };
    let config = create_test_config(vec![test_group], 9992, db_num);
    let (app_state_instance, _) = AppState::new(&config, &dummy_config_path)
        .await
        .expect("AppState failed");
    let app_state = Arc::new(app_state_instance);

    // 3. Call handler for the /health/detailed path
    let response = call_proxy_handler(
        app_state,
        Method::GET,
        "/health/detailed",
        axum::body::Body::empty(),
    )
    .await;

    // 4. Assertions
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Expected status OK (200) for /health/detailed"
    );
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read response body");
    let body_json: serde_json::Value =
        serde_json::from_slice(&body_bytes).expect("Invalid JSON response");
    assert_eq!(
        body_json, mock_response_body,
        "Response body from /health/detailed did not match expected models list"
    );
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
                .is_some_and(|val| val.to_str().unwrap() == expected_content_length);

            if !has_correct_content_length {
                return false;
            }

            // Check body content
            if let Ok(body_json) = serde_json::from_slice::<serde_json::Value>(&req.body) {
                if let Some(top_p) = body_json.get("top_p") {
                    return top_p
                        .as_f64()
                        .is_some_and(|v| (v as f32 - top_p_value).abs() < f32::EPSILON);
                }
            }
            false
        })
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "id": "chatcmpl-456", "object": "chat.completion" }),
        ))
        .mount(&server)
        .await;

    // 2. Setup Config and State
    let db_num = TEST_DB_COUNTER.fetch_add(1, Ordering::SeqCst);
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let dummy_config_path = create_dummy_config_path_for_test(&temp_dir);

    let mut config = create_test_config(
        vec![KeyGroup {
            name: "content-length-group".to_string(),
            api_keys: vec![test_api_key.to_string()],
            model_aliases: vec![],
            target_url: server.uri(),
            proxy_url: None,
            top_p: None,
        }],
        9991,
        db_num,
    );
    config.top_p = Some(top_p_value);

    let (app_state_instance, _) = AppState::new(&config, &dummy_config_path)
        .await
        .expect("AppState failed");
    let app_state = Arc::new(app_state_instance);

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
                        return top_p
                            .as_f64()
                            .is_some_and(|v| (v - client_top_p).abs() < f64::EPSILON);
                    }
                }
            }
            false
        })
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "candidates": [] })),
        )
        .mount(&server)
        .await;

    // 2. Setup Config and State
    let db_num = TEST_DB_COUNTER.fetch_add(1, Ordering::SeqCst);
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let dummy_config_path = create_dummy_config_path_for_test(&temp_dir);
    let test_group = KeyGroup {
        name: "client-precedence-group".to_string(),
        api_keys: vec![test_api_key.to_string()],
        model_aliases: vec![],
        target_url: server.uri(),
        proxy_url: None,
        top_p: Some(server_top_p), // Set a server-side value
    };
    let config = create_test_config(vec![test_group], 9994, db_num);
    let (app_state_instance, _) = AppState::new(&config, &dummy_config_path)
        .await
        .expect("AppState failed");
    let app_state = Arc::new(app_state_instance);

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
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "status": "ok" })),
        )
        .mount(&server)
        .await;

    // 2. Setup Config and State
    let db_num = TEST_DB_COUNTER.fetch_add(1, Ordering::SeqCst);
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let dummy_config_path = create_dummy_config_path_for_test(&temp_dir);
    let test_group = KeyGroup {
        name: "translation-group".to_string(),
        api_keys: vec![test_api_key.to_string()],
        model_aliases: vec![],
        target_url: server.uri(),
        proxy_url: None,
        top_p: None,
    };
    let config = create_test_config(vec![test_group], 9990, db_num);
    let (app_state_instance, _) = AppState::new(&config, &dummy_config_path)
        .await
        .expect("AppState failed");
    let app_state = Arc::new(app_state_instance);

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
async fn test_proxy_handler_returns_502_on_internal_error_from_try_request_with_key() {
    // Configure an invalid target_url to force URL parse/join error inside try_request_with_key,
    // which should be treated as non-UpstreamServiceError and mapped to 502 by proxy_handler.
    let test_api_key = "any-key";
    let invalid_target = "http:// invalid url"; // space makes it invalid

    let db_num = TEST_DB_COUNTER.fetch_add(1, Ordering::SeqCst);
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let dummy_config_path = create_dummy_config_path_for_test(&temp_dir);

    let test_group = KeyGroup {
        name: "invalid-target-group".to_string(),
        api_keys: vec![test_api_key.to_string()],
        model_aliases: vec![],
        target_url: invalid_target.to_string(),
        proxy_url: None,
        top_p: None,
    };
    let config = create_test_config(vec![test_group], 9985, db_num);
    let (app_state_instance, _) = AppState::new(&config, &dummy_config_path)
        .await
        .expect("AppState failed");
    let app_state = Arc::new(app_state_instance);

    let response = call_proxy_handler(
        app_state,
        Method::GET,
        "/v1/models",
        axum::body::Body::empty(),
    )
    .await;

    assert_eq!(
        response.status(),
        StatusCode::BAD_GATEWAY,
        "Non-upstream service errors must map to 502"
    );
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
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "status": "healthy" })),
        )
        .mount(&server)
        .await;

    // 2. Setup Config and State
    let db_num = TEST_DB_COUNTER.fetch_add(1, Ordering::SeqCst);
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let dummy_config_path = create_dummy_config_path_for_test(&temp_dir);
    let test_group = KeyGroup {
        name: "non-translation-group".to_string(),
        api_keys: vec![test_api_key.to_string()],
        model_aliases: vec![],
        target_url: server.uri(),
        proxy_url: None,
        top_p: None,
    };
    let config = create_test_config(vec![test_group], 9989, db_num);
    let (app_state_instance, _) = AppState::new(&config, &dummy_config_path)
        .await
        .expect("AppState failed");
    let app_state = Arc::new(app_state_instance);

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

#[tokio::test]
async fn test_rotates_on_400_with_api_key_invalid_body() {
    // Verifies that if a key returns 400 with "API_KEY_INVALID",
    // the handler marks it as invalid and retries with the next key.

    // 1. Setup Mock Server
    let server = MockServer::start().await;
    let key1_invalid = "key-invalid-400-special";
    let key2_valid = "key-valid-after-400";
    let test_path = "/v1/chat/completions";
    let expected_path = "/v1beta/openai/chat/completions";

    // Mock for the first key (key1_invalid) - returns 400 with specific body
    Mock::given(method("POST"))
        .and(path(expected_path))
        .and(query_param("key", key1_invalid))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_string("API key not valid. Please pass a valid API key. API_KEY_INVALID")
                .insert_header("content-type", "text/plain"),
        )
        .mount(&server)
        .await;

    // Mock for the second key (key2_valid) - returns 200
    Mock::given(method("POST"))
        .and(path(expected_path))
        .and(query_param("key", key2_valid))
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"candidates\": []}"))
        .mount(&server)
        .await;

    // 2. Setup Config and State
    let db_num = TEST_DB_COUNTER.fetch_add(1, Ordering::SeqCst);
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let dummy_config_path = create_dummy_config_path_for_test(&temp_dir);
    let test_group = KeyGroup {
        name: "retry-400-invalid-group".to_string(),
        api_keys: vec![key1_invalid.to_string(), key2_valid.to_string()],
        model_aliases: vec![],
        target_url: server.uri(),
        proxy_url: None,
        top_p: None,
    };
    let config = create_test_config(vec![test_group], 9988, db_num);
    let (app_state_instance, _) = AppState::new(&config, &dummy_config_path)
        .await
        .expect("AppState failed");
    let app_state = Arc::new(app_state_instance);

    // 3. Call handler
    let response = call_proxy_handler(
        app_state.clone(),
        Method::POST,
        test_path,
        axum::body::Body::empty(),
    )
    .await;

    // 4. Assertions
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Expected status OK (200) after retry on 400 with API_KEY_INVALID"
    );
    // Verify that the first key is now marked as invalid
    let key_states = app_state
        .key_manager
        .read()
        .await
        .get_key_states()
        .await
        .unwrap();
    let key1_state = key_states.get(key1_invalid).unwrap();
    assert!(
        key1_state.is_blocked,
        "Expected the first key to be marked as blocked"
    );
}
#[tokio::test]
async fn test_returns_immediately_on_400_with_other_body() {
    // Verifies that if a key returns 400 without "API_KEY_INVALID",
    // the handler immediately returns the 400 response without retrying.

    // 1. Setup Mock Server
    let server = MockServer::start().await;
    let key1 = "key-400-other-error";
    let key2 = "key-should-not-be-used";
    let test_path = "/v1/chat/completions";
    let expected_path = "/v1beta/openai/chat/completions";
    let error_body = "Some other bad request error";

    // Mock for the first key - returns 400 with a generic body
    Mock::given(method("POST"))
        .and(path(expected_path))
        .and(query_param("key", key1))
        .respond_with(ResponseTemplate::new(400).set_body_string(error_body))
        .mount(&server)
        .await;

    // A mock for the second key that should NEVER be called.
    // If it is called, the test will fail because wiremock will report an unhandled request.
    Mock::given(method("POST"))
        .and(path(expected_path))
        .and(query_param("key", key2))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    // 2. Setup Config and State
    let db_num = TEST_DB_COUNTER.fetch_add(1, Ordering::SeqCst);
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let dummy_config_path = create_dummy_config_path_for_test(&temp_dir);
    let test_group = KeyGroup {
        name: "no-retry-400-group".to_string(),
        api_keys: vec![key1.to_string(), key2.to_string()],
        model_aliases: vec![],
        target_url: server.uri(),
        proxy_url: None,
        top_p: None,
    };
    let config = create_test_config(vec![test_group], 9987, db_num);
    let (app_state_instance, _) = AppState::new(&config, &dummy_config_path)
        .await
        .expect("AppState failed");
    let app_state = Arc::new(app_state_instance);

    // 3. Call handler
    let response = call_proxy_handler(
        app_state.clone(),
        Method::POST,
        test_path,
        axum::body::Body::empty(),
    )
    .await;

    // 4. Assertions
    // In the new architecture, if all keys fail, the response of the *last* key is returned.
    // The first key gets a 400, which is now a terminal error. So the handler stops
    // and returns that 400 immediately. The second key is never tried.
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "Expected status 400 to be returned directly"
    );
    let body_bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        String::from_utf8_lossy(&body_bytes),
        error_body,
        "Expected the original error body to be returned"
    );

    // Verify that the first key was NOT marked as invalid
    let key_states = app_state
        .key_manager
        .read()
        .await
        .get_key_states()
        .await
        .unwrap();
    let key1_state = key_states.get(key1).unwrap();
    assert!(
        !key1_state.is_blocked,
        "Expected the key NOT to be marked as blocked for a generic 400 error"
    );
}
