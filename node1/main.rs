use clap::{Parser, Subcommand};
use hyperchain::{config::Config, node::Node, wallet::Wallet};
use secrecy::SecretString;
use std::env;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;
use tokio::signal;
use tracing::{error, info};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

#[derive(Debug, Error)]
enum CliError {
    #[error("Configuration error: {0}")]
    Config(#[from] anyhow::Error),
    #[error("Node initialization failed: {0}")]
    NodeInitialization(String),
    #[error("Node runtime error: {0}")]
    NodeRuntime(#[from] hyperchain::node::NodeError),
    #[error("Wallet operation failed: {0}")]
    Wallet(String),
    #[error("Environment variable error: {0}")]
    EnvVar(#[from] env::VarError),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
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
    Wallet {
        #[command(subcommand)]
        wallet_command: WalletCommands,
    },
    Start {
        #[arg(short, long, value_name = "FILE", default_value = "config.toml")]
        config: PathBuf,
        #[arg(short, long, value_name = "WALLET_FILE", required = true)]
        wallet: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
enum WalletCommands {
    Generate {
        #[arg(short, long, value_name = "OUTPUT_FILE")]
        output: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<(), CliError> {
    dotenvy::dotenv().ok();
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(EnvFilter::from_default_env())
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("Failed to set up logging subscriber.");

    info!("Hyperchain node starting up...");
    let cli = Cli::parse();
    match cli.command {
        Commands::Wallet { wallet_command } => handle_wallet_command(wallet_command)?,
        Commands::Start { config, wallet } => start_node(config, wallet).await?,
    }
    Ok(())
}

fn handle_wallet_command(command: WalletCommands) -> Result<(), CliError> {
    match command {
        WalletCommands::Generate { output } => {
            info!("Generating a new encrypted wallet...");
            let password = prompt_for_password(true)?;
            let new_wallet = Wallet::new().map_err(|e| CliError::Wallet(e.to_string()))?;
            new_wallet
                .save_to_file(&output, &password)
                .map_err(|e| CliError::Wallet(e.to_string()))?;
            info!(
                "Successfully generated and encrypted wallet at '{}'.",
                output.display()
            );
            Ok(())
        }
    }
}

async fn start_node(config_path: PathBuf, wallet_path: PathBuf) -> Result<(), CliError> {
    info!("Loading configuration from '{}'.", config_path.display());
    let config = Config::load(&config_path.display().to_string()).map_err(anyhow::Error::from)?;

    info!("Decrypting wallet from '{}'.", wallet_path.display());
    let password = prompt_for_password(false)?;
    let wallet =
        Wallet::from_file(&wallet_path, &password).map_err(|e| CliError::Wallet(e.to_string()))?;
    info!("Wallet decrypted and loaded successfully.");

    let config_path_str = config_path.display().to_string();
    let wallet_arc = Arc::new(wallet);
    let p2p_identity_path = "p2p_identity_node1.key";
    let peer_cache_path = "peer_cache_node1.json";

    let node = Node::new(
        config,
        config_path_str,
        wallet_arc,
        p2p_identity_path,
        peer_cache_path.to_string(),
    )
    .await
    .map_err(|e| CliError::NodeInitialization(e.to_string()))?;

    info!("Node initialized. Starting Hyperchain node... (Press Ctrl+C for graceful shutdown)");

    tokio::select! {
        res = node.start() => {
            if let Err(e) = res {
                error!("Node runtime error: {}", e);
                return Err(CliError::NodeRuntime(e));
            }
        },
        _ = signal::ctrl_c() => {
            info!("Shutdown signal received. Initiating graceful shutdown...");
        },
    }

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
            return Err(CliError::Wallet("Passwords do not match.".to_string()));
        }
    }
    Ok(SecretString::new(password))
}
