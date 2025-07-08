// src/miner.rs

use crate::emission::Emission;
use crate::hyperdag::{HyperBlock, HyperDAG};
use crate::transaction::{Output, Transaction, DEV_ADDRESS, DEV_FEE_RATE};
use anyhow::Result;
use ed25519_dalek::SigningKey; // <-- ADD THIS IMPORT
use hex;
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
use tracing::{info, instrument, warn};

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
    #[error("Key conversion error")]
    KeyConversion,
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
        let is_genesis_chain = dag_lock
            .blocks
            .blocking_read()
            .values()
            .all(|b| !b.parents.is_empty());
        if tips.is_empty() && !is_genesis_chain {
            warn!("Attempted to mine with no tips on a non-genesis chain. Skipping round.");
            return Ok(None);
        }
        drop(dag_lock);

        let start_time = SystemTime::now();
        let timestamp = start_time.duration_since(UNIX_EPOCH)?.as_secs();

        let final_reward = self.emission.calculate_reward(timestamp)?;
        let dev_fee = (final_reward as f64 * DEV_FEE_RATE).round() as u64;
        let miner_reward = final_reward.saturating_sub(dev_fee);

        // --- FIX: Derive public key from private key for Homomorphic Encryption ---
        let signing_key = SigningKey::from_bytes(
            signing_key_bytes
                .try_into()
                .map_err(|_| MiningError::KeyConversion)?,
        );
        let public_key_bytes = signing_key.verifying_key().to_bytes();

        let mut coinbase_outputs = vec![Output {
            address: self.address.clone(),
            amount: miner_reward,
            homomorphic_encrypted: crate::hyperdag::HomomorphicEncrypted::new(
                miner_reward,
                &public_key_bytes, // Use public key
            ),
        }];
        if dev_fee > 0 {
            coinbase_outputs.push(Output {
                address: DEV_ADDRESS.to_string(),
                amount: dev_fee,
                homomorphic_encrypted: crate::hyperdag::HomomorphicEncrypted::new(
                    dev_fee,
                    &public_key_bytes, // Use public key
                ),
            });
        }

        let coinbase_tx = Transaction::new_coinbase(
            self.address.clone(),
            final_reward,
            signing_key_bytes,
            coinbase_outputs,
        )?;
        let mut block_transactions = vec![coinbase_tx];
        block_transactions.extend(transactions);

        let merkle_root = HyperBlock::compute_merkle_root(&block_transactions)?;

        let pre_hash_data = {
            let mut hasher = Keccak256::new();
            hasher.update(chain_id.to_le_bytes());
            hasher.update(merkle_root.as_bytes());
            hasher.update(timestamp.to_be_bytes());
            hasher.update(self.address.as_bytes());
            for parent in &tips {
                hasher.update(parent.as_bytes());
            }
            hasher.update(self.difficulty.to_le_bytes());
            hasher.finalize().to_vec()
        };

        if self.use_gpu {
            warn!("GPU mining is enabled, but the implementation currently only supports CPU mining. Falling back to CPU.");
        }

        let target_hash_bytes = Miner::calculate_target_from_difficulty(self.difficulty);
        let timeout_duration = Duration::from_secs(self.target_block_time);

        let mining_result = self.mine_cpu(
            &pre_hash_data,
            &target_hash_bytes,
            start_time,
            timeout_duration,
        )?;

        if let Some((found_nonce, effort)) = mining_result {
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
            final_block.lattice_signature = crate::hyperdag::LatticeSignature::sign(
                signing_key_bytes,
                &final_signature_payload,
            )?;
            final_block.id = hex::encode(Keccak256::digest(&final_signature_payload));

            let final_pow_hash = final_block.hash();
            if !Miner::hash_meets_target(&hex::decode(final_pow_hash).unwrap(), &target_hash_bytes)
            {
                warn!("Miner found a nonce but the final block hash was invalid. This may indicate a logic error in PoW hashing.");
                return Ok(None);
            }

            info!(
                "Mined valid block {} with nonce {} and effort {}. Reward: {} $HCN",
                final_block.id, found_nonce, effort, final_reward
            );

            return Ok(Some(final_block));
        }

        Ok(None)
    }

    fn mine_cpu(
        &self,
        block_header_data: &[u8],
        target_hash_value: &[u8],
        start_time: SystemTime,
        timeout_duration: Duration,
    ) -> Result<Option<(u64, u64)>, MiningError> {
        let thread_pool = rayon::ThreadPoolBuilder::new()
            .num_threads(self.threads)
            .build()?;
        let found_signal = Arc::new(AtomicBool::new(false));
        let hashes_tried = Arc::new(AtomicU64::new(0));

        let result = thread_pool.install(|| {
            let nonce_start = rand::thread_rng().gen::<u64>();

            (nonce_start..u64::MAX)
                .into_par_iter()
                .find_map_any(|current_nonce| {
                    if found_signal.load(Ordering::Relaxed) {
                        return None;
                    }
                    let count = hashes_tried.fetch_add(1, Ordering::Relaxed);

                    if count % 1_000_000 == 0 {
                        if let Ok(elapsed) = start_time.elapsed() {
                            if elapsed > timeout_duration {
                                found_signal.store(true, Ordering::Relaxed);
                                return None;
                            }
                        }
                    }

                    let mut hasher = Keccak256::new();
                    hasher.update(block_header_data);
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

    fn calculate_target_from_difficulty(difficulty_value: u64) -> Vec<u8> {
        if difficulty_value == 0 {
            return U256::MAX.to_big_endian_vec();
        }
        let diff = U256::from(difficulty_value);
        let max_target = U256::MAX;
        let target_num = max_target / diff;

        target_num.to_big_endian_vec()
    }

    fn hash_meets_target(hash_bytes: &[u8], target_bytes: &[u8]) -> bool {
        hash_bytes <= target_bytes
    }
}

// U256 struct and its methods are correct from the previous fix.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct U256([u64; 4]);

impl U256 {
    const ZERO: U256 = U256([0, 0, 0, 0]);
    const MAX: U256 = U256([u64::MAX, u64::MAX, u64::MAX, u64::MAX]);

    fn to_big_endian(self, bytes: &mut [u8; 32]) {
        bytes[0..8].copy_from_slice(&self.0[0].to_be_bytes());
        bytes[8..16].copy_from_slice(&self.0[1].to_be_bytes());
        bytes[16..24].copy_from_slice(&self.0[2].to_be_bytes());
        bytes[24..32].copy_from_slice(&self.0[3].to_be_bytes());
    }

    fn to_big_endian_vec(self) -> Vec<u8> {
        let mut buf = [0u8; 32];
        self.to_big_endian(&mut buf);
        buf.to_vec()
    }

    fn get_bit(&self, bit: usize) -> bool {
        if bit >= 256 {
            return false;
        }
        let word_index = bit / 64;
        let bit_in_word = bit % 64;
        (self.0[3 - word_index] >> bit_in_word) & 1 != 0
    }

    fn set_bit(&mut self, bit: usize) {
        if bit >= 256 {
            return;
        }
        let word_index = bit / 64;
        let bit_in_word = bit % 64;
        self.0[3 - word_index] |= 1 << bit_in_word;
    }

    fn shl_1(mut self) -> Self {
        let mut carry = 0;
        for i in (0..4).rev() {
            let next_carry = self.0[i] >> 63;
            self.0[i] = (self.0[i] << 1) | carry;
            carry = next_carry;
        }
        self
    }

    fn sub(self, rhs: Self) -> Self {
        let (d0, borrow) = self.0[3].overflowing_sub(rhs.0[3]);
        let (d1, borrow) = self.0[2].overflowing_sub(rhs.0[2].wrapping_add(borrow as u64));
        let (d2, borrow) = self.0[1].overflowing_sub(rhs.0[1].wrapping_add(borrow as u64));
        let (d3, _) = self.0[0].overflowing_sub(rhs.0[0].wrapping_add(borrow as u64));
        U256([d3, d2, d1, d0])
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
        self.div_rem(rhs).0
    }
}

impl U256 {
    fn div_rem(self, divisor: Self) -> (Self, Self) {
        if divisor == Self::ZERO {
            panic!("division by zero");
        }
        if self < divisor {
            return (Self::ZERO, self);
        }

        let mut quotient = Self::ZERO;
        let mut remainder = Self::ZERO;

        for i in (0..256).rev() {
            remainder = remainder.shl_1();
            if self.get_bit(i) {
                remainder.0[3] |= 1;
            }
            if remainder >= divisor {
                remainder = remainder.sub(divisor);
                quotient.set_bit(i);
            }
        }
        (quotient, remainder)
    }
}