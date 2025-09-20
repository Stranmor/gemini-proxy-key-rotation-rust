#!/bin/bash

# Test script to verify token limit validation works

echo "Testing token limit validation..."

# Start the proxy in the background (assuming config.yaml has max_tokens_per_request: 125000)
echo "Starting Gemini Proxy..."
cargo run --release &
PROXY_PID=$!

# Wait for the proxy to start
sleep 5

# Test 1: Small request (should pass through to API, may fail for other reasons but not token limit)
echo "Test 1: Small request (should not be rejected for token limit)..."
curl -X POST http://localhost:4806/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "messages": [
      {
        "role": "user", 
        "content": "Hello, this is a short message."
      }
    ]
  }' \
  -w "\nHTTP Status: %{http_code}\n" \
  -s

echo -e "\n---\n"

# Test 2: Large request (should be rejected with 400)
echo "Test 2: Large request (should be rejected with 400 Bad Request)..."
LARGE_CONTENT=$(printf "This is a very long message that should exceed the token limit. %.0s" {1..1000})
curl -X POST http://localhost:4806/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d "{
    \"messages\": [
      {
        \"role\": \"user\", 
        \"content\": \"$LARGE_CONTENT\"
      }
    ]
  }" \
  -w "\nHTTP Status: %{http_code}\n" \
  -s

echo -e "\n---\n"

# Test 3: Gemini format large request (should be rejected with 400)
echo "Test 3: Gemini format large request (should be rejected with 400 Bad Request)..."
curl -X POST http://localhost:4806/v1beta/generateContent \
  -H "Content-Type: application/json" \
  -d "{
    \"contents\": [
      {
        \"parts\": [
          {
            \"text\": \"$LARGE_CONTENT\"
          }
        ]
      }
    ]
  }" \
  -w "\nHTTP Status: %{http_code}\n" \
  -s

# Clean up
echo -e "\nStopping proxy..."
kill $PROXY_PID
wait $PROXY_PID 2>/dev/null

echo "Test completed!"