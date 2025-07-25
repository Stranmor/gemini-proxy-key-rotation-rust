# Stage 1: Builder
# This stage compiles the Rust application into a static binary.
# We use Alpine Linux for a small final image size.
# We use a specific Rust version for reproducibility.
# The --target x86_64-unknown-linux-musl flag is crucial for creating a static binary
# that can run on any Linux distribution without needing system dependencies.
FROM rust:1.82-alpine AS builder

# Install build dependencies for Rust and OpenSSL (for static linking).
# musl-dev is for static compilation on Alpine.
# build-base, perl, linux-headers, make, pkgconfig are common build dependencies.
RUN apk add --no-cache musl-dev build-base perl linux-headers make pkgconfig

# Set the working directory inside the container.
WORKDIR /app

# Copy the Cargo configuration file first. This allows Docker to cache dependencies
# and avoid re-downloading them on every build if they haven't changed.
COPY Cargo.toml Cargo.lock ./

# Create a dummy src/main.rs to allow `cargo build --release -Z unstable-options --out-dir` to work
# This is a trick to pre-build dependencies before copying the actual source code.
RUN mkdir src && echo "fn main() {}" > src/main.rs

# Build only the dependencies to leverage Docker layer caching.
RUN cargo build --release --target x86_64-unknown-linux-musl

# Now, copy the actual source code into the container.
COPY src ./src
COPY tests ./tests

# Build the final application binary.
# The --features reqwest/native-tls-vendored is used to statically link OpenSSL,
# avoiding runtime dependency issues on the final image.
RUN cargo build --release --target x86_64-unknown-linux-musl --features reqwest/native-tls-vendored

# ---

# Stage 2: Coverage Report Generator
# This stage is dedicated to generating the code coverage report using cargo-tarpaulin.
# We use a specific tarpaulin image which has all the necessary tools pre-installed.
# This stage is only run when explicitly targeted (e.g., `docker-compose build coverage-report`).
FROM xd009642/tarpaulin:develop-nightly AS coverage

# Install build dependencies required by some of our crates' build scripts.
# Even though tarpaulin image is Debian-based, some dependencies might need these.
RUN apt-get update && apt-get install -y --no-install-recommends build-essential libssl-dev pkg-config

# Set the working directory.
WORKDIR /app

# Copy the entire project context into the container.
COPY . .

# Set the default command for the container to run tarpaulin.
# This allows us to inject security privileges at runtime via docker-compose.
CMD ["cargo", "tarpaulin", "--all-targets", "--workspace", "--out", "Html", "--output-dir", "./coverage_report"]


# ---

# Stage 3: Final Image
# This is the final, small, and secure image that will be run in production.
# It starts from a minimal Alpine base image.
FROM alpine:3.19

# Install ca-certificates, which are required for making HTTPS requests.
RUN apk --no-cache add ca-certificates

# Set the working directory.
WORKDIR /app

# Copy the compiled binary from the 'builder' stage.
# Also copy the default configuration file.
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/gemini-proxy-key-rotation-rust .
COPY --from=builder /app/config.example.yaml .

# Expose the port the application will run on.
EXPOSE 8080

# Set the command to run the application.
CMD ["./gemini-proxy-key-rotation-rust"]
