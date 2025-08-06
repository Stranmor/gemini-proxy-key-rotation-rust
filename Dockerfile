# =================================================================================================
# Multi-stage Dockerfile for Gemini Proxy - Production Optimized
# =================================================================================================

ARG RUST_VERSION=1.75-slim
ARG APP_NAME=gemini-proxy

# -------------------------------------------------------------------------------------------------
# Stage 1: Dependencies Cache
# Builds only dependencies for maximum caching
# -------------------------------------------------------------------------------------------------
FROM rust:${RUST_VERSION} AS dependencies
WORKDIR /app

# Install system dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    perl \
    make \
    g++ \
    && rm -rf /var/lib/apt/lists/*

# Create dummy project for dependency caching
RUN mkdir src
RUN echo "fn main() {}" > src/main.rs

# Copy only dependency files
COPY Cargo.toml Cargo.lock ./

# Build only dependencies
RUN cargo build --release

# -------------------------------------------------------------------------------------------------
# Stage 2: Application Builder
# Builds final binary using cached dependencies
# -------------------------------------------------------------------------------------------------
FROM rust:${RUST_VERSION} AS builder
WORKDIR /app

# Install system dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    perl \
    make \
    g++ \
    && rm -rf /var/lib/apt/lists/*

# Copy cached dependencies
COPY --from=dependencies /app/target target
COPY --from=dependencies /usr/local/cargo /usr/local/cargo

# Copy source code
COPY . .

# Build application with optimizations
RUN cargo build --release

# Strip debug information to reduce size
RUN strip target/release/${APP_NAME}

# Verify binary works
RUN ./target/release/${APP_NAME} --version

# -------------------------------------------------------------------------------------------------
# Stage 3: Runtime Image
# Minimal image for production
# -------------------------------------------------------------------------------------------------
FROM gcr.io/distroless/cc-debian12 AS runtime
WORKDIR /app

ARG APP_NAME=gemini-proxy-key-rotation-rust

# Copy only necessary files
COPY --from=builder /app/target/release/${APP_NAME} ./app
COPY --from=builder /app/static ./static
COPY --from=builder /app/config.example.yaml ./config.example.yaml

# Create unprivileged user
USER 1000:1000

EXPOSE 8080

# Use exec form for better signal handling
CMD ["./app"]

# -------------------------------------------------------------------------------------------------
# Stage 4: Development Image
# Includes additional development tools
# -------------------------------------------------------------------------------------------------
FROM rust:${RUST_VERSION} AS development
WORKDIR /app

# Install development tools
RUN rustup component add clippy rustfmt llvm-tools-preview
RUN cargo install cargo-watch cargo-audit cargo-tarpaulin --locked

# Copy source code
COPY . .

# Install system dependencies for development
RUN apt-get update && apt-get install -y \
    curl \
    jq \
    && rm -rf /var/lib/apt/lists/*

EXPOSE 8080

CMD ["cargo", "run"]

# -------------------------------------------------------------------------------------------------
# Stage 5: Testing
# Optimized image for running tests
# -------------------------------------------------------------------------------------------------
FROM builder AS testing
WORKDIR /app

# Install testing tools
RUN apt-get update && apt-get install -y \
    curl \
    && rm -rf /var/lib/apt/lists/*

CMD ["cargo", "test", "--release", "--all-features"]

# -------------------------------------------------------------------------------------------------
# Stage 6: Coverage Analysis
# Specialized image for code coverage analysis
# -------------------------------------------------------------------------------------------------
FROM rust:${RUST_VERSION} AS coverage
WORKDIR /app

# Install system dependencies
RUN apt-get update && apt-get install -y \
    llvm-dev \
    libffi-dev \
    clang \
    && rm -rf /var/lib/apt/lists/*

# Install Rust components
RUN rustup component add llvm-tools-preview
RUN cargo install cargo-tarpaulin --locked

# Copy cached dependencies and source code
COPY --from=dependencies /app/target target
COPY --from=dependencies /usr/local/cargo /usr/local/cargo
COPY . .

RUN mkdir -p coverage_report

CMD ["cargo", "tarpaulin", \
     "--verbose", \
     "--all-features", \
     "--engine", "Llvm", \
     "--out", "Lcov", \
     "--out", "Html", \
     "--output-dir", "coverage_report", \
     "--skip-clean"]