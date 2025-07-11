// main.rs

use clap::{Parser, Subcommand};
use hyperchain::{
    config::{Config, ConfigError},
    node::{Node, NodeError},
    wallet::{Wallet, WalletError},
    x_phyrus,
};
use secrecy::SecretString;
use std::env;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;
use tokio::task;
use tracing::{error, info};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

pub mod omega;

#[derive(Debug, Error)]
enum CliError {
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),
    #[error("Node initialization failed: {0}")]
    NodeInitialization(#[from] NodeError),
    #[error("Wallet operation failed: {0}")]
    Wallet(#[from] WalletError),
    #[error("Environment variable error: {0}")]
    EnvVar(#[from] env::VarError),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Task join error: {0}")]
    Join(#[from] task::JoinError),
    #[error("{0}")]
    Password(String),
    #[error("X-PHYRUS Pre-boot check failed: {0}")]
    XPPreBoot(#[from] anyhow::Error),
}

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about,
    long_about = "An ultimately secure and robust Hyperchain node."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Manage encrypted wallets.
    Wallet {
        #[command(subcommand)]
        wallet_command: WalletCommands,
    },
    /// Start the Hyperchain node.
    Start {
        #[arg(short, long, value_name = "FILE", default_value = "config.toml")]
        config: PathBuf,
        #[arg(short, long, value_name = "WALLET_FILE", required = true)]
        wallet: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
enum WalletCommands {
    /// Generate a new, securely encrypted wallet file.
    Generate {
        #[arg(short, long, value_name = "OUTPUT_FILE")]
        output: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<(), CliError> {
    dotenvy::dotenv().ok();
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("Failed to set up logging subscriber.");

    info!("Hyperchain node starting up...");
    let cli = Cli::parse();

    match cli.command {
        Commands::Wallet { wallet_command } => handle_wallet_command(wallet_command).await?,
        Commands::Start { config, wallet } => start_node(config, wallet).await?,
    }

    Ok(())
}

async fn handle_wallet_command(command: WalletCommands) -> Result<(), CliError> {
    match command {
        WalletCommands::Generate { output } => {
            info!("Generating a new encrypted wallet...");
            let password = prompt_for_password(true)?;

            info!("Encrypting and saving new wallet...");
            let new_wallet = Wallet::new()?;

            let output_clone = output.clone();
            task::spawn_blocking(move || new_wallet.save_to_file(&output_clone, &password))
                .await??;

            info!("Wallet saved successfully to '{}'.", output.display());
        }
    }
    Ok(())
}

async fn start_node(config_path: PathBuf, wallet_path: PathBuf) -> Result<(), CliError> {
    info!("Loading configuration from '{}'.", config_path.display());
    let config = Config::load(&config_path.display().to_string())?;

    // FIX: Await the now-asynchronous pre-boot sequence.
    x_phyrus::initialize_pre_boot_sequence(&config, &wallet_path).await?;

    let password = prompt_for_password(false)?;
    info!("Decrypting wallet (this may take a while)...");
    let wallet_path_clone = wallet_path.clone();
    let wallet =
        task::spawn_blocking(move || Wallet::from_file(&wallet_path_clone, &password)).await??;
    info!("Wallet decrypted and loaded successfully.");

    info!("Initializing Hyperchain services...");
    let node = Node::new(
        config,
        config_path.display().to_string(),
        Arc::new(wallet),
        "p2p_identity.key",
        "peer_cache.json".to_string(),
    )
    .await?;
    info!("Node initialized. Starting main loop... (Press Ctrl+C for graceful shutdown)");

    node.start().await?;

    info!("Hyperchain node has shut down.");
    Ok(())
}

fn prompt_for_password(confirm: bool) -> Result<SecretString, CliError> {
    print!("Enter wallet password: ");
    io::stdout().flush()?;
    let password = rpassword::read_password()?;

    if confirm {
        print!("Confirm wallet password: ");
        io::stdout().flush()?;
        let confirmation = rpassword::read_password()?;
        if password != confirmation {
            return Err(CliError::Password("Passwords do not match.".to_string()));
        }
    }
    Ok(SecretString::new(password))
}