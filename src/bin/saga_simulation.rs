use hyperchain::hyperdag::{HyperBlock, HyperDAG, UTXO};
use hyperchain::saga::{CarbonOffsetCredential, PalletSaga};
use hyperchain::transaction::{Input, Output, Transaction, TransactionConfig};
use hyperchain::wallet::Wallet;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// A standalone asynchronous main function for the simulation.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize a basic logger for the simulation output.
    env_logger::init();
    println!("--- Running Hyperchain Simulation ---");

    // 1. Setup Wallets and Addresses
    // Create wallets for the validator/miner and a test receiver.
    let validator_wallet = Wallet::new()?;
    let validator_address = validator_wallet.address();
    let receiver_wallet = Wallet::new()?;
    let receiver_address = receiver_wallet.address();
    println!("Validator Address: {validator_address}");
    println!("Receiver Address:  {receiver_address}");

    // 2. Initialize Core Components (SAGA and HyperDAG)
    // The SAGA pallet is the AI/governance core.
    let saga_pallet = Arc::new(PalletSaga::new());
    // The HyperDAG is the core blockchain data structure.
    let dag = Arc::new(RwLock::new(
        HyperDAG::new(
            &validator_address,
            60000, // target block time (ms)
            10,    // initial difficulty
            1,     // number of chains
            &validator_wallet.get_signing_key()?.to_bytes(),
            saga_pallet.clone(),
        )
        .await?,
    ));
    // Initialize the self-referential arc for the DAG.
    dag.write().await.init_self_arc(dag.clone());
    println!("SAGA and HyperDAG initialized.");

    // 3. Create a Genesis UTXO for the Validator
    let utxos = Arc::new(RwLock::new(HashMap::new()));
    {
        let mut utxos_guard = utxos.write().await;
        let genesis_utxo = UTXO {
            address: validator_address.clone(),
            amount: 1000,
            tx_id: "genesis_tx".to_string(),
            output_index: 0,
            explorer_link: "".to_string(),
        };
        utxos_guard.insert("genesis_tx_0".to_string(), genesis_utxo);
    }
    println!("Genesis UTXO created for validator.");

    // 4. Create a Sample Transaction
    // Simulate sending 100 tokens from the validator to the receiver.
    let tx_config = TransactionConfig {
        sender: validator_address.clone(),
        receiver: receiver_address,
        amount: 100,
        fee: 10,
        inputs: vec![Input {
            tx_id: "genesis_tx".to_string(),
            output_index: 0,
        }],
        // Output for the receiver and a change output back to the sender.
        outputs: vec![
            Output {
                address: receiver_wallet.address(),
                amount: 100,
                homomorphic_encrypted: hyperchain::hyperdag::HomomorphicEncrypted {
                    encrypted_amount: "".to_string(),
                },
            },
            Output {
                address: validator_address.clone(),
                amount: 890, // 1000 (input) - 100 (sent) - 10 (fee)
                homomorphic_encrypted: hyperchain::hyperdag::HomomorphicEncrypted {
                    encrypted_amount: "".to_string(),
                },
            },
        ],
        metadata: Some(HashMap::new()),
        signing_key_bytes: &validator_wallet.get_signing_key()?.to_bytes(),
        tx_timestamps: Arc::new(RwLock::new(HashMap::new())),
    };
    let sample_tx = Transaction::new(tx_config).await?;
    println!("Sample transaction created with ID: {}", sample_tx.id);

    // 5. Create a Sample Carbon Credential
    // This demonstrates the new Proof-of-Carbon-Offset functionality.
    let carbon_credential = CarbonOffsetCredential {
        id: uuid::Uuid::new_v4().to_string(),
        issuer_id: "verra".to_string(),
        beneficiary_node: validator_address.clone(),
        tonnes_co2_sequestered: 5.5,
        project_id: "vc-project-123".to_string(),
        vintage_year: 2024,
        // In a real system, this would be a cryptographic signature.
        verification_signature: "signed_by_verra".to_string(),
        // FIX: Add the missing field to satisfy the compiler.
        additionality_proof: "A mock proof statement or hash".to_string(),
    };
    println!(
        "Sample Carbon Offset Credential created: {} tonnes CO2",
        carbon_credential.tonnes_co2_sequestered
    );

    // 6. Create a New HyperBlock
    println!("Creating a new HyperBlock...");
    let block_timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();

    let new_block = HyperBlock {
        chain_id: 0,
        id: "simulated_block_id".to_string(),
        parents: vec![dag
            .read()
            .await
            .get_tips(0)
            .await
            .unwrap_or_default()
            .first()
            .cloned()
            .unwrap_or_default()],
        transactions: vec![sample_tx],
        difficulty: 10,
        validator: validator_address.clone(),
        miner: validator_address.clone(),
        nonce: 12345,
        timestamp: block_timestamp,
        reward: 250,
        effort: 0,
        cross_chain_references: vec![],
        cross_chain_swaps: vec![],
        merkle_root: "simulated_merkle_root".to_string(),
        lattice_signature: hyperchain::hyperdag::LatticeSignature {
            public_key: validator_wallet
                .get_signing_key()?
                .verifying_key()
                .to_bytes()
                .to_vec(),
            signature: vec![],
        },
        homomorphic_encrypted: vec![],
        carbon_credentials: vec![carbon_credential],
        smart_contracts: vec![],
    };

    println!("Successfully created HyperBlock with ID: {}", new_block.id);
    println!(
        "Block contains {} carbon credentials.",
        new_block.carbon_credentials.len()
    );
    println!("--- Simulation Finished Successfully ---");

    Ok(())
}
