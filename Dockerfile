# Use an ARG to define the application name for easier maintenance.
# Replace 'gemini-proxy-key-rotation-rust' with your actual crate name if different.
ARG APP_NAME=gemini-proxy-key-rotation-rust

# Stage 1: Build the application
# Use a specific version of the rust:alpine image for reproducible builds.
FROM rust:1.76-alpine AS builder

# ARG must be redefined in each stage where it's used.
ARG APP_NAME

# Install build dependencies for static linking (musl) and vendored OpenSSL.
RUN apk add --no-cache musl-dev build-base perl linux-headers make pkgconfig openssl-dev

# Create a workspace to cache dependencies, leveraging Docker's layer caching.
# This layer is rebuilt only when Cargo.toml or Cargo.lock changes.
WORKDIR /app
RUN cargo new --bin app_cache
WORKDIR /app/app_cache
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release --target x86_64-unknown-linux-musl --features reqwest/native-tls-vendored

# Copy the actual application source and build it.
# This build will be fast as dependencies are already cached.
COPY src ./src
# The `rm` ensures we do a clean build of the actual application code.
RUN rm -f target/x86_64-unknown-linux-musl/release/deps/app_cache* && \
    cargo build --release --target x86_64-unknown-linux-musl --features reqwest/native-tls-vendored && \
    strip target/x86_64-unknown-linux-musl/release/${APP_NAME}

# Stage 2: Create the final minimal image
# Use a specific version of Alpine for reproducible builds.
FROM alpine:3.19

# ARG must be redefined in the final stage.
ARG APP_NAME

# Install ca-certificates, required for making HTTPS requests at runtime.
RUN apk --no-cache add ca-certificates

# Create a non-privileged user and group for security.
# Using -S creates a system user without a password or home directory.
RUN addgroup -S appgroup && adduser -S appuser -G appgroup

# Set the working directory and switch to the non-privileged user.
WORKDIR /app
USER appuser

# Copy the statically linked and stripped binary from the builder stage.
COPY --from=builder /app/app_cache/target/x86_64-unknown-linux-musl/release/${APP_NAME} .

# Expose the port the application listens on.
EXPOSE 8080

# Define the command to run the application.
CMD ["./${APP_NAME}"]
