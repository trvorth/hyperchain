// src/block.rs

use crate::transaction::Transaction;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct HyperBlock {
    pub id: [u8; 32],
    pub parent_hash: [u8; 32],
    pub merkle_root: [u8; 32],
    pub timestamp: u64,
    pub nonce: u64,
    pub transactions: Vec<Transaction>,
    pub chain_id: u32,
}

impl HyperBlock {
    pub fn new(parent_hash: [u8; 32], transactions: Vec<Transaction>, chain_id: u32) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();

        Self {
            id: [0; 32], // Placeholder, will be set after mining
            parent_hash,
            merkle_root: Self::calculate_merkle_root(&transactions),
            timestamp,
            nonce: 0, // Placeholder, will be set by the miner
            transactions,
            chain_id,
        }
    }

    pub fn header_hash(&self) -> [u8; 32] {
        let mut hasher = Keccak256::new();
        hasher.update(&self.parent_hash);
        hasher.update(&self.merkle_root);
        hasher.update(&self.timestamp.to_be_bytes());
        hasher.update(&self.nonce.to_be_bytes());
        hasher.update(&self.chain_id.to_be_bytes());
        hasher.finalize().into()
    }

    pub fn calculate_merkle_root(transactions: &[Transaction]) -> [u8; 32] {
    if transactions.is_empty() {
        return [0; 32];
    }
    let mut hashes: Vec<[u8; 32]> = transactions.iter().map(|tx| tx.id).collect();
    while hashes.len() > 1 {
        let mut new_hashes = Vec::new();
        for chunk in hashes.chunks(2) {
            let mut hasher = Sha256::new();
            hasher.update(&chunk[0]);
            if chunk.len() > 1 {
                hasher.update(&chunk[1]);
            } else {
                hasher.update(&chunk[0]); // Duplicate last hash if odd number
            }
            new_hashes.push(hasher.finalize().into());
        }
        hashes = new_hashes;
    }
    hashes[0]
}

    pub fn verify(&self) -> bool {
        // 1. Verify the block's ID is the same as its header hash.
        if self.id != self.header_hash() {
            log::warn!("Block ID does not match its header hash.");
            return false;
        }

        // 2. Verify the coinbase transaction. There must be exactly one, and it must be the first.
        let coinbase_count = self.transactions.iter().filter(|tx| tx.is_coinbase()).count();
        if coinbase_count != 1 || !self.transactions.get(0).map_or(false, |tx| tx.is_coinbase()) {
             log::warn!("Invalid coinbase transaction setup: count={}, is_first={}", coinbase_count, self.transactions.get(0).is_some());
            return false;
        }

        // 3. Verify each transaction in the block.
        for tx in &self.transactions {
            if !tx.verify() {
                log::warn!("Transaction validation failed for tx: {}", hex::encode(tx.id));
                return false;
            }
        }

        true
    }
}