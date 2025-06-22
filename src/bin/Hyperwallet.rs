use clap::{Args, Parser, Subcommand};
use hyperchain::{
    hyperdag::HomomorphicEncrypted,
    transaction::{Input, Output, Transaction, TransactionConfig, DEV_ADDRESS, DEV_FEE_RATE, UTXO},
    wallet::Wallet,
};
use log::info;
use reqwest::Client;
use rpassword::prompt_password;
use secrecy::{ExposeSecret, SecretString};
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
    Generate(GenerateArgs),
    Show(ShowArgs),
    Send(SendArgs),
    Import(ImportArgs),
    ImportMnemonic(ImportMnemonicArgs),
    Balance(BalanceArgs),
}

#[derive(Args, Debug)]
struct GenerateArgs {
    #[arg(short, long, default_value = "wallet.key")]
    path: String,
}

#[derive(Args, Debug)]
struct ShowArgs {
    #[arg(short, long, default_value = "wallet.key")]
    path: String,
}

#[derive(Args, Debug)]
struct SendArgs {
    recipient: String,
    amount: u64,
    #[arg(short, long, default_value = "http://127.0.0.1:9071")]
    node: String,
    #[arg(short, long, default_value = "wallet.key")]
    path: String,
}

#[derive(Args, Debug)]
struct ImportArgs {
    private_key: String,
    #[arg(short, long, default_value = "wallet.key")]
    path: String,
}

#[derive(Args, Debug)]
struct ImportMnemonicArgs {
    mnemonic: String,
    #[arg(short, long, default_value = "wallet.key")]
    path: String,
}

#[derive(Args, Debug)]
struct BalanceArgs {
    #[arg(short, long, default_value = "http://127.0.0.1:9071")]
    node: String,
    #[arg(short, long, default_value = "wallet.key")]
    path: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let cli = Cli::parse();

    match &cli.command {
        Commands::Generate(args) => {
            let wallet = Wallet::new()?;
            println!("Public Key (Address): {}", wallet.address());
            let sk = wallet.get_signing_key()?;
            println!("Private Key: {}", hex::encode(sk.to_bytes()));
            let mnemonic_phrase = wallet.mnemonic().expose_secret();
            if !mnemonic_phrase.is_empty() {
                println!("Mnemonic: {mnemonic_phrase}");
            }
            let pass = prompt_password("Enter passphrase to encrypt wallet: ")?;
            let secret_pass = SecretString::new(pass);
            wallet.save_to_file(&args.path, &secret_pass)?;
            info!("Generated and saved new wallet to {}", &args.path);
        }
        Commands::Show(args) => {
            let pass = prompt_password("Enter passphrase: ")?;
            let secret_pass = SecretString::new(pass);
            let wallet = Wallet::from_file(&args.path, &secret_pass)?;
            println!("Showing details for wallet: {}", &args.path);
            println!("Public Key (Address): {}", wallet.address());
        }
        Commands::Send(args) => {
            if args.recipient.len() != 64 || hex::decode(&args.recipient).is_err() {
                return Err("Invalid recipient address".into());
            }
            let pass = prompt_password("Enter passphrase: ")?;
            let secret_pass = SecretString::new(pass);
            let wallet = Wallet::from_file(&args.path, &secret_pass)?;
            let sender_address = wallet.address();
            let signing_key = wallet.get_signing_key()?;
            let he_pub_key_material = &signing_key.verifying_key().to_bytes();
            let signing_key_bytes = signing_key.to_bytes();

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
            let wallet = Wallet::from_private_key(&args.private_key)?;
            println!("Imported wallet with address: {}", wallet.address());
            let pass = prompt_password("Enter passphrase to encrypt wallet: ")?;
            let secret_pass = SecretString::new(pass);
            wallet.save_to_file(&args.path, &secret_pass)?;
            info!("Imported and saved wallet to {}", &args.path);
        }
        Commands::ImportMnemonic(args) => {
            let wallet = Wallet::from_mnemonic(&args.mnemonic)?;
            println!("Imported wallet with address: {}", wallet.address());
            println!("Mnemonic: {}", wallet.mnemonic().expose_secret());
            let pass = prompt_password("Enter passphrase to encrypt wallet: ")?;
            let secret_pass = SecretString::new(pass);
            wallet.save_to_file(&args.path, &secret_pass)?;
            info!("Imported and saved wallet from mnemonic to {}", &args.path);
        }
        Commands::Balance(args) => {
            let pass = prompt_password("Enter passphrase: ")?;
            let secret_pass = SecretString::new(pass);
            let wallet = Wallet::from_file(&args.path, &secret_pass)?;
            let public_key_hex = wallet.address();

            let client = Client::new();
            let balance_url = format!("{}/balance/{}", &args.node, public_key_hex);
            info!("Fetching balance from: {balance_url}");
            let balance: u64 = client.get(&balance_url).send().await?.json().await?;

            println!("Balance for wallet {}: {} units", &args.path, balance);
        }
    }

    Ok(())
}
