#!/bin/bash

set -e

PROJECT_DIR="/Users/trevor/hyperchain"
CARGO_TOML="$PROJECT_DIR/Cargo.toml"
CONFIG_TOML="$PROJECT_DIR/config.toml"
WALLET_KEY="$PROJECT_DIR/wallet.key"
EXPECTED_ADDRESS="2119707c4caf16139cfb5c09c4dcc9bf9cfe6808b571c108d739f49cc14793b9"

cd "$PROJECT_DIR" || { echo "Error: Cannot access $PROJECT_DIR"; exit 1; }

# Step 1: Ensure chrono dependency
if ! grep -q "chrono =" "$CARGO_TOML"; then
    echo "Adding chrono dependency to Cargo.toml..."
    cargo add chrono
else
    echo "chrono dependency already present in Cargo.toml."
fi

# Step 2: Check existing wallet or generate a new one
if [[ -f "$WALLET_KEY" ]]; then
    echo "Checking existing wallet.key..."
    cargo build --bin generate_wallet 2>/dev/null || true
    CURRENT_ADDRESS=$(cargo run --bin generate_wallet 2>/dev/null | tail -n 1)
    if [[ "$CURRENT_ADDRESS" == "$EXPECTED_ADDRESS" ]]; then
        echo "wallet.key matches genesis_validator: $EXPECTED_ADDRESS"
    else
        echo "Warning: wallet.key address ($CURRENT_ADDRESS) does not match genesis_validator ($EXPECTED_ADDRESS)."
        echo "To use $EXPECTED_ADDRESS, provide the corresponding private key or mnemonic and update wallet.key manually."
        echo "For now, updating config.toml to match the existing wallet.key address."
        sed -i '' "s/genesis_validator = \".*\"/genesis_validator = \"$CURRENT_ADDRESS\"/" "$CONFIG_TOML"
    fi
else
    echo "wallet.key not found. Generating new wallet..."
    cat << 'EOF' > src/bin/generate_wallet.rs
use hyperdag::wallet::HyperWallet;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = HyperWallet::new()?;
    let address = wallet.get_address();
    println!("{}", address);
    wallet.save_to_file("wallet.key", None)?;
    Ok(())
}
EOF
    cargo build --bin generate_wallet
    NEW_ADDRESS=$(cargo run --bin generate_wallet | tail -n 1)
    rm src/bin/generate_wallet.rs
    echo "Updating config.toml with new genesis_validator: $NEW_ADDRESS"
    sed -i '' "s/genesis_validator = \".*\"/genesis_validator = \"$NEW_ADDRESS\"/" "$CONFIG_TOML"
fi

# Step 3: Build the project
echo "Building the project..."
cargo build

# Step 4: Run the project
echo "Running the project..."
cargo run --bin hyperdag

echo "Sync and run complete!"
