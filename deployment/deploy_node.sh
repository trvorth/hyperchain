

set -e # Exit immediately if a command exits with a non-zero status.

# --- Configuration ---
PROJECT_ID="hyperchain-testnet-462602"

CONFIG_URL="https://gist.githubusercontent.com/trvorth/f644d67b82df555f10303cac316fdf29/raw/93a4dfa3d0b8f0d57419f728280da48771df6a88/config.testnet.toml"

MACHINE_TYPE="e2-medium"
IMAGE_FAMILY="ubuntu-2204-lts"
IMAGE_PROJECT="ubuntu-os-cloud"
FIREWALL_RULE_NAME="allow-hyperchain-p2p"
P2P_PORT="10333"

# --- Script Logic ---
if [ "$#" -ne 3 ]; then
    echo "Usage: $0 [INSTANCE_NAME] [REGION] [ZONE]"
    echo "Example: $0 hyperchain-seed-us us-central1 us-central1-a"
    exit 1
fi

INSTANCE_NAME=$1
REGION=$2
ZONE=$3

echo ">>> Setting active project to $PROJECT_ID"
gcloud config set project $PROJECT_ID

if ! gcloud compute firewall-rules describe $FIREWALL_RULE_NAME --format="get(name)" &>/dev/null; then
    echo ">>> Creating firewall rule '$FIREWALL_RULE_NAME'..."
    gcloud compute firewall-rules create $FIREWALL_RULE_NAME --allow tcp:$P2P_PORT --description="Allow HyperChain P2P" --target-tags="hyperchain-node"
else
    echo ">>> Firewall rule '$FIREWALL_RULE_NAME' already exists."
fi

echo ">>> Creating VM instance: '$INSTANCE_NAME' in zone '$ZONE'..."
gcloud compute instances create $INSTANCE_NAME \
    --zone=$ZONE \
    --machine-type=$MACHINE_TYPE \
    --image-family=$IMAGE_FAMILY \
    --image-project=$IMAGE_PROJECT \
    --boot-disk-size=30GB \
    --tags="hyperchain-node" \
    --metadata-from-file=startup-script=./deployment/startup-script.sh \
    --metadata=config-url=${CONFIG_URL}

EXTERNAL_IP=$(gcloud compute instances describe $INSTANCE_NAME --zone=$ZONE --format='get(networkInterfaces[0].accessConfigs[0].natIP)')

echo "✅ Deployment of '$INSTANCE_NAME' initiated in '$ZONE'."
echo "   Public IP Address: ${EXTERNAL_IP}"
echo "   To check status, SSH into the VM with: gcloud compute ssh $INSTANCE_NAME --zone=$ZONE"
