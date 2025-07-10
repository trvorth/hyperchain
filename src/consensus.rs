//! --- Hyperchain Hybrid Consensus Engine ---
//! v0.5.0 - Hybrid PoW/PoS/PoSe
//! This module implements the core consensus rules for the Hyperchain network.
//! It uses a hybrid model that combines:
//! 1. Proof-of-Stake (PoS): Validators must hold a minimum stake to create blocks.
//! 2. Proof-of-Work (PoW): Blocks must meet a hash target difficulty.
//! 3. Proof-of-Sentiency (PoSe): A novel mechanism where the PoW difficulty is
//!    dynamically adjusted based on the miner's reputation (Saga Credit Score),
//!    making the network more efficient and secure.

use crate::hyperdag::{HyperBlock, HyperDAG, UTXO};
use crate::saga::PalletSaga;
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, instrument};

#[derive(Error, Debug)]
pub enum ConsensusError {
    #[error("Invalid block structure: {0}")]
    InvalidBlockStructure(String),
    #[error("Invalid transaction: {0}")]
    InvalidTransaction(String),
    #[error("Proof-of-Work check failed: {0}")]
    ProofOfWorkFailed(String),
    #[error("Proof-of-Stake check failed: {0}")]
    ProofOfStakeFailed(String),
    #[error("Block failed SAGA-ΩMEGA security validation: {0}")]
    OmegaRejection(String),
    #[error("Database or state error during validation: {0}")]
    StateError(String),
}

/// The main consensus engine for Hyperchain.
pub struct Consensus {
    saga: Arc<PalletSaga>,
}

impl Consensus {
    /// Creates a new Consensus engine instance.
    pub fn new(saga: Arc<PalletSaga>) -> Self {
        Self { saga }
    }

    /// The primary validation function. It checks a block against all consensus rules.
    #[instrument(skip(self, block, dag, utxos), fields(block_id = %block.id, miner = %block.miner))]
    pub async fn validate_block(
        &self,
        block: &HyperBlock,
        dag: &HyperDAG,
        utxos: &Arc<RwLock<HashMap<String, UTXO>>>,
    ) -> Result<(), ConsensusError> {
        // 1. Basic structural and signature validation
        self.validate_block_structure(block)?;

        // 2. Proof-of-Stake (PoS) Validation: Check if the validator has enough stake.
        self.validate_proof_of_stake(&block.validator, dag).await?;

        // 3. Transaction Validation: Ensure all transactions in the block are valid.
        let utxos_guard = utxos.read().await;
        for tx in &block.transactions {
            if tx.inputs.is_empty() { continue; } // Skip coinbase
            tx.verify(dag, &utxos_guard)
                .await
                .map_err(|e| ConsensusError::InvalidTransaction(e.to_string()))?;
        }
        drop(utxos_guard);

        // 4. Proof-of-Work (PoW) Validation (with PoSe Difficulty Adjustment)
        self.validate_proof_of_work(block).await?;

        Ok(())
    }

    /// Performs all fundamental structural and cryptographic checks on a block.
    fn validate_block_structure(&self, block: &HyperBlock) -> Result<(), ConsensusError> {
        // Check for non-empty fields
        if block.id.is_empty() {
            return Err(ConsensusError::InvalidBlockStructure("Block ID cannot be empty".to_string()));
        }
        if block.transactions.is_empty() {
            return Err(ConsensusError::InvalidBlockStructure("Block must have at least one transaction (coinbase)".to_string()));
        }

        // Verify the block's lattice signature
        if !block.verify_signature().map_err(|e| ConsensusError::InvalidBlockStructure(e.to_string()))? {
            return Err(ConsensusError::InvalidBlockStructure("Block signature verification failed".to_string()));
        }

        // Verify the integrity of the transaction list
        let expected_merkle_root = HyperBlock::compute_merkle_root(&block.transactions)
            .map_err(|e| ConsensusError::InvalidBlockStructure(e.to_string()))?;
        if block.merkle_root != expected_merkle_root {
            return Err(ConsensusError::InvalidBlockStructure("Merkle root mismatch".to_string()));
        }

        // Verify coinbase transaction structure
        let coinbase = &block.transactions[0];
        if !coinbase.inputs.is_empty() {
            return Err(ConsensusError::InvalidBlockStructure("First transaction in a block must be a coinbase (have no inputs)".to_string()));
        }
        if coinbase.outputs.len() < 2 {
            return Err(ConsensusError::InvalidBlockStructure("Coinbase transaction must have at least two outputs (miner reward and dev fee)".to_string()));
        }

        Ok(())
    }


    /// Proof-of-Stake (PoS) check: Verifies the validator meets the minimum stake requirement.
    async fn validate_proof_of_stake(&self, validator_address: &str, dag: &HyperDAG) -> Result<(), ConsensusError> {
        let rules = self.saga.economy.epoch_rules.read().await;
        let min_stake = rules.get("min_validator_stake").map_or(1000.0, |r| r.value) as u64;

        let validators = dag.validators.read().await;
        let validator_stake = validators.get(validator_address).copied().unwrap_or(0);

        if validator_stake < min_stake {
            return Err(ConsensusError::ProofOfStakeFailed(format!(
                "Validator {} has insufficient stake. Required: {}, Found: {}",
                validator_address, min_stake, validator_stake
            )));
        }
        Ok(())
    }


    /// Proof-of-Work (PoW) check: Validates that the block's hash meets the
    /// dynamically adjusted difficulty target required by Proof-of-Sentiency (PoSe).
    async fn validate_proof_of_work(
        &self,
        block: &HyperBlock,
    ) -> Result<(), ConsensusError> {
        let effective_difficulty = self.get_effective_difficulty(&block.miner).await;

        if block.difficulty != effective_difficulty {
            return Err(ConsensusError::ProofOfWorkFailed(format!(
                "Block difficulty mismatch. Claimed: {}, Required by PoSe: {}",
                block.difficulty, effective_difficulty
            )));
        }

        let target_hash = crate::miner::Miner::calculate_target_from_difficulty(effective_difficulty);
        let block_pow_hash = hex::decode(block.hash()).map_err(|_| {
            ConsensusError::StateError("Failed to decode block PoW hash".to_string())
        })?;

        if !crate::miner::Miner::hash_meets_target(&block_pow_hash, &target_hash) {
            return Err(ConsensusError::ProofOfWorkFailed(
                "Block hash does not meet the required PoSe difficulty target.".to_string(),
            ));
        }

        debug!(
            "Proof-of-Work validation passed for miner {}. Effective PoSe difficulty: {}",
            block.miner, effective_difficulty
        );
        Ok(())
    }

    /// Calculates the effective PoW difficulty for a given miner based on their SCS (Proof-of-Sentiency).
    pub async fn get_effective_difficulty(&self, miner_address: &str) -> u64 {
        let rules = self.saga.economy.epoch_rules.read().await;
        let base_difficulty = rules.get("base_difficulty").map_or(10.0, |r| r.value) as u64;

        let scs = self.saga.reputation.credit_scores.read().await
            .get(miner_address)
            .map_or(0.5, |s| s.score);

        // SCS > 0.5 --> easier difficulty
        // SCS < 0.5 --> harder difficulty
        let difficulty_modifier = 1.0 - (scs - 0.5);
        let effective_difficulty = (base_difficulty as f64 * difficulty_modifier).round() as u64;

        // Clamp the difficulty to prevent extreme values (e.g., 50% to 200% of base)
        effective_difficulty.clamp(base_difficulty.saturating_div(2), base_difficulty * 2)
    }
}