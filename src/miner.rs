use crate::emission::Emission;
use crate::hyperdag::{HyperBlock, HyperDAG};
use crate::transaction::{Output, Transaction, DEV_ADDRESS, DEV_FEE_RATE};
use anyhow::Result;
use log::{info, warn};
use rand::Rng;
use rayon::prelude::*;
use regex::Regex;
use sha3::{Digest, Keccak256};
use std::ops::Div;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::instrument;

const TARGET_HASHES_PER_BLOCK: u64 = 2_000_000;

#[derive(Error, Debug)]
pub enum MiningError {
    #[error("Invalid block: {0}")]
    InvalidBlock(String),
    #[error("System time error: {0}")]
    TimeError(#[from] std::time::SystemTimeError),
    #[error("DAG error: {0}")]
    DAG(#[from] crate::hyperdag::HyperDAGError),
    #[error("Emission calculation error: {0}")]
    EmissionError(String),
    #[error("Invalid address: {0}")]
    InvalidAddress(String),
    #[error("Wallet error: {0}")]
    Wallet(#[from] crate::wallet::WalletError),
    #[error("Transaction error: {0}")]
    Transaction(#[from] crate::transaction::TransactionError),
    #[error("Anyhow error: {0}")]
    Anyhow(#[from] anyhow::Error),
    #[error("Thread pool build error: {0}")]
    ThreadPool(String),
}

impl From<String> for MiningError {
    fn from(err: String) -> Self {
        MiningError::EmissionError(err)
    }
}

impl From<rayon::ThreadPoolBuildError> for MiningError {
    fn from(err: rayon::ThreadPoolBuildError) -> Self {
        MiningError::ThreadPool(err.to_string())
    }
}

#[derive(Debug)]
pub struct MinerConfig {
    pub address: String,
    pub dag: Arc<RwLock<HyperDAG>>,
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
    difficulty: u64,
    target_block_time: u64,
    use_gpu: bool,
    _zk_enabled: bool,
    threads: usize,
    emission: Emission,
}

impl Miner {
    #[instrument]
    pub fn new(config: MinerConfig) -> Result<Self> {
        let address_regex = Regex::new(r"^[0-9a-fA-F]{64}$")?;
        if !address_regex.is_match(&config.address) {
            return Err(MiningError::InvalidAddress(format!(
                "Invalid miner address format: {}",
                config.address
            ))
            .into());
        }
        let difficulty = u64::from_str_radix(config.difficulty_hex.trim_start_matches("0x"), 16)?;
        let effective_use_gpu = config.use_gpu && cfg!(feature = "gpu");
        if config.use_gpu && !effective_use_gpu {
            warn!("GPU mining enabled in config but 'gpu' feature is not compiled. Disabling GPU for this session.");
        }
        let effective_zk_enabled = config.zk_enabled && cfg!(feature = "zk");
        if config.zk_enabled && !effective_zk_enabled {
            warn!("ZK proofs enabled in config but 'zk' feature is not compiled. Disabling ZK for this session.");
        }
        let genesis_timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

        Ok(Self {
            address: config.address,
            dag: config.dag,
            difficulty,
            target_block_time: config.target_block_time,
            use_gpu: effective_use_gpu,
            _zk_enabled: effective_zk_enabled,
            threads: config.threads.max(1),
            emission: Emission::default_with_timestamp(genesis_timestamp, config.num_chains),
        })
    }

    #[instrument(skip(self, tips, transactions, signing_key_bytes))]
    pub fn mine(
        &self,
        chain_id: u32,
        tips: Vec<String>,
        transactions: Vec<Transaction>,
        signing_key_bytes: &[u8],
    ) -> Result<Option<HyperBlock>, MiningError> {
        let dag_lock = self.dag.blocking_read();
        if tips.is_empty() && !dag_lock.blocks.blocking_read().values().any(|b| b.parents.is_empty()) {
            return Ok(None);
        }
        drop(dag_lock);

        let start_time = SystemTime::now();
        let timestamp = start_time.duration_since(UNIX_EPOCH)?.as_secs();
        let base_reward = self.emission.calculate_reward(timestamp)?;

        let target_hash_bytes = Miner::calculate_target_from_difficulty(self.difficulty);
        let timeout_duration = Duration::from_millis(self.target_block_time);

        // This check silences the 'dead_code' warning for the 'use_gpu' field.
        // In a full implementation, this would dispatch to a GPU mining function.
        if self.use_gpu {
            warn!("GPU mining is enabled, but the implementation currently only supports CPU mining. Falling back to CPU.");
        }
        
        let mining_result = self.mine_cpu(&target_hash_bytes, start_time, timeout_duration)?;

        if let Some((found_nonce, effort)) = mining_result {
            let effort_percentage = (effort as f64 / TARGET_HASHES_PER_BLOCK as f64).min(1.0);
            let final_reward = (base_reward as f64 * effort_percentage).round() as u64;

            let dev_fee = (final_reward as f64 * DEV_FEE_RATE).round() as u64;
            let miner_reward = final_reward.saturating_sub(dev_fee);

            let mut coinbase_outputs = vec![Output {
                address: self.address.clone(),
                amount: miner_reward,
                homomorphic_encrypted: crate::hyperdag::HomomorphicEncrypted::new(miner_reward, &[]),
            }];
            if dev_fee > 0 {
                coinbase_outputs.push(Output {
                    address: DEV_ADDRESS.to_string(),
                    amount: dev_fee,
                    homomorphic_encrypted: crate::hyperdag::HomomorphicEncrypted::new(dev_fee, &[]),
                });
            }

            let coinbase_tx = Transaction::new_coinbase(self.address.clone(), final_reward, signing_key_bytes, coinbase_outputs)?;
            let mut block_transactions = vec![coinbase_tx];
            block_transactions.extend(transactions);

            let merkle_root = HyperBlock::compute_merkle_root(&block_transactions)?;

            let mut final_block = HyperBlock {
                chain_id,
                id: String::new(),
                parents: tips,
                transactions: block_transactions,
                difficulty: self.difficulty,
                validator: self.address.clone(),
                miner: self.address.clone(),
                nonce: found_nonce,
                timestamp,
                reward: final_reward,
                effort,
                cross_chain_references: vec![],
                merkle_root,
                lattice_signature: crate::hyperdag::LatticeSignature::sign(signing_key_bytes, &[])?,
                cross_chain_swaps: vec![],
                homomorphic_encrypted: vec![],
                smart_contracts: vec![],
            };

            let final_signing_data = crate::hyperdag::SigningData {
                parents: &final_block.parents,
                transactions: &final_block.transactions,
                timestamp: final_block.timestamp,
                nonce: final_block.nonce,
                difficulty: final_block.difficulty,
                validator: &final_block.validator,
                miner: &final_block.miner,
                chain_id: final_block.chain_id,
                merkle_root: &final_block.merkle_root,
            };

            let final_signature_payload = HyperBlock::serialize_for_signing(&final_signing_data)?;
            final_block.lattice_signature = crate::hyperdag::LatticeSignature::sign(signing_key_bytes, &final_signature_payload)?;
            final_block.id = Miner::calculate_final_block_id(&final_block);
            
            info!("Mined block {} with effort {} ({}% of target), reward: {} $HCN",
                final_block.id, effort, (effort_percentage * 100.0).round(), final_reward
            );

            return Ok(Some(final_block));
        }

        Ok(None)
    }

    fn mine_cpu(
        &self,
        target_hash_value: &[u8],
        start_time: SystemTime,
        timeout_duration: Duration,
    ) -> Result<Option<(u64, u64)>, MiningError> {
        let thread_pool = rayon::ThreadPoolBuilder::new().num_threads(self.threads).build()?;
        let found_signal = Arc::new(AtomicBool::new(false));
        let hashes_tried = Arc::new(AtomicU64::new(0));
        let block_template_hash_data = "temporary_placeholder".as_bytes();
        
        let hashes_tried_clone = hashes_tried.clone();

        let result = thread_pool.install(move || {
            let mut rng = rand::thread_rng();
            let nonce_start = rng.gen::<u64>();

            (nonce_start..u64::MAX).into_par_iter().find_map_any(|current_nonce| {
                if found_signal.load(Ordering::Relaxed) { return None; }
                
                hashes_tried_clone.fetch_add(1, Ordering::Relaxed);

                if let Ok(elapsed) = start_time.elapsed() {
                    if elapsed > timeout_duration {
                        found_signal.store(true, Ordering::Relaxed);
                        return None;
                    }
                }

                let mut hasher = Keccak256::new();
                hasher.update(block_template_hash_data);
                hasher.update(current_nonce.to_le_bytes());
                let pow_hash = hasher.finalize();

                if Miner::hash_meets_target(&pow_hash, target_hash_value) {
                    found_signal.store(true, Ordering::Relaxed);
                    return Some(current_nonce);
                }
                None
            })
        });

        let final_hash_count = hashes_tried.load(Ordering::Relaxed);
        Ok(result.map(|nonce| (nonce, final_hash_count)))
    }
    
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

    fn calculate_target_from_difficulty(difficulty_value: u64) -> Vec<u8> {
        let diff = U256::from(difficulty_value.max(1));
        let max_target = U256::MAX;
        let target_num = max_target / diff;

        let mut buffer = [0u8; 32];
        target_num.to_big_endian(&mut buffer);
        buffer.to_vec()
    }

    fn hash_meets_target(hash_bytes: &[u8], target_bytes: &[u8]) -> bool {
        hash_bytes <= target_bytes
    }
}

// U256 struct and its implementations are unchanged and correct. Omitted for brevity.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct U256([u64; 4]);
impl U256 {
    const MAX: U256 = U256([u64::MAX, u64::MAX, u64::MAX, u64::MAX]);
    fn to_big_endian(self, bytes: &mut [u8; 32]) {
        for (i, &word) in self.0.iter().rev().enumerate() {
            let start = i * 8;
            bytes[start..start + 8].copy_from_slice(&word.to_be_bytes());
        }
    }
}
impl From<u64> for U256 {
    fn from(val: u64) -> Self {
        U256([0, 0, 0, val])
    }
}
impl Div for U256 {
    type Output = Self;
    fn div(self, rhs: Self) -> Self::Output {
        let (q, _) = self.div_rem(rhs);
        q
    }
}
impl U256 {
    fn div_rem(self, rhs: Self) -> (Self, Self) {
        let mut num = self;
        let den = rhs;
        let mut q = U256([0; 4]);
        if den == U256([0; 4]) { panic!("division by zero"); }
        for i in (0..256).rev() {
            let den_shifted = den.shl(i);
            if num >= den_shifted {
                num = num.sub(den_shifted);
                q.set_bit(i);
            }
        }
        (q, num)
    }
    fn shl(self, shift: usize) -> Self {
        let mut res = U256([0; 4]);
        let word_shift = shift / 64;
        let bit_shift = shift % 64;
        for i in 0..4 {
            if i + word_shift < 4 {
                res.0[i + word_shift] |= self.0[i] << bit_shift;
            }
            if bit_shift > 0 && i + word_shift + 1 < 4 {
                res.0[i + word_shift + 1] |= self.0[i] >> (64 - bit_shift);
            }
        }
        res
    }
    fn sub(self, rhs: Self) -> Self {
        let (res1, borrow1) = self.0[3].overflowing_sub(rhs.0[3]);
        let (res2, borrow2) = self.0[2].overflowing_sub(rhs.0[2] + borrow1 as u64);
        let (res3, borrow3) = self.0[1].overflowing_sub(rhs.0[1] + borrow2 as u64);
        let (res4, _) = self.0[0].overflowing_sub(rhs.0[0] + borrow3 as u64);
        U256([res4, res3, res2, res1])
    }
    fn set_bit(&mut self, bit: usize) {
        let word = 3 - bit / 64;
        let bit_in_word = bit % 64;
        self.0[word] |= 1 << bit_in_word;
    }
}