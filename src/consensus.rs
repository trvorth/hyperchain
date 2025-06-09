use crate::hyperdag::{HyperBlock, HyperDAG};
use crate::transaction::UTXO;
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;

#[derive(Error, Debug)]
pub enum ConsensusError {
    #[error("Invalid block size")]
    InvalidBlockSize,
    #[error("Invalid transaction size")]
    InvalidTransactionSize,
    #[error("Invalid block structure")]
    InvalidBlockStructure,
    #[error("Invalid transaction")]
    InvalidTransaction,
    #[error("ZK proof failed")]
    ZKProofFailed,
}

pub struct Consensus {
    max_block_size: usize,
    max_transaction_size: usize,
    max_transactions_per_block: usize,
}

impl Consensus {
    pub fn new() -> Self {
        Self {
            max_block_size: 10_000_000,
            max_transaction_size: 100_000,
            max_transactions_per_block: 10_000,
        }
    }

    pub async fn validate_block(
        &self,
        block: &HyperBlock,
        dag: &HyperDAG,
        utxos: &Arc<RwLock<HashMap<String, UTXO>>>,
    ) -> Result<(), ConsensusError> {
        if serde_json::to_vec(&block)
            .map_err(|_| ConsensusError::InvalidBlockStructure)?
            .len()
            > self.max_block_size
        {
            return Err(ConsensusError::InvalidBlockSize);
        }
        if block.transactions.len() > self.max_transactions_per_block {
            return Err(ConsensusError::InvalidBlockStructure); 
        }
        for tx in &block.transactions {
            if serde_json::to_vec(&tx) 
                .map_err(|_| ConsensusError::InvalidTransaction)? 
                .len()
                > self.max_transaction_size
            {
                return Err(ConsensusError::InvalidTransactionSize);
            }
            let utxos_read_guard = utxos.read().await;
            if !dag.validate_transaction(tx, &*utxos_read_guard) {
                return Err(ConsensusError::InvalidTransaction);
            }
        }
        if !dag.is_valid_block(block, utxos).await.map_err(|_| ConsensusError::InvalidBlockStructure)? {
            return Err(ConsensusError::InvalidBlockStructure);
        }
        Ok(())
    }
}
