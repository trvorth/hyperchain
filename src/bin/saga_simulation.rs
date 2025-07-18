//! --- SAGA Simulation v4.0 ---
//! This simulation tests the advanced features of the SAGA v4.0 pallet, including:
//! - Dynamic, tiered fee calculation.
//! - Submission and AI-driven verification of Proof-of-Carbon-Offset credentials.
//! - SAGA's autonomous epoch evolution process.
//! - FIX: Corrected the field name from `additionality_proof` to `additionality_proof_hash`
//!   to match the updated `CarbonOffsetCredential` struct.
//! - REFACTOR: Updated the fee calculation to call the `calculate_dynamic_fee` method on the
//!   `saga_pallet` instance, ensuring the simulation uses the live, governable fee logic.

use anyhow::Result;
use hyperchain::{
    hyperdag::HyperDAG,
    mempool::Mempool,
    saga::{CarbonOffsetCredential, PalletSaga},
    transaction::{Input, Output, Transaction, TransactionConfig},
    wallet::Wallet,
};
use rocksdb::{Options, DB};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    println!("--- Running Hyperchain Simulation v4 (Evolved SAGA) ---");

    // 1. Wallets and Addresses
    let validator_wallet = Wallet::new()?;
    let validator_address = validator_wallet.address();
    let receiver_wallet = Wallet::new()?;
    let receiver_address = receiver_wallet.address();
    println!("Validator Address: {validator_address}");
    println!("Receiver Address:  {receiver_address}");

    // 2. Core Components (SAGA and HyperDAG)
    let saga_pallet = Arc::new(PalletSaga::new(
        #[cfg(feature = "infinite-strata")]
        None,
    ));

    let db_path = "saga_sim_db_temp";
    if std::path::Path::new(db_path).exists() {
        std::fs::remove_dir_all(db_path)?;
    }
    let mut opts = Options::default();
    opts.create_if_missing(true);
    let db = DB::open(&opts, db_path)?;

    let dag_arc = Arc::new(HyperDAG::new(
        &validator_address,
        60, // target block time (seconds)
        10, // initial difficulty
        1,  // number of chains
        &validator_wallet.get_signing_key()?.to_bytes(),
        saga_pallet.clone(),
        db,
    )?);
    println!("SAGA and HyperDAG initialized.");

    // 3. Mempool and UTXO Set
    let mempool_arc = Arc::new(RwLock::new(Mempool::new(3600, 10_000_000, 10_000)));
    let utxos_arc = Arc::new(RwLock::new(HashMap::new()));

    {
        let mut utxos_guard = utxos_arc.write().await;
        utxos_guard.insert(
            "genesis_tx_0".to_string(),
            hyperchain::hyperdag::UTXO {
                address: validator_address.clone(),
                amount: 5_000_000,
                tx_id: "genesis_tx".to_string(),
                output_index: 0,
                explorer_link: "".to_string(),
            },
        );
    }
    println!("Genesis UTXO created for validator.");

    // 4. Create and add a sample transaction to the mempool
    let he_public_key = validator_wallet.get_signing_key()?.verifying_key();
    let he_pub_key_material: &[u8] = he_public_key.as_bytes();

    let mut metadata = HashMap::new();
    metadata.insert("intent".to_string(), "Simulation Test Transfer".to_string());
    metadata.insert(
        "origin_component".to_string(),
        "saga_simulation".to_string(),
    );

    let amount_to_send = 1_500_000;
    // REFACTOR: Use the SAGA pallet to calculate the fee based on its current rules.
    let fee = saga_pallet.calculate_dynamic_fee(amount_to_send).await;
    println!("Sending {amount_to_send} with dynamically calculated fee of {fee}");

    let tx_config = TransactionConfig {
        sender: validator_address.clone(),
        receiver: receiver_address.clone(),
        amount: amount_to_send,
        fee,
        inputs: vec![Input {
            tx_id: "genesis_tx".to_string(),
            output_index: 0,
        }],
        outputs: vec![
            Output {
                address: receiver_address.clone(),
                amount: amount_to_send,
                homomorphic_encrypted: hyperchain::hyperdag::HomomorphicEncrypted::new(
                    amount_to_send,
                    he_pub_key_material,
                ),
            },
            Output {
                address: validator_address.clone(),
                amount: 5_000_000 - amount_to_send - fee,
                homomorphic_encrypted: hyperchain::hyperdag::HomomorphicEncrypted::new(
                    5_000_000 - amount_to_send - fee,
                    he_pub_key_material,
                ),
            },
        ],
        metadata: Some(metadata),
        signing_key_bytes: &validator_wallet.get_signing_key()?.to_bytes(),
        tx_timestamps: Arc::new(RwLock::new(HashMap::new())),
    };

    let sample_tx = Transaction::new(tx_config).await?;
    println!("Sample transaction created with ID: {}", sample_tx.id);

    {
        let utxos_reader = utxos_arc.read().await;
        mempool_arc
            .write()
            .await
            .add_transaction(sample_tx, &utxos_reader, &dag_arc)
            .await?;
    }

    // 5. Use the HyperDAG to create a valid candidate block
    println!("Requesting candidate block from HyperDAG...");
    let mut candidate_block = {
        dag_arc
            .create_candidate_block(
                &validator_wallet.get_signing_key()?.to_bytes(),
                &validator_address,
                &mempool_arc,
                &utxos_arc,
                0, // chain_id
            )
            .await?
    };
    println!(
        "Candidate block created with {} transactions.",
        candidate_block.transactions.len()
    );

    // Add a CarbonOffsetCredential to the block for SAGA to evaluate
    candidate_block
        .carbon_credentials
        .push(CarbonOffsetCredential {
            id: Uuid::new_v4().to_string(),
            issuer_id: "verra".to_string(),
            beneficiary_node: validator_address.clone(),
            tonnes_co2_sequestered: 5.5,
            project_id: "verra-p-981".to_string(),
            vintage_year: 2024,
            verification_signature: "signed_by_verra".to_string(),
            // FIX: Changed field name to match struct definition
            additionality_proof_hash: "mock_hash_of_additionality_docs".to_string(),
            issuer_reputation_score: 0.95,
            geospatial_consistency_score: 0.98,
        });

    // 6. Evaluate the block with SAGA (this happens inside add_block)
    println!("Adding block to HyperDAG for validation and SAGA evaluation...");
    let block_id = candidate_block.id.clone();
    let added = dag_arc.add_block(candidate_block, &utxos_arc).await?;

    if added {
        println!("Block {block_id} added to the DAG successfully!");
    } else {
        println!("Block {block_id} was not added to the DAG.");
    }

    // 7. Process an epoch to see SAGA's autonomous functions
    println!("\n--- Processing Epoch 1 Evolution ---");
    let current_epoch = *dag_arc.current_epoch.read().await + 1;
    saga_pallet
        .process_epoch_evolution(current_epoch, &dag_arc)
        .await;

    println!("\n--- Simulation Finished Successfully ---");

    // Cleanup the temporary database
    DB::destroy(&Options::default(), db_path)?;
    Ok(())
}
