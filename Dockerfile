# Stage 1: Build the application
# Use the official Rust image with Alpine Linux for a smaller builder image
FROM rust:alpine AS builder

# Install dependencies:
# - musl-dev: for static linking against musl
# - build-base, perl, linux-headers, make: required to build OpenSSL from source (vendored feature)
RUN apk add --no-cache musl-dev build-base perl linux-headers make

# Set the working directory
WORKDIR /app

# Copy the Cargo configuration files
COPY Cargo.toml Cargo.lock ./

# Copy the source code
COPY src ./src

# Build the application in release mode targeting musl for a static binary
# Enable the "vendored" feature for openssl via the reqwest crate's feature flag.
# Vendoring implies static linking for musl targets, so OPENSSL_STATIC is removed.
RUN cargo build --release --target x86_64-unknown-linux-musl --features reqwest/native-tls-vendored \
    && strip /app/target/x86_64-unknown-linux-musl/release/gemini-proxy-key-rotation-rust

# Stage 2: Create the final minimal image
FROM alpine:latest

# Install ca-certificates needed for making HTTPS requests at runtime
RUN apk --no-cache add ca-certificates

# Set the working directory
WORKDIR /app

# Copy the static binary from the builder stage
# If the build fails, this step will error out indicating the binary wasn't created.
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/gemini-proxy-key-rotation-rust ./gemini-proxy-key-rotation-rust

# Copy the example configuration file - this will be overwritten by the volume mount
# but having it ensures the container can technically start without a mount (though non-functional)
# COPY config.example.yaml /app/config.yaml # Removed: Configuration primarily via env vars and mounts

# Expose the port the application listens on (defaulting to 8080, adjust if needed)
# Make sure this matches the port in your config.yaml
EXPOSE 8080

# Define the entry point for the container
# It runs the binary. The configuration path is expected at /app/config.yaml by default.
ENTRYPOINT ["/app/gemini-proxy-key-rotation-rust"]

# No default CMD needed as ENTRYPOINT is sufficient and config is mounted.