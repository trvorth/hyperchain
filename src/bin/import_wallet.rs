use hyperdag::wallet::HyperWallet;
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: cargo run --bin import_wallet <private_key>");
        std::process::exit(1);
    }
    let private_key = &args[1];
    let wallet = HyperWallet::from_private_key(private_key)?;
    wallet.save_to_file("wallet.key", None)?;
    println!(
        "Wallet imported successfully with address: {}",
        wallet.get_address()
    );
    Ok(())
}
