#!/bin/bash

# This script uses a robust tarball method to deploy.
# It creates a compressed archive, uploads it, and extracts it on the server.

set -e

# --- Configuration ---
SERVER_USER="hyperuser"
SSH_PRIVATE_KEY="/Users/trevor/.ssh/google_compute_engine"

# Ensure these IPs are correct from `gcloud compute instances list`
SERVER_IP_1="34.126.103.74"
SERVER_IP_2="104.197.0.155"
SERVER_IP_3="35.195.207.69"

# --- File Paths ---
PROJECT_DIR="/home/${SERVER_USER}/hyperchain"
LOCAL_ARCHIVE="hyperchain_payload.tar.gz"
PAYLOAD_DIR="./payload"

# --- Deployment ---

# Step 1: Build the Linux Binary
echo "---"
echo "STEP 1 of 3: Building the Linux binary..."
echo "---"
DOCKER_DEFAULT_PLATFORM=linux/amd64 cross build --release --target=x86_64-unknown-linux-gnu
echo "✅ Build complete."
echo ""

# Step 2: Create a compressed archive of all necessary files
echo "---"
echo "STEP 2 of 3: Creating deployment archive..."
echo "---"
rm -rf ${PAYLOAD_DIR} ${LOCAL_ARCHIVE} # Clean up old files before creating new ones
mkdir -p ${PAYLOAD_DIR}
cp ./target/x86_64-unknown-linux-gnu/release/hyperdag ${PAYLOAD_DIR}/
cp ./config.toml ${PAYLOAD_DIR}/
cp ./wallet.key ${PAYLOAD_DIR}/
tar -czf ${LOCAL_ARCHIVE} -C ${PAYLOAD_DIR} .
rm -r ${PAYLOAD_DIR}
echo "✅ Archive '${LOCAL_ARCHIVE}' created."
echo ""

# Step 3: Deploy to all nodes
echo "---"
echo "STEP 3 of 3: Deploying archive to all 3 nodes..."
echo "---"

deploy_node() {
    local node_ip=$1
    local node_name=$2
    echo ""
    echo "--- Deploying to ${node_name} (${node_ip}) ---"
    
    echo "--> Uploading archive..."
    scp -i ${SSH_PRIVATE_KEY} ./${LOCAL_ARCHIVE} ${SERVER_USER}@${node_ip}:/tmp/
    
    echo "--> Extracting archive and starting node on server..."
    ssh -i ${SSH_PRIVATE_KEY} ${SERVER_USER}@${node_ip} "
        mkdir -p ${PROJECT_DIR} && \
        tar -xzf /tmp/${LOCAL_ARCHIVE} -C ${PROJECT_DIR} && \
        rm /tmp/${LOCAL_ARCHIVE} && \
        cd ${PROJECT_DIR} && \
        tmux start-server && \
        (tmux kill-session -t hyperdag-node || true) && \
        tmux new -s hyperdag-node -d && \
        tmux send-keys -t hyperdag-node 'nohup ./hyperdag --config-path config.toml &' C-m
    "
    echo "✅ Node ${node_name} Deployed."
}

deploy_node ${SERVER_IP_1} "Node 1 (Asia)"
deploy_node ${SERVER_IP_2} "Node 2 (US)"
deploy_node ${SERVER_IP_3} "Node 3 (EU)"

# Clean up the local archive file
rm ${LOCAL_ARCHIVE}

echo ""
echo "🎉 All nodes deployed successfully!"

