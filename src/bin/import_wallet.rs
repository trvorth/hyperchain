use hyperchain::wallet::HyperWallet;
use std::env;
use std::error::Error;

/// The main function for the `import_wallet` binary.
fn main() -> Result<(), Box<dyn Error>> {
    // Collect command-line arguments into a vector of strings.
    let args: Vec<String> = env::args().collect();

    // Check for the correct number of arguments. The program expects
    // the command itself and one argument (the private key or mnemonic).
    if args.len() != 2 {
        eprintln!("Usage: cargo run --bin import_wallet \"<private_key_or_mnemonic>\"");
        std::process::exit(1);
    }

    // The private key or mnemonic is the second argument.
    let key_or_mnemonic = &args[1];

    // Attempt to create a wallet instance from the provided string.
    // This will try to interpret it as a private key first, then a mnemonic.
    let wallet = if key_or_mnemonic.split_whitespace().count() == 12 {
        println!("Attempting to import from mnemonic...");
        HyperWallet::from_mnemonic(key_or_mnemonic)?
    } else {
        println!("Attempting to import from private key...");
        HyperWallet::from_private_key(key_or_mnemonic)?
    };

    // Save the imported wallet to a file.
    wallet.save_to_file("wallet.key", None)?;

    println!(
        "Wallet imported and saved successfully!\n  Address: {}",
        wallet.get_address()
    );

    Ok(())
}
