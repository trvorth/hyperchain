use hyperchain::wallet::Wallet;
use rpassword::prompt_password;
use secrecy::SecretString;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = Wallet::new()?;
    let address = wallet.address();
    println!("Generated new wallet with address: {address}");

    // Corrected the call to prompt_password with a prompt string.
    let pass = prompt_password("Create a password to encrypt the new wallet: ")?;
    let secret_pass = SecretString::new(pass);

    wallet.save_to_file("wallet.key", &secret_pass)?;
    println!("Wallet saved to wallet.key");
    Ok(())
}
