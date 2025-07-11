// src/transaction.rs

use crate::hyperdag::{
    HomomorphicEncrypted, HyperDAG, LatticeSignature, UTXO,
};
use crate::omega;
use hex;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak512};
use sp_core::H256;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::instrument;
use zeroize::Zeroize;

const MAX_TRANSACTIONS_PER_MINUTE: u64 = 1000;
const MAX_METADATA_PAIRS: usize = 16;
const MAX_METADATA_KEY_LEN: usize = 64;
const MAX_METADATA_VALUE_LEN: usize = 256;

#[derive(Error, Debug)]
pub enum TransactionError {
    #[error("ΛΣ-ΩMEGA Protocol rejected the action as unstable")]
    OmegaRejection,
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
    #[error("Invalid metadata: {0}")]
    InvalidMetadata(String),
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

#[derive(Debug)]
pub struct TransactionConfig<'a> {
    pub sender: String,
    pub receiver: String,
    pub amount: u64,
    pub fee: u64,
    pub inputs: Vec<Input>,
    pub outputs: Vec<Output>,
    pub metadata: Option<HashMap<String, String>>,
    pub signing_key_bytes: &'a [u8],
    pub tx_timestamps: Arc<RwLock<HashMap<String, u64>>>,
}

/// **FIX**: This struct was created to resolve the `too_many_arguments` clippy warning.
/// It encapsulates all data that needs to be serialized for signing or verification,
/// making the function signatures cleaner and the code more maintainable.
#[derive(Debug)]
struct TransactionSigningPayload<'a> {
    sender: &'a str,
    receiver: &'a str,
    amount: u64,
    fee: u64,
    inputs: &'a [Input],
    outputs: &'a [Output],
    metadata: &'a HashMap<String, String>,
    timestamp: u64,
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
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

impl Transaction {
    #[instrument(skip(config))]
    pub async fn new(config: TransactionConfig<'_>) -> Result<Self, TransactionError> {
        Self::validate_structure_pre_creation(
            &config.sender,
            &config.receiver,
            config.amount,
            &config.inputs,
            config.metadata.as_ref(),
        )?;
        Self::check_rate_limit(&config.tx_timestamps, MAX_TRANSACTIONS_PER_MINUTE).await?;
        Self::validate_addresses(&config.sender, &config.receiver, &config.outputs)?;

        let timestamp = Self::get_current_timestamp()?;
        let metadata = config.metadata.unwrap_or_default();

        // FIX: Use the new payload struct.
        let signing_payload = TransactionSigningPayload {
            sender: &config.sender,
            receiver: &config.receiver,
            amount: config.amount,
            fee: config.fee,
            inputs: &config.inputs,
            outputs: &config.outputs,
            metadata: &metadata,
            timestamp,
        };

        let signature_data = Self::serialize_for_signing(&signing_payload)?;

        let action_hash =
            H256::from_slice(Keccak512::digest(&signature_data).as_slice()[..32].as_ref());
        if !omega::reflect_on_action(action_hash).await {
            return Err(TransactionError::OmegaRejection);
        }

        let signature_obj = LatticeSignature::sign(config.signing_key_bytes, &signature_data)
            .map_err(|_| TransactionError::LatticeSignatureVerification)?;

        let mut tx = Self {
            id: String::new(),
            sender: config.sender,
            receiver: config.receiver,
            amount: config.amount,
            fee: config.fee,
            inputs: config.inputs,
            outputs: config.outputs,
            lattice_signature: signature_obj.signature,
            public_key: signature_obj.public_key,
            timestamp,
            metadata,
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

    pub(crate) fn new_coinbase(
        receiver: String,
        reward: u64,
        signing_key_bytes: &[u8],
        outputs: Vec<Output>,
    ) -> Result<Self, TransactionError> {
        let sender = "0000000000000000000000000000000000000000000000000000000000000000".to_string();
        let timestamp = Self::get_current_timestamp()?;
        let metadata = HashMap::new(); // Coinbase transactions have no metadata.

        // FIX: Use the new payload struct.
        let signing_payload = TransactionSigningPayload {
            sender: &sender,
            receiver: &receiver,
            amount: reward,
            fee: 0,
            inputs: &[],
            outputs: &outputs,
            metadata: &metadata,
            timestamp,
        };

        let signature_data = Self::serialize_for_signing(&signing_payload)?;

        let signature_obj = LatticeSignature::sign(signing_key_bytes, &signature_data)
            .map_err(|_| TransactionError::LatticeSignatureVerification)?;

        let mut tx = Self {
            id: String::new(),
            sender,
            receiver,
            amount: reward,
            fee: 0,
            inputs: vec![],
            outputs,
            lattice_signature: signature_obj.signature,
            public_key: signature_obj.public_key,
            timestamp,
            metadata,
        };
        tx.id = tx.compute_hash();
        Ok(tx)
    }

    fn validate_structure_pre_creation(
        sender: &str,
        receiver: &str,
        amount: u64,
        inputs: &[Input],
        metadata: Option<&HashMap<String, String>>,
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
        if let Some(md) = metadata {
            if md.len() > MAX_METADATA_PAIRS {
                return Err(TransactionError::InvalidMetadata(format!(
                    "Exceeded max metadata pairs limit of {MAX_METADATA_PAIRS}"
                )));
            }
            for (k, v) in md {
                if k.len() > MAX_METADATA_KEY_LEN || v.len() > MAX_METADATA_VALUE_LEN {
                    return Err(TransactionError::InvalidMetadata(format!("Metadata key/value length exceeded (max k: {MAX_METADATA_KEY_LEN}, v: {MAX_METADATA_VALUE_LEN})")));
                }
            }
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

    /// **FIX**: This function was refactored to accept a single `TransactionSigningPayload` struct.
    /// This resolves the `too_many_arguments` clippy warning and improves code organization.
    fn serialize_for_signing(
        payload: &TransactionSigningPayload,
    ) -> Result<Vec<u8>, TransactionError> {
        let mut hasher = Keccak512::new();
        hasher.update(payload.sender.as_bytes());
        hasher.update(payload.receiver.as_bytes());
        hasher.update(payload.amount.to_be_bytes());
        hasher.update(payload.fee.to_be_bytes());
        for input in payload.inputs {
            hasher.update(input.tx_id.as_bytes());
            hasher.update(input.output_index.to_be_bytes());
        }
        for output in payload.outputs {
            hasher.update(output.address.as_bytes());
            hasher.update(output.amount.to_be_bytes());
        }
        let mut sorted_metadata: Vec<_> = payload.metadata.iter().collect();
        sorted_metadata.sort_by_key(|(k, _)| *k);
        for (key, value) in sorted_metadata {
            hasher.update(key.as_bytes());
            hasher.update(value.as_bytes());
        }

        hasher.update(payload.timestamp.to_be_bytes());
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
        let mut sorted_metadata: Vec<_> = self.metadata.iter().collect();
        sorted_metadata.sort_by_key(|(k, _)| *k);
        for (key, value) in sorted_metadata {
            hasher.update(key.as_bytes());
            hasher.update(value.as_bytes());
        }
        hasher.update(self.timestamp.to_be_bytes());
        hex::encode(&hasher.finalize()[..32])
    }

    #[instrument(skip(self, dag, utxos))]
    pub async fn verify(
        &self,
        dag: &HyperDAG,
        utxos: &HashMap<String, UTXO>,
    ) -> Result<(), TransactionError> {
        let verifier_lattice_sig = LatticeSignature {
            public_key: self.public_key.clone(),
            signature: self.lattice_signature.clone(),
        };

        // FIX: Use the new payload struct for verification.
        let signing_payload = TransactionSigningPayload {
            sender: &self.sender,
            receiver: &self.receiver,
            amount: self.amount,
            fee: self.fee,
            inputs: &self.inputs,
            outputs: &self.outputs,
            metadata: &self.metadata,
            timestamp: self.timestamp,
        };
        let signature_data_to_verify = Transaction::serialize_for_signing(&signing_payload)?;

        if !verifier_lattice_sig.verify(&signature_data_to_verify) {
            return Err(TransactionError::LatticeSignatureVerification);
        }

        if self.inputs.is_empty() {
            // This is a coinbase transaction, special validation rules apply.
            if self.fee != 0 {
                return Err(TransactionError::InvalidStructure(
                    "Coinbase transaction fee must be 0".to_string(),
                ));
            }

            // The total output must match the block's reward field.
            // Note: This check is now primarily done in `HyperDAG::is_valid_block`,
            // as the transaction itself doesn't know the block's context.
            // We perform a basic sanity check here.
            let total_output_amount: u64 = self.outputs.iter().map(|o| o.amount).sum();
            if total_output_amount == 0 {
                 return Err(TransactionError::InvalidStructure(
                    "Coinbase transaction must have a non-zero output".to_string(),
                ));
            }

        } else {
            // This is a regular transaction.
            let mut total_input_value = 0;
            for input_val in &self.inputs {
                let utxo_id = format!("{}_{}", input_val.tx_id, input_val.output_index);
                let utxo_entry = utxos.get(&utxo_id).ok_or_else(|| {
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

            let total_output_value: u64 = self.outputs.iter().map(|o| o.amount).sum();

            // The total value of inputs must equal the total value of outputs plus the fee.
            if total_input_value != total_output_value + self.fee {
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
        // Placeholder for a more complex anomaly detection model.
        // A real implementation would compare against historical averages.
        let placeholder_avg_amount = 1000.0;
        let anomaly_score = if self.amount > 0 {
            (self.amount as f64 / placeholder_avg_amount).abs()
        } else {
            0.0
        };
        // A transaction 1000x the average amount is considered highly anomalous.
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
        self.metadata.clear();
    }
}

impl Drop for Transaction {
    fn drop(&mut self) {
        self.zeroize();
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::hyperdag::HyperDAG;
    use crate::saga::PalletSaga;
    use crate::wallet::Wallet;
    use serial_test::serial;

    #[tokio::test]
    #[serial]
    async fn test_transaction_creation_and_verification() -> Result<(), Box<dyn std::error::Error>>
    {
        if std::path::Path::new("hyperdag_db").exists() {
            std::fs::remove_dir_all("hyperdag_db")?;
        }

        let wallet = Arc::new(Wallet::new()?);

        let signing_key_dalek = wallet.get_signing_key()?;
        let signing_key_bytes_slice: &[u8] = &signing_key_dalek.to_bytes();

        let sender_address = wallet.address();

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

        let he_public_key_dalek = wallet.get_signing_key()?.verifying_key();
        let he_pub_key_material_slice: &[u8] = he_public_key_dalek.as_bytes();

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

        let mut metadata = HashMap::new();
        metadata.insert("memo".to_string(), "Test transaction".to_string());

        let tx_timestamps_map = Arc::new(RwLock::new(HashMap::new()));

        let tx_config = TransactionConfig {
            sender: sender_address.clone(),
            receiver: "0000000000000000000000000000000000000000000000000000000000000001"
                .to_string(),
            amount: amount_to_receiver,
            fee,
            inputs: inputs_for_tx.clone(),
            outputs: outputs_for_tx.clone(),
            metadata: Some(metadata),
            signing_key_bytes: signing_key_bytes_slice,
            tx_timestamps: tx_timestamps_map.clone(),
        };

        let tx = Transaction::new(tx_config).await?;

        let dag_signing_key_dalek_for_dag = wallet.get_signing_key()?;
        let dag_signing_key_bytes_slice: &[u8] = &dag_signing_key_dalek_for_dag.to_bytes();

        let saga_pallet = Arc::new(PalletSaga::new());
        
        let dag_instance = HyperDAG::new(
            &sender_address,
            60000,
            100,
            1,
            dag_signing_key_bytes_slice,
            saga_pallet,
        )
        .await
        .map_err(|e| format!("DAG creation error for test: {e:?}"))?;

        let dag_arc_for_test = Arc::new(RwLock::new(dag_instance));
        dag_arc_for_test.write().await.init_self_arc(dag_arc_for_test.clone());


        let utxos_arc_for_test = Arc::new(RwLock::new(initial_utxos_map));

        let dag_read_guard = dag_arc_for_test.read().await;
        let utxos_read_guard = utxos_arc_for_test.read().await;
        tx.verify(&dag_read_guard, &utxos_read_guard)
            .await
            .map_err(|e| format!("TX verification error: {e:?}"))?;

        let generated_utxo_instance = tx.generate_utxo(0);
        assert_eq!(generated_utxo_instance.tx_id, tx.id);
        assert_eq!(generated_utxo_instance.amount, amount_to_receiver);

        let anomaly_score_value = tx
            .detect_anomaly(&tx_timestamps_map)
            .await
            .map_err(|e| format!("Anomaly detection error: {e:?}"))?;
        assert!(anomaly_score_value >= 0.0);

        Ok(())
    }
}
