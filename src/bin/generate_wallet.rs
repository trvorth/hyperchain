use hyperdag::wallet::HyperWallet;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = HyperWallet::new()?;
    let address = wallet.get_address();
    println!("{}", address);
    wallet.save_to_file("wallet.key", None)?;
    Ok(())
}
