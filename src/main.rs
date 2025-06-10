use dotenvy::dotenv;
use anyhow::{Context, Result};
use chrono::Local;
use clap::Parser;
use hyperdag::{config::Config, node::Node, wallet::HyperWallet};
use log::{error, info, warn};
use std::sync::Arc;
use tokio::signal;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(long, default_value = "config.toml")]
    config_path: String,
    #[clap(long)]
    node_log_prefix: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok(); 
    let args = Args::parse();
    let log_prefix: String = args.node_log_prefix.map_or_else(String::new, |p| format!("[{}] ", p));
    let log_prefix_for_format = log_prefix.clone();

    let initial_config = Config::load(&args.config_path)
        .context(format!("{}Failed to load config from {}", log_prefix, args.config_path))?;

    let log_directives = format!("{},libp2p_swarm=debug,libp2p_noise=trace,libp2p_mdns=debug", &initial_config.logging.level);
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(&log_directives))
        .format(move |buf, record| {
            use std::io::Write;
            writeln!(
                buf,
                "{} {} {} {} {}:{} {}",
                chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
                record.level(),
                record.target(),
                log_prefix_for_format,
                record.file().unwrap_or("unknown"),
                record.line().unwrap_or(0),
                record.args()
            )
        })
        .try_init()
        .context(format!("{}Failed to initialize logger", log_prefix))?;

    info!("{}Starting HyperDAG node at {:?}", log_prefix, Local::now());
    info!("{}Config loaded from: \"{}\"", log_prefix, args.config_path);

    initial_config.validate().context(format!("{}Config validation failed", log_prefix))?;

    let wallet_file_name = "wallet.key";
    let validator_wallet = match HyperWallet::from_file(wallet_file_name, None) {
        Ok(wallet) => {
            info!("{}Loaded wallet from {} with address: {}", log_prefix, wallet_file_name, wallet.get_address());
            wallet
        }
        Err(e) => {
            warn!("{}Failed to load or parse {}: {}. Generating new wallet.", log_prefix, wallet_file_name, e);
            let new_wallet = HyperWallet::new().context(format!("{}Failed to generate new wallet", log_prefix))?;
            new_wallet.save_to_file(wallet_file_name, None)
                .context(format!("{}Failed to save new wallet to {}", log_prefix, wallet_file_name))?;
            info!(
                "{}Generated new wallet with address: {}. Update config.toml with this genesis_validator if needed.",
                log_prefix, new_wallet.get_address()
            );
            new_wallet
        }
    };

    let wallet_arc = Arc::new(validator_wallet);

    info!("{}Initializing and starting Node instance...", log_prefix);

    let identity_key_path = "p2p_identity.key";
    let peer_cache_path = "peer_cache.json".to_string();
    let node_instance = Node::new(initial_config, args.config_path.clone(), wallet_arc, identity_key_path, peer_cache_path).await?;

    let node_handle = tokio::spawn(async move {
        if let Err(e) = node_instance.start().await {
            error!("Node main task execution failed: {}", e);
        } else {
            info!("Node main task completed.");
        }
    });

    info!("{}Node tasks started. Main thread will wait for Ctrl+C.", log_prefix);

    signal::ctrl_c().await?;
    info!("{}Received Ctrl+C. Shutting down.", log_prefix);

    node_handle.abort();
    if let Err(e) = node_handle.await {
        if !e.is_cancelled() {
            error!("{}Node task join error: {:?}", log_prefix, e);
        }
    }

    info!("{}Shutdown complete.", log_prefix);
    Ok(())
}
