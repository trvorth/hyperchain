use anyhow::{Context, Result};
use clap::Parser;
use hyperdag::{config::Config, node::Node, wallet::HyperWallet};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::signal;

#[derive(Parser, Debug)]
#[clap(author, version, about = "HyperDAG Test Node 3")]
struct Args {
    #[clap(long, default_value = "./node3/node3_config.toml")]
    config_path: PathBuf,
}

// A simple struct to hold our cached peers
#[derive(Serialize, Deserialize, Debug, Default)]
struct PeerCache {
    peers: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let log_prefix = "[Node 3]";

    // --- Path Definitions ---
    let config_path = &args.config_path;
    let identity_key_path = "./node3/p2p_identity.key";
    let wallet_path = "./node3/node3_wallet.key";
    let peer_cache_path = "./node3/peer_cache.json";

    // --- Config Loading ---
    let mut config = Config::load(config_path.to_str().unwrap()).context(format!(
        "{log_prefix} Failed to load config from {config_path:?}"
    ))?;

    // --- Logger Initialization ---
    let log_directives = format!(
        "{},libp2p=info,libp2p_kad=info", // Adjusted for less noise, can be set to debug
        &config.logging.level
    );
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(&log_directives))
        .format_target(false)
        .format_timestamp_micros()
        .try_init()
        .ok();

    info!("{log_prefix} Starting HyperDAG Node...");

    // --- UPGRADE: Load cached peers ---
    let cached_peers: Vec<String> = fs::read_to_string(peer_cache_path)
        .ok()
        .and_then(|contents| serde_json::from_str::<PeerCache>(&contents).ok())
        .map_or_else(Vec::new, |cache| cache.peers);

    if !cached_peers.is_empty() {
        info!(
            "{} Loaded {} peers from cache file.",
            log_prefix,
            cached_peers.len()
        );
        // Combine and deduplicate peers from config and cache
        let mut all_peers = HashSet::new();
        all_peers.extend(config.peers.iter().cloned());
        all_peers.extend(cached_peers.into_iter());
        config.peers = all_peers.into_iter().collect();
    }

    // --- Wallet Loading/Generation ---
    if let Some(parent_dir) = PathBuf::from(wallet_path).parent() {
        fs::create_dir_all(parent_dir)?;
    }
    let validator_wallet = match HyperWallet::from_file(wallet_path, None) {
        Ok(wallet) => wallet,
        Err(_) => {
            warn!("{log_prefix} Wallet not found at {wallet_path}. Ensure it has been copied from the root wallet.key.");
            return Err(anyhow::anyhow!("Wallet file not found at {}", wallet_path));
        }
    };
    let wallet_arc = Arc::new(validator_wallet);

    // --- Node Initialization ---
    let node_instance = Node::new(
        config,
        config_path.to_str().unwrap().to_string(),
        wallet_arc,
        identity_key_path,
        peer_cache_path.to_string(), // Pass the cache path to the node
    )
    .await?;

    // --- Start and Shutdown Logic ---
    let node_handle = tokio::spawn(async move {
        if let Err(e) = node_instance.start().await {
            error!("[Node 3] Node task failed: {e}");
        }
    });

    info!("{log_prefix} Node started. Waiting for Ctrl+C.");
    signal::ctrl_c().await?;
    info!("{log_prefix} Shutting down.");
    node_handle.abort();
    let _ = node_handle.await;
    info!("{log_prefix} Shutdown complete.");
    Ok(())
}
