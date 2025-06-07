use crate::emission::Emission;
use crate::mempool::Mempool;
use crate::transaction::Transaction;
use hex;
use log::{error, info, warn};
use lru::LruCache;
use prometheus::{register_int_counter, IntCounter};
use rayon::prelude::*;
use rocksdb::{DB, Options};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::instrument;
use rand::Rng;

const MAX_BLOCK_SIZE: usize = 20_000_000;
pub const MAX_TRANSACTIONS_PER_BLOCK: usize = 25_000;
const DEV_ADDRESS: &str = "2119707c4caf16139cfb5c09c4dcc9bf9cfe6808b571c108d739f49cc14793b9";
const DEV_FEE_RATE: f64 = 0.0304;
const FINALIZATION_DEPTH: u64 = 8;
const SHARD_THRESHOLD: u32 = 3;
const TEMPORAL_CONSENSUS_WINDOW: u64 = 600;
const MAX_BLOCKS_PER_MINUTE: u64 = 15;
const MIN_VALIDATOR_STAKE: u64 = 50;
const SLASHING_PENALTY: u64 = 30;
const CACHE_SIZE: usize = 1_000;

lazy_static::lazy_static! {
    static ref BLOCKS_PROCESSED: IntCounter = register_int_counter!("blocks_processed_total", "Total blocks processed").unwrap();
    static ref TRANSACTIONS_PROCESSED: IntCounter = register_int_counter!("transactions_processed_total", "Total transactions processed").unwrap();
    static ref ANOMALIES_DETECTED: IntCounter = register_int_counter!("anomalies_detected_total", "Total anomalies detected").unwrap();
}

#[derive(Error, Debug)]
pub enum HyperDAGError {
    #[error("Invalid block: {0}")]
    InvalidBlock(String),
    #[error("Invalid transaction: {0}")]
    InvalidTransaction(String),
    #[error("Invalid parent: {0}")]
    InvalidParent(String),
    #[error("Cross-chain reference error: {0}")]
    CrossChainReferenceError(String),
    #[error("Reward mismatch: expected {0}, got {1}")]
    RewardMismatch(u64, u64),
    #[error("System time error")]
    TimeError,
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("ZKP verification failed")]
    ZKPVerification,
    #[error("Governance proposal failed: {0}")]
    Governance(String),
    #[error("Lattice signature verification failed")]
    LatticeSignatureVerification,
    #[error("Homomorphic encryption error: {0}")]
    HomomorphicError(String),
    #[error("IDS anomaly detected: {0}")]
    IDSAnomaly(String),
    #[error("BFT consensus failure: {0}")]
    BFTFailure(String),
    #[error("Smart contract execution failed: {0}")]
    SmartContractError(String),
    #[error("Cross-chain atomic swap failed: {0}")]
    CrossChainSwapError(String),
    #[error("Database error: {0}")]
    DatabaseError(String),
    #[error("Emission calculation error: {0}")]
    EmissionError(String),
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct LatticeSignature { // Made pub
    pub public_key: Vec<u8>,
    pub signature: Vec<u8>,
}

impl LatticeSignature {
    #[instrument]
    pub fn new(signing_key: &[u8]) -> Self {
        let mut public_key = signing_key.to_vec(); 
        for _ in 0..32 { 
            let mut hasher = Keccak256::new();
            hasher.update(&public_key);
            let hash = hasher.finalize();
            public_key.extend_from_slice(hash.as_slice()); 
        }
        let mut signature = vec![0u8; 64]; 
        for (i, byte) in signing_key.iter().enumerate() {
            signature[i % 64] ^= *byte;
        }
        Self { public_key: public_key[..32.min(public_key.len())].to_vec(), signature }
    }

    #[instrument]
    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        let mut hasher = Keccak256::new();
        hasher.update(message);
        hasher.update(&self.public_key); 
        let hash = hasher.finalize();
        let mut signed = self.signature.clone(); 
        for (i, byte) in hash.iter().enumerate() {
            signed[i % 64] ^= *byte;
        }
        signed
    }

    #[instrument]
    pub fn verify(&self, message: &[u8], signature_to_verify: &[u8]) -> bool {
        let mut hasher = Keccak256::new();
        hasher.update(message);
        hasher.update(&self.public_key);
        let hash = hasher.finalize();
        let mut expected_signature = self.signature.clone(); 
        for (i, byte) in hash.iter().enumerate() {
            expected_signature[i % 64] ^= *byte;
        }
        signature_to_verify == &expected_signature
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct HomomorphicEncrypted { // Made pub
    pub encrypted_amount: String,
}

impl HomomorphicEncrypted {
    #[instrument]
    pub fn new(amount: u64, public_key_material: &[u8]) -> Self {
        let mut hasher = Keccak256::new();
        hasher.update(amount.to_be_bytes());
        hasher.update(public_key_material);
        let encrypted = hex::encode(hasher.finalize());
        Self { encrypted_amount: encrypted }
    }

    #[instrument]
    pub fn decrypt(&self, _private_key_material: &[u8]) -> Result<u64, HyperDAGError> {
        if self.encrypted_amount == hex::encode(Keccak256::digest(&0u64.to_be_bytes())) {
            Ok(0)
        } else {
            Err(HyperDAGError::HomomorphicError("Placeholder decryption cannot recover original value.".to_string()))
        }
    }

    #[instrument]
    pub fn add(&self, other: &Self) -> Result<Self, HyperDAGError> {
        let mut hasher = Keccak256::new();
        hasher.update(self.encrypted_amount.as_bytes());
        hasher.update(other.encrypted_amount.as_bytes());
        let sum = hex::encode(hasher.finalize());
        Ok(Self { encrypted_amount: sum })
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct CrossChainSwap { // Made pub
    pub swap_id: String,
    pub source_chain: u32,
    pub target_chain: u32,
    pub source_block_id: String,
    pub target_block_id: String, 
    pub amount: u64,
    pub initiator: String, 
    pub responder: String, 
    pub timelock: u64,     
    pub state: SwapState,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub enum SwapState { // Made pub
    Initiated, 
    Accepted,  
    Completed, 
    Refunded,  
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct SmartContract { // Made pub
    pub contract_id: String,
    pub code: String, 
    pub storage: HashMap<String, String>, 
    pub owner: String,   
}

impl SmartContract {
    #[instrument]
    pub fn execute(&mut self, input: &str) -> Result<String, HyperDAGError> {
        if self.code.contains("echo") {
            self.storage.insert("last_input".to_string(), input.to_string());
            Ok(format!("echo: {}", input))
        } else if self.code.contains("increment_counter") {
            let counter = self.storage.entry("counter".to_string()).or_insert_with(|| "0".to_string());
            let current_val: u64 = counter.parse().unwrap_or(0);
            *counter = (current_val + 1).to_string();
            Ok(format!("counter updated to: {}", counter))
        }
        else {
            Err(HyperDAGError::SmartContractError("Unsupported contract code or execution logic".to_string()))
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct HyperBlock {
    pub chain_id: u32,
    pub id: String, 
    pub parents: Vec<String>, 
    pub transactions: Vec<Transaction>,
    pub difficulty: u64, 
    pub validator: String, 
    pub miner: String,     
    pub nonce: u64,        
    pub timestamp: u64,    
    pub reward: u64,       
    pub cross_chain_references: Vec<(u32, String)>, 
    pub cross_chain_swaps: Vec<CrossChainSwap>,
    pub merkle_root: String, 
    pub lattice_signature: LatticeSignature, 
    pub homomorphic_encrypted: Vec<HomomorphicEncrypted>, 
    pub smart_contracts: Vec<SmartContract>, 
}

impl HyperBlock {
    #[instrument]
    pub fn new(
        chain_id: u32,
        parents: Vec<String>,
        transactions: Vec<Transaction>,
        difficulty: u64,
        validator: String, 
        miner: String,     
        signing_key_material: &[u8], 
    ) -> Result<Self, HyperDAGError> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| HyperDAGError::TimeError)?
            .as_secs();
        let nonce = 0; 
        
        let merkle_root = Self::compute_merkle_root(&transactions)?;
        
        let lattice_signer = LatticeSignature::new(signing_key_material); 
        let public_key_for_signature = lattice_signer.public_key.clone();

        let pre_signature_data_for_id = Self::serialize_for_signing(
            &parents, 
            &transactions,
            timestamp, 
            nonce, 
            difficulty, 
            &validator, 
            &miner, 
            chain_id,
            &merkle_root, 
        )?;
        let id = hex::encode(Keccak256::digest(&pre_signature_data_for_id));

        let final_signature_payload = Self::serialize_for_signing(
            &parents, &transactions, timestamp, nonce, difficulty, &validator, &miner, chain_id, &merkle_root
        )?;
        let signature_bytes = lattice_signer.sign(&final_signature_payload);

        let homomorphic_encrypted_data = transactions
            .iter()
            .map(|tx| HomomorphicEncrypted::new(tx.amount, &public_key_for_signature))
            .collect();

        Ok(Self {
            chain_id, id, parents, transactions, difficulty, validator, miner, nonce, timestamp,
            reward: 0, 
            cross_chain_references: vec![], cross_chain_swaps: vec![], merkle_root,
            lattice_signature: LatticeSignature {
                public_key: public_key_for_signature,
                signature: signature_bytes,
            },
            homomorphic_encrypted: homomorphic_encrypted_data,
            smart_contracts: vec![],
        })
    }

    fn serialize_for_signing(
        parents: &[String],
        _transactions: &[Transaction], 
        timestamp: u64,
        nonce: u64,
        difficulty: u64,
        validator: &str,
        miner: &str,
        chain_id: u32,
        merkle_root: &str,
    ) -> Result<Vec<u8>, HyperDAGError> {
        let mut hasher = Keccak256::new();
        hasher.update(chain_id.to_le_bytes());
        hasher.update(merkle_root.as_bytes());
        for parent in parents {
            hasher.update(parent.as_bytes());
        }
        hasher.update(timestamp.to_be_bytes());
        hasher.update(nonce.to_be_bytes());
        hasher.update(difficulty.to_be_bytes());
        hasher.update(validator.as_bytes());
        hasher.update(miner.as_bytes());
        Ok(hasher.finalize().to_vec())
    }

    #[instrument]
    pub fn compute_merkle_root(transactions: &[Transaction]) -> Result<String, HyperDAGError> {
        if transactions.is_empty() {
            return Ok(hex::encode(Keccak256::digest(&[])));
        }
        let mut leaves: Vec<Vec<u8>> = transactions
            .par_iter()
            .map(|tx| Keccak256::digest(tx.id.as_bytes()).to_vec())
            .collect();
        
        if leaves.is_empty() {
             return Ok(hex::encode(Keccak256::digest(&[])));
        }

        while leaves.len() > 1 {
            if leaves.len() % 2 != 0 {
                leaves.push(leaves.last().unwrap().clone());
            }
            leaves = leaves
                .chunks_exact(2)
                .map(|chunk| {
                    let mut hasher = Keccak256::new();
                    hasher.update(&chunk[0]);
                    hasher.update(&chunk[1]);
                    hasher.finalize().to_vec()
                })
                .collect();
        }
        Ok(hex::encode(leaves.first().ok_or_else(|| HyperDAGError::InvalidBlock("Merkle root computation failed".to_string()))?))
    }

    #[instrument]
    pub fn hash(&self) -> String { 
        let mut hasher = Keccak256::new();
        hasher.update(self.chain_id.to_le_bytes());
        hasher.update(self.merkle_root.as_bytes());
        hasher.update(self.timestamp.to_be_bytes());
        hasher.update(self.miner.as_bytes());
        for parent_id in &self.parents {
            hasher.update(parent_id.as_bytes());
        }
        hasher.update(self.difficulty.to_le_bytes());
        hasher.update(self.nonce.to_le_bytes());
        hex::encode(hasher.finalize())
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct GovernanceProposal { // Made pub
    pub proposal_id: String,
    pub proposer: String,
    pub description: String,
    pub votes_for: u64,
    pub votes_against: u64,
    pub active: bool,
}

#[derive(Debug)]
pub struct HyperDAG {
    pub blocks: Arc<RwLock<HashMap<String, HyperBlock>>>,
    pub tips: Arc<RwLock<HashMap<u32, HashSet<String>>>>,
    pub validators: Arc<RwLock<HashMap<String, u64>>>,
    pub target_block_time: u64,
    pub difficulty: Arc<RwLock<u64>>,
    pub emission: Emission,
    pub num_chains: Arc<RwLock<u32>>,
    pub finalized_blocks: Arc<RwLock<HashSet<String>>>,
    pub governance_proposals: Arc<RwLock<HashMap<String, GovernanceProposal>>>,
    pub chain_loads: Arc<RwLock<HashMap<u32, u64>>>,
    pub difficulty_history: Arc<RwLock<Vec<(u64, u64)>>>, 
    pub block_creation_timestamps: Arc<RwLock<HashMap<String, u64>>>, 
    pub anomaly_history: Arc<RwLock<HashMap<String, u64>>>, 
    pub cross_chain_swaps: Arc<RwLock<HashMap<String, CrossChainSwap>>>,
    pub smart_contracts: Arc<RwLock<HashMap<String, SmartContract>>>,
    pub cache: Arc<RwLock<LruCache<String, HyperBlock>>>, 
    pub db: Arc<DB>, 
}

impl HyperDAG {
    #[instrument]
    pub async fn new(
        initial_validator: &str,
        target_block_time: u64,
        difficulty: u64,
        num_chains: u32,
        signing_key: &[u8],
    ) -> Result<Self, HyperDAGError> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        let db = Arc::new(
            DB::open(&opts, "hyperdag_db").map_err(|e| HyperDAGError::DatabaseError(e.to_string()))?,
        );
        let mut blocks_map = HashMap::new();
        let mut tips_map = HashMap::new();
        let mut validators_map = HashMap::new();
        let genesis_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| HyperDAGError::TimeError)?
            .as_secs();

        for chain_id_val in 0..num_chains {
            let mut genesis_block = HyperBlock::new(
                chain_id_val, vec![], vec![], difficulty, 
                initial_validator.to_string(), initial_validator.to_string(), signing_key,
            )?;
            genesis_block.reward = 0;
            let genesis_id = genesis_block.id.clone();

            blocks_map.insert(genesis_id.clone(), genesis_block);
            tips_map.entry(chain_id_val).or_insert_with(HashSet::new).insert(genesis_id);
        }
        validators_map.insert(initial_validator.to_string(), MIN_VALIDATOR_STAKE * num_chains as u64 * 2);

        Ok(Self {
            blocks: Arc::new(RwLock::new(blocks_map)),
            tips: Arc::new(RwLock::new(tips_map)),
            validators: Arc::new(RwLock::new(validators_map)),
            target_block_time,
            difficulty: Arc::new(RwLock::new(difficulty.max(1))),
            emission: Emission::default_with_timestamp(genesis_timestamp, num_chains),
            num_chains: Arc::new(RwLock::new(num_chains.max(1))),
            finalized_blocks: Arc::new(RwLock::new(HashSet::new())),
            governance_proposals: Arc::new(RwLock::new(HashMap::new())),
            chain_loads: Arc::new(RwLock::new(HashMap::new())),
            difficulty_history: Arc::new(RwLock::new(Vec::new())),
            block_creation_timestamps: Arc::new(RwLock::new(HashMap::new())),
            anomaly_history: Arc::new(RwLock::new(HashMap::new())),
            cross_chain_swaps: Arc::new(RwLock::new(HashMap::new())),
            smart_contracts: Arc::new(RwLock::new(HashMap::new())),
            cache: Arc::new(RwLock::new(LruCache::new(NonZeroUsize::new(CACHE_SIZE.max(1)).unwrap()))),
            db,
        })
    }

    #[instrument]
    pub async fn get_id(&self) -> u32 { 
        0 // This seems to be a placeholder; typically might return a node ID or similar.
    }

    #[instrument]
    pub async fn get_tips(&self, chain_id: u32) -> Option<Vec<String>> {
        self.tips.read().await.get(&chain_id).map(|tips_set| tips_set.iter().cloned().collect())
    }

    #[instrument]
    pub async fn add_validator(&self, address: String, stake: u64) { // Changed to take &self
        let mut validators_guard = self.validators.write().await;
        validators_guard.insert(address, stake.max(MIN_VALIDATOR_STAKE));
    }

    #[instrument]
    pub async fn initiate_cross_chain_swap(
        &self, // Changed to take &self
        source_chain: u32,
        target_chain: u32,
        source_block_id: String,
        amount: u64,
        initiator: String,
        responder: String,
        timelock_duration: u64,
    ) -> Result<String, HyperDAGError> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| HyperDAGError::TimeError)?.as_secs();
        let swap_id = hex::encode(Keccak256::digest(format!("swap_{}_{}_{}_{}", initiator, responder, amount, now).as_bytes()));
        let swap = CrossChainSwap {
            swap_id: swap_id.clone(), source_chain, target_chain, source_block_id,
            target_block_id: String::new(), amount, initiator, responder,
            timelock: now + timelock_duration, state: SwapState::Initiated,
        };
        self.cross_chain_swaps.write().await.insert(swap_id.clone(), swap);
        Ok(swap_id)
    }

    #[instrument]
    pub async fn accept_cross_chain_swap(&self, swap_id: String, target_block_id: String) -> Result<(), HyperDAGError> { // Changed to take &self
        let mut swaps_guard = self.cross_chain_swaps.write().await;
        let swap = swaps_guard.get_mut(&swap_id)
            .ok_or_else(|| HyperDAGError::CrossChainSwapError(format!("Swap ID {} not found", swap_id)))?;
        if swap.state != SwapState::Initiated {
            return Err(HyperDAGError::CrossChainSwapError(format!("Swap {} is not in Initiated state, current state: {:?}", swap_id, swap.state)));
        }
        swap.target_block_id = target_block_id;
        swap.state = SwapState::Accepted;
        Ok(())
    }
    
    #[instrument]
    pub async fn deploy_smart_contract(&self, code: String, owner: String) -> Result<String, HyperDAGError> { // Changed to take &self
        let contract_id = hex::encode(Keccak256::digest(code.as_bytes()));
        let contract = SmartContract { contract_id: contract_id.clone(), code, storage: HashMap::new(), owner };
        self.smart_contracts.write().await.insert(contract_id.clone(), contract);
        Ok(contract_id)
    }

    #[instrument]
    pub async fn create_candidate_block(
        &self, // Changed to take &self
        validator_signing_key: &[u8],
        validator_address: &str,
        mempool_arc: &Arc<RwLock<Mempool>>,
        utxos_arc: &Arc<RwLock<HashMap<String, crate::transaction::UTXO>>>,
        chain_id_val: u32,
    ) -> Result<HyperBlock, HyperDAGError> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| HyperDAGError::TimeError)?.as_secs();

        {
            let mut timestamps_guard = self.block_creation_timestamps.write().await;
            let recent_blocks = timestamps_guard.values().filter(|&&t| now.saturating_sub(t) < 60).count() as u64;
            if recent_blocks >= MAX_BLOCKS_PER_MINUTE {
                return Err(HyperDAGError::InvalidBlock(format!("Rate limit exceeded: {} blocks in last minute", recent_blocks)));
            }
            if timestamps_guard.len() > 1000 {
                timestamps_guard.retain(|_, t_val| now.saturating_sub(*t_val) < 3600);
            }
        }

        {
            let validators_guard = self.validators.read().await;
            let stake = validators_guard.get(validator_address)
                .ok_or_else(|| HyperDAGError::InvalidBlock(format!("Validator {} not found or no stake", validator_address)))?;
            if *stake < MIN_VALIDATOR_STAKE {
                return Err(HyperDAGError::InvalidBlock(format!("Insufficient stake for validator {}: {} < {}", validator_address, stake, MIN_VALIDATOR_STAKE)));
            }
        }

        let selected_transactions = {
            let mempool_guard = mempool_arc.read().await;
            let utxos_guard_inner = utxos_arc.read().await;
            mempool_guard.select_transactions(self, &utxos_guard_inner, MAX_TRANSACTIONS_PER_BLOCK).await
        };

        let parent_tips: Vec<String> = {
            let tips_guard = self.tips.read().await;
            tips_guard.get(&chain_id_val).cloned().unwrap_or_default().into_iter().collect()
        };
        
        let reward = self.emission.calculate_reward(now).map_err(HyperDAGError::EmissionError)?;
        let dev_fee = (reward as f64 * DEV_FEE_RATE) as u64;
        let miner_reward = reward.saturating_sub(dev_fee);
        
        let temp_lattice_signer = LatticeSignature::new(validator_signing_key);
        let reward_outputs = vec![
            crate::transaction::Output {
                address: validator_address.to_string(),
                amount: miner_reward,
                homomorphic_encrypted: HomomorphicEncrypted::new(miner_reward, &temp_lattice_signer.public_key),
            },
            crate::transaction::Output {
                address: DEV_ADDRESS.to_string(),
                amount: dev_fee,
                homomorphic_encrypted: HomomorphicEncrypted::new(dev_fee, &temp_lattice_signer.public_key),
            },
        ];
        let reward_tx = Transaction {
            id: hex::encode(Keccak256::digest(format!("coinbase_{}_{}", now, chain_id_val).as_bytes())),
            sender: validator_address.to_string(), 
            receiver: validator_address.to_string(),
            amount: reward, fee: 0, inputs: vec![], outputs: reward_outputs,
            lattice_signature: temp_lattice_signer.sign(&now.to_be_bytes()),
            public_key: temp_lattice_signer.public_key.clone(),
            timestamp: now,
        };

        let mut transactions_for_block = vec![reward_tx];
        transactions_for_block.extend(selected_transactions);

        let mut cross_chain_references = vec![];
        let num_chains_val = *self.num_chains.read().await;
        if num_chains_val > 1 {
            let prev_chain = (chain_id_val + num_chains_val - 1) % num_chains_val;
            let next_chain = (chain_id_val + 1) % num_chains_val;
            let tips_guard = self.tips.read().await;
            if let Some(prev_tips_set) = tips_guard.get(&prev_chain) {
                if let Some(tip_val) = prev_tips_set.iter().next() { cross_chain_references.push((prev_chain, tip_val.clone())); }
            }
            if prev_chain != next_chain { // Avoid double-adding if only 2 chains
                if let Some(next_tips_set) = tips_guard.get(&next_chain) {
                     if let Some(tip_val) = next_tips_set.iter().next() { cross_chain_references.push((next_chain, tip_val.clone())); }
                }
            }
        }
        
        let current_difficulty = *self.difficulty.read().await;
        let mut block = HyperBlock::new(
            chain_id_val, parent_tips, transactions_for_block, current_difficulty, 
            validator_address.to_string(), validator_address.to_string(), 
            validator_signing_key
        )?;
        block.cross_chain_references = cross_chain_references;
        block.reward = reward;

        self.block_creation_timestamps.write().await.insert(block.id.clone(), now);
        
        {
            let mut chain_loads_guard = self.chain_loads.write().await;
            *chain_loads_guard.entry(chain_id_val).or_insert(0) += block.transactions.len() as u64;
        }
        Ok(block)
    }

    #[instrument]
    pub async fn is_valid_block(
        &self, 
        block: &HyperBlock, 
        utxos_arc: &Arc<RwLock<HashMap<String, crate::transaction::UTXO>>>
    ) -> Result<bool, HyperDAGError> {
        let serialized_size = serde_json::to_vec(&block)?.len();
        if block.transactions.len() > MAX_TRANSACTIONS_PER_BLOCK || serialized_size > MAX_BLOCK_SIZE {
            return Err(HyperDAGError::InvalidBlock(format!("Block exceeds size limits: {} txns, {} bytes", block.transactions.len(), serialized_size)));
        }

        if HyperBlock::compute_merkle_root(&block.transactions)? != block.merkle_root {
            return Err(HyperDAGError::InvalidBlock("Invalid merkle root".to_string()));
        }
        
        let signature_data = HyperBlock::serialize_for_signing(
            &block.parents, &block.transactions, block.timestamp, block.nonce,
            block.difficulty, &block.validator, &block.miner, block.chain_id, &block.merkle_root
        )?;
        if !block.lattice_signature.verify(&signature_data, &block.lattice_signature.signature) {
            return Err(HyperDAGError::LatticeSignatureVerification);
        }

        // These variables were marked as unused, if their checks are important, they should be re-enabled.
        // let _total_encrypted = block
        //     .homomorphic_encrypted
        //     .iter()
        //     .try_fold(HomomorphicEncrypted::new(0, &block.lattice_signature.public_key), |acc, x| acc.add(x))?;
        // let _total_decrypted_tx_amount = block.transactions.iter().map(|tx| tx.amount).sum::<u64>();

        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| HyperDAGError::TimeError)?.as_secs();
        if block.timestamp > now + TEMPORAL_CONSENSUS_WINDOW || block.timestamp < now.saturating_sub(TEMPORAL_CONSENSUS_WINDOW) {
            return Err(HyperDAGError::InvalidBlock(format!("Timestamp {} outside consensus window (now: {})", block.timestamp, now)));
        }

        let anomaly_score = self.detect_anomaly(block).await?;
        if anomaly_score > 0.7 {
            let mut anomaly_history_guard = self.anomaly_history.write().await;
            let count = anomaly_history_guard.entry(block.id.clone()).or_insert(0);
            *count += 1;
            if *count > 3 {
                ANOMALIES_DETECTED.inc();
                return Err(HyperDAGError::IDSAnomaly(format!("Multiple anomalies ({}) detected for block {}", count, block.id)));
            }
        }
        
        // let _current_difficulty = *self.difficulty.read().await; // Unused
        let target = u64::MAX / block.difficulty.max(1); 
        let block_pow_hash = block.hash(); 
        let hash_value_str = &block_pow_hash[..std::cmp::min(16, block_pow_hash.len())]; 
        let hash_value = u64::from_str_radix(hash_value_str, 16).unwrap_or(u64::MAX);
        if hash_value > target {
            return Err(HyperDAGError::InvalidBlock(format!("Difficulty not met. Hash value {} > target {}", hash_value, target)));
        }

        { 
            let blocks_guard = self.blocks.read().await;
            if !block.parents.is_empty() {
                // let mut _validator_count_on_parents = 0; // Unused
                for parent_id_val in &block.parents {
                    let parent_block = blocks_guard.get(parent_id_val)
                        .ok_or_else(|| HyperDAGError::InvalidParent(format!("Parent block {} not found", parent_id_val)))?;
                    if parent_block.chain_id != block.chain_id {
                        return Err(HyperDAGError::InvalidParent(format!("Parent {} on chain {} but block {} on chain {}", parent_id_val, parent_block.chain_id, block.id, block.chain_id)));
                    }
                    // _validator_count_on_parents += 1;
                }
            }

            for (ref_chain_id_val, ref_block_id_val) in &block.cross_chain_references {
                let ref_block = blocks_guard.get(ref_block_id_val)
                    .ok_or_else(|| HyperDAGError::CrossChainReferenceError(format!("Reference block {} not found", ref_block_id_val)))?;
                if ref_block.chain_id != *ref_chain_id_val {
                    return Err(HyperDAGError::CrossChainReferenceError(format!("Reference block {} on chain {} but expected chain {}", ref_block_id_val, ref_block.chain_id, ref_chain_id_val)));
                }
            }
        }

        let expected_reward = self.emission.calculate_reward(block.timestamp).map_err(HyperDAGError::EmissionError)?;
        if block.reward != expected_reward {
            return Err(HyperDAGError::RewardMismatch(expected_reward, block.reward));
        }
        
        let mut tasks = vec![];
        for tx_val in &block.transactions {
            let tx_clone = tx_val.clone();
            let dag_clone = self.clone(); // HyperDAG needs to be Clone
            let utxos_clone = utxos_arc.clone();
            tasks.push(tokio::spawn(async move {
                // Transaction::verify takes Arc<RwLock<HyperDAG>>. dag_clone is HyperDAG.
                // It needs to be Arc<RwLock<HyperDAG>>.
                // This requires HyperDAG::clone() to be efficient or to pass the Arc directly if possible.
                // For now, assuming HyperDAG::clone() works and wrapping it.
                let dag_arc_for_tx = Arc::new(RwLock::new(dag_clone));
                tx_clone.verify(&dag_arc_for_tx, &utxos_clone).await.is_ok()
            }));
        }
        
        if !futures::future::join_all(tasks).await.into_iter().all(|res| res.unwrap_or(false)) {
            return Err(HyperDAGError::InvalidTransaction("One or more transactions in block are invalid".to_string()));
        }

        for swap_val in &block.cross_chain_swaps {
            if swap_val.state == SwapState::Accepted {
                let mut swaps_guard = self.cross_chain_swaps.write().await;
                if let Some(s_val) = swaps_guard.get_mut(&swap_val.swap_id) {
                    s_val.state = SwapState::Completed;
                }
            }
        }

        let mut sc_guard = self.smart_contracts.write().await;
        for contract_val in &block.smart_contracts {
            sc_guard.insert(contract_val.contract_id.clone(), contract_val.clone());
        }

        Ok(true)
    }

    #[instrument]
    async fn detect_anomaly(&self, block: &HyperBlock) -> Result<f64, HyperDAGError> {
        let blocks_guard = self.blocks.read().await;
        if blocks_guard.is_empty() { return Ok(0.0); }
        let avg_tx_count: f64 = blocks_guard.values().map(|b_val| b_val.transactions.len() as f64).sum::<f64>() / (blocks_guard.len() as f64).max(1.0);
        let anomaly_score = (block.transactions.len() as f64 - avg_tx_count).abs() / avg_tx_count.max(1.0);
        Ok(anomaly_score)
    }

    #[instrument]
    pub fn validate_transaction(&self, tx: &Transaction, utxos_map: &HashMap<String, crate::transaction::UTXO>) -> bool {
        if tx.inputs.is_empty() { 
            let expected_reward = match self.emission.calculate_reward(tx.timestamp) {
                Ok(reward) => reward,
                Err(_) => return false,
            };
            let dev_fee_expected = (expected_reward as f64 * DEV_FEE_RATE) as u64;
            let total_output_amount: u64 = tx.outputs.iter().map(|o_val| o_val.amount).sum();
            
            tx.outputs.len() >= 1 && // Coinbase txs usually have 1 or 2 outputs (miner + dev)
            tx.outputs.iter().any(|o_val| o_val.address == DEV_ADDRESS && o_val.amount == dev_fee_expected) &&
            total_output_amount == expected_reward &&
            tx.fee == 0 // Coinbase transactions typically have no fee field or it's zero.
        } else {
            // For regular transactions, use the async verify method.
            // This sync validate_transaction might need to be async or call verify carefully.
            // Wrapping the async call in block_on for a sync context.
            let dag_arc = Arc::new(RwLock::new(self.clone())); // Cloning HyperDAG might be heavy.
            let utxos_arc_val = Arc::new(RwLock::new(utxos_map.clone()));
            // Note: block_on in an async context can lead to issues. This method should ideally be async.
            // If this is called from a non-async context, then block_on is one way, but be cautious.
            match tokio::runtime::Handle::try_current() {
                Ok(handle) => handle.block_on(tx.verify(&dag_arc, &utxos_arc_val)).is_ok(),
                Err(_) => {
                    // Fallback if not in a Tokio runtime context, e.g., create a temporary one.
                    // This is generally not recommended for library code.
                    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                    rt.block_on(tx.verify(&dag_arc, &utxos_arc_val)).is_ok()
                }
            }
        }
    }
    
    #[instrument]
    pub async fn add_block(
        &mut self, // Changed to &mut self
        block: HyperBlock,
        utxos_arc: &Arc<RwLock<HashMap<String, crate::transaction::UTXO>>>,
    ) -> Result<bool, HyperDAGError> {
        {
            let blocks_guard = self.blocks.read().await;
            if blocks_guard.contains_key(&block.id) {
                // Block already exists, log and return or handle as per desired logic
                warn!("Attempted to add block {} which already exists.", block.id);
                return Ok(false); // Or Err if this is an invalid state
            }
        }

        if !self.is_valid_block(&block, utxos_arc).await? {
             return Err(HyperDAGError::InvalidBlock(format!("Block {} failed validation in add_block", block.id)));
        }
        
        let mut blocks_write_guard = self.blocks.write().await;
        // Double check after acquiring write lock, though less likely with proper is_valid_block flow
        if blocks_write_guard.contains_key(&block.id) {
             warn!("Block {} already exists (double check after write lock).", block.id);
             return Ok(false);
        }

        let stake_to_check_and_slash: Option<u64> = {
            let current_validators_guard = self.validators.read().await;
            current_validators_guard.get(&block.validator).copied()
        };

        if let Some(actual_stake_value) = stake_to_check_and_slash {
            let anomaly_score = self.detect_anomaly(&block).await?;
            if anomaly_score > 0.9 { // High threshold for slashing
                let mut validators_write_guard = self.validators.write().await;
                let penalty = actual_stake_value * SLASHING_PENALTY / 100;
                let new_stake = actual_stake_value.saturating_sub(penalty);
                validators_write_guard.insert(block.validator.clone(), new_stake);
                info!("Slashed validator {} by {} for anomaly (score: {})", block.validator, penalty, anomaly_score);
            }
        }
        
        let reward_val = block.reward;
        let block_id_val = block.id.clone();
        let chain_id_val = block.chain_id;

        {
            let mut utxos_write_guard = utxos_arc.write().await;
            for tx_val in &block.transactions {
                // Remove spent UTXOs
                for input_val in &tx_val.inputs {
                    let utxo_id = format!("{}_{}", input_val.tx_id, input_val.output_index);
                    utxos_write_guard.remove(&utxo_id);
                }
                // Add new UTXOs
                for (index, output_val) in tx_val.outputs.iter().enumerate() {
                    let utxo_id = format!("{}_{}", tx_val.id, index);
                    utxos_write_guard.insert(
                        utxo_id.clone(),
                        crate::transaction::UTXO {
                            address: output_val.address.clone(),
                            amount: output_val.amount,
                            tx_id: tx_val.id.clone(),
                            output_index: index as u32,
                            explorer_link: format!("https://hyperblockexplorer.org/utxo/{}", utxo_id),
                        },
                    );
                }
            }
        }

        {
            let mut tips_write_guard = self.tips.write().await;
            let chain_tips = tips_write_guard.entry(chain_id_val).or_insert_with(HashSet::new);
            // Remove parents of this block from tips
            for parent_id_val in &block.parents {
                chain_tips.remove(parent_id_val);
            }
            // Add this block as a new tip
            chain_tips.insert(block_id_val.clone());
        }
        
        let block_for_cache_and_db = block.clone(); 
        blocks_write_guard.insert(block_id_val.clone(), block); // block is moved here
        
        self.cache.write().await.put(block_id_val.clone(), block_for_cache_and_db.clone());
        
        self.db
            .put(block_id_val.as_bytes(), serde_json::to_vec(&block_for_cache_and_db)?)
            .map_err(|e| HyperDAGError::DatabaseError(e.to_string()))?;

        // Emission update. add_block takes &mut self, Emission::update_supply takes &mut self.
        self.emission.update_supply(reward_val).map_err(HyperDAGError::EmissionError)?;


        // These methods take &self and operate on Arc<RwLock> fields internally
        self.adjust_difficulty().await?; 
        self.finalize_blocks().await?;
        self.dynamic_sharding().await?;
        
        BLOCKS_PROCESSED.inc();
        TRANSACTIONS_PROCESSED.inc_by(block_for_cache_and_db.transactions.len() as u64);
        Ok(true)
    }

    #[instrument]
    pub async fn adjust_difficulty(&self) -> Result<(), HyperDAGError> {
        let blocks_guard = self.blocks.read().await;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| HyperDAGError::TimeError)?.as_secs();
        
        let mut sorted_timestamps: Vec<u64> = blocks_guard
            .values()
            .filter(|b_val| now.saturating_sub(b_val.timestamp) < 21600) // Consider blocks from last 6 hours
            .map(|b_val| b_val.timestamp)
            .collect();
        
        if sorted_timestamps.len() < 10 { return Ok(()); } // Need enough samples
        sorted_timestamps.sort_unstable();

        let time_span = sorted_timestamps.last().unwrap_or(&now).saturating_sub(*sorted_timestamps.first().unwrap_or(&now));
        let block_count_in_span = sorted_timestamps.len() as u64;

        let actual_time_per_block = if block_count_in_span > 1 {
            time_span / (block_count_in_span - 1) // Time between first and last block
        } else {
            self.target_block_time // Not enough blocks to calculate, default
        };

        if actual_time_per_block == 0 { return Ok(()); } // Avoid division by zero

        let adjustment_factor = self.target_block_time as f64 / actual_time_per_block as f64;
        drop(blocks_guard); 

        let mut difficulty_history_guard = self.difficulty_history.write().await;
        difficulty_history_guard.push((now, actual_time_per_block));
        if difficulty_history_guard.len() > 100 { difficulty_history_guard.remove(0); }
        
        let predictive_factor = if !difficulty_history_guard.is_empty() {
            let avg_hist_time: u64 = difficulty_history_guard.iter().map(|&(_, t)| t).sum::<u64>() 
                                    / (difficulty_history_guard.len() as u64).max(1);
            if avg_hist_time == 0 { 1.0 } else { self.target_block_time as f64 / avg_hist_time as f64 }
        } else { 1.0 };
        drop(difficulty_history_guard); 

        let mut difficulty_val = self.difficulty.write().await;
        let new_difficulty_f64 = *difficulty_val as f64 * adjustment_factor.clamp(0.5, 2.0) * predictive_factor.clamp(0.8, 1.2);
        *difficulty_val = new_difficulty_f64.max(1.0) as u64; // Ensure difficulty is at least 1
        
        info!("Adjusted difficulty to {}. Actual time/block: {}, Target: {}, Factor: {:.2}, Predictive: {:.2}", 
              *difficulty_val, actual_time_per_block, self.target_block_time, adjustment_factor, predictive_factor);
        Ok(())
    }

    #[instrument]
    pub async fn finalize_blocks(&self) -> Result<(), HyperDAGError> {
        let blocks_guard = self.blocks.read().await;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| HyperDAGError::TimeError)?.as_secs();
        let mut finalized_guard = self.finalized_blocks.write().await;
        let tips_guard = self.tips.read().await;
        let num_chains_val = *self.num_chains.read().await;

        for chain_id_val in 0..num_chains_val {
            if let Some(chain_tips_set) = tips_guard.get(&chain_id_val) {
                for tip_id_val in chain_tips_set {
                    let mut depth = 0;
                    let mut current_id_val = tip_id_val.clone();
                    let mut path_to_finalize = Vec::new(); 

                    while let Some(block_val) = blocks_guard.get(&current_id_val) {
                        if finalized_guard.contains(&current_id_val) { break; }
                        path_to_finalize.push(current_id_val.clone());
                        depth += 1;
                        // Finalize if deep enough OR if very old (stuck tip)
                        if depth >= FINALIZATION_DEPTH || (now.saturating_sub(block_val.timestamp) > 86400) { // 86400 seconds = 1 day
                            break; 
                        }
                        if block_val.parents.is_empty() { break; } // Reached genesis
                        // Simple path traversal, for a multi-parent DAG, this logic might need to be more sophisticated.
                        current_id_val = block_val.parents[0].clone(); // Assuming the first parent is the main one for depth calculation
                    }
                    
                    // Check if the end of the traced path (oldest unfinalized block in path) is itself finalizable
                    let last_block_in_path_id = path_to_finalize.last().cloned().unwrap_or_else(|| tip_id_val.clone());
                    let last_block_is_finalizable = blocks_guard.get(&last_block_in_path_id)
                        .map_or(false, |b| (now.saturating_sub(b.timestamp) > 86400) || b.parents.is_empty());


                    if depth >= FINALIZATION_DEPTH || last_block_is_finalizable {
                        for id_to_finalize in path_to_finalize {
                            if finalized_guard.insert(id_to_finalize.clone()) {
                                log::debug!("Finalized block: {}", id_to_finalize);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    #[instrument]
    pub async fn dynamic_sharding(&self) -> Result<(), HyperDAGError> {
        let mut chain_loads_guard = self.chain_loads.write().await;
        let mut num_chains_val_mut = self.num_chains.write().await; // num_chains needs to be mutable for update
        
        if chain_loads_guard.is_empty() { return Ok(()); }

        let avg_load: u64 = chain_loads_guard.values().sum::<u64>() / (chain_loads_guard.len() as u64).max(1);
        let mut new_shards_created_count = 0;
        let mut new_shard_creation_details = vec![]; // To store (new_chain_id, load_for_new_chain)
        
        // Iterate over a copy of keys to allow modification of chain_loads_guard
        let current_chain_ids: Vec<u32> = chain_loads_guard.keys().cloned().collect();

        for chain_id_val in current_chain_ids {
            if let Some(load_val_mut) = chain_loads_guard.get_mut(&chain_id_val) { // get_mut from the guard
                // Check if load significantly exceeds average and we haven't hit max chains
                if *load_val_mut > avg_load.saturating_mul(SHARD_THRESHOLD as u64) && *num_chains_val_mut < u32::MAX -1 {
                    info!("High load on chain {}: {}. Avg load: {}. Threshold multiplier: {}", chain_id_val, *load_val_mut, avg_load, SHARD_THRESHOLD);
                    *num_chains_val_mut += 1;
                    new_shards_created_count += 1;
                    let new_chain_id_val = *num_chains_val_mut - 1; // New chain ID is the new count - 1
                    
                    let new_load_for_old_chain = *load_val_mut / 2;
                    let new_load_for_new_chain = *load_val_mut - new_load_for_old_chain;
                    
                    *load_val_mut = new_load_for_old_chain; // Update existing chain's load
                    new_shard_creation_details.push((new_chain_id_val, new_load_for_new_chain));
                }
            }
        }
        // Release locks before potentially re-acquiring or calling other async functions
        drop(chain_loads_guard); 
        drop(num_chains_val_mut); // ThisRwLockWriteGuard for num_chains is dropped, subsequent reads will get updated value.

        if !new_shard_creation_details.is_empty() {
            let mut tips_guard = self.tips.write().await;
            let mut blocks_write_guard = self.blocks.write().await;
            let mut chain_loads_reacquired_guard = self.chain_loads.write().await; // Re-acquire for inserts
            let num_chains_current_val = *self.num_chains.read().await; // Read the potentially updated num_chains

            // Placeholder signing key for new genesis blocks. This should be a system/foundation key.
            let placeholder_key = vec![0u8; 32]; 
            let initial_validator_placeholder = DEV_ADDRESS.to_string(); // Or a configured system address

            for (new_chain_id_val, load_val) in new_shard_creation_details {
                tips_guard.insert(new_chain_id_val, HashSet::new());
                chain_loads_reacquired_guard.insert(new_chain_id_val, load_val);

                // Create a genesis block for the new shard
                let current_difficulty_val = *self.difficulty.read().await;
                let mut genesis_block_new_shard = HyperBlock::new(
                    new_chain_id_val, vec![], vec![], current_difficulty_val, 
                    initial_validator_placeholder.clone(), initial_validator_placeholder.clone(), 
                    &placeholder_key // Pass as slice
                )?;
                genesis_block_new_shard.reward = 0; // Genesis blocks typically have no reward from emission
                let new_genesis_id = genesis_block_new_shard.id.clone();

                blocks_write_guard.insert(new_genesis_id.clone(), genesis_block_new_shard);
                tips_guard.get_mut(&new_chain_id_val).unwrap().insert(new_genesis_id); // unwrap is safe as we just inserted the key
                info!("Created new shard {} with initial load {}", new_chain_id_val, load_val);
            }
            info!("Total new shards created: {}. Total chains now: {}", new_shards_created_count, num_chains_current_val);
        }
        Ok(())
    }

    #[instrument]
    pub async fn propose_governance(&self, proposer: String, description: String) -> Result<String, HyperDAGError> {
        let proposal_id_val;
        { // Scope for validators_guard
            let validators_guard = self.validators.read().await;
            let stake_val = validators_guard.get(&proposer)
                .ok_or_else(|| HyperDAGError::Governance("Proposer not found or has no stake".to_string()))?;
            if *stake_val < MIN_VALIDATOR_STAKE * 10 { // Example: proposer needs 10x min stake
                return Err(HyperDAGError::Governance("Insufficient stake to propose".to_string()));
            }
            // Generate proposal ID based on description (or other unique factors)
            proposal_id_val = hex::encode(Keccak256::digest(description.as_bytes()));
        } // validators_guard is dropped here

        let proposal_obj = GovernanceProposal {
            proposal_id: proposal_id_val.clone(), proposer, description,
            votes_for: 0, votes_against: 0, active: true,
        };
        self.governance_proposals.write().await.insert(proposal_id_val.clone(), proposal_obj);
        Ok(proposal_id_val)
    }

    #[instrument]
    pub async fn vote_governance(&self, voter: String, proposal_id: String, vote_for: bool) -> Result<(), HyperDAGError> {
        let stake_val: u64; 
        let total_stake_val: u64; 
        { // Scope for validators_guard
            let validators_guard = self.validators.read().await;
            stake_val = *validators_guard.get(&voter)
                .ok_or_else(|| HyperDAGError::Governance("Voter not found or no stake".to_string()))?;
            total_stake_val = validators_guard.values().sum();
        } // validators_guard is dropped here
        
        let mut proposals_guard = self.governance_proposals.write().await;
        let proposal_obj = proposals_guard.get_mut(&proposal_id)
            .ok_or_else(|| HyperDAGError::Governance("Proposal not found".to_string()))?;
        
        if !proposal_obj.active { return Err(HyperDAGError::Governance("Proposal is not active".to_string())); }

        if vote_for { proposal_obj.votes_for += stake_val; } 
        else { proposal_obj.votes_against += stake_val; }

        // Check for proposal outcome
        if total_stake_val == 0 { // Avoid division by zero if no stake in system
            warn!("Total stake in the system is 0, governance vote cannot pass/fail based on stake percentage.");
            return Ok(());
        }

        if proposal_obj.votes_for > total_stake_val * 2 / 3 { // Example: 2/3 majority to pass
            info!("Governance proposal {} passed: {}", proposal_id, proposal_obj.description);
            proposal_obj.active = false;
            // Here you would implement the action of the proposal, e.g., change parameters
        } else if proposal_obj.votes_against > total_stake_val / 2 { // Example: >50% against to reject early
            info!("Governance proposal {} rejected: {}", proposal_id, proposal_obj.description);
            proposal_obj.active = false;
        }
        Ok(())
    }

    #[instrument]
    pub async fn aggregate_blocks(
        &self,
        blocks_vec: Vec<HyperBlock>,
        utxos_arc: &Arc<RwLock<HashMap<String, crate::transaction::UTXO>>>,
    ) -> Result<Option<HyperBlock>, HyperDAGError> {
        if blocks_vec.is_empty() {
            return Ok(None);
        }
        for block_val in &blocks_vec {
            if !self.is_valid_block(block_val, utxos_arc).await? {
                warn!("Invalid block {} found during aggregation, aggregation attempt failed.", block_val.id);
                return Ok(None); 
            }
        }
        // Simple aggregation: just take the first valid block if any.
        // More complex logic could merge transactions, select based on criteria, etc.
        Ok(blocks_vec.into_iter().next()) 
    }

    #[instrument]
    pub async fn select_validator(&self) -> Option<String> {
        let validators_guard = self.validators.read().await;
        if validators_guard.is_empty() { return None; }

        let total_stake_val: u64 = validators_guard.values().sum();
        if total_stake_val == 0 {
             // If total stake is 0, select randomly among all registered validators
             let validator_keys: Vec<String> = validators_guard.keys().cloned().collect();
             if validator_keys.is_empty() { return None; } // Should not happen if validators_guard is not empty
             return Some(validator_keys[rand::thread_rng().gen_range(0..validator_keys.len())].clone());
        }
        
        let mut rand_num = rand::thread_rng().gen_range(0..total_stake_val);
        for (validator_addr, stake_val) in validators_guard.iter() {
            if rand_num < *stake_val { return Some(validator_addr.clone()); }
            rand_num -= *stake_val;
        }
        // Fallback (should ideally not be reached if rand_num is generated correctly within total_stake)
        validators_guard.keys().next().cloned()
    }

    #[instrument]
    pub async fn get_state_snapshot(&self, chain_id_val: u32) -> (HashMap<String, HyperBlock>, HashMap<String, crate::transaction::UTXO>) {
        let blocks_guard = self.blocks.read().await;
        let mut chain_blocks_map = HashMap::new();
        let mut utxos_map_for_chain = HashMap::new();

        for (id_val, block_val) in blocks_guard.iter() {
            if block_val.chain_id == chain_id_val {
                chain_blocks_map.insert(id_val.clone(), block_val.clone());
                // This UTXO collection is a simplified view based on block outputs.
                // A more accurate UTXO set would be maintained globally and passed in or queried.
                for tx_val in &block_val.transactions {
                    for (index_val, output_val) in tx_val.outputs.iter().enumerate() {
                        let utxo_id_val = format!("{}_{}", tx_val.id, index_val);
                        // This might overwrite UTXOs if multiple blocks on this chain create same output ID
                        // (which shouldn't happen for valid tx IDs).
                        utxos_map_for_chain.insert(
                            utxo_id_val.clone(),
                            crate::transaction::UTXO {
                                address: output_val.address.clone(), amount: output_val.amount,
                                tx_id: tx_val.id.clone(), output_index: index_val as u32,
                                explorer_link: format!("https://hyperblockexplorer.org/utxo/{}", utxo_id_val),
                            },
                        );
                    }
                }
            }
        }
        (chain_blocks_map, utxos_map_for_chain)
    }
}

impl Clone for HyperDAG {
    fn clone(&self) -> Self {
        Self {
            blocks: self.blocks.clone(),
            tips: self.tips.clone(),
            validators: self.validators.clone(),
            target_block_time: self.target_block_time,
            difficulty: self.difficulty.clone(),
            emission: self.emission.clone(), // Emission needs to be Clone
            num_chains: self.num_chains.clone(),
            finalized_blocks: self.finalized_blocks.clone(),
            governance_proposals: self.governance_proposals.clone(),
            chain_loads: self.chain_loads.clone(),
            difficulty_history: self.difficulty_history.clone(),
            block_creation_timestamps: self.block_creation_timestamps.clone(),
            anomaly_history: self.anomaly_history.clone(),
            cross_chain_swaps: self.cross_chain_swaps.clone(),
            smart_contracts: self.smart_contracts.clone(),
            cache: self.cache.clone(),
            db: self.db.clone(), // Arc<DB> is Clone
        }
    }
}