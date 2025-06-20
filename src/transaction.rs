use crate::hyperdag::{HomomorphicEncrypted, HyperDAG, LatticeSignature};
use hex;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak512};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::instrument;
use zeroize::Zeroize;

pub const DEV_ADDRESS: &str = "2119707c4caf16139cfb5c09c4dcc9bf9cfe6808b571c108d739f49cc14793b9";
pub const DEV_FEE_RATE: f64 = 0.0304;
const MAX_TRANSACTIONS_PER_MINUTE: u64 = 1000;

#[derive(Error, Debug)]
pub enum TransactionError {
    #[error("Invalid address format")]
    InvalidAddress,
    #[error("Lattice signature verification failed")]
    LatticeSignatureVerification,
    #[error("Insufficient funds")]
    InsufficientFunds,
    #[error("Missing developer fee")]
    MissingDevFee,
    #[error("Invalid transaction structure: {0}")]
    InvalidStructure(String),
    #[error("Homomorphic encryption error: {0}")]
    HomomorphicError(String),
    #[error("Rate limit exceeded")]
    RateLimitExceeded,
    #[error("Anomaly detected: {0}")]
    AnomalyDetected(String),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Timestamp error")]
    TimestampError,
    #[error("Emission calculation error: {0}")]
    EmissionError(String),
    #[error("Wallet error: {0}")]
    Wallet(#[from] crate::wallet::WalletError),
}

#[derive(Clone, Serialize, Deserialize, Debug, Hash, Eq, PartialEq)]
pub struct Input {
    pub tx_id: String,
    pub output_index: u32,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Output {
    pub address: String,
    pub amount: u64,
    pub homomorphic_encrypted: HomomorphicEncrypted,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct UTXO {
    pub address: String,
    pub amount: u64,
    pub tx_id: String,
    pub output_index: u32,
    pub explorer_link: String,
}

// Struct to hold configuration for a new Transaction
#[derive(Debug)]
pub struct TransactionConfig<'a> {
    pub sender: String,
    pub receiver: String,
    pub amount: u64,
    pub fee: u64,
    pub inputs: Vec<Input>,
    pub outputs: Vec<Output>,
    pub signing_key_bytes: &'a [u8],
    pub tx_timestamps: Arc<RwLock<HashMap<String, u64>>>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Transaction {
    pub id: String,
    pub sender: String,
    pub receiver: String,
    pub amount: u64,
    pub fee: u64,
    pub inputs: Vec<Input>,
    pub outputs: Vec<Output>,
    pub lattice_signature: Vec<u8>,
    pub public_key: Vec<u8>,
    pub timestamp: u64,
}

impl Transaction {
    #[instrument(skip(config))]
    pub async fn new(config: TransactionConfig<'_>) -> Result<Self, TransactionError> {
        Self::validate_structure_pre_creation(
            &config.sender,
            &config.receiver,
            config.amount,
            &config.inputs,
        )?;
        Self::check_rate_limit(&config.tx_timestamps, MAX_TRANSACTIONS_PER_MINUTE).await?;
        Self::validate_addresses(&config.sender, &config.receiver, &config.outputs)?;

        let timestamp = Self::get_current_timestamp()?;

        let lattice_signature_obj = LatticeSignature::new(config.signing_key_bytes);

        let signature_data = Self::serialize_for_signing(
            &config.sender,
            &config.receiver,
            config.amount,
            config.fee,
            &config.inputs,
            &config.outputs,
            timestamp,
        )?;
        let signature = lattice_signature_obj.sign(&signature_data);

        let mut tx = Self {
            id: String::new(),
            sender: config.sender,
            receiver: config.receiver,
            amount: config.amount,
            fee: config.fee,
            inputs: config.inputs,
            outputs: config.outputs,
            lattice_signature: signature,
            public_key: lattice_signature_obj.public_key.clone(),
            timestamp,
        };
        tx.id = tx.compute_hash();

        let mut timestamps_guard = config.tx_timestamps.write().await;
        timestamps_guard.insert(tx.id.clone(), timestamp);
        if timestamps_guard.len() > (MAX_TRANSACTIONS_PER_MINUTE * 2) as usize {
            let current_time = Self::get_current_timestamp().unwrap_or(0);
            timestamps_guard
                .retain(|_, &mut stored_ts| current_time.saturating_sub(stored_ts) < 3600);
        }
        Ok(tx)
    }

    fn validate_structure_pre_creation(
        sender: &str,
        receiver: &str,
        amount: u64,
        inputs: &[Input],
    ) -> Result<(), TransactionError> {
        if sender.is_empty() {
            return Err(TransactionError::InvalidStructure(
                "Sender address cannot be empty".to_string(),
            ));
        }
        if receiver.is_empty() {
            return Err(TransactionError::InvalidStructure(
                "Receiver address cannot be empty".to_string(),
            ));
        }
        if amount == 0 && !inputs.is_empty() {
            return Err(TransactionError::InvalidStructure(
                "Amount cannot be zero for regular (non-coinbase) transactions".to_string(),
            ));
        }
        Ok(())
    }

    async fn check_rate_limit(
        tx_timestamps: &Arc<RwLock<HashMap<String, u64>>>,
        max_txs: u64,
    ) -> Result<(), TransactionError> {
        let now = Self::get_current_timestamp()?;
        let timestamps_guard = tx_timestamps.read().await;
        let recent_tx_count = timestamps_guard
            .values()
            .filter(|&&t| now.saturating_sub(t) < 60)
            .count() as u64;
        if recent_tx_count >= max_txs {
            return Err(TransactionError::RateLimitExceeded);
        }
        Ok(())
    }

    fn validate_addresses(
        sender: &str,
        receiver: &str,
        outputs: &[Output],
    ) -> Result<(), TransactionError> {
        if !Self::is_valid_address(sender) {
            return Err(TransactionError::InvalidAddress);
        }
        if !Self::is_valid_address(receiver) {
            return Err(TransactionError::InvalidAddress);
        }
        for output in outputs {
            if !Self::is_valid_address(&output.address) {
                return Err(TransactionError::InvalidAddress);
            }
        }
        Ok(())
    }

    fn is_valid_address(address: &str) -> bool {
        address.len() == 64 && hex::decode(address).is_ok()
    }

    fn get_current_timestamp() -> Result<u64, TransactionError> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .map_err(|_| TransactionError::TimestampError)
    }

    fn serialize_for_signing(
        sender: &str,
        receiver: &str,
        amount: u64,
        fee: u64,
        inputs: &[Input],
        outputs: &[Output],
        timestamp: u64,
    ) -> Result<Vec<u8>, TransactionError> {
        let mut hasher = Keccak512::new();
        hasher.update(sender.as_bytes());
        hasher.update(receiver.as_bytes());
        hasher.update(amount.to_be_bytes());
        hasher.update(fee.to_be_bytes());
        for input in inputs {
            hasher.update(input.tx_id.as_bytes());
            hasher.update(input.output_index.to_be_bytes());
        }
        for output in outputs {
            hasher.update(output.address.as_bytes());
            hasher.update(output.amount.to_be_bytes());
        }
        hasher.update(timestamp.to_be_bytes());
        Ok(hasher.finalize().to_vec())
    }

    fn compute_hash(&self) -> String {
        let mut hasher = Keccak512::new();
        hasher.update(self.sender.as_bytes());
        hasher.update(self.receiver.as_bytes());
        hasher.update(self.amount.to_be_bytes());
        hasher.update(self.fee.to_be_bytes());
        for input_val in &self.inputs {
            hasher.update(input_val.tx_id.as_bytes());
            hasher.update(input_val.output_index.to_be_bytes());
        }
        for output_val in &self.outputs {
            hasher.update(output_val.address.as_bytes());
            hasher.update(output_val.amount.to_be_bytes());
        }
        hasher.update(self.timestamp.to_be_bytes());
        hex::encode(&hasher.finalize()[..32])
    }

    #[instrument]
    pub async fn verify(
        &self,
        dag: &Arc<RwLock<HyperDAG>>,
        utxos: &Arc<RwLock<HashMap<String, UTXO>>>,
    ) -> Result<(), TransactionError> {
        let dag_read_guard = dag.read().await;
        let utxos_read_guard = utxos.read().await;

        let verifier_lattice_sig = LatticeSignature {
            public_key: self.public_key.clone(),
            signature: self.lattice_signature.clone(),
        };
        let signature_data_to_verify = Transaction::serialize_for_signing(
            &self.sender,
            &self.receiver,
            self.amount,
            self.fee,
            &self.inputs,
            &self.outputs,
            self.timestamp,
        )?;
        if !verifier_lattice_sig.verify(&signature_data_to_verify, &self.lattice_signature) {
            return Err(TransactionError::LatticeSignatureVerification);
        }

        let mut total_input_value = 0;
        for input_val in &self.inputs {
            let utxo_id = format!("{}_{}", input_val.tx_id, input_val.output_index);
            let utxo_entry = utxos_read_guard.get(&utxo_id).ok_or_else(|| {
                TransactionError::InvalidStructure(format!("UTXO {utxo_id} not found for input"))
            })?;
            if utxo_entry.address != self.sender {
                return Err(TransactionError::InvalidStructure(format!(
                    "Input UTXO {} does not belong to sender {}",
                    utxo_id, self.sender
                )));
            }
            total_input_value += utxo_entry.amount;
        }

        let calculated_sum_of_outputs = self.outputs.iter().map(|o| o.amount).sum::<u64>();

        if self.inputs.is_empty() {
            // Coinbase transaction
            let expected_reward = dag_read_guard
                .emission
                .calculate_reward(self.timestamp)
                .map_err(TransactionError::EmissionError)?;
            if self.fee != 0 {
                return Err(TransactionError::InvalidStructure(
                    "Coinbase transaction fee must be 0".to_string(),
                ));
            }
            if calculated_sum_of_outputs != expected_reward {
                return Err(TransactionError::InvalidStructure(
                    format!("Invalid coinbase transaction output sum: expected {expected_reward}, got {calculated_sum_of_outputs}"),
                ));
            }
        } else {
            // Regular transaction
            let dev_fee_on_transfer_amount = (self.amount as f64 * DEV_FEE_RATE).round() as u64;
            if dev_fee_on_transfer_amount > 0
                && !self
                    .outputs
                    .iter()
                    .any(|o| o.address == DEV_ADDRESS && o.amount == dev_fee_on_transfer_amount)
            {
                return Err(TransactionError::MissingDevFee);
            }
            if total_input_value < calculated_sum_of_outputs + self.fee {
                return Err(TransactionError::InsufficientFunds);
            }
        }
        Ok(())
    }

    #[instrument]
    pub fn generate_utxo(&self, index: u32) -> UTXO {
        let output = &self.outputs[index as usize];
        let utxo_id = format!("{}_{}", self.id, index);
        UTXO {
            address: output.address.clone(),
            amount: output.amount,
            tx_id: self.id.clone(),
            output_index: index,
            explorer_link: format!("https://hyperblockexplorer.org/utxo/{utxo_id}"),
        }
    }

    #[instrument]
    pub async fn detect_anomaly(
        &self,
        recent_tx_timestamps: &Arc<RwLock<HashMap<String, u64>>>,
    ) -> Result<f64, TransactionError> {
        let now = Self::get_current_timestamp()?;
        let timestamps_guard = recent_tx_timestamps.read().await;
        let _recent_tx_count = timestamps_guard
            .values()
            .filter(|&&t| now.saturating_sub(t) < 60)
            .count();
        let placeholder_avg_amount = 1.0;
        let anomaly_score = if self.amount > 0 {
            (self.amount as f64 / placeholder_avg_amount).abs()
        } else {
            0.0
        };
        if anomaly_score > 1000.0 {
            return Err(TransactionError::AnomalyDetected(format!(
                "Transaction amount {} is anomalously large (score: {})",
                self.amount, anomaly_score
            )));
        }
        Ok(anomaly_score / 1000.0)
    }

    #[instrument]
    pub async fn apply(
        &mut self,
        utxos: &mut Arc<RwLock<HashMap<String, UTXO>>>,
    ) -> Result<(), TransactionError> {
        let mut utxos_writer_guard = utxos.write().await;
        for input_val in &self.inputs {
            let utxo_id = format!("{}_{}", input_val.tx_id, input_val.output_index);
            utxos_writer_guard.remove(&utxo_id).ok_or_else(|| {
                TransactionError::InvalidStructure(format!(
                    "UTXO {utxo_id} not found for removal during apply"
                ))
            })?;
        }
        for (index_val, _output_val) in self.outputs.iter().enumerate() {
            let utxo_to_add = self.generate_utxo(index_val as u32);
            utxos_writer_guard.insert(format!("{}_{}", self.id, index_val), utxo_to_add);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hyperdag::HyperDAG;
    use crate::wallet::HyperWallet;
    use serial_test::serial; // Added

    #[tokio::test]
    #[serial] // Added to ensure serial execution
    async fn test_transaction_creation_and_verification() -> Result<(), Box<dyn std::error::Error>>
    {
        // Clean up the database directory before the test to ensure a clean state
        if std::path::Path::new("hyperdag_db").exists() {
            std::fs::remove_dir_all("hyperdag_db")?;
        }

        let wallet = Arc::new(HyperWallet::new()?);

        let signing_key_dalek = wallet.get_signing_key()?;
        let signing_key_bytes_slice: &[u8] = &signing_key_dalek.to_bytes();

        let sender_address = wallet.get_address();

        let amount_to_receiver = 50;
        let fee = 5;
        let dev_fee_on_transfer = (amount_to_receiver as f64 * DEV_FEE_RATE).round() as u64;

        let mut initial_utxos_map = HashMap::new();
        let input_utxo_amount = amount_to_receiver + fee + dev_fee_on_transfer + 10;
        let genesis_utxo_for_test = UTXO {
            address: sender_address.clone(),
            amount: input_utxo_amount,
            tx_id: "genesis_tx_id_for_test_0".to_string(),
            output_index: 0,
            explorer_link: String::new(),
        };
        initial_utxos_map.insert(
            "genesis_tx_id_for_test_0_0".to_string(),
            genesis_utxo_for_test,
        );

        let inputs_for_tx = vec![Input {
            tx_id: "genesis_tx_id_for_test_0".to_string(),
            output_index: 0,
        }];

        let change_amount = input_utxo_amount - amount_to_receiver - fee - dev_fee_on_transfer;

        let he_public_key_dalek = wallet.get_public_key()?;
        let he_pub_key_material_slice: &[u8] = &he_public_key_dalek.to_bytes();

        let mut outputs_for_tx = vec![Output {
            address: "0000000000000000000000000000000000000000000000000000000000000001".to_string(),
            amount: amount_to_receiver,
            homomorphic_encrypted: HomomorphicEncrypted::new(
                amount_to_receiver,
                he_pub_key_material_slice,
            ),
        }];
        if dev_fee_on_transfer > 0 {
            outputs_for_tx.push(Output {
                address: DEV_ADDRESS.to_string(),
                amount: dev_fee_on_transfer,
                homomorphic_encrypted: HomomorphicEncrypted::new(
                    dev_fee_on_transfer,
                    he_pub_key_material_slice,
                ),
            });
        }
        if change_amount > 0 {
            outputs_for_tx.push(Output {
                address: sender_address.clone(),
                amount: change_amount,
                homomorphic_encrypted: HomomorphicEncrypted::new(
                    change_amount,
                    he_pub_key_material_slice,
                ),
            });
        }

        let tx_timestamps_map = Arc::new(RwLock::new(HashMap::new()));

        let tx_config = TransactionConfig {
            sender: sender_address.clone(),
            receiver: "0000000000000000000000000000000000000000000000000000000000000001"
                .to_string(),
            amount: amount_to_receiver,
            fee,
            inputs: inputs_for_tx.clone(),
            outputs: outputs_for_tx.clone(),
            signing_key_bytes: signing_key_bytes_slice,
            tx_timestamps: tx_timestamps_map.clone(),
        };

        let tx = Transaction::new(tx_config).await?;

        let dag_signing_key_dalek_for_dag = wallet.get_signing_key()?;
        let dag_signing_key_bytes_slice: &[u8] = &dag_signing_key_dalek_for_dag.to_bytes();

        let dag_instance_for_test =
            HyperDAG::new(&sender_address, 60000, 100, 1, dag_signing_key_bytes_slice)
                .await
                .map_err(|e| format!("DAG creation error for test: {:?}", e))?;

        let dag_arc_for_test = Arc::new(RwLock::new(dag_instance_for_test));
        let utxos_arc_for_test = Arc::new(RwLock::new(initial_utxos_map));

        tx.verify(&dag_arc_for_test, &utxos_arc_for_test)
            .await
            .map_err(|e| format!("TX verification error: {:?}", e))?;

        let generated_utxo_instance = tx.generate_utxo(0);
        assert_eq!(generated_utxo_instance.tx_id, tx.id);
        assert_eq!(generated_utxo_instance.amount, amount_to_receiver);

        let anomaly_score_value = tx
            .detect_anomaly(&tx_timestamps_map)
            .await
            .map_err(|e| format!("Anomaly detection error: {:?}", e))?;
        assert!(anomaly_score_value >= 0.0);

        Ok(())
    }
}

impl Zeroize for Transaction {
    fn zeroize(&mut self) {
        self.id.zeroize();
        self.sender.zeroize();
        self.receiver.zeroize();
        self.amount = 0;
        self.fee = 0;
        self.inputs.clear();
        self.outputs.clear();
        self.lattice_signature.zeroize();
        self.public_key.zeroize();
        self.timestamp = 0;
    }
}

impl Drop for Transaction {
    fn drop(&mut self) {
        self.zeroize();
    }
}
