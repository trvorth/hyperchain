use crate::emission::Emission;
use crate::hyperdag::{HyperBlock, HyperDAG};
use crate::mempool::Mempool;
use crate::transaction::UTXO;
use anyhow::{Context as AnyhowContext, Result};
use log::{debug, info, warn};
use rand::Rng;
use rayon::prelude::*;
use regex::Regex;
use sha3::{Digest, Keccak256};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::instrument;

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

// Struct to hold configuration for the Miner
#[derive(Debug)]
pub struct MinerConfig {
    pub address: String,
    pub dag: Arc<RwLock<HyperDAG>>,
    pub mempool: Arc<RwLock<Mempool>>,
    pub difficulty_hex: String,
    pub target_block_time: u64,
    pub use_gpu: bool,
    pub zk_enabled: bool,
    pub threads: usize,
    pub num_chains: u32,
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
    // Refactored to use the MinerConfig struct
    pub fn new(config: MinerConfig) -> Result<Self> {
        let address_regex = Regex::new(r"^[0-9a-fA-F]{64}$")
            .context("Failed to compile address regex for miner")?;
        if !address_regex.is_match(&config.address) {
            return Err(MiningError::InvalidAddress(format!(
                "Invalid miner address format: {}",
                config.address
            ))
            .into());
        }

        let difficulty = u64::from_str_radix(config.difficulty_hex.trim_start_matches("0x"), 16)
            .context(format!("Failed to parse difficulty hex: {}", config.difficulty_hex))?;

        let mut effective_use_gpu = config.use_gpu;
        if config.use_gpu && !cfg!(feature = "gpu") {
            warn!("GPU mining enabled in config but 'gpu' feature is not compiled. Disabling GPU for this session.");
            effective_use_gpu = false;
        }

        let mut effective_zk_enabled = config.zk_enabled;
        if config.zk_enabled && !cfg!(feature = "zk") {
            warn!("ZK proofs enabled in config but 'zk' feature is not compiled. Disabling ZK for this session.");
            effective_zk_enabled = false;
        }

        let genesis_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("System time is before UNIX EPOCH")?
            .as_secs();

        Ok(Self {
            address: config.address,
            dag: config.dag,
            mempool: config.mempool,
            difficulty,
            target_block_time: config.target_block_time,
            use_gpu: effective_use_gpu,
            _zk_enabled: effective_zk_enabled,
            threads: config.threads.max(1),
            emission: Emission::default_with_timestamp(genesis_timestamp, config.num_chains),
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
            warn!("Chain {chain_id} has no tips or blocks; genesis block might be required.");
            return Ok(None);
        } else if tips.is_empty() {
            warn!("No tips for chain {chain_id}, skipping mining round.");
            return Ok(None);
        }

        let parents = tips.into_iter().collect::<Vec<String>>();
        let utxos_guard = utxos_arc.read().await;
        let transactions = mempool_lock
            .select_transactions(
                &dag_lock,
                &utxos_guard,
                crate::hyperdag::MAX_TRANSACTIONS_PER_BLOCK,
            )
            .await;

        if transactions.is_empty() {
            debug!("No valid transactions to mine for chain {chain_id}.");
            return Ok(None);
        }
        if transactions.len() > crate::hyperdag::MAX_TRANSACTIONS_PER_BLOCK {
            warn!("Miner selected {} transactions, exceeding MAX_TRANSACTIONS_PER_BLOCK {}. This should be handled by mempool.select_transactions.", transactions.len(), crate::hyperdag::MAX_TRANSACTIONS_PER_BLOCK);
        }

        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let reward = self
            .emission
            .calculate_reward(timestamp)
            .map_err(|e| MiningError::EmissionError(e.to_string()))?;

        let merkle_root = HyperBlock::compute_merkle_root(&transactions).map_err(|e| {
            MiningError::InvalidBlock(format!("Merkle root computation error: {e}"))
        })?;
        
        let signing_key_placeholder = [0u8; 32];

        let mut initial_block_candidate = HyperBlock::new(
            chain_id,
            parents,
            transactions,
            self.difficulty,
            self.address.clone(),
            self.address.clone(),
            &signing_key_placeholder,
        )
        .map_err(MiningError::DAG)?;

        initial_block_candidate.timestamp = timestamp;
        initial_block_candidate.reward = reward;
        initial_block_candidate.merkle_root = merkle_root;

        let target_hash_bytes = Miner::calculate_target_from_difficulty(self.difficulty);
        let start_time = SystemTime::now();
        let timeout_duration = Duration::from_millis(self.target_block_time);

        if self._zk_enabled {
            info!("ZK features are enabled in this miner (actual ZK logic not yet implemented in mine function).");
        }

        let mining_result = if self.use_gpu && cfg!(feature = "gpu") {
            warn!("GPU mining selected but not implemented, falling back to CPU.");
            self.mine_cpu(
                initial_block_candidate,
                &target_hash_bytes,
                start_time,
                timeout_duration,
            )
            .await?
        } else {
            self.mine_cpu(
                initial_block_candidate,
                &target_hash_bytes,
                start_time,
                timeout_duration,
            )
            .await?
        };

        if let Some(ref mined_block) = mining_result {
            info!(
                "Successfully mined block {} on chain {} with {} transactions.",
                mined_block.id,
                mined_block.chain_id,
                mined_block.transactions.len()
            );
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
    ) -> Result<Option<HyperBlock>> {
        let thread_pool = rayon::ThreadPoolBuilder::new()
            .num_threads(self.threads)
            .build()
            .context("Failed to build thread pool for CPU mining")?;

        let found_block_option: Option<HyperBlock> = thread_pool.install(move || {
            let mut rng = rand::thread_rng();
            let nonce_start = rng.gen::<u64>();

            (nonce_start..u64::MAX)
                .into_par_iter()
                .find_map_any(|current_nonce| {
                    if let Ok(elapsed) = start_time.elapsed() {
                        if elapsed > timeout_duration {
                            return None;
                        }
                    }

                    let mut candidate_block = block_template.clone();
                    candidate_block.nonce = current_nonce;
                    let pow_hash = Miner::calculate_pow_hash(&candidate_block);

                    if Miner::hash_meets_target(&pow_hash, target_hash_value) {
                        candidate_block.id = Miner::calculate_final_block_id(&candidate_block);
                        return Some(candidate_block);
                    }

                    if current_nonce % 1_000_000 == 0 {
                        debug!(
                            "CPU mining on chain {}, current nonce chunk starting near: {}",
                            block_template.chain_id, current_nonce
                        );
                    }
                    None
                })
        });

        Ok(found_block_option)
    }

    #[instrument]
    fn calculate_final_block_id(block: &HyperBlock) -> String {
        let mut hasher = Keccak256::new();
        hasher.update(block.chain_id.to_le_bytes());
        hasher.update(block.merkle_root.as_bytes());
        hasher.update(block.timestamp.to_be_bytes());
        hasher.update(block.nonce.to_le_bytes());
        hasher.update(block.miner.as_bytes());
        for parent_id in &block.parents {
            hasher.update(parent_id.as_bytes());
        }
        hasher.update(block.difficulty.to_le_bytes());
        hex::encode(hasher.finalize())
    }

    #[instrument]
    fn calculate_pow_hash(block: &HyperBlock) -> Vec<u8> {
        let mut hasher = Keccak256::new();
        hasher.update(block.chain_id.to_le_bytes());
        hasher.update(block.merkle_root.as_bytes());
        hasher.update(block.timestamp.to_be_bytes());
        hasher.update(block.miner.as_bytes());
        for parent_id in &block.parents {
            hasher.update(parent_id.as_bytes());
        }
        hasher.update(block.difficulty.to_le_bytes());
        hasher.update(block.nonce.to_le_bytes());
        hasher.finalize().to_vec()
    }

    #[instrument]
    fn calculate_target_from_difficulty(difficulty_value: u64) -> Vec<u8> {
        let target_num = u64::MAX / difficulty_value.max(1);
        target_num.to_be_bytes().to_vec()
    }

    #[instrument]
    fn hash_meets_target(hash_bytes: &[u8], target_bytes: &[u8]) -> bool {
        let n_target = target_bytes.len();
        if hash_bytes.len() < n_target {
            return false;
        }
        hash_bytes[..n_target] <= *target_bytes
    }
}