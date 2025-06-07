use crate::hyperdag::{HyperBlock, HyperDAG};
use crate::mempool::Mempool;
use crate::emission::Emission;
use crate::transaction::UTXO;
use log::{debug, info, warn};
use sha3::{Digest, Keccak256};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::sync::RwLock;
use rayon::prelude::*;
use regex::Regex;
use rand::Rng;
use tracing::instrument;
use anyhow::{Result, Context as AnyhowContext};

#[derive(Error, Debug)]
pub enum MiningError {
    #[error("Invalid block: {0}")]
    InvalidBlock(String),
    #[error("Mining failed: {0}")]
    MiningFailed(String),
    #[error("System time error: {0}")]
    TimeError(#[from] std::time::SystemTimeError),
    #[error("DAG error: {0}")]
    DAG(#[from] crate::hyperdag::HyperDAGError),
    #[error("Emission calculation error: {0}")]
    EmissionError(String),
    #[error("Invalid address: {0}")]
    InvalidAddress(String),
}

#[derive(Clone, Debug)]
pub struct Miner {
    address: String,
    dag: Arc<RwLock<HyperDAG>>,
    mempool: Arc<RwLock<Mempool>>,
    difficulty: u64,
    target_block_time: u64,
    use_gpu: bool,
    _zk_enabled: bool, // Prefixed to indicate it's currently unused
    threads: usize,
    emission: Emission,
}

impl Miner {
    #[instrument]
    pub fn new(
        address: String,
        dag: Arc<RwLock<HyperDAG>>,
        mempool: Arc<RwLock<Mempool>>,
        difficulty_hex: String,
        target_block_time: u64,
        use_gpu: bool,
        zk_enabled: bool, // Parameter name is still zk_enabled
        threads: usize,
        num_chains: u32,
    ) -> Result<Self> {
        let address_regex = Regex::new(r"^[0-9a-fA-F]{64}$")
            .context("Failed to compile address regex for miner")?;
        if !address_regex.is_match(&address) {
            return Err(MiningError::InvalidAddress(format!("Invalid miner address format: {}", address)).into());
        }

        let difficulty = u64::from_str_radix(difficulty_hex.trim_start_matches("0x"), 16)
            .context(format!("Failed to parse difficulty hex: {}", difficulty_hex))?;

        let mut effective_use_gpu = use_gpu;
        if use_gpu && !cfg!(feature = "gpu") {
            warn!("GPU mining enabled in config but 'gpu' feature is not compiled. Disabling GPU for this session.");
            effective_use_gpu = false;
        }
        
        let mut effective_zk_enabled = zk_enabled;
        if zk_enabled && !cfg!(feature = "zk") {
            warn!("ZK proofs enabled in config but 'zk' feature is not compiled. Disabling ZK for this session.");
            effective_zk_enabled = false;
        }

        let genesis_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("System time is before UNIX EPOCH")?
            .as_secs();

        Ok(Self {
            address,
            dag,
            mempool,
            difficulty,
            target_block_time,
            use_gpu: effective_use_gpu,
            _zk_enabled: effective_zk_enabled, // Assign to the prefixed struct field
            threads: threads.max(1),
            emission: Emission::default_with_timestamp(genesis_timestamp, num_chains),
        })
    }

    #[instrument]
    pub fn get_address(&self) -> String {
        self.address.clone()
    }

    #[instrument]
    pub async fn mine(
        &self,
        utxos_arc: &Arc<RwLock<HashMap<String, UTXO>>>,
    ) -> Result<Option<HyperBlock>> {
        let dag_lock = self.dag.read().await;
        let mempool_lock = self.mempool.read().await;
        let chain_id = dag_lock.get_id().await;
        let tips = dag_lock.get_tips(chain_id).await.unwrap_or_default();

        if tips.is_empty() && dag_lock.blocks.read().await.is_empty() {
            warn!("Chain {} has no tips or blocks; genesis block might be required.", chain_id);
            return Ok(None);
        } else if tips.is_empty() {
            warn!("No tips for chain {}, skipping mining round.", chain_id);
            return Ok(None);
        }

        let parents = tips.into_iter().collect::<Vec<String>>();
        let utxos_guard = utxos_arc.read().await;
        let transactions = mempool_lock.select_transactions(&dag_lock, &utxos_guard, crate::hyperdag::MAX_TRANSACTIONS_PER_BLOCK).await; // Use constant

        if transactions.is_empty() {
            debug!("No valid transactions to mine for chain {}.", chain_id);
            return Ok(None);
        }
        // The select_transactions should ideally not return more than MAX_TRANSACTIONS_PER_BLOCK
        // but an explicit check here before creating the block is a good safeguard.
        if transactions.len() > crate::hyperdag::MAX_TRANSACTIONS_PER_BLOCK {
             warn!("Miner selected {} transactions, exceeding MAX_TRANSACTIONS_PER_BLOCK {}. This should be handled by mempool.select_transactions.", transactions.len(), crate::hyperdag::MAX_TRANSACTIONS_PER_BLOCK);
             // Depending on policy, either truncate or return error. For now, let's assume select_transactions respects the limit.
        }


        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_secs();
        let reward = self.emission.calculate_reward(timestamp)
            .map_err(|e| MiningError::EmissionError(e.to_string()))?;

        let merkle_root = HyperBlock::compute_merkle_root(&transactions)
            .map_err(|e| MiningError::InvalidBlock(format!("Merkle root computation error: {}", e)))?;
        
        // Placeholder: In a real scenario, the signing key for LatticeSignature would come from the miner's wallet.
        // HyperBlock::new now takes the signing_key_material.
        // We'll assume for this mining function, the block is constructed first, then signed if PoW is met.
        // The HyperBlock struct's LatticeSignature field will be populated correctly by the PoW solution or a finalization step.
        // For now, the HyperBlock::new in DAG creates the genesis block with a key. Here, we're creating a candidate.
        let signing_key_placeholder = [0u8; 32]; // This needs to be the actual miner's signing key

        let mut initial_block_candidate = HyperBlock::new(
            chain_id,
            parents,
            transactions, // Transactions are moved here
            self.difficulty,
            self.address.clone(), // Validator
            self.address.clone(), // Miner
            &signing_key_placeholder, // Placeholder signing material for now
        ).map_err(MiningError::DAG)?;
        
        // HyperBlock::new sets the timestamp and nonce (to 0 initially). Update reward.
        initial_block_candidate.timestamp = timestamp; // Ensure miner's timestamp is used
        initial_block_candidate.reward = reward;
        // Merkle root is already computed by HyperBlock::new if transactions are passed. Re-assign if needed.
        initial_block_candidate.merkle_root = merkle_root;


        let target_hash_bytes = Miner::calculate_target_from_difficulty(self.difficulty);
        let start_time = SystemTime::now();
        let timeout_duration = Duration::from_millis(self.target_block_time);

        // Example check for the (now prefixed) _zk_enabled field
        if self._zk_enabled {
            info!("ZK features are enabled in this miner (actual ZK logic not yet implemented in mine function).");
        }

        let mining_result = if self.use_gpu && cfg!(feature = "gpu") {
            warn!("GPU mining selected but not implemented, falling back to CPU.");
            // In a real GPU miner, you would pass data to the GPU and await results.
            // For now, it falls back to CPU.
            self.mine_cpu(initial_block_candidate, &target_hash_bytes, start_time, timeout_duration).await?
        } else {
            self.mine_cpu(initial_block_candidate, &target_hash_bytes, start_time, timeout_duration).await?
        };

        if let Some(ref mined_block) = mining_result {
            info!("Successfully mined block {} on chain {} with {} transactions.", mined_block.id, mined_block.chain_id, mined_block.transactions.len());
            // Potentially sign the block now with the definitive nonce using the actual miner's key
            // This depends on whether HyperBlock::new and hash_meets_target handle the final signature or if it's done post-PoW.
        }

        Ok(mining_result)
    }

    #[instrument]
    async fn mine_cpu(
        &self,
        block_template: HyperBlock, 
        target_hash_value: &[u8],
        start_time: SystemTime,
        timeout_duration: Duration,
    ) -> Result<Option<HyperBlock>> { // Added Result wrapper
        let thread_pool = rayon::ThreadPoolBuilder::new()
            .num_threads(self.threads)
            .build()
            .context("Failed to build thread pool for CPU mining")?;

        let found_block_option: Option<HyperBlock> = thread_pool.install(move || {
            let mut rng = rand::thread_rng(); // Initialize RNG for each task if needed, or share. For nonce_start, once is fine.
            let nonce_start = rng.gen::<u64>(); // Each parallel task can start from a random point or a segment.

            (nonce_start..u64::MAX) // Iterate through possible nonces
                .into_par_iter() // Parallelize the search
                .find_map_any(|current_nonce| { // Find the first nonce that satisfies the condition
                    if let Ok(elapsed) = start_time.elapsed() {
                        if elapsed > timeout_duration {
                            return None; // Timeout
                        }
                    }

                    let mut candidate_block = block_template.clone(); // Clone the template for each attempt
                    candidate_block.nonce = current_nonce;
                    // The ID is typically derived from the block's contents including the nonce.
                    // The PoW hash is what's checked against the target.
                    let pow_hash = Miner::calculate_pow_hash(&candidate_block);

                    if Miner::hash_meets_target(&pow_hash, target_hash_value) {
                        // If PoW is met, calculate the final block ID (which might be the same as PoW hash or derived differently)
                        candidate_block.id = Miner::calculate_final_block_id(&candidate_block);
                        // At this point, the block has a valid PoW.
                        // The lattice_signature should be set with the actual signature using the miner's key.
                        // Assuming calculate_final_block_id gives the ID that will be part of the signed payload.
                        // The initial_block_candidate's lattice_signature was a placeholder.
                        // A real implementation would now sign the block with the miner's key.
                        // For this example, we'll assume the placeholder is acceptable or handled by HyperBlock::new.
                        return Some(candidate_block);
                    }

                    if current_nonce % 1_000_000 == 0 { // Log progress occasionally
                        debug!("CPU mining on chain {}, current nonce chunk starting near: {}", block_template.chain_id, current_nonce);
                    }
                    None
                })
        });

        Ok(found_block_option)
    }
    
    #[instrument]
    fn calculate_final_block_id(block: &HyperBlock) -> String {
        // The ID should be a hash of the block's consensus-critical components,
        // including the final nonce.
        let mut hasher = Keccak256::new();
        hasher.update(block.chain_id.to_le_bytes());
        hasher.update(block.merkle_root.as_bytes());
        hasher.update(block.timestamp.to_be_bytes());
        hasher.update(block.nonce.to_le_bytes()); // Nonce is critical for the ID
        hasher.update(block.miner.as_bytes());
        for parent_id in &block.parents {
            hasher.update(parent_id.as_bytes());
        }
        hasher.update(block.difficulty.to_le_bytes());
        // Potentially include other fields if they are part of the block's unique identity after PoW
        hex::encode(hasher.finalize())
    }

    #[instrument]
    fn calculate_pow_hash(block: &HyperBlock) -> Vec<u8> {
        // This hash is used for the Proof-of-Work check.
        let mut hasher = Keccak256::new();
        hasher.update(block.chain_id.to_le_bytes());
        hasher.update(block.merkle_root.as_bytes());
        hasher.update(block.timestamp.to_be_bytes());
        hasher.update(block.miner.as_bytes());
        for parent_id in &block.parents {
            hasher.update(parent_id.as_bytes());
        }
        hasher.update(block.difficulty.to_le_bytes());
        hasher.update(block.nonce.to_le_bytes()); // Nonce is iterated to find a suitable hash
        hasher.finalize().to_vec()
    }

    #[instrument]
    fn calculate_target_from_difficulty(difficulty_value: u64) -> Vec<u8> {
        // Higher difficulty means lower target value (harder to find a hash below it).
        // Standard target calculation often involves a base target divided by difficulty.
        // If u64::MAX / difficulty is used, a higher difficulty results in a smaller target.
        let target_num = u64::MAX / difficulty_value.max(1); // .max(1) to avoid division by zero
        target_num.to_be_bytes().to_vec() // Using 8 bytes for the target from u64
    }

    #[instrument]
    fn hash_meets_target(hash_bytes: &[u8], target_bytes: &[u8]) -> bool {
        // Compare the hash (typically 32 bytes for Keccak256) with the target (8 bytes from u64).
        // The hash must be numerically less than or equal to the target.
        // For a fair comparison, we should compare them as large numbers.
        // If target_bytes is shorter, we compare the most significant part of hash_bytes.
        let n_target = target_bytes.len(); // Should be 8
        
        // Ensure hash_bytes has at least n_target bytes for a meaningful comparison.
        // If PoW hash is shorter than target, it can't meet the target (unless target is huge).
        if hash_bytes.len() < n_target {
            return false; // Or handle based on desired PoW rules, typically hash is longer or equal.
        }

        // Compare the most significant 'n_target' bytes of the hash.
        // A hash is "smaller" if it has more leading zeros.
        hash_bytes[..n_target] <= *target_bytes
    }
}