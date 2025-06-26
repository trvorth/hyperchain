#!/bin/bash
#
# A script to deploy a HyperChain node to Google Cloud for the public testnet.
# It creates a VM, sets up a firewall rule, and starts the node.
#
# Usage: ./deployment/deploy_node.sh [INSTANCE_NAME]
# Example: ./deployment/deploy_node.sh hyperchain-seed-1
#

set -e # Exit immediately if a command exits with a non-zero status.

# --- Configuration ---
PROJECT_ID="hyperchain-testnet-462602"

MACHINE_TYPE="e2-medium"
REGION="us-central1"
ZONE="us-central1-a"
IMAGE_FAMILY="ubuntu-2204-lts"
IMAGE_PROJECT="ubuntu-os-cloud"
FIREWALL_RULE_NAME="allow-hyperchain-p2p"
P2P_PORT="10333"
CONFIG_TEMPLATE_PATH="./deployment/templates/config.testnet.toml"

# --- Script Logic ---
INSTANCE_NAME=${1:-"hyperchain-node-$(date +%s)"} # Use provided name or generate a unique one

if ! command -v gcloud &> /dev/null
then
    echo "ERROR: gcloud command could not be found. Please install the Google Cloud SDK and ensure it is in your PATH."
    exit 1
fi

if [ ! -f "$CONFIG_TEMPLATE_PATH" ]; then
    echo "ERROR: Configuration template not found at '$CONFIG_TEMPLATE_PATH'"
    echo "Please ensure the file exists and you are running this script from the root of the hyperchain project."
    exit 1
fi

echo ">>> Setting active project to $PROJECT_ID"
gcloud config set project $PROJECT_ID

echo ">>> Setting compute region to $REGION"
gcloud config set compute/region $REGION

# Create firewall rule if it doesn't already exist
if ! gcloud compute firewall-rules describe $FIREWALL_RULE_NAME --format="get(name)" &>/dev/null; then
    echo ">>> Creating firewall rule '$FIREWALL_RULE_NAME' to allow TCP traffic on port $P2P_PORT..."
    gcloud compute firewall-rules create $FIREWALL_RULE_NAME \
        --allow tcp:$P2P_PORT \
        --description="Allow HyperChain P2P connections" \
        --target-tags="hyperchain-node"
else
    echo ">>> Firewall rule '$FIREWALL_RULE_NAME' already exists."
fi

# Read the content of the config file to pass as metadata
CONFIG_CONTENT=$(cat $CONFIG_TEMPLATE_PATH)

# --- VM Creation ---
echo ">>> Creating VM instance: '$INSTANCE_NAME'..."
gcloud compute instances create $INSTANCE_NAME \
    --zone=$ZONE \
    --machine-type=$MACHINE_TYPE \
    --image-family=$IMAGE_FAMILY \
    --image-project=$IMAGE_PROJECT \
    --tags="hyperchain-node" \
    --metadata startup-script="#! /bin/bash
        # Log everything to a startup script log file for debugging
        exec > >(tee /var/log/startup-script.log|logger -t startup-script -s 2>/dev/console) 2>&1

        echo '--- Updating and installing dependencies ---'
        apt-get update
        apt-get install -y build-essential clang librocksdb-dev git curl screen

        echo '--- Installing Rust ---'
        curl --proto \"=https\" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        # Add cargo to the path for the current script
        source \$HOME/.cargo/env

        echo '--- Cloning HyperChain repository ---'
        git clone https://github.com/trvorth/hyperchain.git
        cd hyperchain

        echo '--- Creating configuration file ---'
        # The quotes around CONFIG_CONTENT are important to preserve formatting
        echo \"${CONFIG_CONTENT}\" > ./config.toml

        echo '--- Building HyperChain (this will take several minutes) ---'
        cargo build --release

        echo '--- Starting HyperChain node in a background screen session ---'
        screen -dmS hyperchain_node ./target/release/start_node
        
        echo '✅ Startup script finished.'
    "

echo "✅ Deployment of '$INSTANCE_NAME' initiated."
echo "   It may take several minutes for the node to build and start."
echo "   To check the status, SSH into the VM with: gcloud compute ssh $INSTANCE_NAME"
echo "   Once inside, you can check the logs with 'tail -f /var/log/startup-script.log' or attach to the running node with 'screen -r hyperchain_node'"
