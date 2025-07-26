# Stage 0: Dependencies Builder
# This stage is dedicated to caching Rust dependencies.
# It only rebuilds when Cargo.toml or Cargo.lock change.
FROM rust:1.82-alpine AS dependencies_builder

# Install build dependencies for Rust and OpenSSL (for static linking).
RUN apk add --no-cache musl-dev build-base perl linux-headers make pkgconfig

# Set the working directory.
WORKDIR /app

# Copy only the Cargo configuration files.
COPY Cargo.toml Cargo.lock ./

# Create a dummy main.rs to allow `cargo build` to resolve dependencies.
# This ensures that this layer is only invalidated when Cargo.toml/Cargo.lock change.
RUN mkdir src && echo "fn main() {}" > src/main.rs

# Build only the dependencies. This will download and compile all crates.
RUN cargo build --release --target x86_64-unknown-linux-musl --locked

# ---

# Stage 1: Application Builder
# This stage compiles the actual application, leveraging cached dependencies.
FROM rust:1.82-alpine AS builder

# Install build dependencies (already cached from dependencies_builder, but good practice)
RUN apk add --no-cache musl-dev build-base perl linux-headers make pkgconfig

# Set the working directory.
WORKDIR /app

# Copy the cached registry and target directory from the dependencies_builder.
# This significantly speeds up subsequent builds as dependencies are pre-compiled.
COPY --from=dependencies_builder /usr/local/cargo/registry /usr/local/cargo/registry
COPY --from=dependencies_builder /app/target /app/target

# Copy the entire source code. This layer is invalidated only when source code changes.
COPY . .

# Build the final application binary.
# This will be much faster as dependencies are already compiled.
RUN cargo build --release --target x86_64-unknown-linux-musl --locked

# ---

# Stage 2: Final Image
# This is the final, small, and secure image that will be run in production.
FROM alpine:3.19

# Install ca-certificates, which are required for making HTTPS requests.
RUN apk --no-cache add ca-certificates

# Set the working directory.
WORKDIR /app

# Copy the compiled binary from the 'builder' stage.
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/gemini-proxy-key-rotation-rust .
COPY config.example.yaml .
COPY static ./static

# Expose the port the application will run on.
EXPOSE 8080

# Set the command to run the application.
CMD ["./gemini-proxy-key-rotation-rust"]
