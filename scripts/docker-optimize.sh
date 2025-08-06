#!/bin/bash
# Docker build optimization script for Gemini Proxy

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Logging functions
log() {
    echo -e "${BLUE}[$(date +'%Y-%m-%d %H:%M:%S')]${NC} $1"
}

success() {
    echo -e "${GREEN}✅ $1${NC}"
}

warning() {
    echo -e "${YELLOW}⚠️  $1${NC}"
}

error() {
    echo -e "${RED}❌ $1${NC}"
}

# Check dependencies
check_dependencies() {
    log "Checking dependencies..."
    
    if ! command -v docker &> /dev/null; then
        error "Docker is not installed"
        exit 1
    fi
    
    if ! command -v docker-compose &> /dev/null; then
        error "Docker Compose is not installed"
        exit 1
    fi
    
    success "All dependencies are installed"
}

# Clean up old images
cleanup_old_images() {
    log "Cleaning up old images..."
    
    # Remove unused images
    docker image prune -f
    
    # Remove old versions of our image
    docker images | grep "gemini-proxy" | grep -v "latest" | awk '{print $3}' | xargs -r docker rmi -f
    
    success "Old images cleaned up"
}

# Build with caching
build_with_cache() {
    log "Building image with optimized caching..."
    
    # Enable BuildKit for better caching
    export DOCKER_BUILDKIT=1
    export COMPOSE_DOCKER_CLI_BUILD=1
    
    # Build with cache
    docker build \
        --target runtime \
        --cache-from gemini-proxy:latest \
        --tag gemini-proxy:latest \
        --tag gemini-proxy:$(date +%Y%m%d-%H%M%S) \
        .
    
    success "Image built successfully"
}

# Analyze image size
analyze_image_size() {
    log "Analyzing image size..."
    
    echo "Image layer sizes:"
    docker history gemini-proxy:latest --format "table {{.CreatedBy}}\t{{.Size}}"
    
    echo ""
    echo "Total image size:"
    docker images gemini-proxy:latest --format "table {{.Repository}}\t{{.Tag}}\t{{.Size}}"
}

# Security scan of image
security_scan() {
    log "Running security scan of image..."
    
    if command -v trivy &> /dev/null; then
        trivy image gemini-proxy:latest
    else
        warning "Trivy is not installed, skipping security scan"
        echo "Install Trivy for scanning: https://aquasecurity.github.io/trivy/"
    fi
}

# Build performance test
benchmark_build() {
    log "Running build performance test..."
    
    # First build (cold cache)
    docker build --no-cache --target runtime -t gemini-proxy:benchmark-cold . > /dev/null 2>&1
    
    # Second build (warm cache)
    start_time=$(date +%s)
    docker build --target runtime -t gemini-proxy:benchmark-warm . > /dev/null 2>&1
    end_time=$(date +%s)
    
    build_time=$((end_time - start_time))
    success "Build time with cache: ${build_time} seconds"
    
    # Clean up test images
    docker rmi gemini-proxy:benchmark-cold gemini-proxy:benchmark-warm > /dev/null 2>&1
}

# Optimize Docker system
optimize_docker_system() {
    log "Optimizing Docker system..."
    
    # Configure BuildKit
    if ! grep -q "DOCKER_BUILDKIT=1" ~/.bashrc; then
        echo "export DOCKER_BUILDKIT=1" >> ~/.bashrc
        echo "export COMPOSE_DOCKER_CLI_BUILD=1" >> ~/.bashrc
        success "BuildKit enabled in ~/.bashrc"
    fi
    
    # Clean up system
    docker system prune -f
    
    success "Docker system optimized"
}

# Create optimized .env file
create_optimized_env() {
    log "Creating optimized .env file..."
    
    cat > .env.optimized << EOF
# Optimized Docker settings
COMPOSE_DOCKER_CLI_BUILD=1
DOCKER_BUILDKIT=1

# Ports
PROXY_PORT=8080
REDIS_PORT=6379
REDIS_UI_PORT=8082
DEV_PORT=8081

# Logging
RUST_LOG=info
RUST_BACKTRACE=0

# Redis UI
REDIS_UI_USER=admin
REDIS_UI_PASSWORD=secure_password_here

# Resources
COMPOSE_HTTP_TIMEOUT=120
EOF
    
    if [ ! -f .env ]; then
        cp .env.optimized .env
        success "Created optimized .env file"
    else
        success "Created .env.optimized file (existing .env unchanged)"
    fi
}

# Main function
main() {
    echo "🚀 Docker Build Optimization for Gemini Proxy"
    echo "=============================================="
    
    check_dependencies
    cleanup_old_images
    create_optimized_env
    optimize_docker_system
    build_with_cache
    analyze_image_size
    benchmark_build
    security_scan
    
    echo ""
    success "Optimization completed!"
    echo ""
    echo "Next steps:"
    echo "1. Configure config.yaml with your API keys"
    echo "2. Run: make docker-run"
    echo "3. For development: make docker-run-dev"
    echo "4. For monitoring: make docker-run-with-tools"
}

# Обработка аргументов командной строки
case "${1:-main}" in
    "cleanup")
        cleanup_old_images
        ;;
    "build")
        build_with_cache
        ;;
    "analyze")
        analyze_image_size
        ;;
    "security")
        security_scan
        ;;
    "benchmark")
        benchmark_build
        ;;
    "optimize")
        optimize_docker_system
        ;;
    "main"|*)
        main
        ;;
esac