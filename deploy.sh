#!/bin/bash

# Exit immediately if a command fails
set -e

# --- Configuration ---
# PLEASE EDIT THESE VALUES WITH YOUR SERVER DETAILS

SERVER_USER="admin" # Your username on the cloud servers (e.g., ubuntu, ec2-user)

# Replace with your actual server IP addresses
SERVER_IP_1="192.168.1.3"
SERVER_IP_2="198.51.100.22"
SERVER_IP_3="203.0.113.11"

# --- Local File Paths (should not need changing) ---
PROJECT_DIR="/home/${SERVER_USER}/hyperchain"
LOCAL_BINARY_PATH="./target/release/hyperdag"
LOCAL_CONFIG_PATH="./config.toml"

# --- Deployment ---

echo "🚀 Starting HyperChain Devnet Deployment..."

# Deploy to Node 1
echo "Deploying to Node 1 (${SERVER_IP_1})..."
ssh ${SERVER_USER}@${SERVER_IP_1} "mkdir -p ${PROJECT_DIR}"
scp ${LOCAL_BINARY_PATH} ${SERVER_USER}@${SERVER_IP_1}:${PROJECT_DIR}/
scp ${LOCAL_CONFIG_PATH} ${SERVER_USER}@${SERVER_IP_1}:${PROJECT_DIR}/
ssh ${SERVER_USER}@${SERVER_IP_1} "cd ${PROJECT_DIR} && tmux new -s hyperdag-node -d && tmux send-keys -t hyperdag-node 'nohup ./hyperdag --config-path config.toml &' C-m"
echo "✅ Node 1 Deployed."

# Deploy to Node 2
echo "Deploying to Node 2 (${SERVER_IP_2})..."
ssh ${SERVER_USER}@${SERVER_IP_2} "mkdir -p ${PROJECT_DIR}"
scp ${LOCAL_BINARY_PATH} ${SERVER_USER}@${SERVER_IP_2}:${PROJECT_DIR}/
scp ${LOCAL_CONFIG_PATH} ${SERVER_USER}@${SERVER_IP_2}:${PROJECT_DIR}/
ssh ${SERVER_USER}@${SERVER_IP_2} "cd ${PROJECT_DIR} && tmux new -s hyperdag-node -d && tmux send-keys -t hyperdag-node 'nohup ./hyperdag --config-path config.toml &' C-m"
echo "✅ Node 2 Deployed."

# Deploy to Node 3
echo "Deploying to Node 3 (${SERVER_IP_3})..."
ssh ${SERVER_USER}@${SERVER_IP_3} "mkdir -p ${PROJECT_DIR}"
scp ${LOCAL_BINARY_PATH} ${SERVER_USER}@${SERVER_IP_3}:${PROJECT_DIR}/
scp ${LOCAL_CONFIG_PATH} ${SERVER_USER}@${SERVER_IP_3}:${PROJECT_DIR}/
ssh ${SERVER_USER}@${SERVER_IP_3} "cd ${PROJECT_DIR} && tmux new -s hyperdag-node -d && tmux send-keys -t hyperdag-node 'nohup ./hyperdag --config-path config.toml &' C-m"
echo "✅ Node 3 Deployed."

echo "🎉 All nodes deployed successfully!"
