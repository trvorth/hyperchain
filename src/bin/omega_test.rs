// src/bin/omega_test.rs
use hyperchain::omega;
use sp_core::H256;

#[tokio::main]
async fn main() {
    env_logger::init();
    println!("--- Running ΛΣ-ΩMEGA Standalone Test ---");

    // FIX: Await the now-async simulation function
    omega::simulation::run_simulation().await;

    println!("\n--- Direct Action Reflection Test ---");
    let sample_hash = H256::random();
    println!("Reflecting on action with hash: {sample_hash:?}");

    // FIX: Await the async function call to get the bool result
    let result = omega::reflect_on_action(sample_hash).await;

    println!(
        "Security reflex result: {}",
        if result { "Approved" } else { "Rejected" }
    );
}