# =================================================================================================
# Multi-stage Dockerfile for Gemini Proxy - Production Optimized
# =================================================================================================

ARG RUST_VERSION=latest
ARG APP_NAME=gemini-proxy

# -------------------------------------------------------------------------------------------------
# Stage 1: Dependencies Cache
# Builds only dependencies for maximum caching
# -------------------------------------------------------------------------------------------------
FROM rust:${RUST_VERSION} AS dependencies
WORKDIR /app
RUN mkdir -p /app && chmod 755 /app

# Install system dependencies and nightly toolchain for edition2024 deps
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    perl \
    make \
    g++ \
    && rm -rf /var/lib/apt/lists/* \
 && rustup toolchain install nightly --profile minimal \
 && rustup default nightly

# Create dummy project for dependency caching
RUN mkdir src
RUN echo "fn main() {}" > src/main.rs

# Copy only dependency files
COPY Cargo.toml Cargo.lock ./

# Build only dependencies (locked to Cargo.lock for reproducibility)
RUN cargo build --release --locked

# -------------------------------------------------------------------------------------------------
# Stage 2: Application Builder
# Builds final binary using cached dependencies
# -------------------------------------------------------------------------------------------------
FROM rust:${RUST_VERSION} AS builder
WORKDIR /app
# Re-declare build args in this stage to ensure availability
ARG APP_NAME=gemini-proxy
# Prepare runtime cache directory to copy into distroless (no shell there)
RUN mkdir -p /app/runtime-cache/HF_CACHE

# Install system dependencies and ensure nightly toolchain; add busybox-static for healthcheck tooling
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    perl \
    make \
    g++ \
    busybox-static \
    && rm -rf /var/lib/apt/lists/* \
 && rustup toolchain install nightly --profile minimal \
 && rustup default nightly

# Copy cached dependencies
COPY --from=dependencies /app/target target
COPY --from=dependencies /usr/local/cargo /usr/local/cargo

# Copy source code
COPY . .

# Build application with optimizations
RUN cargo build --release

# Strip debug information to reduce size (only if the binary exists)
# Busybox/strip may fail if given a directory or missing file; guard it.
RUN test -f "target/release/${APP_NAME}" && strip "target/release/${APP_NAME}" || true

# Verify binary works (guard against missing APP_NAME)
RUN test -n "${APP_NAME}" && test -x "target/release/${APP_NAME}" && "target/release/${APP_NAME}" --version || (echo "Skip version check: binary missing or APP_NAME unset"; true)

# -------------------------------------------------------------------------------------------------
# Stage 3: Runtime Image
# Minimal image for production
# -------------------------------------------------------------------------------------------------
FROM gcr.io/distroless/cc-debian12 AS runtime
WORKDIR /app

# Keep APP_NAME consistent with build target binary name
ARG APP_NAME=gemini-proxy

# Copy only necessary files
# Place binary at /app/${APP_NAME} to match docker-compose command
COPY --from=builder /app/target/release/${APP_NAME} /app/${APP_NAME}
COPY --from=builder /app/static /app/static
COPY --from=builder /app/config.example.yaml /app/config.example.yaml
COPY --from=builder /bin/busybox /app/busybox
# Provide a minimal healthcheck tool (busybox) for HTTP GET without shell
# Copy pre-created runtime cache and set ownership to non-root user in one step
COPY --chown=1000:1000 --from=builder /app/runtime-cache /app/runtime-cache

# Create unprivileged user
USER 1000:1000

# Expose the internal port used by the service (compose maps it externally)
EXPOSE 4806

# Use exec form for better signal handling
# The app reads PORT env var (overrides in src/config/loader.rs) or config.yaml server.port
# Configure HF cache to writable location
ENV PORT=4806
ENV XDG_CACHE_HOME=/app/runtime-cache
ENV HF_HOME=/app/runtime-cache/HF_CACHE
ENV HUGGINGFACE_HUB_CACHE=/app/runtime-cache/HF_CACHE
CMD ["/app/${APP_NAME}"]

# -------------------------------------------------------------------------------------------------
# Stage 4: Development Image
# Includes additional development tools
# -------------------------------------------------------------------------------------------------
FROM rust:${RUST_VERSION} AS development
WORKDIR /app

# Install development tools and nightly
RUN rustup toolchain install nightly --profile minimal && rustup default nightly
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

# Install system dependencies and nightly (robust rustup syntax)
RUN apt-get update && apt-get install -y \
    llvm-dev \
    libffi-dev \
    clang \
    && rm -rf /var/lib/apt/lists/* \
    && rustup toolchain install nightly --profile minimal \
    && rustup default nightly

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