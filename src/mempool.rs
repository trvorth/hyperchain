use crate::hyperdag::{HyperDAG, UTXO};
use crate::transaction::{Transaction, TransactionError};
use log::{info, warn};
use prometheus::{register_gauge, register_int_counter, Gauge, IntCounter};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::instrument;

lazy_static::lazy_static! {
    static ref MEMPOOL_SIZE: Gauge = register_gauge!("mempool_size_bytes", "Current size of the mempool in bytes").unwrap();
    static ref MEMPOOL_TRANSACTIONS: Gauge = register_gauge!("mempool_transactions_total", "Current number of transactions in the mempool").unwrap();
    static ref TRANSACTIONS_ADDED: IntCounter = register_int_counter!("mempool_transactions_added_total", "Total transactions added to mempool").unwrap();
    static ref TRANSACTIONS_REMOVED: IntCounter = register_int_counter!("mempool_transactions_removed_total", "Total transactions removed from mempool").unwrap();
    static ref TRANSACTIONS_EXPIRED: IntCounter = register_int_counter!("mempool_transactions_expired_total", "Total transactions expired from mempool").unwrap();
}

#[derive(Error, Debug)]
pub enum MempoolError {
    #[error("Transaction validation failed: {0}")]
    TransactionValidation(String),
    #[error("Mempool full")]
    MempoolFull,
    #[error("Transaction error: {0}")]
    Tx(#[from] TransactionError),
    #[error("Timestamp error")]
    TimestampError,
}

#[derive(Clone, Debug)]
pub struct Mempool {
    transactions: Arc<RwLock<HashMap<String, Transaction>>>,
    max_age: Duration,
    max_size_bytes: usize,
    max_transactions: usize,
    current_size_bytes: Arc<RwLock<usize>>,
}

impl Mempool {
    #[instrument]
    pub fn new(max_age_secs: u64, max_size_bytes: usize, max_transactions: usize) -> Self {
        Self {
            transactions: Arc::new(RwLock::new(HashMap::new())),
            max_age: Duration::from_secs(max_age_secs),
            max_size_bytes,
            max_transactions,
            current_size_bytes: Arc::new(RwLock::new(0)),
        }
    }

    #[instrument(skip(self, tx, utxos, dag))]
    pub async fn add_transaction(
        &self,
        tx: Transaction,
        utxos: &HashMap<String, UTXO>,
        dag: &HyperDAG,
    ) -> Result<(), MempoolError> {
        // Run verification before getting a write lock
        if let Err(e) = tx.verify(dag, utxos).await {
            return Err(MempoolError::TransactionValidation(e.to_string()));
        }

        let mut transactions = self.transactions.write().await;
        let mut current_size_bytes_guard = self.current_size_bytes.write().await;

        if transactions.len() >= self.max_transactions {
            warn!("Mempool is full (max transactions reached)");
            return Err(MempoolError::MempoolFull);
        }

        let tx_size = serde_json::to_vec(&tx).unwrap_or_default().len();
        if *current_size_bytes_guard + tx_size > self.max_size_bytes && !transactions.is_empty() {
            warn!(
                "Mempool is full (max bytes reached, tx_size: {}, current_size: {}, max_size: {})",
                tx_size, *current_size_bytes_guard, self.max_size_bytes
            );
            return Err(MempoolError::MempoolFull);
        }

        let tx_id_for_log = tx.id.clone();
        if transactions.insert(tx.id.clone(), tx).is_none() {
            *current_size_bytes_guard += tx_size;
            MEMPOOL_SIZE.set(*current_size_bytes_guard as f64);
            MEMPOOL_TRANSACTIONS.set(transactions.len() as f64);
            TRANSACTIONS_ADDED.inc();
        }
        info!("Added transaction {} to mempool", tx_id_for_log);
        Ok(())
    }

    #[instrument]
    pub async fn get_transaction(&self, tx_id: &str) -> Option<Transaction> {
        self.transactions.read().await.get(tx_id).cloned()
    }

    #[instrument]
    pub async fn remove_transaction(&self, tx_id: &str) -> Option<Transaction> {
        let mut transactions_guard = self.transactions.write().await;
        if let Some(tx) = transactions_guard.remove(tx_id) {
            let tx_size = serde_json::to_vec(&tx).unwrap_or_default().len();
            let mut current_size_bytes_guard = self.current_size_bytes.write().await;
            *current_size_bytes_guard = current_size_bytes_guard.saturating_sub(tx_size);

            MEMPOOL_SIZE.set(*current_size_bytes_guard as f64);
            MEMPOOL_TRANSACTIONS.set(transactions_guard.len() as f64);
            TRANSACTIONS_REMOVED.inc();
            Some(tx)
        } else {
            None
        }
    }
    
    /// **NEW**: Efficiently removes a slice of transactions from the mempool.
    #[instrument(skip(self, txs_to_remove))]
    pub async fn remove_transactions(&self, txs_to_remove: &[Transaction]) {
        if txs_to_remove.is_empty() {
            return;
        }

        let mut transactions_guard = self.transactions.write().await;
        let mut current_size_bytes_guard = self.current_size_bytes.write().await;
        let mut bytes_removed = 0;
        let mut count_removed = 0;

        for tx_to_remove in txs_to_remove {
            if let Some(removed_tx) = transactions_guard.remove(&tx_to_remove.id) {
                bytes_removed += serde_json::to_vec(&removed_tx).unwrap_or_default().len();
                count_removed += 1;
            }
        }

        if count_removed > 0 {
            *current_size_bytes_guard = current_size_bytes_guard.saturating_sub(bytes_removed);
            MEMPOOL_SIZE.set(*current_size_bytes_guard as f64);
            MEMPOOL_TRANSACTIONS.set(transactions_guard.len() as f64);
            TRANSACTIONS_REMOVED.inc_by(count_removed);
            info!("Removed {} transactions included in new block.", count_removed);
        }
    }

    #[instrument]
    pub async fn prune_expired(&self) {
        let now_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| MempoolError::TimestampError)
            .unwrap_or_else(|_| Duration::from_secs(0))
            .as_secs();

        let mut transactions_guard = self.transactions.write().await;
        let mut current_size_bytes_guard = self.current_size_bytes.write().await;
        let max_age_secs = self.max_age.as_secs();

        let mut expired_count = 0;
        let mut bytes_removed = 0;

        transactions_guard.retain(|_tx_id, tx| {
            if now_ts.saturating_sub(tx.timestamp) > max_age_secs {
                warn!("Pruning expired transaction: {}", tx.id);
                bytes_removed += serde_json::to_vec(tx).unwrap_or_default().len();
                expired_count += 1;
                false
            } else {
                true
            }
        });

        if expired_count > 0 {
            *current_size_bytes_guard = current_size_bytes_guard.saturating_sub(bytes_removed);
            MEMPOOL_SIZE.set(*current_size_bytes_guard as f64);
            MEMPOOL_TRANSACTIONS.set(transactions_guard.len() as f64);
            TRANSACTIONS_EXPIRED.inc_by(expired_count as u64);
            info!("Pruned {expired_count} expired transactions, {bytes_removed} bytes removed.");
        }
    }

    #[instrument(skip(self, _dag, _utxos))]
    pub async fn select_transactions(
        &self,
        _dag: &HyperDAG,
        _utxos: &HashMap<String, UTXO>,
        max_count: usize,
    ) -> Vec<Transaction> {
        let transactions_guard = self.transactions.read().await;
        transactions_guard
            .values()
            .take(max_count)
            .cloned()
            .collect()
    }

    #[instrument]
    pub async fn size(&self) -> usize {
        self.transactions.read().await.len()
    }

    #[instrument]
    pub async fn total_fees(&self) -> u64 {
        self.transactions
            .read()
            .await
            .values()
            .map(|tx| tx.fee)
            .sum()
    }

    #[instrument]
    pub async fn get_transactions(&self) -> HashMap<String, Transaction> {
        self.transactions.read().await.clone()
    }
}