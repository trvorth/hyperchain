#!/bin/bash

# Log everything to a startup script log file for debugging
exec > >(tee /var/log/startup-script.log|logger -t startup-script -s 2>/dev/console) 2>&1

echo "--- [HyperChain Startup] Updating and installing dependencies ---"
apt-get update
apt-get install -y build-essential clang librocksdb-dev git curl screen

echo "--- [HyperChain Startup] Installing Rust ---"
# Run the installer as the root user
curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y

# Explicitly use the root user's home directory path
echo "--- [HyperChain Startup] Sourcing Rust environment ---"
source /root/.cargo/env

echo "--- [HyperChain Startup] Cloning HyperChain repository ---"
git clone https://github.com/trvorth/hyperchain.git
cd hyperchain

echo "--- [HyperChain Startup] Downloading testnet configuration ---"
# Retrieve the config URL from the instance's custom metadata
CONFIG_URL=$(curl -H "Metadata-Flavor: Google" http://metadata.google.internal/computeMetadata/v1/instance/attributes/config-url)
curl -L -o ./config.toml "${CONFIG_URL}"

echo "--- [HyperChain Startup] Building HyperChain (this will take several minutes) ---"
# Use the full, absolute path to cargo to avoid PATH issues
/root/.cargo/bin/cargo build --release

echo "--- [HyperChain Startup] Starting HyperChain node in a background screen session ---"
# Use the full path to the compiled binary
screen -dmS hyperchain_node ./target/release/start_node

echo "✅ [HyperChain Startup] Script finished."
