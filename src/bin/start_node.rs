use anyhow::Context;
use clap::Parser;
use hyperdag::{config::Config, node::Node, wallet::HyperWallet};
use log::{error, info, warn};
use std::path::Path;
use std::sync::Arc;
use tokio::signal;

#[derive(Parser, Debug)]
#[clap(author, version, about = "A generic HyperDAG node starter.")]
struct Args {
    #[clap(long, default_value = "config.toml")]
    config_path: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Enhancement: Check for config file existence before loading.
    if !Path::new(&args.config_path).exists() {
        anyhow::bail!("Configuration file not found at path: {}", &args.config_path);
    }
    
    let config = Config::load(&args.config_path)
        .context(format!("Failed to load config from {}", &args.config_path))?;
    config.validate()?;

    let log_directives = format!("{},libp2p_swarm=debug,libp2p_noise=trace,libp2p_mdns=debug", &config.logging.level);
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(&log_directives)).init();
    info!("Starting HyperDAG node (from start_node.rs)...");

    let wallet = HyperWallet::new()?;
    let wallet_arc = Arc::new(wallet);

    // SECURITY: Ensure private key files have restrictive permissions.
    let identity_key_path = "start_node_p2p_identity.key";
    if Path::new(identity_key_path).exists() {
        warn!("SECURITY: Reusing existing P2P identity key at '{identity_key_path}'. For production, ensure this file is secure and has restricted permissions.");
    }

    let peer_cache_path = "start_node_peer_cache.json".to_string();
    let node = Node::new(config, args.config_path.clone(), wallet_arc, identity_key_path, peer_cache_path).await?;

    let node_handle = tokio::spawn(async move {
        if let Err(e) = node.start().await {
            error!("Node failed: {e}");
        }
    });

    signal::ctrl_c().await?;
    info!("Received Ctrl+C, shutting down.");

    node_handle.abort();
    let _ = node_handle.await;

    info!("Shutdown complete.");
    Ok(())
}