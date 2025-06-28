#!/bin/bash
#
# A script to deploy a HyperChain node to Google Cloud for the public testnet.
#
# Usage: ./deployment/deploy_node.sh [INSTANCE_NAME]
# Example: ./deployment/deploy_node.sh hyperchain-seed-1
#

set -e # Exit immediately if a command exits with a non-zero status.

# --- Configuration ---
PROJECT_ID="hyperchain-testnet-462602"
CONFIG_URL="https://gist.githubusercontent.com/trvorth/your-gist-id/raw/config.testnet.toml" # <-- IMPORTANT: SET YOUR GIST URL
MACHINE_TYPE="e2-medium"
REGION="us-central1"
ZONE="us-central1-a"
FIREWALL_RULE_NAME="allow-hyperchain-p2p"
P2P_PORT="10333"

# --- Script Logic ---
INSTANCE_NAME=${1:-"hyperchain-node-$(date +%s)"}

echo ">>> Setting active project to $PROJECT_ID"
gcloud config set project $PROJECT_ID

echo ">>> Setting compute region to $REGION"
gcloud config set compute/region $REGION

# Create firewall rule if it doesn't already exist
if ! gcloud compute firewall-rules describe $FIREWALL_RULE_NAME --format="get(name)" &>/dev/null; then
    echo ">>> Creating firewall rule '$FIREWALL_RULE_NAME'..."
    gcloud compute firewall-rules create $FIREWALL_RULE_NAME --allow tcp:$P2P_PORT --description="Allow HyperChain P2P" --target-tags="hyperchain-node"
else
    echo ">>> Firewall rule '$FIREWALL_RULE_NAME' already exists."
fi

# --- VM Creation ---
echo ">>> Creating VM instance: '$INSTANCE_NAME'..."
gcloud compute instances create $INSTANCE_NAME \
    --zone=$ZONE \
    --machine-type=$MACHINE_TYPE \
    --image-family="ubuntu-2204-lts" \
    --image-project="ubuntu-os-cloud" \
    --boot-disk-size=30GB \
    --tags="hyperchain-node" \
    --metadata-from-file=startup-script=./deployment/startup-script.sh \
    --metadata=config-url=${CONFIG_URL}

echo "✅ Deployment of '$INSTANCE_NAME' initiated."
echo "   It may take several minutes for the node to build and start."
echo "   To check the status, SSH into the VM with: gcloud compute ssh $INSTANCE_NAME --zone=$ZONE"
echo "   Once inside, you can check the logs with 'tail -f /var/log/startup-script.log' or attach to the running node with 'screen -r hyperchain_node'"
