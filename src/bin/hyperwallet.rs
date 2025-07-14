use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use hyperchain::{
    // NOTE: TransactionMetadata struct is no longer directly used here.
    hyperdag::UTXO,
    transaction::{Input, Output, Transaction, TransactionConfig},
    wallet::{Wallet, WalletError},
};
use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

const DEV_ADDRESS: &str = "2119707c4caf16139cfb5c09c4dcc9bf9cfe6808b571c108d739f49cc14793b9";
const DEV_FEE_RATE: f64 = 0.0304;

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "A command-line wallet for the Hyperchain network.",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// The URL of the Hyperchain node API.
    #[arg(long, global = true, default_value = "http://127.0.0.1:8080")]
    node_url: String,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Generate a new, securely encrypted wallet file.
    Generate {
        #[arg(short, long, value_name = "OUTPUT_FILE", default_value = "wallet.key")]
        output: PathBuf,
    },
    /// Show the public address for a given wallet file.
    ShowAddress {
        #[arg(short, long, value_name = "WALLET_FILE", default_value = "wallet.key")]
        wallet: PathBuf,
    },
    /// Get the balance for the address in a given wallet file.
    Balance {
        #[arg(short, long, value_name = "WALLET_FILE", default_value = "wallet.key")]
        wallet: PathBuf,
    },
    /// Send an amount to a receiver address.
    Send {
        #[arg(short, long, value_name = "WALLET_FILE", default_value = "wallet.key")]
        wallet: PathBuf,
        #[arg(short, long)]
        to: String,
        #[arg(short, long)]
        amount: u64,
        #[arg(long, default_value_t = 1)]
        fee: u64,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Generate { output } => generate_wallet(output).await,
        Commands::ShowAddress { wallet } => show_address(wallet).await,
        Commands::Balance { wallet } => get_balance(&cli.node_url, wallet).await,
        Commands::Send {
            wallet,
            to,
            amount,
            fee,
        } => send_transaction(&cli.node_url, wallet, to, amount, fee).await,
    };

    if let Err(e) = result {
        eprintln!("\nError: {e:?}");
        std::process::exit(1);
    }

    Ok(())
}

async fn generate_wallet(output: PathBuf) -> Result<()> {
    println!("Generating a new encrypted wallet...");
    let password = prompt_for_password(true)?;
    let new_wallet = Wallet::new()?;

    new_wallet
        .save_to_file(&output, &password)
        .context("Failed to save new wallet")?;

    println!("\nWallet generated successfully!");
    println!("Address: {}", new_wallet.address());
    println!("Mnemonic: {}", new_wallet.mnemonic().expose_secret());
    println!("Saved to: {}", output.display());
    println!("\nIMPORTANT: Store your mnemonic phrase in a secure location. It is the only way to recover your wallet.");

    Ok(())
}

async fn show_address(wallet_path: PathBuf) -> Result<()> {
    println!(
        "Enter password to display address for '{}':",
        wallet_path.display()
    );
    let password = prompt_for_password(false)?;
    let wallet = Wallet::from_file(&wallet_path, &password).context(format!(
        "Failed to load wallet from '{}'",
        wallet_path.display()
    ))?;

    println!("Wallet Address: {}", wallet.address());
    Ok(())
}

async fn get_balance(node_url: &str, wallet_path: PathBuf) -> Result<()> {
    println!(
        "Enter password to check balance for '{}':",
        wallet_path.display()
    );
    let password = prompt_for_password(false)?;
    let wallet = Wallet::from_file(&wallet_path, &password).context(format!(
        "Failed to load wallet from '{}'",
        wallet_path.display()
    ))?;
    let address = wallet.address();

    let client = Client::new();
    let url = format!("{node_url}/balance/{address}");

    println!("Querying balance for address: {address}");
    println!("From node: {node_url}");

    let res = client
        .get(&url)
        .send()
        .await
        .context(format!("Failed to connect to node at {url}"))?;

    if res.status().is_success() {
        let balance: u64 = res
            .json()
            .await
            .context("Failed to parse balance from response")?;
        println!("\nBalance: {balance}");
    } else {
        let error_text = res
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(anyhow!("Node returned an error: {}", error_text));
    }

    Ok(())
}

async fn send_transaction(
    node_url: &str,
    wallet_path: PathBuf,
    to: String,
    amount: u64,
    fee: u64,
) -> Result<()> {
    println!("Preparing to send {amount} to address {to}");
    let password = prompt_for_password(false)?;
    let wallet = Wallet::from_file(&wallet_path, &password).context(format!(
        "Failed to load wallet from '{}'",
        wallet_path.display()
    ))?;

    let sender_address = wallet.address();
    let client = Client::new();

    // 1. Fetch UTXOs for the sender's address
    let utxo_url = format!("{node_url}/utxos/{sender_address}");
    println!("Fetching available funds (UTXOs) from {node_url}...");

    let res = client
        .get(&utxo_url)
        .send()
        .await
        .context("Failed to fetch UTXOs")?;
    if !res.status().is_success() {
        return Err(anyhow!(
            "Node failed to provide UTXOs: {}",
            res.text().await?
        ));
    }
    let available_utxos: HashMap<String, UTXO> =
        res.json().await.context("Failed to parse UTXOs")?;

    if available_utxos.is_empty() {
        return Err(anyhow!("No funds available for address {}", sender_address));
    }
    println!("Found {} UTXOs.", available_utxos.len());

    // 2. Select UTXOs to cover the transaction amount + fees
    let dev_fee = (amount as f64 * DEV_FEE_RATE).round() as u64;
    let total_needed = amount + fee + dev_fee;
    let mut inputs = vec![];
    let mut total_input_amount = 0;

    for (_utxo_id, utxo) in available_utxos {
        if total_input_amount >= total_needed {
            break;
        }
        total_input_amount += utxo.amount;
        inputs.push(Input {
            tx_id: utxo.tx_id,
            output_index: utxo.output_index,
        });
    }

    if total_input_amount < total_needed {
        return Err(anyhow!(
            "Insufficient funds. Needed: {}, Available: {}",
            total_needed,
            total_input_amount
        ));
    }

    // 3. Construct outputs (to receiver, dev fee, and change back to self)
    let he_public_key = wallet.get_signing_key()?.verifying_key();
    let he_pub_key_material: &[u8] = he_public_key.as_bytes();

    let mut outputs = vec![Output {
        address: to.clone(),
        amount,
        homomorphic_encrypted: hyperchain::hyperdag::HomomorphicEncrypted::new(
            amount,
            he_pub_key_material,
        ),
    }];

    if dev_fee > 0 {
        outputs.push(Output {
            address: DEV_ADDRESS.to_string(),
            amount: dev_fee,
            homomorphic_encrypted: hyperchain::hyperdag::HomomorphicEncrypted::new(
                dev_fee,
                he_pub_key_material,
            ),
        });
    }

    let change = total_input_amount - total_needed;
    if change > 0 {
        outputs.push(Output {
            address: sender_address.clone(),
            amount: change,
            homomorphic_encrypted: hyperchain::hyperdag::HomomorphicEncrypted::new(
                change,
                he_pub_key_material,
            ),
        });
    }

    // 4. Create and sign the transaction
    let signing_key = wallet.get_signing_key()?;

    // FIX: Construct metadata as a HashMap to match the expected type in TransactionConfig.
    let mut metadata_map = HashMap::new();
    metadata_map.insert(
        "origin_component".to_string(),
        "hyperwallet-cli".to_string(),
    );
    metadata_map.insert("intent".to_string(), "Standard P2P Transfer".to_string());

    let tx_config = TransactionConfig {
        sender: sender_address,
        receiver: to,
        amount,
        fee,
        inputs,
        outputs,
        signing_key_bytes: signing_key.as_bytes(),
        tx_timestamps: Arc::new(RwLock::new(HashMap::new())),
        // Pass the correctly typed HashMap wrapped in Some().
        metadata: Some(metadata_map),
    };

    let tx = Transaction::new(tx_config)
        .await
        .context("Failed to create transaction")?;
    println!("Transaction created with ID: {}", tx.id);

    // 5. Submit the transaction to the node
    let tx_url = format!("{node_url}/transaction");
    println!("Submitting transaction to {tx_url}...");

    let res = client
        .post(&tx_url)
        .json(&tx)
        .send()
        .await
        .context("Failed to send transaction")?;

    if res.status().is_success() {
        let tx_id_response: String = res
            .json()
            .await
            .context("Failed to parse transaction ID from response")?;
        println!("\nTransaction submitted successfully!");
        println!("Transaction ID: {tx_id_response}");
    } else {
        let error_text = res
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(anyhow!("Node rejected transaction: {}", error_text));
    }

    Ok(())
}

fn prompt_for_password(confirm: bool) -> Result<SecretString, WalletError> {
    print!("Enter wallet password: ");
    io::stdout().flush().map_err(WalletError::Io)?;
    let password = rpassword::read_password().map_err(WalletError::Io)?;

    if confirm {
        print!("Confirm wallet password: ");
        io::stdout().flush().map_err(WalletError::Io)?;
        let confirmation = rpassword::read_password().map_err(WalletError::Io)?;
        if password != confirmation {
            return Err(WalletError::Passphrase(
                "Passwords do not match.".to_string(),
            ));
        }
    }
    Ok(SecretString::new(password))
}
