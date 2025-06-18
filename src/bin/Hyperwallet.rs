use clap::{Args, Parser, Subcommand};
use hyperdag::{
    hyperdag::HomomorphicEncrypted,
    transaction::{Input, Output, Transaction, TransactionConfig, DEV_ADDRESS, DEV_FEE_RATE, UTXO},
    wallet::HyperWallet,
};
use log::info;
use reqwest::Client;
use rpassword::prompt_password;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
struct Cli {
    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Generate a new wallet and save it to a file
    Generate(GenerateArgs),
    /// Show wallet details from a specific file
    Show(ShowArgs),
    /// Send a transaction from a specific wallet
    Send(SendArgs),
    /// Import a wallet from a private key and save it
    Import(ImportArgs),
    /// Import a wallet from a mnemonic phrase and save it
    ImportMnemonic(ImportMnemonicArgs),
    /// Check the balance of a specific wallet
    Balance(BalanceArgs),
}

#[derive(Args, Debug)]
struct GenerateArgs {
    /// Path to save the new wallet file
    #[arg(short, long, default_value = "wallet.key")]
    path: String,
    /// Encrypt the wallet with a passphrase
    #[arg(long)]
    passphrase: bool,
}

#[derive(Args, Debug)]
struct ShowArgs {
    /// Path to the wallet file
    #[arg(short, long, default_value = "wallet.key")]
    path: String,
    /// Use a passphrase to decrypt the wallet
    #[arg(long)]
    passphrase: bool,
}

#[derive(Args, Debug)]
struct SendArgs {
    /// The recipient's 64-character hex address
    recipient: String,
    /// The amount to send
    amount: u64,
    /// The API endpoint of the node
    #[arg(short, long, default_value = "http://127.0.0.1:9071")]
    node: String,
    /// Path to the sender's wallet file
    #[arg(short, long, default_value = "wallet.key")]
    path: String,
    /// Use a passphrase to decrypt the sender's wallet
    #[arg(long)]
    passphrase: bool,
}

#[derive(Args, Debug)]
struct ImportArgs {
    /// The private key to import
    private_key: String,
    /// Path to save the new wallet file
    #[arg(short, long, default_value = "wallet.key")]
    path: String,
    /// Encrypt the new wallet with a passphrase
    #[arg(long)]
    passphrase: bool,
}

#[derive(Args, Debug)]
struct ImportMnemonicArgs {
    /// The mnemonic phrase to import
    mnemonic: String,
    /// Path to save the new wallet file
    #[arg(short, long, default_value = "wallet.key")]
    path: String,
    /// Encrypt the new wallet with a passphrase
    #[arg(long)]
    passphrase: bool,
}

#[derive(Args, Debug)]
struct BalanceArgs {
    /// The API endpoint of the node
    #[arg(short, long, default_value = "http://127.0.0.1:9071")]
    node: String,
    /// Path to the wallet file to check the balance of
    #[arg(short, long, default_value = "wallet.key")]
    path: String,
    /// Use a passphrase to decrypt the wallet
    #[arg(long)]
    passphrase: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let cli = Cli::parse();

    match &cli.command {
        Commands::Generate(args) => {
            let wallet = HyperWallet::new()?;
            println!("Public Key (Address): {}", wallet.get_address());
            if let Ok(sk) = wallet.get_signing_key() {
                println!("Private Key: {}", hex::encode(sk.to_bytes()));
            }
            if let Some(mnemonic) = wallet.get_mnemonic() {
                println!("Mnemonic: {mnemonic}");
            }
            let pass = if args.passphrase {
                Some(prompt_password("Enter passphrase to encrypt wallet: ")?)
            } else {
                None
            };
            wallet.save_to_file(&args.path, pass.as_deref())?;
            info!("Generated and saved new wallet to {}", &args.path);
        }
        Commands::Show(args) => {
            let pass = if args.passphrase {
                Some(prompt_password("Enter passphrase: ")?)
            } else {
                None
            };
            let wallet = HyperWallet::from_file(&args.path, pass.as_deref())?;
            println!("Showing details for wallet: {}", &args.path);
            println!("Public Key (Address): {}", wallet.get_address());
        }
        Commands::Send(args) => {
            if args.recipient.len() != 64 || hex::decode(&args.recipient).is_err() {
                return Err("Invalid recipient address".into());
            }
            let pass = if args.passphrase {
                Some(prompt_password("Enter passphrase: ")?)
            } else {
                None
            };
            let wallet = HyperWallet::from_file(&args.path, pass.as_deref())?;
            let sender_address = wallet.get_address();
            let he_pub_key_material = &wallet.get_public_key()?.to_bytes();
            let signing_key_bytes = wallet.get_signing_key()?.to_bytes();

            let client = Client::new();
            let fee: u64 = 1;

            let utxos_url = format!("{}/utxos/{}", &args.node, sender_address);
            info!("Fetching UTXOs from: {utxos_url}");
            let available_utxos: HashMap<String, UTXO> =
                client.get(&utxos_url).send().await?.json().await?;
            info!(
                "Found {} UTXOs for address {}",
                available_utxos.len(),
                sender_address
            );

            let mut inputs_for_tx = vec![];
            let mut total_input_value = 0;
            let dev_fee_on_transfer = (args.amount as f64 * DEV_FEE_RATE).round() as u64;
            let total_amount_to_cover = args.amount + fee + dev_fee_on_transfer;

            for (_id, utxo) in available_utxos {
                if utxo.address == sender_address {
                    inputs_for_tx.push(Input {
                        tx_id: utxo.tx_id,
                        output_index: utxo.output_index,
                    });
                    total_input_value += utxo.amount;
                    if total_input_value >= total_amount_to_cover {
                        break;
                    }
                }
            }

            if total_input_value < total_amount_to_cover {
                return Err(format!("Insufficient funds. Required: {total_amount_to_cover}, Available: {total_input_value}").into());
            }

            let mut outputs_for_tx = vec![Output {
                address: args.recipient.clone(),
                amount: args.amount,
                homomorphic_encrypted: HomomorphicEncrypted::new(args.amount, he_pub_key_material),
            }];

            if dev_fee_on_transfer > 0 {
                outputs_for_tx.push(Output {
                    address: DEV_ADDRESS.to_string(),
                    amount: dev_fee_on_transfer,
                    homomorphic_encrypted: HomomorphicEncrypted::new(
                        dev_fee_on_transfer,
                        he_pub_key_material,
                    ),
                });
            }

            let change_amount = total_input_value - total_amount_to_cover;
            if change_amount > 0 {
                outputs_for_tx.push(Output {
                    address: sender_address.clone(),
                    amount: change_amount,
                    homomorphic_encrypted: HomomorphicEncrypted::new(
                        change_amount,
                        he_pub_key_material,
                    ),
                });
            }

            let tx_timestamps = Arc::new(RwLock::new(HashMap::new()));
            let tx_config = TransactionConfig {
                sender: sender_address,
                receiver: args.recipient.clone(),
                amount: args.amount,
                fee,
                inputs: inputs_for_tx,
                outputs: outputs_for_tx,
                signing_key_bytes: &signing_key_bytes,
                tx_timestamps,
            };
            let tx = Transaction::new(tx_config).await?;

            let api_url = format!("{}/transaction", &args.node);
            let res = client.post(&api_url).json(&tx).send().await?;

            if res.status().is_success() {
                println!("Transaction sent successfully! ID: {}", tx.id);
            } else {
                return Err(format!(
                    "Failed to send transaction: {} - {}",
                    res.status(),
                    res.text().await?
                )
                .into());
            }
        }
        Commands::Import(args) => {
            let wallet = HyperWallet::from_private_key(&args.private_key)?;
            println!("Imported wallet with address: {}", wallet.get_address());
            let pass = if args.passphrase {
                Some(prompt_password("Enter passphrase to encrypt wallet: ")?)
            } else {
                None
            };
            wallet.save_to_file(&args.path, pass.as_deref())?;
            info!("Imported and saved wallet to {}", &args.path);
        }
        Commands::ImportMnemonic(args) => {
            let wallet = HyperWallet::from_mnemonic(&args.mnemonic)?;
            println!("Imported wallet with address: {}", wallet.get_address());
            println!("Mnemonic: {}", wallet.get_mnemonic().unwrap_or_default());
            let pass = if args.passphrase {
                Some(prompt_password("Enter passphrase to encrypt wallet: ")?)
            } else {
                None
            };
            wallet.save_to_file(&args.path, pass.as_deref())?;
            info!("Imported and saved wallet from mnemonic to {}", &args.path);
        }
        Commands::Balance(args) => {
            let pass = if args.passphrase {
                Some(prompt_password("Enter passphrase: ")?)
            } else {
                None
            };
            let wallet = HyperWallet::from_file(&args.path, pass.as_deref())?;
            let public_key_hex = wallet.get_address();

            let client = Client::new();
            let balance_url = format!("{}/balance/{}", &args.node, public_key_hex);
            info!("Fetching balance from: {balance_url}");
            let balance: u64 = client.get(&balance_url).send().await?.json().await?;

            println!("Balance for wallet {}: {} units", &args.path, balance);
        }
    }

    Ok(())
}
