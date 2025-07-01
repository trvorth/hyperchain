use anyhow::Context;
use clap::Parser;
use hyperchain::{config::Config, node::Node, wallet::Wallet};
use log::{error, info, warn};
use secrecy::Secret;
use std::path::Path;
use std::sync::Arc;
use tokio::signal;

/// Command-line arguments for the node starter.
#[derive(Parser, Debug)]
#[clap(author, version, about = "A generic HyperDAG node starter.")]
struct Args {
    /// Path to the configuration file.
    #[clap(long, default_value = "config.toml")]
    config_path: String,

    /// Path to the wallet file.
    #[clap(long, default_value = "wallet.key")]
    wallet_path: String,
}

/// The main asynchronous function that sets up and runs the node.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Check for config file existence before loading.
    if !Path::new(&args.config_path).exists() {
        anyhow::bail!(
            "Configuration file not found at path: {}",
            &args.config_path
        );
    }

    // Load the configuration.
    let config = Config::load(&args.config_path)
        .context(format!("Failed to load config from {}", &args.config_path))?;

    // Initialize the logger.
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(&config.logging.level),
    )
    .init();
    info!("Starting HyperDAG node (from start_node.rs)...");

    // Load the wallet.
    if !Path::new(&args.wallet_path).exists() {
        anyhow::bail!(
            "Wallet file not found at path: {}. Please create or import a wallet.",
            &args.wallet_path
        );
    }

    let passphrase = rpassword::prompt_password("Enter passphrase to unlock wallet: ")?;
    let secret_passphrase = Secret::new(passphrase);

    let wallet = Wallet::from_file(&args.wallet_path, &secret_passphrase)
        .context("Failed to load wallet from file. Check the wallet path and passphrase.")?;
    let wallet_arc = Arc::new(wallet);

    // SECURITY: Ensure private key files have restrictive permissions.
    let identity_key_path = "start_node_p2p_identity.key";
    if Path::new(identity_key_path).exists() {
        warn!("SECURITY: Reusing existing P2P identity key at '{identity_key_path}'. For production, ensure this file is secure and has restricted permissions.");
    }

    // Initialize the node with its configuration and wallet.
    let peer_cache_path = "start_node_peer_cache.json".to_string();
    let node = Node::new(
        config,
        args.config_path.clone(),
        wallet_arc,
        identity_key_path,
        peer_cache_path,
    )
    .await?;

    // Spawn the node's main event loop in a separate Tokio task.
    let node_handle = tokio::spawn(async move {
        if let Err(e) = node.start().await {
            error!("Node failed: {e}");
        }
    });

    // Wait for shutdown signal (Ctrl+C).
    signal::ctrl_c().await?;
    info!("Received Ctrl+C, shutting down.");

    node_handle.abort();
    let _ = node_handle.await;

    info!("Shutdown complete.");
    Ok(())
}
