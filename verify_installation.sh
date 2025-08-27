#!/bin/bash

# Verification script for Gemini Proxy installation
set -e

echo "🔍 Verifying Gemini Proxy installation..."

# Check if Rust is available
if ! command -v cargo &> /dev/null; then
    echo "❌ Rust/Cargo not found. Please install from https://rustup.rs/"
    exit 1
fi

echo "✅ Rust/Cargo found"

# Check if Docker is available (optional)
if command -v docker &> /dev/null; then
    echo "✅ Docker found"
    DOCKER_AVAILABLE=true
else
    echo "⚠️  Docker not found (optional for direct binary usage)"
    DOCKER_AVAILABLE=false
fi

# Test build
echo "🔨 Testing build..."
if make build; then
    echo "✅ Build successful"
else
    echo "❌ Build failed"
    exit 1
fi

# Test configuration setup
echo "📝 Testing configuration setup..."
if make setup-config; then
    echo "✅ Configuration setup successful"
else
    echo "❌ Configuration setup failed"
    exit 1
fi

# Test that config.yaml exists and is valid
if [ -f "config.yaml" ]; then
    echo "✅ config.yaml exists"
else
    echo "❌ config.yaml not found"
    exit 1
fi

# Run tests
echo "🧪 Running tests..."
if make test; then
    echo "✅ All tests passed"
else
    echo "❌ Some tests failed"
    exit 1
fi

# Test Docker build if Docker is available
if [ "$DOCKER_AVAILABLE" = true ]; then
    echo "🐳 Testing Docker build..."
    if make docker-build; then
        echo "✅ Docker build successful"
    else
        echo "❌ Docker build failed"
        exit 1
    fi
    
    # Test UAT
    echo "🧪 Running UAT..."
    if make uat; then
        echo "✅ UAT passed"
        make docker-stop
    else
        echo "❌ UAT failed"
        make docker-stop
        exit 1
    fi
fi

echo ""
echo "🎉 Installation verification completed successfully!"
echo ""
echo "Next steps:"
echo "1. Edit config.yaml and add your Gemini API keys"
echo "2. Run 'make run' for direct execution"
echo "3. Or run 'make docker-run' for Docker deployment"
echo ""
echo "For help: make help"