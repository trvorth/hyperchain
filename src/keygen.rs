use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use rand::Rng; // Added this line to bring the 'gen' method into scope
use bip39::{Language, Mnemonic};
use std::fs::File;
use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut os_rng = OsRng; // OsRng needs to be mutable to call gen
    let signing_key = SigningKey::generate(&mut os_rng); // Pass as mutable reference
    let private_key = signing_key.to_bytes();
    let public_key = signing_key.verifying_key().to_bytes();

    // OsRng itself can be used to generate entropy for the mnemonic directly if the trait is in scope
    let entropy = os_rng.gen::<[u8; 16]>(); // Now OsRng can use .gen()
    let mnemonic = Mnemonic::from_entropy_in(Language::English, &entropy)?;
    let mnemonic_phrase = mnemonic.to_string();

    println!("Private Key: {}", hex::encode(private_key));
    println!("Public Address (genesis_validator): {}", hex::encode(public_key));
    println!("Mnemonic Phrase: {mnemonic_phrase}");

    let mut file = File::create("validator_key.txt")?;
    writeln!(file, "Private Key: {}", hex::encode(private_key))?;
    writeln!(file, "Public Address: {}", hex::encode(public_key))?;
    writeln!(file, "Mnemonic Phrase: {mnemonic_phrase}")?;
    println!("Keys saved to validator_key.txt");

    Ok(())
}