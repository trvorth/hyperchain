use clap::Parser;
use hyperdag::{config::Config, node::Node, wallet::HyperWallet};
use log::{error, info};
use std::sync::Arc;
use tokio::signal;
use anyhow::{Result, Context};

#[derive(Parser, Debug)]
#[clap(author, version, about = "A generic HyperDAG node starter.")]
struct Args {
    #[clap(long, default_value = "config.toml")]
    config_path: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let config = Config::load(&args.config_path)
        .context(format!("Failed to load config from {}", &args.config_path))?;
    config.validate()?;

    let log_directives = format!("{},libp2p_swarm=debug,libp2p_noise=trace,libp2p_mdns=debug", &config.logging.level);
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(&log_directives)).init();
    info!("Starting HyperDAG node (from start_node.rs)...");

    let wallet = HyperWallet::new()?;
    let wallet_arc = Arc::new(wallet);

    let identity_key_path = "start_node_p2p_identity.key";
    let peer_cache_path = "start_node_peer_cache.json".to_string();
    let node = Node::new(config, args.config_path.clone(), wallet_arc, identity_key_path, peer_cache_path).await?;

    let node_handle = tokio::spawn(async move {
        if let Err(e) = node.start().await {
            error!("Node failed: {}", e);
        }
    });

    signal::ctrl_c().await?;
    info!("Received Ctrl+C, shutting down.");

    node_handle.abort();
    let _ = node_handle.await;

    info!("Shutdown complete.");
    Ok(())
}
