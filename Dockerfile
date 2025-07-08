# --- Stage 1: Builder ---
# Use the official Rust image as a build environment.
# Using a specific version ensures reproducible builds.
FROM rust:1.78 as builder

# Create a new, empty workspace.
WORKDIR /usr/src/hyperchain

# Copy over project files.
COPY . .

# Install dependencies needed for some crates (e.g., rocksdb).
RUN apt-get update && apt-get install -y clang libclang-dev

# Build the project in release mode for performance.
# This will cache dependencies and speed up subsequent builds.
# We enable the 'ai' feature flag here.
RUN cargo build --release --features ai

# --- Stage 2: Final Image ---
# Use a slim base image to keep the final container size small.
FROM debian:bullseye-slim

# Install necessary runtime dependencies (like OpenSSL).
RUN apt-get update && \
    apt-get install -y openssl ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Copy the compiled binary from the builder stage.
COPY --from=builder /usr/src/hyperchain/target/release/hyperchain /usr/local/bin/hyperchain

# Create a directory for the node's data (config, wallet, db).
WORKDIR /data
RUN mkdir /data/db

# Expose the API and P2P ports.
# Replace with actual ports from config.toml.
EXPOSE 8080
EXPOSE 4001

# Define the entrypoint for the container.
# This command will run when the container starts.
# It expects config.toml and wallet.key to be mounted into /data.
ENTRYPOINT ["hyperchain", "start", "--config", "/data/config.toml", "--wallet", "/data/wallet.key"]
