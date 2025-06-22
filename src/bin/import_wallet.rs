use hyperchain::wallet::Wallet;
use secrecy::SecretString;
use std::env;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: cargo run --bin import_wallet \"<private_key_or_mnemonic>\"");
        std::process::exit(1);
    }
    let key_or_mnemonic = &args[1];
    let wallet = if key_or_mnemonic.split_whitespace().count() >= 12 {
        println!("Attempting to import from mnemonic...");
        Wallet::from_mnemonic(key_or_mnemonic)?
    } else {
        println!("Attempting to import from private key...");
        Wallet::from_private_key(key_or_mnemonic)?
    };
    let password =
        rpassword::prompt_password("Create a password to encrypt the imported wallet: ")?;
    let secret_password = SecretString::new(password);
    wallet.save_to_file("wallet.key", &secret_password)?;
    println!(
        "Wallet imported and saved successfully!\n  Address: {}",
        wallet.address()
    );
    Ok(())
}
