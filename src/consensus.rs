//! --- Hyperchain Hybrid Consensus Engine ---
//! v2.0.0 - Hardened & Adaptive
//!
//! This module implements the core consensus rules for the Hyperchain network.
//! It uses a hybrid model that combines three critical, non-replaceable mechanisms,
//! each serving a distinct and vital purpose.
//!
//! 1.  **Proof-of-Stake (PoS): The "Right to Participate"**. This is the foundational
//!     layer for eligibility. To create blocks, a node must be a registered validator
//!     with a minimum amount of HCN tokens staked. This ensures that block producers
//!     have a financial "stake" in the network's success and are disincentivized from
//!     acting maliciously. This is a non-negotiable prerequisite for block production.
//!
//! 2.  **Proof-of-Work (PoW): The "Cost of Truth"**. This is the fundamental security
//!     and finality layer. Every valid block, regardless of who creates it, MUST contain
//!     a valid Proof-of-Work solution (a nonce that results in a block hash below a
//!     certain target). This makes rewriting history computationally expensive and secures
//!     the chain against 51% attacks. PoW is a permanent and core part of the consensus
//!     mechanism that cannot be bypassed.
//!
//! 3.  **Proof-of-Sentiency (PoSe): The "Intelligence Layer"**. This is Hyperchain's
//!     novel innovation, powered by the SAGA pallet. PoSe does NOT replace PoW.
//!     Instead, it *dynamically adjusts the PoW difficulty target* for each miner
//!     based on their reputation (Saga Credit Score - SCS). Reputable, trusted miners
//!     face a lower difficulty, making the network highly efficient and reducing its
//!     energy footprint. Malicious or untrusted miners face a much higher difficulty,
//!     making attacks prohibitively expensive. PoSe makes PoW smarter, adaptive, and
//!     more secure, ensuring the most trustworthy participants are the most efficient.

use crate::hyperdag::{HyperBlock, HyperDAG, HyperDAGError, UTXO};
use crate::saga::{PalletSaga, SagaError};
use crate::transaction::TransactionError;
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, instrument, warn};

#[derive(Error, Debug)]
pub enum ConsensusError {
    #[error("Invalid block structure: {0}")]
    InvalidBlockStructure(String),
    #[error("Invalid transaction: {0}")]
    InvalidTransaction(#[from] TransactionError),
    #[error("Proof-of-Work check failed: {0}")]
    ProofOfWorkFailed(String),
    #[error("Proof-of-Stake check failed: {0}")]
    ProofOfStakeFailed(String),
    #[error("Block failed SAGA-ΩMEGA security validation: {0}")]
    OmegaRejection(String),
    #[error("Database or state error during validation: {0}")]
    StateError(String),
    #[error("Saga error: {0}")]
    SagaError(#[from] SagaError),
    #[error("Anyhow error: {0}")]
    Anyhow(#[from] anyhow::Error),
    #[error("HyperDAG error: {0}")]
    HyperDAG(#[from] HyperDAGError),
}

/// The main consensus engine for Hyperchain. It orchestrates the various validation
/// mechanisms to ensure network integrity.
pub struct Consensus {
    saga: Arc<PalletSaga>,
}

impl Consensus {
    /// Creates a new Consensus engine instance, linking it to the SAGA pallet.
    pub fn new(saga: Arc<PalletSaga>) -> Self {
        Self { saga }
    }

    /// The primary validation function. It checks a block against all consensus rules
    /// in a specific, non-negotiable order: structure, stake, transactions, and finally the computational work.
    #[instrument(skip(self, block, dag_arc, utxos), fields(block_id = %block.id, miner = %block.miner))]
    pub async fn validate_block(
        &self,
        block: &HyperBlock,
        dag_arc: &Arc<HyperDAG>,
        utxos: &Arc<RwLock<HashMap<String, UTXO>>>,
    ) -> Result<(), ConsensusError> {
        // --- Rule 1: Structural & Cryptographic Integrity (Fastest Check) ---
        // This performs basic sanity checks on the block's format and signatures.
        self.validate_block_structure(block, dag_arc).await?;

        // --- Rule 2: Proof-of-Stake (PoS) - The "Right to Participate" ---
        // This verifies that the block's creator is a registered validator with enough stake.
        self.validate_proof_of_stake(&block.validator, dag_arc)
            .await?;

        // --- Rule 3: Transaction Validity ---
        // This ensures every transaction in the block is valid and spends existing UTXOs.
        let utxos_guard = utxos.read().await;
        for tx in block.transactions.iter().skip(1) {
            // Skip coinbase
            tx.verify(dag_arc, &utxos_guard).await?;
        }
        drop(utxos_guard);

        // --- Rule 4: Proof-of-Work (PoW) with Proof-of-Sentiency (PoSe) Adjustment ---
        // The final and most computationally intensive check. It verifies that the block's
        // hash meets the difficulty target, which has been dynamically adjusted by SAGA.
        self.validate_proof_of_work(block).await?;

        debug!("All consensus checks passed for block {}", block.id);
        Ok(())
    }

    /// Performs all fundamental structural and cryptographic checks on a block.
    async fn validate_block_structure(
        &self,
        block: &HyperBlock,
        dag_arc: &Arc<HyperDAG>,
    ) -> Result<(), ConsensusError> {
        if block.id.is_empty() || block.merkle_root.is_empty() || block.validator.is_empty() {
            return Err(ConsensusError::InvalidBlockStructure(
                "Core fields (ID, Merkle Root, Validator) cannot be empty".to_string(),
            ));
        }
        if block.transactions.is_empty() {
            return Err(ConsensusError::InvalidBlockStructure(
                "Block must have at least one transaction (coinbase)".to_string(),
            ));
        }

        if !block.verify_signature()? {
            return Err(ConsensusError::InvalidBlockStructure(
                "Block signature verification failed".to_string(),
            ));
        }

        let expected_merkle_root = HyperBlock::compute_merkle_root(&block.transactions)
            .map_err(|e| ConsensusError::InvalidBlockStructure(e.to_string()))?;
        if block.merkle_root != expected_merkle_root {
            return Err(ConsensusError::InvalidBlockStructure(
                "Merkle root mismatch".to_string(),
            ));
        }

        let coinbase = &block.transactions[0];
        if !coinbase.is_coinbase() {
            return Err(ConsensusError::InvalidBlockStructure(
                "First transaction must be coinbase".to_string(),
            ));
        }
        if coinbase.outputs.is_empty() {
            return Err(ConsensusError::InvalidBlockStructure(
                "Coinbase must have at least one output".to_string(),
            ));
        }

        let expected_reward = self.saga.calculate_dynamic_reward(block, dag_arc).await?;
        if block.reward != expected_reward {
            return Err(ConsensusError::InvalidBlockStructure(format!(
                "Block reward mismatch. Claimed: {}, Expected (from SAGA): {}",
                block.reward, expected_reward
            )));
        }

        Ok(())
    }

    /// Validates that the block producer has sufficient stake to participate in consensus.
    async fn validate_proof_of_stake(
        &self,
        validator_address: &str,
        dag: &HyperDAG,
    ) -> Result<(), ConsensusError> {
        let rules = self.saga.economy.epoch_rules.read().await;
        let min_stake = rules
            .get("min_validator_stake")
            .map_or(1000.0, |r| r.value) as u64;

        let validators = dag.validators.read().await;
        let validator_stake = validators.get(validator_address).copied().unwrap_or(0);

        if validator_stake < min_stake {
            return Err(ConsensusError::ProofOfStakeFailed(format!(
                "Insufficient stake for validator {}. Required: {}, Found: {}",
                validator_address, min_stake, validator_stake
            )));
        }
        Ok(())
    }

    /// Validates the block's Proof-of-Work against the dynamically adjusted difficulty target from SAGA.
    async fn validate_proof_of_work(&self, block: &HyperBlock) -> Result<(), ConsensusError> {
        let effective_difficulty = self.get_effective_difficulty(&block.miner).await;

        if block.difficulty != effective_difficulty {
            warn!(
                "Block {} has difficulty mismatch. Claimed: {}, Required (by PoSe): {}",
                block.id, block.difficulty, effective_difficulty
            );
            return Err(ConsensusError::ProofOfWorkFailed(format!(
                "Difficulty mismatch. Claimed: {}, Required by PoSe: {}",
                block.difficulty, effective_difficulty
            )));
        }

        let target_hash =
            crate::miner::Miner::calculate_target_from_difficulty(effective_difficulty);
        let block_pow_hash = hex::decode(block.hash()).map_err(|_| {
            ConsensusError::StateError("Failed to decode block PoW hash".to_string())
        })?;

        if !crate::miner::Miner::hash_meets_target(&block_pow_hash, &target_hash) {
            return Err(ConsensusError::ProofOfWorkFailed(
                "Block hash does not meet PoSe difficulty target.".to_string(),
            ));
        }

        debug!(
            "PoW validation passed for miner {}. Effective PoSe difficulty: {}",
            block.miner, effective_difficulty
        );
        Ok(())
    }

    /// Retrieves the effective PoW difficulty for a given miner.
    /// This is the core of PoSe, where SAGA's intelligence modifies the base PoW.
    pub async fn get_effective_difficulty(&self, miner_address: &str) -> u64 {
        let rules = self.saga.economy.epoch_rules.read().await;
        let base_difficulty = rules
            .get("base_difficulty")
            .map_or(10.0, |r| r.value) as u64;

        // Fetch the miner's Saga Credit Score (SCS). Default to 0.5 (neutral) if not found.
        let scs = self
            .saga
            .reputation
            .credit_scores
            .read()
            .await
            .get(miner_address)
            .map_or(0.5, |s| s.score);

        // An SCS of 1.0 (perfect) gives a significant advantage.
        // An SCS of 0.0 (malicious) gives a significant disadvantage.
        // An SCS of 0.5 (neutral) results in the base difficulty.
        let difficulty_modifier = 1.0 - (scs - 0.5);
        let effective_difficulty = (base_difficulty as f64 * difficulty_modifier).round() as u64;

        // Clamp the difficulty to prevent extreme values, ensuring the network remains stable.
        effective_difficulty.clamp(
            base_difficulty.saturating_div(2),
            base_difficulty.saturating_mul(2),
        )
    }
}
