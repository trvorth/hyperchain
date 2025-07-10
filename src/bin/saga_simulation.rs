use hyperchain::hyperdag::{HyperBlock, HyperDAG, HomomorphicEncrypted, LatticeSignature};
use hyperchain::saga::PalletSaga;
// The TransactionMetadata struct is no longer used here.
use hyperchain::transaction::{Transaction, Output, Input};
use std::sync::Arc;
use tokio::sync::RwLock;
use futures::FutureExt;
use ed25519_dalek::Signer;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    println!("--- [SAGA BEHAVIORAL ANALYSIS SIMULATION] ---");

    let saga_pallet = Arc::new(PalletSaga::new());
    let mock_dag_arc = Arc::new(RwLock::new(
        HyperDAG::new("mock_validator", 60, 10, 1, &[0;32], saga_pallet.clone())
            .now_or_never()
            .unwrap()
            .unwrap(),
    ));
    mock_dag_arc.write().await.init_self_arc(mock_dag_arc.clone());

    println!("\n--- SCENARIO 1: HONEST MINER ---");
    let good_block = create_mock_block("good_miner", 20);
    saga_pallet.evaluate_block_with_saga(&good_block, &mock_dag_arc).await.unwrap();
    let good_scs = saga_pallet.reputation.credit_scores.read().await.get("good_miner").cloned().unwrap();
    println!("[Honest Miner] Final SCS: {:.4}", good_scs.score);
    println!("[Honest Miner] Score Breakdown: {:#?}", good_scs.factors);

    println!("\n--- SCENARIO 2: FAULTY MINER (INVALID COINBASE) ---");
    let mut bad_block = create_mock_block("bad_miner", 5);
    bad_block.transactions[0].outputs.clear(); // Invalid coinbase
    saga_pallet.evaluate_block_with_saga(&bad_block, &mock_dag_arc).await.unwrap();
    let bad_scs = saga_pallet.reputation.credit_scores.read().await.get("bad_miner").cloned().unwrap();
    println!("[Faulty Miner] Final SCS: {:.4}", bad_scs.score);
    println!("[Faulty Miner] Score Breakdown: {:#?}", bad_scs.factors);

    println!("\n--- SCENARIO 3: FEE SPAMMER ---");
    let spam_block = create_mock_block("spam_miner", 100); // High tx count
    saga_pallet.evaluate_block_with_saga(&spam_block, &mock_dag_arc).await.unwrap();
    let spam_scs = saga_pallet.reputation.credit_scores.read().await.get("spam_miner").cloned().unwrap();
    println!("[Spam Miner] Final SCS: {:.4}", spam_scs.score);
    println!("[Spam Miner] Score Breakdown: {:#?}", spam_scs.factors);
}

fn create_mock_block(miner: &str, tx_count: usize) -> HyperBlock {
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&[1; 32]);
    let dummy_pub_key = signing_key.verifying_key().to_bytes();
    
    // FIX: Construct metadata as a HashMap to match the Transaction struct.
    let mut coinbase_metadata = HashMap::new();
    coinbase_metadata.insert("origin_component".to_string(), "coinbase".to_string());
    coinbase_metadata.insert("intent".to_string(), "Block Reward".to_string());

    let coinbase = Transaction {
        id: format!("coinbase_{miner}"),
        sender: "network".to_string(), receiver: miner.to_string(),
        public_key: dummy_pub_key.to_vec(),
        inputs: vec![],
        outputs: vec![
            Output { address: miner.to_string(), amount: 225, homomorphic_encrypted: HomomorphicEncrypted::new(225, &dummy_pub_key) },
            Output { address: "dev_fee_addr".to_string(), amount: 25, homomorphic_encrypted: HomomorphicEncrypted::new(25, &dummy_pub_key) }
        ],
        amount: 250, fee: 0, timestamp: 0,
        lattice_signature: signing_key.sign(b"c").to_bytes().to_vec(),
        metadata: coinbase_metadata,
    };
    let mut transactions = vec![coinbase];

    for i in 0..tx_count {
        let mut tx_metadata = HashMap::new();
        tx_metadata.insert("origin_component".to_string(), "simulation-wallet".to_string());
        tx_metadata.insert("intent".to_string(), "Mock Transfer".to_string());

        transactions.push(Transaction {
            id: format!("tx_{miner}_{i}"), sender: "sender".to_string(), receiver: "receiver".to_string(),
            public_key: dummy_pub_key.to_vec(),
            inputs: vec![Input { tx_id: "prev_tx".to_string(), output_index: 0 }],
            outputs: vec![Output { address: "receiver".to_string(), amount: 10, homomorphic_encrypted: HomomorphicEncrypted::new(10, &dummy_pub_key) }],
            amount: 10, fee: 1, timestamp: i as u64,
            lattice_signature: signing_key.sign(b"tx").to_bytes().to_vec(),
            metadata: tx_metadata,
        });
    }
    
    HyperBlock {
        id: format!("block_{miner}"),
        parents: vec!["genesis".to_string()], miner: miner.to_string(),
        timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
        transactions: transactions.clone(),
        difficulty: 1, nonce: 0, chain_id: 1, effort: 0,
        merkle_root: HyperBlock::compute_merkle_root(&transactions).unwrap(),
        reward: 250, validator: "validator".to_string(),
        cross_chain_references: vec![], cross_chain_swaps: vec![], smart_contracts: vec![],
        lattice_signature: LatticeSignature::sign(&[1;32], b"b").unwrap(),
        homomorphic_encrypted: vec![],
    }
}