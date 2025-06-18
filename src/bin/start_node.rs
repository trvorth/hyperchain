use anyhow::Context;
use clap::Parser;
use hyperchain::{config::Config, node::Node, wallet::HyperWallet};
use log::{error, info, warn};
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
}

/// The main asynchronous function that sets up and runs the node.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Enhancement: Check for config file existence before loading.
    if !Path::new(&args.config_path).exists() {
        anyhow::bail!(
            "Configuration file not found at path: {}",
            &args.config_path
        );
    }

    // Load and validate the configuration.
    let config = Config::load(&args.config_path)
        .context(format!("Failed to load config from {}", &args.config_path))?;
    config.validate()?;

    // Initialize the logger based on the level specified in the config.
    let log_directives = format!(
        "{},libp2p_swarm=debug,libp2p_noise=trace,libp2p_mdns=debug",
        &config.logging.level
    );
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(&log_directives))
        .init();
    info!("Starting HyperDAG node (from start_node.rs)...");

    // Create a new wallet instance. In a real scenario, you might load an existing one.
    let wallet = HyperWallet::new()?;
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

    // Abort the node task to trigger shutdown logic and wait for it to complete.
    node_handle.abort();
    let _ = node_handle.await;

    info!("Shutdown complete.");
    Ok(())
}
