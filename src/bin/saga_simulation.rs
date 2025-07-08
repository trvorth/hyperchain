use hyperchain::hyperdag::{HyperBlock, HyperDAG, HomomorphicEncrypted, LatticeSignature};
use hyperchain::saga::PalletSaga;
use hyperchain::transaction::{Transaction, Output, Input};
use std::sync::Arc;
use tokio::sync::RwLock;

#[tokio::main]
async fn main() {
    println!("--- [SAGA SIMULATION] ---");

    let mock_dag = Arc::new(RwLock::new(HyperDAG::new("mock_validator", 60, 10, 1, &[0;32]).await.unwrap()));
    let saga_pallet = PalletSaga::new();

    // --- Scenario 1: A good, honest node ---
    let good_block = create_mock_block("good_miner", 20);
    let good_score = saga_pallet.evaluate_block_with_saga(&good_block, &mock_dag).await.unwrap();
    let good_reward = saga_pallet.calculate_dynamic_reward(&good_block).await.unwrap();
    println!("[Honest Miner] SCS: {good_score:.2}, Dynamic Reward: {good_reward}");

    // --- Scenario 2: A node producing a block with a malformed coinbase tx ---
    let mut bad_block = create_mock_block("bad_miner", 5);
    bad_block.transactions[0].outputs.remove(1); // Remove dev fee output
    let bad_score = saga_pallet.evaluate_block_with_saga(&bad_block, &mock_dag).await.unwrap();
    let bad_reward = saga_pallet.calculate_dynamic_reward(&bad_block).await.unwrap();
    println!("[Faulty Miner] SCS: {bad_score:.2}, Dynamic Reward: {bad_reward}");

    // --- Scenario 3: The faulty node tries again ---
    let bad_block_2 = create_mock_block("bad_miner", 8);
    let bad_score_2 = saga_pallet.evaluate_block_with_saga(&bad_block_2, &mock_dag).await.unwrap();
    let bad_reward_2 = saga_pallet.calculate_dynamic_reward(&bad_block_2).await.unwrap();
    println!("[Faulty Miner, 2nd attempt] SCS: {bad_score_2:.2}, Dynamic Reward: {bad_reward_2}");
}

// Helper function to create mock blocks for the simulation
fn create_mock_block(miner: &str, tx_count: usize) -> HyperBlock {
    // Dummy data for cryptographic operations
    let dummy_pub_key = [0u8; 32];

    // Create a fully-formed, valid coinbase transaction
    let coinbase = Transaction {
        id: "coinbase_tx".to_string(),
        sender: "network".to_string(),
        receiver: miner.to_string(),
        public_key: vec![0; 32],
        inputs: vec![],
        outputs: vec![
            Output { address: miner.to_string(), amount: 225, homomorphic_encrypted: HomomorphicEncrypted::new(225, &dummy_pub_key) },
            Output { address: "dev_address".to_string(), amount: 25, homomorphic_encrypted: HomomorphicEncrypted::new(25, &dummy_pub_key) }
        ],
        amount: 250,
        fee: 0,
        timestamp: 0,
        lattice_signature: vec![0; 64],
    };
    let mut transactions = vec![coinbase];

    // Create fully-formed, standard transactions
    for i in 0..tx_count {
        transactions.push(Transaction {
            id: format!("tx_{i}"),
            sender: "mock_sender".to_string(),
            receiver: "mock_receiver".to_string(),
            public_key: vec![0; 32],
            inputs: vec![Input {
                tx_id: "prev_tx".to_string(),
                output_index: 0,
            }],
            outputs: vec![Output {
                address: "mock_receiver".to_string(),
                amount: 10,
                homomorphic_encrypted: HomomorphicEncrypted::new(10, &dummy_pub_key),
            }],
            amount: 10,
            fee: 1,
            timestamp: i as u64,
            lattice_signature: vec![0; 64],
        });
    }

    // Initialize HyperBlock with all required fields as per the compiler errors
    HyperBlock {
        id: "mock_block_id".to_string(),
        parents: vec!["parent1".to_string()],
        miner: miner.to_string(),
        timestamp: 123456789,
        transactions,
        difficulty: 1,
        nonce: 0,
        chain_id: 1,
        effort: 0,
        merkle_root: "mock_merkle_root".to_string(),
        reward: 0,
        validator: "mock_validator".to_string(),
        cross_chain_references: vec![],
        cross_chain_swaps: vec![],
        smart_contracts: vec![],
        lattice_signature: LatticeSignature::sign(&[0;32], b"mock_block_signature").unwrap(),
        homomorphic_encrypted: vec![HomomorphicEncrypted::new(0, &dummy_pub_key)],
    }
}