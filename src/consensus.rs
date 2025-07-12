//! --- Hyperchain Hybrid Consensus Engine ---
//! v1.0.0 - Hybrid PoW/PoS/PoSe (Hardened & Eco-Sentiency Aware)
//! This module implements the core consensus rules for the Hyperchain network.
//! It uses a hybrid model that combines three critical, non-replaceable mechanisms:
//!
//! 1.  **Proof-of-Stake (PoS):** The foundational layer for eligibility. To be eligible
//!     to create blocks, a node must be a registered validator with a minimum amount of
//!     HCN tokens staked. This ensures that block producers have a financial "stake"
//!     in the network's success and are disincentivized from acting maliciously. This is a
//!     non-negotiable prerequisite for block production.
//!
//! 2.  **Proof-of-Work (PoW):** The fundamental security and finality layer. Every valid block,
//!     regardless of who creates it, MUST contain a valid Proof-of-Work solution (a nonce
//!     that results in a block hash below a certain target). This makes rewriting history
//!     computationally expensive and secures the chain against 51% attacks. PoW is a
//!     permanent and core part of the consensus mechanism that cannot be bypassed.
//!
//! 3.  **Proof-of-Sentiency (PoSe):** The intelligence and efficiency layer. This is
//!     Hyperchain's novel innovation, powered by the SAGA pallet. PoSe does NOT replace
//!     PoW. Instead, it *dynamically adjusts the PoW difficulty target* for each miner
//!     based on their reputation (Saga Credit Score - SCS). Reputable, trusted miners
//!     face a lower difficulty, making the network highly efficient and reducing its
//!     energy footprint. Malicious or untrusted miners face a much higher difficulty,
//!     making attacks prohibitively expensive. PoSe makes PoW smarter, adaptive, and
//!     more secure.

use crate::hyperdag::{HyperBlock, HyperDAG, UTXO};
use crate::saga::PalletSaga;
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
    #[instrument(skip(self, block, dag, utxos), fields(block_id = %block.id, miner = %block.miner))]
    pub async fn validate_block(
        &self,
        block: &HyperBlock,
        dag: &HyperDAG,
        utxos: &Arc<RwLock<HashMap<String, UTXO>>>,
    ) -> Result<(), ConsensusError> {
        // --- Rule 1: Structural & Cryptographic Integrity (Fastest Check) ---
        // First, ensure the block is well-formed and its signature is valid. This is a cheap
        // check that can quickly discard malformed or fraudulent blocks without further processing.
        self.validate_block_structure(block)?;

        // --- Rule 2: Proof-of-Stake (PoS) - The "Right to Participate" ---
        // Verify that the block's creator (the validator) has sufficient stake locked in the
        // network. This ensures that block producers have skin in the game. This is a mandatory
        // prerequisite before any further checks are performed.
        self.validate_proof_of_stake(&block.validator, dag).await?;

        // --- Rule 3: Transaction Validity ---
        // Iterate through all transactions (except the coinbase) and ensure they are valid
        // (e.g., inputs exist, signatures are correct, funds are sufficient). This is the
        // ledger integrity check.
        let utxos_guard = utxos.read().await;
        for tx in block.transactions.iter().skip(1) {
            // Skip coinbase
            tx.verify(dag, &utxos_guard).await?;
        }
        drop(utxos_guard);

        // --- Rule 4: Proof-of-Work (PoW) with Proof-of-Sentiency (PoSe) Adjustment ---
        // The final and most computationally intensive check. Verify that the block's hash
        // meets the difficulty target. This target is NOT static; it is dynamically adjusted
        // by SAGA based on the miner's reputation (SCS). This is the core of PoSe, making
        // PoW adaptive and intelligent without replacing it.
        self.validate_proof_of_work(block).await?;

        debug!("All consensus checks passed for block {}", block.id);
        Ok(())
    }

    /// Performs all fundamental structural and cryptographic checks on a block.
    fn validate_block_structure(&self, block: &HyperBlock) -> Result<(), ConsensusError> {
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

        // Verify the block's own internal signature.
        if !block.verify_signature().map_err(|e| {
            warn!("Block signature verification failed: {}", e);
            ConsensusError::InvalidBlockStructure("Block signature verification failed".to_string())
        })? {
            return Err(ConsensusError::InvalidBlockStructure(
                "Block signature verification failed".to_string(),
            ));
        }

        // Verify the integrity of the transaction list against the Merkle root.
        let expected_merkle_root = HyperBlock::compute_merkle_root(&block.transactions)
            .map_err(|e| ConsensusError::InvalidBlockStructure(e.to_string()))?;
        if block.merkle_root != expected_merkle_root {
            return Err(ConsensusError::InvalidBlockStructure(
                "Merkle root mismatch".to_string(),
            ));
        }

        // Verify coinbase transaction structure.
        let coinbase = &block.transactions[0];
        if !coinbase.inputs.is_empty() {
            return Err(ConsensusError::InvalidBlockStructure(
                "First transaction in a block must be a coinbase (have no inputs)".to_string(),
            ));
        }
        if coinbase.outputs.len() < 2 {
            return Err(ConsensusError::InvalidBlockStructure(
                "Coinbase transaction must have at least two outputs (miner reward and dev fee)"
                    .to_string(),
            ));
        }

        Ok(())
    }

    /// Proof-of-Stake (PoS) check: Verifies the validator meets the minimum stake requirement.
    /// This is the gatekeeper function for consensus participation.
    async fn validate_proof_of_stake(
        &self,
        validator_address: &str,
        dag: &HyperDAG,
    ) -> Result<(), ConsensusError> {
        let rules = self.saga.economy.epoch_rules.read().await;
        // The minimum stake is a dynamic rule controlled by SAGA governance.
        let min_stake = rules
            .get("min_validator_stake")
            .map_or(1000.0, |r| r.value) as u64;

        let validators = dag.validators.read().await;
        let validator_stake = validators.get(validator_address).copied().unwrap_or(0);

        if validator_stake < min_stake {
            return Err(ConsensusError::ProofOfStakeFailed(format!(
                "Validator {validator_address} has insufficient stake. Required: {min_stake}, Found: {validator_stake}"
            )));
        }
        Ok(())
    }

    /// Proof-of-Work (PoW) check: Validates that the block's hash meets the
    /// dynamically adjusted difficulty target required by Proof-of-Sentiency (PoSe).
    async fn validate_proof_of_work(&self, block: &HyperBlock) -> Result<(), ConsensusError> {
        // PoSe: Get the specific difficulty required for *this* miner from SAGA.
        let effective_difficulty = self.get_effective_difficulty(&block.miner).await;

        // The block must explicitly claim the difficulty it was trying to solve. This prevents
        // miners from solving for an easier difficulty and claiming a harder one.
        if block.difficulty != effective_difficulty {
            return Err(ConsensusError::ProofOfWorkFailed(format!(
                "Block difficulty mismatch. Claimed: {}, Required by PoSe: {}",
                block.difficulty, effective_difficulty
            )));
        }

        // Calculate the hash target from the SAGA-provided difficulty.
        let target_hash =
            crate::miner::Miner::calculate_target_from_difficulty(effective_difficulty);
        let block_pow_hash = hex::decode(block.hash()).map_err(|_| {
            ConsensusError::StateError("Failed to decode block PoW hash".to_string())
        })?;

        // The core PoW check. The block's hash must be less than or equal to the target.
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

    /// **Proof-of-Sentiency (PoSe) Core Logic**
    /// Calculates the effective PoW difficulty for a given miner based on their Saga Credit Score (SCS).
    /// This is the heart of PoSe: reputable miners have an easier time, making the network
    /// more efficient, while disreputable miners must expend more energy, securing the network.
    pub async fn get_effective_difficulty(&self, miner_address: &str) -> u64 {
        let rules = self.saga.economy.epoch_rules.read().await;
        let base_difficulty = rules
            .get("base_difficulty")
            .map_or(10.0, |r| r.value) as u64;

        let scs = self
            .saga
            .reputation
            .credit_scores
            .read()
            .await
            .get(miner_address)
            .map_or(0.5, |s| s.score); // Default to a neutral score of 0.5 if unknown

        // The difficulty modifier is inversely proportional to the score's deviation from neutral.
        // - SCS = 1.0 (perfect) -> modifier = 0.5 (50% easier)
        // - SCS = 0.5 (neutral) -> modifier = 1.0 (no change)
        // - SCS = 0.0 (terrible) -> modifier = 1.5 (50% harder)
        let difficulty_modifier = 1.0 - (scs - 0.5);
        let effective_difficulty = (base_difficulty as f64 * difficulty_modifier).round() as u64;

        // Clamp the difficulty to a sane range (e.g., 50% to 200% of base) to prevent extreme values
        // that could either halt the chain or make it too easy to mine.
        effective_difficulty.clamp(
            base_difficulty.saturating_div(2),
            base_difficulty.saturating_mul(2),
        )
    }
}