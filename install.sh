#!/bin/bash

# Gemini Proxy Key Rotation - Easy Installation Script
# This script automates the installation and setup process

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
REPO_URL="https://github.com/stranmor/gemini-proxy-key-rotation-rust.git"
INSTALL_DIR="$HOME/gemini-proxy"
SERVICE_NAME="gemini-proxy"

print_header() {
    echo -e "${BLUE}"
    echo "╔══════════════════════════════════════════════════════════════╗"
    echo "║                 Gemini Proxy Key Rotation                   ║"
    echo "║                    Easy Installation                        ║"
    echo "╚══════════════════════════════════════════════════════════════╝"
    echo -e "${NC}"
}

print_step() {
    echo -e "${GREEN}[STEP]${NC} $1"
}

print_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

check_dependencies() {
    print_step "Checking dependencies..."
    
    # Check if running on supported OS
    if [[ "$OSTYPE" != "linux-gnu"* ]] && [[ "$OSTYPE" != "darwin"* ]]; then
        print_error "Unsupported operating system: $OSTYPE"
        print_info "This installer supports Linux and macOS only"
        exit 1
    fi
    
    # Check for required tools
    local missing_deps=()
    
    if ! command -v git &> /dev/null; then
        missing_deps+=("git")
    fi
    
    if ! command -v curl &> /dev/null; then
        missing_deps+=("curl")
    fi
    
    if [ ${#missing_deps[@]} -ne 0 ]; then
        print_error "Missing required dependencies: ${missing_deps[*]}"
        print_info "Please install them and run this script again"
        exit 1
    fi
    
    print_info "✓ All dependencies found"
}

install_rust() {
    if command -v cargo &> /dev/null; then
        print_info "✓ Rust is already installed"
        return
    fi
    
    print_step "Installing Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
    print_info "✓ Rust installed successfully"
}

install_docker() {
    if command -v docker &> /dev/null; then
        print_info "✓ Docker is already installed"
        return
    fi
    
    print_step "Installing Docker..."
    if [[ "$OSTYPE" == "linux-gnu"* ]]; then
        # Linux installation
        curl -fsSL https://get.docker.com -o get-docker.sh
        sudo sh get-docker.sh
        sudo usermod -aG docker $USER
        rm get-docker.sh
        print_warning "Please log out and back in for Docker permissions to take effect"
    elif [[ "$OSTYPE" == "darwin"* ]]; then
        # macOS installation
        print_info "Please install Docker Desktop from: https://www.docker.com/products/docker-desktop"
        print_warning "After installing Docker Desktop, run this script again"
        exit 1
    fi
    
    print_info "✓ Docker installed successfully"
}

clone_repository() {
    print_step "Cloning repository..."
    
    if [ -d "$INSTALL_DIR" ]; then
        print_warning "Installation directory already exists: $INSTALL_DIR"
        read -p "Do you want to remove it and continue? (y/N): " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            rm -rf "$INSTALL_DIR"
        else
            print_info "Installation cancelled"
            exit 0
        fi
    fi
    
    git clone "$REPO_URL" "$INSTALL_DIR"
    cd "$INSTALL_DIR"
    print_info "✓ Repository cloned to $INSTALL_DIR"
}

setup_configuration() {
    print_step "Setting up configuration..."
    
    if [ ! -f "config.yaml" ]; then
        cp config.example.yaml config.yaml
        print_info "✓ Created config.yaml from example"
    else
        print_info "✓ config.yaml already exists"
    fi
    
    print_warning "IMPORTANT: You need to edit config.yaml and add your Gemini API keys!"
    print_info "Example: nano config.yaml"
    echo
}

build_application() {
    print_step "Building application..."
    
    # Build with optimizations
    cargo build --release
    
    print_info "✓ Application built successfully"
}

create_systemd_service() {
    if [[ "$OSTYPE" != "linux-gnu"* ]]; then
        print_info "Skipping systemd service creation (not on Linux)"
        return
    fi
    
    print_step "Creating systemd service..."
    
    local service_file="/etc/systemd/system/${SERVICE_NAME}.service"
    local user=$(whoami)
    
    sudo tee "$service_file" > /dev/null <<EOF
[Unit]
Description=Gemini Proxy Key Rotation Service
After=network.target
Wants=network.target

[Service]
Type=simple
User=$user
WorkingDirectory=$INSTALL_DIR
ExecStart=$INSTALL_DIR/target/release/gemini-proxy-key-rotation-rust
Restart=always
RestartSec=5
Environment=RUST_LOG=info

# Security settings
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=$INSTALL_DIR

[Install]
WantedBy=multi-user.target
EOF
    
    sudo systemctl daemon-reload
    sudo systemctl enable "$SERVICE_NAME"
    
    print_info "✓ Systemd service created and enabled"
    print_info "  Start: sudo systemctl start $SERVICE_NAME"
    print_info "  Status: sudo systemctl status $SERVICE_NAME"
    print_info "  Logs: sudo journalctl -u $SERVICE_NAME -f"
}

create_docker_compose() {
    print_step "Creating Docker Compose configuration..."
    
    cat > docker-compose.yml <<EOF
version: '3.8'

services:
  gemini-proxy:
    build: .
    container_name: gemini-proxy
    ports:
      - "\${PROXY_PORT:-8081}:8081"
    volumes:
      - ./config.yaml:/app/config.yaml:ro
      - ./logs:/app/logs
    environment:
      - RUST_LOG=info
    restart: unless-stopped
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8081/health"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 40s

  redis:
    image: redis:7-alpine
    container_name: gemini-proxy-redis
    ports:
      - "6379:6379"
    volumes:
      - redis_data:/data
    restart: unless-stopped
    command: redis-server --appendonly yes

volumes:
  redis_data:
EOF
    
    # Create .env file
    cat > .env <<EOF
# Gemini Proxy Configuration
PROXY_PORT=8081
RUST_LOG=info
EOF
    
    print_info "✓ Docker Compose configuration created"
    print_info "  Start: docker-compose up -d"
    print_info "  Logs: docker-compose logs -f"
    print_info "  Stop: docker-compose down"
}

run_tests() {
    print_step "Running tests to verify installation..."
    
    # Run critical tests
    if cargo test --test security_tests --test monitoring_tests --test error_handling_tests --quiet; then
        print_info "✓ All critical tests passed"
    else
        print_warning "Some tests failed, but installation can continue"
    fi
}

print_completion() {
    print_step "Installation completed!"
    echo
    echo -e "${GREEN}╔══════════════════════════════════════════════════════════════╗"
    echo -e "║                     NEXT STEPS                               ║"
    echo -e "╚══════════════════════════════════════════════════════════════╝${NC}"
    echo
    echo -e "${YELLOW}1. Configure your API keys:${NC}"
    echo "   cd $INSTALL_DIR"
    echo "   nano config.yaml  # Add your Gemini API keys"
    echo
    echo -e "${YELLOW}2. Choose your deployment method:${NC}"
    echo
    echo -e "${BLUE}   Option A - Docker (Recommended):${NC}"
    echo "   docker-compose up -d"
    echo
    echo -e "${BLUE}   Option B - Direct binary:${NC}"
    echo "   ./target/release/gemini-proxy-key-rotation-rust"
    echo
    if [[ "$OSTYPE" == "linux-gnu"* ]]; then
        echo -e "${BLUE}   Option C - Systemd service:${NC}"
        echo "   sudo systemctl start $SERVICE_NAME"
        echo
    fi
    echo -e "${YELLOW}3. Verify installation:${NC}"
    echo "   curl http://localhost:8081/health"
    echo
    echo -e "${YELLOW}4. View documentation:${NC}"
    echo "   cat README.md"
    echo "   cat SECURITY.md"
    echo
    echo -e "${GREEN}🎉 Happy proxying!${NC}"
}

# Main installation flow
main() {
    print_header
    
    # Check if user wants to proceed
    echo "This script will install Gemini Proxy Key Rotation on your system."
    echo "It will install Rust, Docker (if needed), and set up the application."
    echo
    read -p "Do you want to continue? (y/N): " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        print_info "Installation cancelled"
        exit 0
    fi
    
    check_dependencies
    install_rust
    install_docker
    clone_repository
    setup_configuration
    build_application
    create_systemd_service
    create_docker_compose
    run_tests
    print_completion
}

# Run main function
main "$@"