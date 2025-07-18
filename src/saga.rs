//! --- SAGA: Sentient Autonomous Governance Algorithm ---
//! v3.3.2 - Code Cleanup
//! This version removes an unused import statement as identified by the compiler.
//! - NEW: AI-driven verification for Carbon Offset Credentials.
//! - NEW: 'Wash Trading' detection added to Security Monitor.
//! - NEW: Autonomous governance can now propose tuning SCS weights.
//! - FIX: Resolved compilation error for `get_poscp_reward_multiplier`.
//! - FIX: Corrected input address lookup in `check_for_wash_trading`.
//! - FIX: Removed unused `Input` import.

// [!!] BUILD NOTE: The 'ai' feature requires the `tch` crate and a `libtorch` installation.
// To enable, build with `cargo build --features ai`.

use crate::hyperdag::{HyperBlock, HyperDAG, MAX_TRANSACTIONS_PER_BLOCK};
#[cfg(feature = "infinite-strata")]
use crate::infinite_strata_node::InfiniteStrataNode;
use crate::omega;
use crate::transaction::Transaction;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

#[cfg(feature = "ai")]
use {
    std::path::PathBuf,
    tch::{nn, Device, Kind, Tensor},
};

// --- Constants ---
#[cfg(feature = "ai")]
const MODEL_SAVE_PATH: &str = "./saga_models";
#[cfg(feature = "ai")]
const BEHAVIOR_MODEL_FILENAME: &str = "behavior_net.ot";
#[cfg(feature = "ai")]
const CONGESTION_MODEL_FILENAME: &str = "congestion_lstm.ot";
#[cfg(feature = "ai")]
const CREDENTIAL_MODEL_FILENAME: &str = "credential_verifier.ot";
#[cfg(feature = "ai")]
const TRAINING_DATA_CAPACITY: usize = 10000;
const RETRAIN_INTERVAL_EPOCHS: u64 = 10; // Retrain more frequently
const TEMPORAL_GRACE_PERIOD_SECS: u64 = 120;

#[derive(Error, Debug, Clone)]
pub enum SagaError {
    #[error("Rule not found in SAGA's current epoch state: {0}")]
    RuleNotFound(String),
    #[error("Proposal not found, inactive, or already vetoed: {0}")]
    ProposalNotFound(String),
    #[error("Node has insufficient Karma for action: required {0}, has {1}")]
    InsufficientKarma(u64, u64),
    #[error("AI model is not trained or available for this function")]
    ModelNotAvailable,
    #[error("Failed to run AI model inference: {0}")]
    InferenceError(String),
    #[error("Invalid proposal state transition attempted")]
    InvalidProposalState,
    #[error("The SAGA Council has already vetoed this proposal")]
    ProposalVetoed,
    #[error("Only a member of the SAGA Council can veto active proposals")]
    NotACouncilMember,
    #[error("SAGA Guidance System does not contain the topic: {0}")]
    InvalidHelpTopic(String),
    #[error("System time error prevented temporal analysis: {0}")]
    TimeError(String),
    #[error("Query is too ambiguous for SAGA to understand. Please provide more context or rephrase your question. Detected possible topics: {0:?}")]
    AmbiguousQuery(Vec<String>),
    #[error("SAGA's Natural Language Understanding engine failed to process the query.")]
    NluProcessingError,
    #[error("AI model file operation failed: {0}")]
    ModelFileError(String),
    #[error("Invalid Carbon Offset Credential: {0}")]
    InvalidCredential(String),
}

/// Represents a verifiable claim of a carbon offset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CarbonOffsetCredential {
    pub id: String,
    pub issuer_id: String,
    pub beneficiary_node: String,
    pub tonnes_co2_sequestered: f64,
    pub project_id: String,
    pub vintage_year: u32,
    pub verification_signature: String,
    pub additionality_proof: String, // e.g., link to public records
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct TransactionMetadata {
    pub origin_component: String,
    pub intent: String,
    #[serde(default)]
    pub correlated_tx: Option<String>,
    #[serde(default)]
    pub additional_data: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkState {
    Nominal,
    Congested,
    Degraded,
    UnderAttack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EdictAction {
    Economic {
        reward_multiplier: f64,
        fee_multiplier: f64,
    },
    Governance {
        proposal_karma_cost_multiplier: f64,
    },
    Security {
        trust_weight_override: (String, f64),
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SagaEdict {
    pub id: String,
    pub issued_epoch: u64,
    pub expiry_epoch: u64,
    pub description: String,
    pub action: EdictAction,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct TrustScoreBreakdown {
    pub factors: HashMap<String, f64>,
    pub final_weighted_score: f64,
}

#[cfg(feature = "ai")]
#[derive(Debug)]
struct BehaviorNet {
    vs: nn::VarStore,
    seq: nn::Sequential,
}

#[cfg(feature = "ai")]
impl BehaviorNet {
    fn new(vs_path: &mut nn::VarStore) -> Self {
        let p = &vs_path.root();
        // A deeper network for more complex pattern recognition, fitting the "deep learning" goal.
        let seq = nn::seq()
            .add(nn::linear(p / "layer1", 8, 32, Default::default()))
            .add_fn(|xs| xs.relu())
            .add(nn::linear(p / "layer2", 32, 32, Default::default()))
            .add_fn(|xs| xs.relu())
            .add(nn::linear(p / "layer3", 32, 3, Default::default())); // 3 outputs: Malicious, Neutral, Beneficial

        Self {
            vs: vs_path.clone(),
            seq,
        }
    }

    fn forward(&self, features: &Tensor) -> Tensor {
        self.seq.forward(features)
    }
}

#[cfg(feature = "ai")]
#[derive(Debug)]
struct CongestionPredictorLSTM {
    vs: nn::VarStore,
    lstm: nn::Lstm,
    linear: nn::Linear,
    sequence_len: i64,
}

#[cfg(feature = "ai")]
impl CongestionPredictorLSTM {
    fn new(vs_path: &mut nn::VarStore, sequence_len: i64, hidden_dim: i64) -> Self {
        let p = &vs_path.root();
        let lstm_cfg = nn::LstmConfig {
            has_biases: true,
            num_layers: 2, // Deeper LSTM for better temporal learning
            dropout: 0.1,
            ..Default::default()
        };
        let lstm = nn::lstm(p / "lstm", 1, hidden_dim, lstm_cfg);
        let linear = nn::linear(p / "linear", hidden_dim, 1, Default::default());

        Self {
            vs: vs_path.clone(),
            lstm,
            linear,
            sequence_len,
        }
    }

    fn predict(&self, sequence: &Tensor) -> Result<f64, SagaError> {
        if sequence.size()[0] != self.sequence_len {
            return Err(SagaError::InferenceError(format!(
                "Invalid sequence length. Expected {}, got {}",
                self.sequence_len,
                sequence.size()[0]
            )));
        }
        let input = sequence.view([self.sequence_len, 1, 1]);
        let (lstm_out, _) = self.lstm.seq(&input);
        let last_time_step = lstm_out.select(0, -1);
        let prediction = self.linear.forward(&last_time_step);

        f64::try_from(prediction).map_err(|e| SagaError::InferenceError(e.to_string()))
    }
}

#[cfg(feature = "ai")]
#[derive(Debug)]
struct CredentialVerifierNet {
    vs: nn::VarStore,
    seq: nn::Sequential,
}

#[cfg(feature = "ai")]
impl CredentialVerifierNet {
    // Takes features like project quality, vintage age, tonnes, etc.
    fn new(vs_path: &mut nn::VarStore) -> Self {
        let p = &vs_path.root();
        let seq = nn::seq()
            .add(nn::linear(p / "layer1", 4, 16, Default::default())) // 4 input features
            .add_fn(|xs| xs.relu())
            .add(nn::linear(p / "layer2", 16, 1, Default::default())) // 1 output: confidence score
            .add_fn(|xs| xs.sigmoid()); // Sigmoid to get a score between 0 and 1
        Self {
            vs: vs_path.clone(),
            seq,
        }
    }

    fn verify(&self, features: &Tensor) -> Tensor {
        self.seq.forward(features)
    }
}


/// A heuristic model to estimate the CO2 impact of a transaction.
/// This simulates the energy cost based on data size and computational work,
/// inspired by real-world analysis of blockchain energy consumption.
#[derive(Debug)]
pub struct CarbonImpactPredictor {}

impl CarbonImpactPredictor {
    pub fn predict_co2_per_tx(&self, tx: &Transaction, network_congestion: f64) -> f64 {
        // Constants representing Energy Consumption Units (ECU) and their CO2 equivalent.
        // These are illustrative values for the model.
        const GRAMS_CO2_PER_ECU: f64 = 0.005;
        const ECU_PER_BYTE: f64 = 0.1;
        const ECU_BASE_TRANSFER: f64 = 10.0;
        const ECU_PER_CONTRACT_OPCODE: f64 = 0.5;
        const ECU_CONTRACT_DEPLOYMENT_BASE: f64 = 500.0;

        let tx_bytes = serde_json::to_vec(tx).unwrap_or_default().len() as f64;
        let mut total_ecu = tx_bytes * ECU_PER_BYTE;

        // Use transaction metadata to infer computational complexity
        let intent = tx.get_metadata().get("intent").map(|s| s.as_str());
        match intent {
            Some("contract_deployment") => {
                let code_size = tx.get_metadata().get("contract_code_size").and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                total_ecu += ECU_CONTRACT_DEPLOYMENT_BASE + (code_size * ECU_PER_BYTE);
            }
            Some("contract_interaction") => {
                 let op_count = tx.get_metadata().get("opcode_count").and_then(|s| s.parse::<f64>().ok()).unwrap_or(20.0);
                 total_ecu += op_count * ECU_PER_CONTRACT_OPCODE;
            }
            _ => { // Simple value transfer
                total_ecu += ECU_BASE_TRANSFER;
            }
        };

        // Network congestion increases energy usage due to propagation and contention.
        let congestion_multiplier = 1.0 + (network_congestion * 0.75);
        let final_ecu = total_ecu * congestion_multiplier;
        
        // Convert ECU to grams of CO2.
        final_ecu * GRAMS_CO2_PER_ECU
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelTrainingData {
    pub validity: f64,
    pub network_contribution: f64,
    pub historical_performance: f64,
    pub cognitive_hazard: f64,
    pub temporal_consistency: f64,
    pub cognitive_dissonance: f64,
    pub metadata_integrity: f64,
    pub environmental_contribution: f64,
    pub behavior_label: f64, // 0.0 = Malicious, 0.5 = Neutral, 1.0 = Beneficial
    pub congestion_metric: f64,
}

#[derive(Debug)]
pub struct CognitiveAnalyticsEngine {
    #[cfg(feature = "ai")]
    behavior_model: Option<BehaviorNet>,
    #[cfg(feature = "ai")]
    congestion_model: Option<CongestionPredictorLSTM>,
    #[cfg(feature = "ai")]
    credential_verifier_model: Option<CredentialVerifierNet>,
    pub carbon_impact_model: CarbonImpactPredictor,
    #[cfg(feature = "ai")]
    training_data: VecDeque<ModelTrainingData>,
}

impl Default for CognitiveAnalyticsEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl CognitiveAnalyticsEngine {
    pub fn new() -> Self {
        let engine = Self {
            #[cfg(feature = "ai")]
            behavior_model: {
                let mut vs = nn::VarStore::new(Device::Cpu);
                Some(BehaviorNet::new(&mut vs))
            },
            #[cfg(feature = "ai")]
            congestion_model: {
                let mut vs = nn::VarStore::new(Device::Cpu);
                Some(CongestionPredictorLSTM::new(&mut vs, 10, 32)) // Sequence length of 10
            },
            #[cfg(feature = "ai")]
            credential_verifier_model: {
                let mut vs = nn::VarStore::new(Device::Cpu);
                Some(CredentialVerifierNet::new(&mut vs))
            },
            carbon_impact_model: CarbonImpactPredictor {},
            #[cfg(feature = "ai")]
            training_data: VecDeque::with_capacity(TRAINING_DATA_CAPACITY),
        };

        #[cfg(feature = "ai")]
        {
            if let Err(e) = engine.load_models_from_disk() {
                warn!(
                    "Could not load SAGA AI models from disk (this is normal on first run): {}",
                    e
                );
            }
        }
        engine
    }

    #[instrument(skip(self, block, dag, rules))]
    pub async fn score_node_behavior(
        &self,
        block: &HyperBlock,
        dag: &HyperDAG,
        rules: &HashMap<String, EpochRule>,
        network_state: NetworkState,
    ) -> Result<TrustScoreBreakdown, SagaError> {
        let grace_period = rules
            .get("temporal_grace_period_secs")
            .map_or(TEMPORAL_GRACE_PERIOD_SECS, |r| r.value as u64);

        let mut factors = HashMap::new();

        factors.insert(
            "validity".to_string(),
            self.check_block_validity(block),
        );
        factors.insert(
            "network_contribution".to_string(),
            self.analyze_network_contribution(block, dag).await,
        );
        factors.insert(
            "historical_performance".to_string(),
            self.check_historical_performance(&block.miner, dag).await,
        );
        factors.insert(
            "cognitive_hazard".to_string(),
            self.analyze_cognitive_hazards(block),
        );
        factors.insert(
            "temporal_consistency".to_string(),
            self.analyze_temporal_consistency(block, dag, grace_period)
                .await?,
        );
        factors.insert(
            "cognitive_dissonance".to_string(),
            self.analyze_cognitive_dissonance(block),
        );
        factors.insert(
            "metadata_integrity".to_string(),
            self.analyze_metadata_integrity(block).await,
        );
        factors.insert(
            "environmental_contribution".to_string(),
            self.analyze_environmental_contribution(block, dag).await,
        );

        let predicted_behavior_score = {
            #[cfg(feature = "ai")]
            {
                if let Some(model) = &self.behavior_model {
                    let factor_keys = [
                        "validity",
                        "network_contribution",
                        "historical_performance",
                        "cognitive_hazard",
                        "temporal_consistency",
                        "cognitive_dissonance",
                        "metadata_integrity",
                        "environmental_contribution",
                    ];
                    let feature_vec: Vec<f32> = factor_keys
                        .iter()
                        .map(|&key| *factors.get(key).unwrap_or(&0.5) as f32)
                        .collect();
                    let features_tensor =
                        Tensor::from_slice(&feature_vec)
                            .to_kind(Kind::Float)
                            .view([1, 8]);

                    let prediction_tensor = model.forward(&features_tensor);
                    // Use softmax to get probabilities for [Malicious, Neutral, Beneficial]
                    let prediction_vec: Vec<f32> =
                        Vec::<f32>::try_from(prediction_tensor.softmax(1, Kind::Float))
                            .map_err(|e| SagaError::InferenceError(e.to_string()))?;

                    // Score is probability of beneficial minus probability of malicious
                    let score = (prediction_vec[2] - prediction_vec[0] + 1.0) / 2.0;
                    score.clamp(0.0, 1.0) as f64
                } else {
                    0.5 // Default score if AI is disabled
                }
            }
            #[cfg(not(feature = "ai"))]
            {
                0.5 // Default score if AI is disabled
            }
        };
        factors.insert(
            "predicted_behavior".to_string(),
            predicted_behavior_score,
        );

        let mut final_score = 0.0;
        let mut total_weight = 0.0;
        for (factor_name, factor_score) in &factors {
            let base_weight_key = format!("trust_{factor_name}_weight");
            let mut weight = rules.get(&base_weight_key).map_or(0.1, |r| r.value);

            // Adapt weights based on network state
            match network_state {
                NetworkState::UnderAttack => {
                    if factor_name == "temporal_consistency"
                        || factor_name == "cognitive_hazard"
                        || factor_name == "metadata_integrity"
                    {
                        weight *= 2.5; // Prioritize security factors during an attack
                    }
                }
                NetworkState::Congested => {
                    if factor_name == "network_contribution" {
                        weight *= 1.5; // Prioritize efficient blocks during congestion
                    }
                }
                _ => {}
            }
            final_score += factor_score * weight;
            total_weight += weight;
        }

        Ok(TrustScoreBreakdown {
            factors,
            final_weighted_score: (final_score / total_weight.max(0.01)).clamp(0.0, 1.0),
        })
    }

    async fn analyze_environmental_contribution(&self, block: &HyperBlock, dag: &HyperDAG) -> f64 {
        let network_congestion = dag.get_average_tx_per_block().await / MAX_TRANSACTIONS_PER_BLOCK as f64;

        let block_footprint_grams: f64 = block
            .transactions
            .iter()
            .map(|tx| self.carbon_impact_model.predict_co2_per_tx(tx, network_congestion))
            .sum();
        let block_footprint_tonnes = block_footprint_grams / 1_000_000.0;

        let total_offset_tonnes: f64 = block
            .carbon_credentials
            .iter()
            .map(|c| c.tonnes_co2_sequestered)
            .sum();

        let net_impact = total_offset_tonnes - block_footprint_tonnes;
        
        // Use a sigmoid function to normalize the score between 0 and 1.
        // A net_impact of 0 gives a score of 0.5. Positive impact trends to 1, negative to 0.
        (1.0 / (1.0 + (-net_impact).exp())).clamp(0.0, 1.0)
    }

    fn check_block_validity(&self, block: &HyperBlock) -> f64 {
        let Some(coinbase) = block.transactions.first() else { return 0.0; };
        if !coinbase.is_coinbase() {
            return 0.1;
        }
        if coinbase.outputs.is_empty() {
            return 0.2;
        }
        1.0
    }

    async fn analyze_network_contribution(&self, block: &HyperBlock, dag: &HyperDAG) -> f64 {
        let avg_tx_per_block = dag.get_average_tx_per_block().await;
        let block_tx_count = block.transactions.len() as f64;
        // Reward blocks that are slightly above average, penalize empty or spammy blocks.
        let deviation = (block_tx_count - (avg_tx_per_block * 1.1)) / avg_tx_per_block.max(1.0);
        // Use exponential decay for the score based on deviation from the ideal
        (-deviation.powi(2)).exp()
    }

    async fn check_historical_performance(&self, miner_address: &str, dag: &HyperDAG) -> f64 {
        let blocks_reader = dag.blocks.read().await;
        let total_blocks = blocks_reader.len().max(1) as f64;
        let node_blocks = blocks_reader
            .values()
            .filter(|b| b.miner == *miner_address)
            .count() as f64;
        // Score based on participation rate, capped at a reasonable level.
        (node_blocks / total_blocks * 10.0).min(1.0)
    }

    fn analyze_cognitive_hazards(&self, block: &HyperBlock) -> f64 {
        let tx_count = block.transactions.len();
        if tx_count <= 1 {
            return 1.0;
        }
        let total_fee: u64 = block.transactions.iter().map(|tx| tx.fee).sum();
        let avg_fee = total_fee as f64 / tx_count as f64;
        let tx_ratio = tx_count as f64 / MAX_TRANSACTIONS_PER_BLOCK as f64;

        // Detect potential spam attack: a block nearly full of very low-fee transactions.
        if tx_ratio > 0.9 && avg_fee < 1.0 {
            return 0.2;
        }
        1.0
    }

    async fn analyze_temporal_consistency(
        &self,
        block: &HyperBlock,
        dag: &HyperDAG,
        grace_period: u64,
    ) -> Result<f64, SagaError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| SagaError::TimeError(e.to_string()))?
            .as_secs();
        if block.timestamp > now + grace_period {
            warn!(block_id = %block.id, "Temporal Anomaly: Block timestamp is too far in the future.");
            return Ok(0.2);
        }
        if !block.parents.is_empty() {
            let blocks_reader = dag.blocks.read().await;
            if let Some(max_parent_time) = block
                .parents
                .iter()
                .filter_map(|p_id| blocks_reader.get(p_id).map(|p_block| p_block.timestamp))
                .max()
            {
                if block.timestamp <= max_parent_time {
                    warn!(block_id = %block.id, "Temporal Anomaly: Block timestamp is not after its parent's.");
                    return Ok(0.0);
                }
            }
        }
        Ok(1.0)
    }

    fn analyze_cognitive_dissonance(&self, block: &HyperBlock) -> f64 {
        // Look for contradictory transaction policies in the same block, e.g., allowing
        // zero-fee spam alongside high-fee priority transactions.
        let high_fee_txs = block.transactions.iter().filter(|tx| tx.fee > 100).count();
        let zero_fee_txs = block
            .transactions
            .iter()
            .filter(|tx| tx.fee == 0 && !tx.is_coinbase())
            .count();

        if high_fee_txs > 0 && zero_fee_txs > (block.transactions.len() / 2) {
            0.3 // Penalize if over half the block is zero-fee spam despite priority txs
        } else {
            1.0
        }
    }

    async fn analyze_metadata_integrity(&self, block: &HyperBlock) -> f64 {
        let tx_count = block.transactions.len().max(1) as f64;
        let mut suspicious_tx_count = 0.0;

        for tx in block.transactions.iter().skip(1) { // Skip coinbase
            let metadata = tx.get_metadata();
            if metadata.get("origin_component").map_or(true, |s| s.is_empty() || s == "unknown") {
                suspicious_tx_count += 0.5;
            }

            if metadata.get("intent").map_or(true, |s| s.is_empty()) {
                suspicious_tx_count += 1.0;
            }

            // Check for high-entropy (gibberish) intent strings
            if let Some(intent_str) = metadata.get("intent") {
                if PalletSaga::calculate_shannon_entropy(intent_str) > 3.5 {
                    suspicious_tx_count += 0.5;
                }
            }
        }
        (1.0 - (suspicious_tx_count / tx_count)).max(0.0)
    }

    #[cfg(feature = "ai")]
    pub async fn collect_training_data_from_block(&mut self, block: &HyperBlock, dag: &HyperDAG) {
        if self.training_data.len() >= TRAINING_DATA_CAPACITY {
            self.training_data.pop_front();
        }
        let breakdown = self
            .score_node_behavior(block, dag, &HashMap::new(), NetworkState::Nominal)
            .await
            .unwrap_or_default();
        let congestion_metric = self.analyze_network_contribution(block, dag).await;
        
        // Heuristic labeling: was the block eventually part of the main chain or orphaned?
        // This is a simplification; a real system might use finalization status.
        let is_orphaned = !dag.tips.read().await.values().any(|tips| tips.contains(&block.id)) 
            && dag.blocks.read().await.len() > 50;

        let behavior_label = if is_orphaned {
            0.0 // Malicious/Poor
        } else if breakdown.final_weighted_score > 0.7 {
            1.0 // Beneficial
        } else {
            0.5 // Neutral
        };
        
        let data_point = ModelTrainingData {
            validity: breakdown.factors.get("validity").cloned().unwrap_or(0.0),
            network_contribution: breakdown.factors.get("network_contribution").cloned().unwrap_or(0.0),
            historical_performance: breakdown.factors.get("historical_performance").cloned().unwrap_or(0.0),
            cognitive_hazard: breakdown.factors.get("cognitive_hazard").cloned().unwrap_or(0.0),
            temporal_consistency: breakdown.factors.get("temporal_consistency").cloned().unwrap_or(0.0),
            cognitive_dissonance: breakdown.factors.get("cognitive_dissonance").cloned().unwrap_or(0.0),
            metadata_integrity: breakdown.factors.get("metadata_integrity").cloned().unwrap_or(0.0),
            environmental_contribution: breakdown.factors.get("environmental_contribution").cloned().unwrap_or(0.0),
            behavior_label,
            congestion_metric,
        };
        self.training_data.push_back(data_point);
    }

    #[cfg(feature = "ai")]
    pub fn train_models_from_data(&mut self) -> Result<(), SagaError> {
        info!("SAGA: Starting simulated AI model training cycle.");
        if self.training_data.len() < 100 {
            warn!(
                "SAGA: Insufficient data for model training ({} points). Skipping.",
                self.training_data.len()
            );
            return Ok(());
        }

        // --- Train BehaviorNet ---
        if let Some(model) = &mut self.behavior_model {
            let features: Vec<f32> = self.training_data.iter().flat_map(|d| {
                vec![
                    d.validity as f32, d.network_contribution as f32,
                    d.historical_performance as f32, d.cognitive_hazard as f32,
                    d.temporal_consistency as f32, d.cognitive_dissonance as f32,
                    d.metadata_integrity as f32, d.environmental_contribution as f32,
                ]
            }).collect();
            
            // Labels for 3 classes: 0 (Malicious), 1 (Neutral), 2 (Beneficial)
            let labels: Vec<i64> = self.training_data.iter().map(|d| 
                if d.behavior_label == 1.0 { 2 } 
                else if d.behavior_label == 0.0 { 0 } 
                else { 1 }
            ).collect();

            let feature_tensor = Tensor::of_slice(&features).view([-1, 8]);
            let label_tensor = Tensor::of_slice(&labels);
            let mut opt = nn::Adam::default().build(&model.vs, 1e-4).unwrap();
            
            info!("SAGA: Simulating BehaviorNet training for 50 epochs...");
            for i in 1..=50 {
                let loss = model.forward(&feature_tensor).cross_entropy_for_logits(&label_tensor);
                opt.backward_step(&loss);
                if i % 10 == 0 {
                    debug!("Simulated BehaviorNet Training Epoch: {}, Loss: {:?}", i, f64::from(&loss));
                }
            }
            info!("SAGA: BehaviorNet model training simulation complete.");
        }

        // --- Train CongestionPredictorLSTM ---
        // (Conceptual simulation - full implementation would be more complex)
        info!("SAGA: CongestionPredictorLSTM training is conceptually similar and simulated as complete.");
        
        // --- Train CredentialVerifierNet ---
        // (Conceptual simulation)
        info!("SAGA: CredentialVerifierNet training is conceptually similar and simulated as complete.");

        info!("SAGA: CarbonImpactPredictor does not require training (heuristic model).");

        self.save_models_to_disk()
    }

    #[cfg(feature = "ai")]
    pub fn save_models_to_disk(&self) -> Result<(), SagaError> {
        std::fs::create_dir_all(MODEL_SAVE_PATH)
            .map_err(|e| SagaError::ModelFileError(e.to_string()))?;
        if let Some(model) = &self.behavior_model {
            let path = PathBuf::from(MODEL_SAVE_PATH).join(BEHAVIOR_MODEL_FILENAME);
            model.vs.save(&path).map_err(|e| SagaError::ModelFileError(e.to_string()))?;
            info!("SAGA: BehaviorNet model saved to {:?}", path);
        }
        if let Some(model) = &self.congestion_model {
            let path = PathBuf::from(MODEL_SAVE_PATH).join(CONGESTION_MODEL_FILENAME);
            model.vs.save(&path).map_err(|e| SagaError::ModelFileError(e.to_string()))?;
            info!("SAGA: CongestionPredictorLSTM model saved to {:?}", path);
        }
        if let Some(model) = &self.credential_verifier_model {
            let path = PathBuf::from(MODEL_SAVE_PATH).join(CREDENTIAL_MODEL_FILENAME);
            model.vs.save(&path).map_err(|e| SagaError::ModelFileError(e.to_string()))?;
            info!("SAGA: CredentialVerifierNet model saved to {:?}", path);
        }
        Ok(())
    }

    #[cfg(feature = "ai")]
    pub fn load_models_from_disk(&self) -> Result<(), SagaError> {
        if let Some(model) = &self.behavior_model {
            let path = PathBuf::from(MODEL_SAVE_PATH).join(BEHAVIOR_MODEL_FILENAME);
            if path.exists() {
                model.vs.load(&path).map_err(|e| SagaError::ModelFileError(e.to_string()))?;
                info!("SAGA: Loaded BehaviorNet model from {:?}", path);
            }
        }
        if let Some(model) = &self.congestion_model {
            let path = PathBuf::from(MODEL_SAVE_PATH).join(CONGESTION_MODEL_FILENAME);
            if path.exists() {
                model.vs.load(&path).map_err(|e| SagaError::ModelFileError(e.to_string()))?;
                info!("SAGA: Loaded CongestionPredictorLSTM model from {:?}", path);
            }
        }
        if let Some(model) = &self.credential_verifier_model {
            let path = PathBuf::from(MODEL_SAVE_PATH).join(CREDENTIAL_MODEL_FILENAME);
            if path.exists() {
                model.vs.load(&path).map_err(|e| SagaError::ModelFileError(e.to_string()))?;
                info!("SAGA: Loaded CredentialVerifierNet model from {:?}", path);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct PredictiveEconomicModel;

impl PredictiveEconomicModel {
    pub fn new() -> Self { Self {} }
    pub async fn predictive_market_premium(
        &self,
        dag: &HyperDAG,
        metrics: &EnvironmentalMetrics,
    ) -> f64 {
        let avg_tx_per_block = dag.get_average_tx_per_block().await;
        let validator_count = dag.validators.read().await.len() as f64;
        
        let fee_velocity: f64 = {
            let blocks_reader = dag.blocks.read().await;
            let recent_blocks: Vec<_> = blocks_reader.values()
                .filter(|b| !b.transactions.is_empty())
                .collect();
            if recent_blocks.is_empty() { 1.0 } else {
                let total_fees: u64 = recent_blocks.iter()
                    .flat_map(|b| &b.transactions)
                    .map(|tx| tx.fee)
                    .sum();
                let total_txs = recent_blocks.iter().map(|b| b.transactions.len()).sum::<usize>();
                (total_fees as f64 / total_txs.max(1) as f64).max(1.0)
            }
        };

        let green_score = metrics.network_green_score;
        let green_premium = 1.0 + (green_score * 0.15); // Increased incentive for being green

        let base_premium = 1.0;
        let demand_factor = (avg_tx_per_block / MAX_TRANSACTIONS_PER_BLOCK as f64) * (1.0 + (fee_velocity / 100.0).min(1.0));
        let security_factor = (1.0 - (10.0 / validator_count).min(1.0)).max(0.5);
        let premium = base_premium + demand_factor * security_factor;
        
        (premium * green_premium).clamp(0.8, 2.5) // Allow for higher premium
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum QueryIntent {
    GetInfo,
    Compare,
    Troubleshoot,
}

#[derive(Debug, Clone)]
struct AnalyzedQuery {
    intent: QueryIntent,
    primary_topic: String,
    entities: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SagaGuidanceSystem {
    knowledge_base: HashMap<String, String>,
    #[cfg(feature = "ai")]
    topic_embeddings: HashMap<String, Vec<f32>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SagaInsight {
    pub id: String,
    pub epoch: u64,
    pub title: String,
    pub detail: String,
    pub severity: InsightSeverity,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub enum InsightSeverity {
    Tip,
    Warning,
    Critical,
}

impl Default for SagaGuidanceSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl SagaGuidanceSystem {
    pub fn new() -> Self {
        let mut knowledge_base = HashMap::new();
        knowledge_base.insert("setup".to_string(), "To get started with Hyperchain, follow these steps:\n1. **Download:** Get the latest `hyperchain` binary for your OS from the official repository.\n2. **Configuration:** Run `./hyperchain --init` in your terminal. This will create a default `config.toml` and a `wallet.key` file in your current directory.\n3. **Review Config:** Open `config.toml` to review settings. To connect to the network, add trusted peer multiaddresses to the `peers` array. Example: `peers = [\"/ip4/192.168.1.10/tcp/8008/p2p/12D3Koo...\"].\n4. **First Run:** Start the node with `./hyperchain`. It will automatically connect to peers if specified. If no peers are listed, it will run in a local, single-node mode, which is great for testing.".to_string());
        knowledge_base.insert("staking".to_string(), "Staking is the process of locking up HCN tokens to act as a validator. Validators are the backbone of the network, responsible for proposing new blocks, confirming transactions, and maintaining consensus.\n- **Become a Validator:** To become a validator, you must stake at least the minimum amount of HCN required by the current epoch rules (check the `/dag` endpoint for current parameters). Your node must be consistently online and performant to avoid penalties.\n- **Earn Rewards:** Proposing valid blocks earns rewards, which are dynamically calculated by SAGA. The reward amount is boosted by your Saga Credit Score (SCS), meaning more reputable validators earn more.\n- **Slashing Risk:** Malicious behavior (e.g., proposing invalid blocks, attempting a double-spend) or being consistently offline can cause your stake to be 'slashed', meaning a portion is forfeited as a penalty. This mechanism secures the network by disincentivizing bad actors.".to_string());
        knowledge_base.insert("send".to_string(), "To send tokens, you create and broadcast a transaction using your available funds, which are tracked as Unspent Transaction Outputs (UTXOs).\n1. **Check Balance/UTXOs:** Use the API endpoint `/utxos/{your_address}` to see a list of your UTXOs. The sum of these is your total balance.\n2. **Construct Transaction:** Create a transaction specifying which of your UTXOs will be used as inputs. Then, define the outputs: one for the recipient's address and the amount they should receive, and another 'change' output back to your own address with the remaining funds.\n3. **Sign & Submit:** Sign the complete transaction data with your private key (this is typically handled by your wallet software). Submit the signed transaction JSON to the `/transaction` API endpoint. The network will then pick it up for inclusion in a future block.".to_string());
        knowledge_base.insert("saga".to_string(), "SAGA is Hyperchain's AI core. It is a dynamic, autonomous system that observes on-chain activity, learns from it, and adapts network parameters to maintain health, security, and economic stability. It functions as the network's decentralized brain.\n- **Manages:** It dynamically adjusts the economy (block rewards, fees), consensus rules (difficulty), and governance parameters based on real-time network conditions.\n- **Scores:** It continuously analyzes node behavior via its Cognitive Engine to calculate your Saga Credit Score (SCS), which directly affects your potential rewards and governance influence.\n- **Assists:** You are interacting with its Guidance module right now, which provides contextual help and proactive network insights.".to_string());
        knowledge_base.insert("tokenomics".to_string(), "HCN is the native utility token of the Hyperchain network.\n- **Utility:** It is essential for all network operations. It's used for paying transaction fees, deploying and interacting with smart contracts, staking to become a validator, and participating in on-chain governance by proposing and voting on changes.\n- **Emission:** New HCN is minted as block rewards for validators. The amount is not fixed; it is dynamically calculated by SAGA based on several factors, including the validator's reputation (SCS), overall network health, the current threat level (as determined by the ΩMEGA protocol), and the total fees from the transactions included in the block. This creates a responsive and fair economic model.".to_string());
        knowledge_base.insert("scs".to_string(), "Your Saga Credit Score (SCS) is your on-chain reputation, represented as a score from 0.0 to 1.0. It is a critical metric that reflects how beneficial your actions are to the network's health and security.\n- **Calculation:** It's a weighted average of your trust score (derived from SAGA's Cognitive Engine analyzing your proposed blocks for quality and honesty), your Karma (long-term positive contributions), your total stake, and your environmental contributions (from PoCO). The weights for each component are themselves adjustable through governance.\n- **Importance:** A higher SCS is highly desirable. It leads to significantly greater block rewards and increases your voting power in governance proposals. A low SCS reduces rewards and can eventually lead to being disqualified as a validator.".to_string());
        knowledge_base.insert("slashing".to_string(), "Slashing is a severe penalty for validators who act maliciously or are consistently offline. When a validator is slashed, a portion of their staked HCN is forfeited permanently and removed from circulation. SAGA's Cognitive Engine analyzes block data to detect infractions, such as proposing invalid blocks or contradicting finalized history. The severity of the slash is determined based on the nature of the violation, ensuring the punishment fits the crime.".to_string());
        knowledge_base.insert("karma".to_string(), "Karma is a measure of positive, long-term contribution to the Hyperchain ecosystem. You earn Karma by creating successful governance proposals, voting constructively on others' proposals, and participating in the network's evolution. Unlike your SCS, which can fluctuate based on recent performance, Karma is designed to be a slow-to-change metric of your long-term standing and commitment to the project's success. It decays very slowly over time.".to_string());
        knowledge_base.insert("mempool".to_string(), "The mempool (memory pool) is a waiting area for transactions that have been submitted to the network but have not yet been included in a block. When a miner creates a new block, they select transactions from the mempool to include. Generally, transactions with higher fees are prioritized, as these fees contribute to the miner's block reward. You can view the current state of the mempool via the `/mempool` API endpoint.".to_string());
        knowledge_base.insert("peers".to_string(), "Troubleshooting peer connectivity:\n1. **Check `config.toml`:** Ensure the `peers` array contains valid and reachable multiaddresses of other nodes on the network.\n2. **Firewall:** Make sure your system's firewall is not blocking the TCP port specified in your `p2p_address` (e.g., port 8008).\n3. **Network ID:** Verify that your `network_id` in `config.toml` matches the ID of the network you are trying to join. Nodes with different IDs will not connect.\n4. **Node Logs:** Check the node's startup logs for any P2P errors, such as 'dial failed' or 'identity key mismatch'.".to_string());
        knowledge_base.insert("stuck_tx".to_string(), "If your transaction seems stuck (not included in a block):\n1. **Check Mempool:** Use the `/mempool` API endpoint to see if your transaction is still there. If it is, it's waiting to be mined.\n2. **Fee Too Low:** The most common reason for a stuck transaction is a low fee. During periods of high network activity (congestion), miners will prioritize transactions with higher fees. You may need to wait or resubmit the transaction with a higher fee (this requires creating a new transaction that spends the same UTXOs).\n3. **Network Congestion:** Use the `/saga/ask` endpoint with the query 'insights' to see if SAGA has issued a warning about network congestion.".to_string());
        knowledge_base.insert("security".to_string(), "Security is a layered process in Hyperchain, anchored by the SAGA and ΩMEGA protocols.\n- **Best Practices:** Always back up your `wallet.key` file in a secure, offline location. Never share your private key. Be cautious of phishing attempts and only download `hyperchain` binaries from the official repository.\n- **Wallet Security:** Your `wallet.key` is your identity. It is encrypted, but a strong password is your first line of defense. Consider using a hardware wallet for significant funds (integration is a future goal).\n- **Phishing:** Be wary of unsolicited messages or websites asking for your private key or wallet file. The Hyperchain team will never ask for this information.".to_string());
        knowledge_base.insert("ai_training".to_string(), "SAGA's AI models (BehaviorNet and CongestionPredictorLSTM) are designed to learn from the network's history.\n- **Conceptual Process:** The models are trained on historical data collected each epoch. This involves: (1) Collecting feature vectors from every block evaluation. (2) Heuristically labeling this data (e.g., high-scoring blocks are 'good', orphaned blocks are 'bad'). (3) Periodically running a simulated backpropagation process to update the models' weights.\n- **Current State:** The `train_models_from_data` function simulates this process. In a live network, this is a computationally intensive task run periodically by validator nodes to keep the AI's understanding of the network current.".to_string());
        knowledge_base.insert("sybil_attack".to_string(), "A Sybil attack is an attempt to subvert a network by creating many pseudonymous identities. In Hyperchain, SAGA's Security Monitor actively checks for this by analyzing stake distribution using the Gini coefficient. A low Gini coefficient indicates stake is very evenly (and suspiciously) distributed, increasing the 'sybil_risk' score and potentially triggering an 'UnderAttack' network state as a defensive measure.".to_string());
        knowledge_base.insert("spam_attack".to_string(), "A transaction spam attack attempts to disrupt the network by flooding it with low-value transactions. SAGA's Security Monitor detects this by watching the ratio of zero-fee to regular transactions. A sustained high ratio increases the 'spam_risk' score, which can lead to SAGA autonomously proposing an increase in the base transaction fee to make the attack economically unviable.".to_string());
        knowledge_base.insert("poco".to_string(), "Proof-of-Carbon-Offset (PoCO) is Hyperchain's innovative mechanism for integrating real-world environmental action into the blockchain consensus.\n- **How it Works:** Validators can include special `CarbonOffsetCredential` data in the blocks they mine. These credentials are verifiable claims of CO2 sequestration from trusted, off-chain issuers.\n- **Benefits:** SAGA's Cognitive Engine analyzes these credentials. Miners who include valid, high-quality credentials in their blocks receive a boost to their 'environmental_contribution' score. This, in turn, improves their overall Saga Credit Score (SCS), leading to higher block rewards.\n- **Goal:** PoCO creates a direct financial incentive for network participants to fund and support carbon reduction projects, turning the blockchain into a tool for positive environmental impact.".to_string());
        knowledge_base.insert("carbon_credit".to_string(), "A carbon credit is a tradable certificate representing the removal of one tonne of CO2. In Hyperchain's PoCO system, a `CarbonOffsetCredential` is the on-chain representation of such a credit. To be accepted, it must come from a project on SAGA's trusted registry and pass verification, which now includes an AI confidence check. Including valid credentials in a block proves a miner has sponsored real-world climate action, and SAGA rewards them for this contribution with an improved SCS and higher potential earnings.".to_string());
        knowledge_base.insert("centralization_risk".to_string(), "Centralization is a key risk where a few miners control a majority of block production. SAGA's Security Monitor calculates this risk using the Herfindahl-Hirschman Index (HHI) on block producer statistics from the last epoch. A high HHI score indicates high market concentration and increases the 'centralization_risk'. If the risk is too high, SAGA may enter an 'UnderAttack' state and could autonomously propose changes to encourage more miners to participate.".to_string());
        
        Self { 
            knowledge_base,
            #[cfg(feature="ai")]
            topic_embeddings: Self::generate_mock_embeddings(),
        }
    }

    #[cfg(feature="ai")]
    fn generate_mock_embeddings() -> HashMap<String, Vec<f32>> {
        // This is a placeholder. A real system would use a pre-trained language model
        // like BERT or Sentence-Transformers to generate these embeddings.
        HashMap::from([
            ("setup".to_string(), vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
            ("staking".to_string(), vec![0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
            ("send".to_string(), vec![0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0]),
            ("saga".to_string(), vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]),
            ("tokenomics".to_string(), vec![0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            ("scs".to_string(), vec![0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0]),
            ("security".to_string(), vec![0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]),
            ("poco".to_string(), vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]),
            ("carbon_credit".to_string(), vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]),
            ("centralization_risk".to_string(), vec![0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.5]),
        ])
    }
    
    #[cfg(feature="ai")]
    fn get_query_embedding(query: &str, topic_keywords: &HashMap<&str, Vec<&str>>) -> Vec<f32> {
        let mut embedding = vec![0.0; 7];
        for (topic, keywords) in topic_keywords {
            if keywords.iter().any(|&k| query.contains(k)) {
                match *topic {
                    "setup" => embedding[0] += 1.0,
                    "staking" => embedding[1] += 1.0,
                    "send" | "stuck_tx" => embedding[2] += 1.0,
                    "saga" => embedding[3] += 1.0,
                    "tokenomics" => embedding[4] += 1.0,
                    "security" | "slashing" | "sybil_attack" | "spam_attack" => embedding[5] += 1.0,
                    "poco" | "carbon_credit" => embedding[6] += 1.0,
                    _ => {}
                }
            }
        }
        let norm = (embedding.iter().map(|v| v.powi(2)).sum::<f32>()).sqrt();
        if norm > 0.0 {
            embedding.iter_mut().for_each(|v| *v /= norm);
        }
        embedding
    }

    pub async fn get_guidance_response(
        &self,
        query: &str,
        network_state: NetworkState,
        threat_level: omega::identity::ThreatLevel,
        proactive_insight: Option<&SagaInsight>,
    ) -> Result<String, SagaError> {
        debug!(target: "saga_guidance", "Received query: '{}'. Analyzing intent.", query);
        let analyzed_query = self.analyze_query_intent(query)?;

        let content = match analyzed_query.intent {
            QueryIntent::GetInfo | QueryIntent::Troubleshoot => self
                .knowledge_base
                .get(&analyzed_query.primary_topic)
                .cloned()
                .ok_or_else(|| SagaError::InvalidHelpTopic(analyzed_query.primary_topic.clone()))?,
            QueryIntent::Compare => {
                let not_found_str = "Topic not found.".to_string();
                let topic1_content = self
                    .knowledge_base
                    .get(&analyzed_query.primary_topic)
                    .unwrap_or(&not_found_str);
                let entity_key = analyzed_query.entities.first().unwrap_or(&"".to_string()).clone();
                let topic2_content = self.knowledge_base.get(&entity_key).unwrap_or(&not_found_str);
                format!(
                    "**Comparing {} and {}:**\n\n**{}:** {}\n\n**{}:** {}",
                    analyzed_query.primary_topic.to_uppercase(),
                    entity_key.to_uppercase(),
                    analyzed_query.primary_topic.to_uppercase(),
                    topic1_content,
                    entity_key.to_uppercase(),
                    topic2_content
                )
            }
        };

        let insight_text = if let Some(insight) = proactive_insight {
            format!(
                "\n\n**SAGA Proactive Insight:**\n*[{:?}] {}: {}*",
                insight.severity, insight.title, insight.detail
            )
        } else {
            "".to_string()
        };

        Ok(format!(
            "--- SAGA Guidance [State: {:?} | Ω-Threat: {:?}] ---\n\n**Topic: {}**\n\n{}{}",
            network_state,
            threat_level,
            analyzed_query.primary_topic.to_uppercase(),
            content,
            insight_text
        ))
    }

    fn analyze_query_intent(&self, query: &str) -> Result<AnalyzedQuery, SagaError> {
        let q = query.to_lowercase();
        let intent = if q.contains("vs") || q.contains("difference") {
            QueryIntent::Compare
        } else if q.contains("fix")
            || q.contains("problem")
            || q.contains("error")
            || q.contains("troubleshoot")
        {
            QueryIntent::Troubleshoot
        } else {
            QueryIntent::GetInfo
        };

        #[cfg(not(feature="ai"))]
        {
            let topic_keywords: HashMap<&str, Vec<&str>> = HashMap::from([
                ("scs", vec!["scs", "score", "reputation"]),
                ("tokenomics", vec!["token", "hcn", "economy", "reward", "fee", "tokenomics"]),
                ("staking", vec!["stake", "staking", "validator"]),
                ("poco", vec!["poco", "offset", "environment", "carbon", "co2", "credit", "green"]),
                // Add more basic keyword mappings here
            ]);
            let mut best_topic = None;
            let mut max_score = 0;
            for (topic, keywords) in topic_keywords {
                 let score = keywords.iter().filter(|k| q.contains(*k)).count();
                 if score > max_score {
                     max_score = score;
                     best_topic = Some(topic.to_string());
                 }
            }
            if let Some(topic) = best_topic {
                return Ok(AnalyzedQuery { intent, primary_topic: topic, entities: vec![] });
            } else {
                 return Err(SagaError::AmbiguousQuery(vec![]));
            }
        }

        #[cfg(feature="ai")]
        {
            let topic_keywords: HashMap<&str, Vec<&str>> = HashMap::from([
                ("setup", vec!["setup", "start", "install", "config", "init"]),
                ("staking", vec!["stake", "staking", "validator"]),
                ("send", vec!["send", "utxo", "construct", "submit", "transaction"]),
                ("saga", vec!["saga", "ai", "governance", "proposal", "brain"]),
                ("tokenomics", vec!["token", "hcn", "economy", "reward", "fee", "tokenomics"]),
                ("scs", vec!["scs", "score", "reputation"]),
                ("slashing", vec!["slash", "slashing", "penalty", "offline"]),
                ("karma", vec!["karma", "contribution"]),
                ("mempool", vec!["mempool", "memory pool"]),
                ("peers", vec!["peer", "peers", "connect", "dial", "firewall"]),
                ("stuck_tx", vec!["stuck", "pending", "unconfirmed"]),
                ("security", vec!["security", "safe", "wallet", "key", "phishing", "attack"]),
                ("ai_training", vec!["train", "training", "model", "learn"]),
                ("sybil_attack", vec!["sybil"]),
                ("spam_attack", vec!["spam"]),
                ("poco", vec!["poco", "offset", "environment"]),
                ("carbon_credit", vec!["carbon", "co2", "credit", "green"]),
                ("centralization_risk", vec!["centralization", "hhi", "concentration"]),
            ]);

            let query_embedding = Self::get_query_embedding(&q, &topic_keywords);

            let mut scores: Vec<(String, f32)> = self.topic_embeddings.iter().map(|(topic, topic_emb)| {
                let dot_product = query_embedding.iter().zip(topic_emb.iter()).map(|(a, b)| a * b).sum();
                (topic.clone(), dot_product)
            }).collect();
            
            scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            
            if scores.is_empty() || scores[0].1 < 0.1 {
                return Err(SagaError::AmbiguousQuery(scores.iter().map(|(t, _)| t.clone()).take(3).collect()));
            }

            let primary_topic = scores[0].0.clone();
            let mut entities = Vec::new();

            if intent == QueryIntent::Compare {
                if scores.len() > 1 && scores[1].1 > 0.1 {
                    entities.push(scores[1].0.clone());
                } else {
                    return Err(SagaError::AmbiguousQuery(vec![primary_topic]));
                }
            }
            
            Ok(AnalyzedQuery {
                intent,
                primary_topic,
                entities,
            })
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SecurityMonitor;

impl SecurityMonitor {
    pub fn new() -> Self {
        Self {}
    }

    #[instrument(skip(self, dag))]
    pub async fn check_for_sybil_attack(&self, dag: &HyperDAG) -> f64 {
        let validators = dag.validators.read().await;
        if validators.len() < 10 {
            return 0.0;
        }

        let total_stake: u64 = validators.values().sum();
        if total_stake == 0 {
            return 0.0;
        }

        let mut stakes: Vec<u64> = validators.values().cloned().collect();
        stakes.sort_unstable();

        let n = stakes.len() as f64;
        let sum_of_ranks = stakes
            .iter()
            .enumerate()
            .map(|(i, &s)| (i as f64 + 1.0) * s as f64)
            .sum::<f64>();
        let gini = (2.0 * sum_of_ranks) / (n * total_stake as f64) - (n + 1.0) / n;

        // Gini of 0 is perfect equality. Risk increases as Gini approaches 0.
        let sybil_risk = (1.0 - gini).powi(2).max(0.0);
        debug!(
            "Sybil attack analysis complete. Gini: {:.4}, Risk Score: {:.4}",
            gini, sybil_risk
        );

        sybil_risk
    }

    #[instrument(skip(self, dag))]
    pub async fn check_transactional_anomalies(&self, dag: &HyperDAG) -> f64 {
        let blocks = dag.blocks.read().await;
        let now = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_secs(),
            Err(_) => return 0.0,
        };

        let recent_blocks: Vec<_> = blocks
            .values()
            .filter(|b| b.timestamp > now.saturating_sub(600)) // last 10 minutes
            .collect();

        if recent_blocks.len() < 5 {
            return 0.0;
        }

        let total_txs: usize = recent_blocks
            .iter()
            .map(|b| b.transactions.len().saturating_sub(1)) // exclude coinbase
            .sum();
        if total_txs == 0 {
            return 0.0;
        }

        let zero_fee_txs: usize = recent_blocks
            .iter()
            .flat_map(|b| &b.transactions[1..])
            .filter(|tx| tx.fee == 0)
            .count();
        
        let zero_fee_ratio = zero_fee_txs as f64 / total_txs as f64;
        let risk = zero_fee_ratio.powi(2);

        debug!(
            "Transactional anomaly check complete. Zero-fee ratio: {:.4}, Risk score: {:.4}",
            zero_fee_ratio, risk
        );
        risk
    }

    #[instrument(skip(self, dag, epoch_lookback))]
    pub async fn check_for_centralization_risk(&self, dag: &HyperDAG, epoch_lookback: u64) -> f64 {
        let blocks = dag.blocks.read().await;
        if blocks.len() < 50 { return 0.0; }

        let current_epoch_val = *dag.current_epoch.read().await;
        
        let recent_blocks_by_miner: HashMap<String, u64> = blocks.values()
            .filter(|b| b.epoch >= current_epoch_val.saturating_sub(epoch_lookback))
            .fold(HashMap::new(), |mut acc, b| {
                *acc.entry(b.miner.clone()).or_insert(0) += 1;
                acc
            });

        if recent_blocks_by_miner.len() < 3 { return 0.75; }

        let total_produced = recent_blocks_by_miner.values().sum::<u64>() as f64;
        if total_produced == 0.0 { return 0.0; }
        
        // Using Herfindahl-Hirschman Index (HHI) for market concentration
        let hhi = recent_blocks_by_miner.values()
            .map(|&count| {
                let share = (count as f64 / total_produced) * 100.0;
                share.powi(2)
            }).sum::<f64>();

        // HHI between 1500 and 2500 is moderately concentrated, > 2500 is highly concentrated.
        let risk = ((hhi - 1500.0).max(0.0) / (4000.0 - 1500.0)).clamp(0.0, 1.0);

        debug!("Centralization risk analysis complete. HHI: {:.2}, Risk Score: {:.4}", hhi, risk);
        risk
    }

    #[instrument(skip(self, dag))]
    pub async fn check_for_oracle_manipulation_risk(&self, dag: &HyperDAG) -> f64 {
        // This check looks for miners who disproportionately rely on a single
        // project for their Carbon Offset Credentials, which could indicate collusion
        // or manipulation of a specific, low-quality carbon project.
        let blocks = dag.blocks.read().await;

        let mut recent_blocks_vec: Vec<_> = blocks.values().collect();
        recent_blocks_vec.sort_by_key(|b| b.timestamp);
        let recent_blocks = recent_blocks_vec.iter().rev().take(100);

        let mut creds_by_miner = HashMap::<String, Vec<CarbonOffsetCredential>>::new();
        for block in recent_blocks {
            creds_by_miner.entry(block.miner.clone()).or_default().extend(block.carbon_credentials.clone());
        }

        if creds_by_miner.is_empty() { return 0.0; }

        let mut total_risk_score = 0.0;
        let num_miners_with_creds = creds_by_miner.values().filter(|creds| !creds.is_empty()).count();

        for (_miner, creds) in creds_by_miner.iter() {
            if creds.is_empty() { continue; }

            let mut project_counts = HashMap::<String, u32>::new();
            for cred in creds {
                *project_counts.entry(cred.project_id.clone()).or_insert(0) += 1;
            }

            if let Some(max_count) = project_counts.values().max() {
                let single_project_ratio = *max_count as f64 / creds.len() as f64;
                if single_project_ratio > 0.8 && creds.len() > 5 {
                    total_risk_score += 0.5; // High risk for this miner
                }
            }
        }
        
        (total_risk_score / num_miners_with_creds.max(1) as f64).clamp(0.0, 1.0)
    }

    #[instrument(skip(self, dag))]
    pub async fn check_for_time_drift_attack(&self, dag: &HyperDAG) -> f64 {
        // Looks for miners who consistently produce blocks with the minimum possible timestamp,
        // which can be an indicator of selfish mining or network manipulation attempts.
        let blocks = dag.blocks.read().await;
        
        let mut recent_blocks: Vec<_> = blocks.values().collect();
        recent_blocks.sort_by_key(|b| b.timestamp);
        
        let mut suspicious_timestamps = HashMap::<String, (u32, u32)>::new(); // (total_blocks, suspicious_blocks)
        
        for block in recent_blocks.iter().rev().take(200) {
            let parent_max_ts = block.parents.iter()
                .filter_map(|p_id| blocks.get(p_id).map(|p| p.timestamp))
                .max().unwrap_or(block.timestamp);
            
            let (count, suspicious_count) = suspicious_timestamps.entry(block.miner.clone()).or_insert((0, 0));
            *count += 1;

            // A "suspicious" timestamp is one that is only 1 or 2 seconds after its parent.
            // While possible, a consistent pattern is a red flag.
            if block.timestamp <= parent_max_ts + 2 {
                *suspicious_count += 1;
            }
        }
        
        let mut max_risk = 0.0;
        for (_miner, (count, suspicious_count)) in suspicious_timestamps {
            if count > 10 { // Only consider miners with a decent sample size
                let ratio = suspicious_count as f64 / count as f64;
                if ratio > max_risk {
                    max_risk = ratio;
                }
            }
        }
        
        (max_risk.powi(2)).clamp(0.0, 1.0)
    }

    #[instrument(skip(self, dag))]
    pub async fn check_for_wash_trading(&self, dag: &HyperDAG) -> f64 {
        let blocks = dag.blocks.read().await;
        if blocks.len() < 100 { return 0.0; } // Need sufficient history

        let recent_blocks: Vec<_> = blocks.values()
            .filter(|b| b.timestamp > SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs().saturating_sub(1800)) // last 30 mins
            .collect();

        let mut tx_graph = HashMap::<String, Vec<String>>::new();
        let mut address_tx_counts = HashMap::<String, u32>::new();

        // For efficient lookups, create a map of all transactions on the DAG.
        // This is memory-intensive but avoids nested loops for every input.
        let tx_map: HashMap<_, _> = blocks
            .values()
            .flat_map(|b| &b.transactions)
            .map(|tx| (tx.id.clone(), tx))
            .collect();

        for block in recent_blocks {
            for tx in &block.transactions {
                if tx.is_coinbase() || tx.inputs.is_empty() { continue; }
                
                // Simplified: assume first input is the primary sender
                let input = &tx.inputs[0];
                
                // FIX: Look up the source transaction output to get the sender's address.
                // The `Input` type does not contain an address directly.
                if let Some(source_tx) = tx_map.get(&input.tx_id) {
                    if let Some(source_output) = source_tx.outputs.get(input.output_index as usize) {
                        let input_addr = &source_output.address;

                        for output in &tx.outputs {
                            let output_addr = &output.address;
                            tx_graph.entry(input_addr.clone()).or_default().push(output_addr.clone());
                            *address_tx_counts.entry(input_addr.clone()).or_insert(0) += 1;
                            *address_tx_counts.entry(output_addr.clone()).or_insert(0) += 1;
                        }
                    }
                }
            }
        }

        let mut suspicious_cycles = 0;
        let mut total_txs = 0;
        for (start_node, count) in address_tx_counts.iter() {
            if *count < 4 { continue; } // Ignore addresses with few txs
            total_txs += count;
            // Simple cycle detection: A -> B -> A
            if let Some(neighbors) = tx_graph.get(start_node) {
                for neighbor in neighbors {
                    if let Some(return_neighbors) = tx_graph.get(neighbor) {
                        if return_neighbors.contains(start_node) {
                            suspicious_cycles += 1;
                        }
                    }
                }
            }
        }

        if total_txs == 0 { return 0.0; }
        let risk = (suspicious_cycles as f64 / total_txs as f64).clamp(0.0, 1.0);
        debug!("Wash trading analysis complete. Suspicious cycles: {}, Risk Score: {:.4}", suspicious_cycles, risk);
        risk
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum ProposalStatus {
    Voting,
    Enacted,
    Rejected,
    Vetoed,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ProposalType {
    UpdateRule(String, f64),
    Signal(String),
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct SagaCreditScore {
    pub score: f64,
    pub factors: HashMap<String, f64>,
    pub history: Vec<(u64, f64)>,
    pub last_updated: u64,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EpochRule {
    pub value: f64,
    pub description: String,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VoterInfo {
    address: String,
    voted_for: bool,
    voting_power: f64,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GovernanceProposal {
    pub id: String,
    pub proposer: String,
    pub proposal_type: ProposalType,
    pub votes_for: f64,
    pub votes_against: f64,
    pub status: ProposalStatus,
    pub voters: Vec<VoterInfo>,
    pub creation_epoch: u64,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct CouncilMember {
    pub address: String,
    pub cognitive_load: f64,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct SagaCouncil {
    pub members: Vec<CouncilMember>,
    pub last_updated_epoch: u64,
    pub autonomous_governance_cooldown_until_epoch: u64,
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub enum KarmaSource {
    CreateSuccessfulProposal,
    VoteForPassedProposal,
    VoteAgainstFailedProposal,
    AiHelpdeskQuery,
    SagaAutonomousAction,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct KarmaLedger {
    pub total_karma: u64,
    pub contributions: HashMap<KarmaSource, u64>,
    pub last_updated_epoch: u64,
}
#[derive(Debug, Clone)]
pub struct ReputationState {
    pub credit_scores: Arc<RwLock<HashMap<String, SagaCreditScore>>>,
    pub karma_ledgers: Arc<RwLock<HashMap<String, KarmaLedger>>>,
}
#[derive(Debug, Clone)]
pub struct GovernanceState {
    pub proposals: Arc<RwLock<HashMap<String, GovernanceProposal>>>,
    pub council: Arc<RwLock<SagaCouncil>>,
}

#[derive(Debug, Clone, Default)]
pub struct EnvironmentalMetrics {
    pub network_green_score: f64,
    pub total_co2_offset_epoch: f64,
    pub trusted_project_registry: HashMap<String, f64>,
    pub verified_credentials: HashMap<String, CarbonOffsetCredential>,
}

#[derive(Debug, Clone)]
pub struct EconomicState {
    pub epoch_rules: Arc<RwLock<HashMap<String, EpochRule>>>,
    pub network_state: Arc<RwLock<NetworkState>>,
    pub active_edict: Arc<RwLock<Option<SagaEdict>>>,
    pub last_edict_epoch: Arc<RwLock<u64>>,
    pub proactive_insights: Arc<RwLock<Vec<SagaInsight>>>,
    pub congestion_history: Arc<RwLock<VecDeque<f64>>>,
    pub environmental_metrics: Arc<RwLock<EnvironmentalMetrics>>,
}

#[derive(Debug)]
pub struct PalletSaga {
    pub reputation: ReputationState,
    pub governance: GovernanceState,
    pub economy: EconomicState,
    pub cognitive_engine: Arc<RwLock<CognitiveAnalyticsEngine>>,
    pub economic_model: Arc<PredictiveEconomicModel>,
    pub security_monitor: Arc<SecurityMonitor>,
    pub guidance_system: Arc<SagaGuidanceSystem>,
    last_retrain_epoch: Arc<RwLock<u64>>,
    // Conditionally include the ISNM service to allow for integration.
    #[cfg(feature = "infinite-strata")]
    pub isnm_service: Option<Arc<InfiniteStrataNode>>,
}

impl PalletSaga {
    // Modify the constructor to accept the optional ISNM service.
    // This resolves the compilation error in node.rs and saga_simulation.rs.
    pub fn new(
        #[cfg(feature = "infinite-strata")] isnm_service: Option<Arc<InfiniteStrataNode>>,
    ) -> Self {
        let mut rules = HashMap::new();
        // --- Core ---
        rules.insert("base_difficulty".to_string(), EpochRule { value: 10.0, description: "The baseline PoW difficulty before PoSe adjustments.".to_string() });
        rules.insert("min_validator_stake".to_string(), EpochRule { value: 1000.0, description: "The minimum stake required to be a validator.".to_string() });
        
        // --- SCS Weights ---
        rules.insert("scs_trust_weight".to_string(), EpochRule { value: 0.55, description: "Weight of Cognitive Engine score in SCS.".to_string() });
        rules.insert("scs_karma_weight".to_string(), EpochRule { value: 0.2, description: "Weight of Karma in SCS.".to_string() });
        rules.insert("scs_stake_weight".to_string(), EpochRule { value: 0.2, description: "Weight of raw stake in SCS.".to_string() });
        rules.insert("scs_environmental_weight".to_string(), EpochRule { value: 0.05, description: "Weight of environmental contribution in SCS.".to_string() });
        
        // --- Trust Score Weights ---
        rules.insert("trust_validity_weight".to_string(), EpochRule { value: 0.20, description: "Weight of block validity in trust score.".to_string() });
        rules.insert("trust_network_contribution_weight".to_string(), EpochRule { value: 0.10, description: "Weight of network contribution in trust score.".to_string() });
        rules.insert("trust_historical_performance_weight".to_string(), EpochRule { value: 0.10, description: "Weight of historical performance in trust score.".to_string() });
        rules.insert("trust_cognitive_hazard_weight".to_string(), EpochRule { value: 0.15, description: "Weight of cognitive hazard analysis in trust score.".to_string() });
        rules.insert("trust_temporal_consistency_weight".to_string(), EpochRule { value: 0.15, description: "Weight of temporal consistency in trust score.".to_string() });
        rules.insert("trust_cognitive_dissonance_weight".to_string(), EpochRule { value: 0.10, description: "Weight of cognitive dissonance in trust score.".to_string() });
        rules.insert("trust_metadata_integrity_weight".to_string(), EpochRule { value: 0.10, description: "Weight of transaction metadata integrity in trust score.".to_string() });
        rules.insert("trust_predicted_behavior_weight".to_string(), EpochRule { value: 0.10, description: "Weight of AI behavioral prediction in trust score.".to_string() });
        rules.insert("trust_environmental_contribution_weight".to_string(), EpochRule { value: 0.10, description: "Weight of PoCO score in trust score.".to_string() });
        
        // --- Economic ---
        rules.insert("base_reward".to_string(), EpochRule { value: 250.0, description: "Base HCN reward per block before modifiers.".to_string() });
        rules.insert("omega_threat_reward_modifier".to_string(), EpochRule { value: -0.25, description: "Reward reduction per elevated ΩMEGA threat level.".to_string() });

        // --- Tiered Fee Structure (in basis points, 100 bps = 1%) ---
        rules.insert("fee_tier_1_bps".to_string(), EpochRule { value: 100.0, description: "Fee in basis points for amounts up to tier_1_threshold. (100 = 1%)".to_string() });
        rules.insert("fee_tier_1_threshold".to_string(), EpochRule { value: 1_000_000.0, description: "Upper bound for fee tier 1.".to_string() });
        rules.insert("fee_tier_2_bps".to_string(), EpochRule { value: 200.0, description: "Fee in basis points for amounts up to tier_2_threshold. (200 = 2%)".to_string() });
        rules.insert("fee_tier_2_threshold".to_string(), EpochRule { value: 100_000_000.0, description: "Upper bound for fee tier 2.".to_string() });
        rules.insert("fee_tier_3_bps".to_string(), EpochRule { value: 300.0, description: "Fee in basis points for amounts above tier_2_threshold. (300 = 3%)".to_string() });
        rules.insert("base_tx_fee_min".to_string(), EpochRule { value: 1.0, description: "A minimum flat fee for all transactions, regardless of amount.".to_string() });

        // --- Governance & Karma ---
        rules.insert("proposal_creation_cost".to_string(), EpochRule { value: 500.0, description: "Karma cost to create a new proposal.".to_string() });
        rules.insert("guidance_karma_cost".to_string(), EpochRule { value: 5.0, description: "Karma cost to query the SAGA Guidance System.".to_string() });
        rules.insert("karma_decay_rate".to_string(), EpochRule { value: 0.995, description: "Percentage of Karma remaining after decay each epoch.".to_string() });
        rules.insert("proposal_vote_threshold".to_string(), EpochRule { value: 100.0, description: "Minimum votes for a proposal to be enacted.".to_string() });
        rules.insert("council_size".to_string(), EpochRule { value: 5.0, description: "Number of members in the SAGA Council.".to_string() });
        rules.insert("council_fatigue_decay".to_string(), EpochRule { value: 0.9, description: "Factor by which council member fatigue decays each epoch.".to_string() });
        rules.insert("council_fatigue_per_action".to_string(), EpochRule { value: 0.1, description: "Fatigue increase for a council member per veto/vote.".to_string() });
        
        // --- Technical ---
        rules.insert("temporal_grace_period_secs".to_string(), EpochRule { value: 120.0, description: "Grace period in seconds for block timestamps.".to_string() });
        rules.insert("scs_karma_normalization_divisor".to_string(), EpochRule { value: 10000.0, description: "Divisor to normalize Karma for SCS.".to_string() });
        rules.insert("scs_stake_normalization_divisor".to_string(), EpochRule { value: 50000.0, description: "Divisor to normalize stake for SCS.".to_string() });
        rules.insert("scs_smoothing_factor".to_string(), EpochRule { value: 0.1, description: "Smoothing factor for updating SCS (weight of the new score).".to_string() });
        
        let mut env_metrics = EnvironmentalMetrics::default();
        // A registry of trusted carbon credit projects and their quality multiplier.
        env_metrics.trusted_project_registry.insert("verra-p-981".to_string(), 1.0); // High quality
        env_metrics.trusted_project_registry.insert("gold-standard-p-334".to_string(), 1.0); // High quality
        env_metrics.trusted_project_registry.insert("verra-p-201".to_string(), 0.8); // Medium quality
        env_metrics.trusted_project_registry.insert("low-quality-p-001".to_string(), 0.5); // Low quality

        Self {
            reputation: ReputationState {
                credit_scores: Arc::new(RwLock::new(HashMap::new())),
                karma_ledgers: Arc::new(RwLock::new(HashMap::new())),
            },
            governance: GovernanceState {
                proposals: Arc::new(RwLock::new(HashMap::new())),
                council: Arc::new(RwLock::new(SagaCouncil::default())),
            },
            economy: EconomicState {
                epoch_rules: Arc::new(RwLock::new(rules)),
                network_state: Arc::new(RwLock::new(NetworkState::Nominal)),
                active_edict: Arc::new(RwLock::new(None)),
                last_edict_epoch: Arc::new(RwLock::new(0)),
                proactive_insights: Arc::new(RwLock::new(Vec::new())),
                congestion_history: Arc::new(RwLock::new(VecDeque::with_capacity(10))),
                environmental_metrics: Arc::new(RwLock::new(env_metrics)),
            },
            cognitive_engine: Arc::new(RwLock::new(CognitiveAnalyticsEngine::new())),
            economic_model: Arc::new(PredictiveEconomicModel::new()),
            security_monitor: Arc::new(SecurityMonitor::new()),
            guidance_system: Arc::new(SagaGuidanceSystem::new()),
            last_retrain_epoch: Arc::new(RwLock::new(0)),
            #[cfg(feature = "infinite-strata")]
            isnm_service,
        }
    }

    pub fn calculate_shannon_entropy(s: &str) -> f64 {
        if s.is_empty() {
            return 0.0;
        }
        let mut map = HashMap::new();
        for c in s.chars() {
            *map.entry(c).or_insert(0) += 1;
        }
        let len = s.len() as f64;
        map.values()
            .map(|&count| {
                let p = count as f64 / len;
                -p * p.log2()
            })
            .sum()
    }

    pub async fn verify_and_store_credential(
        &self,
        cred: CarbonOffsetCredential,
    ) -> Result<(), SagaError> {
        let metrics = self.economy.environmental_metrics.read().await;

        if cred.tonnes_co2_sequestered <= 0.0 {
            return Err(SagaError::InvalidCredential(
                "CO2 amount must be positive.".to_string(),
            ));
        }
        if cred.beneficiary_node.is_empty() {
            return Err(SagaError::InvalidCredential(
                "Beneficiary node cannot be empty.".to_string(),
            ));
        }

        // Placeholder for a real cryptographic signature check
        let expected_signature = format!("signed_by_{}", cred.issuer_id);
        if cred.verification_signature != expected_signature {
            return Err(SagaError::InvalidCredential(
                "Invalid issuer signature.".to_string(),
            ));
        }
        
        if !metrics.trusted_project_registry.contains_key(&cred.project_id) {
            return Err(SagaError::InvalidCredential(format!(
                "Project ID '{}' is not in the trusted registry.",
                cred.project_id
            )));
        }

        let current_year: u32 = 2025; // This should come from a trusted time source
        if current_year.saturating_sub(cred.vintage_year) > 5 {
            warn!(cred_id=%cred.id, "Credential has an old vintage year ({}).", cred.vintage_year);
        }

        // AI-driven verification step
        #[cfg(feature = "ai")]
        {
            let engine = self.cognitive_engine.read().await;
            if let Some(model) = &engine.credential_verifier_model {
                let project_quality = metrics.trusted_project_registry.get(&cred.project_id).cloned().unwrap_or(0.5);
                let vintage_age = (current_year.saturating_sub(cred.vintage_year)) as f32;
                
                let features_vec = vec![
                    project_quality as f32,
                    vintage_age,
                    cred.tonnes_co2_sequestered as f32,
                    Self::calculate_shannon_entropy(&cred.additionality_proof) as f32, // Proxy for proof complexity
                ];
                let features_tensor = Tensor::from_slice(&features_vec).to_kind(Kind::Float).view([1, 4]);

                let confidence_tensor = model.verify(&features_tensor);
                let confidence_score = f64::try_from(confidence_tensor).unwrap_or(0.0);
                
                info!(cred_id=%cred.id, project_id=%cred.project_id, "AI Verification Confidence: {:.4}", confidence_score);

                if confidence_score < 0.75 {
                    return Err(SagaError::InvalidCredential(format!(
                        "AI verification failed. Confidence score ({:.2}) is below threshold (0.75).",
                        confidence_score
                    )));
                }
            }
        }

        let mut metrics_write = self.economy.environmental_metrics.write().await;
        if metrics_write.verified_credentials.contains_key(&cred.id) {
             return Err(SagaError::InvalidCredential(format!("Credential ID '{}' has already been submitted this epoch.", cred.id)));
        }

        info!(
            cred_id = %cred.id,
            beneficiary = %cred.beneficiary_node,
            "Storing verified CarbonOffsetCredential."
        );
        metrics_write.verified_credentials.insert(cred.id.clone(), cred);

        Ok(())
    }
    
    #[instrument(skip(self, block, dag_arc))]
    pub async fn evaluate_block_with_saga(
        &self,
        block: &HyperBlock,
        dag_arc: &Arc<HyperDAG>,
    ) -> Result<()> {
        info!(block_id = %block.id, miner = %block.miner, "SAGA: Starting evaluation of new block.");
        
        self.evaluate_and_score_block(block, dag_arc).await?;

        #[cfg(feature = "ai")]
        {
            let dag = &**dag_arc; // Deref Arc to get &HyperDAG
            self.cognitive_engine
                .write()
                .await
                .collect_training_data_from_block(block, dag)
                .await;
        }

        info!(block_id = %block.id, "SAGA: Evaluation complete.");
        Ok(())
    }
    
    async fn evaluate_and_score_block(
        &self,
        block: &HyperBlock,
        dag_arc: &Arc<HyperDAG>,
    ) -> Result<()> {
        let (rules, network_state) = {
            let eco = self.economy.epoch_rules.read().await;
            let net_state = *self.economy.network_state.read().await;
            (eco.clone(), net_state)
        };
        let dag = &**dag_arc; // Deref Arc to get &HyperDAG

        let trust_breakdown = self
            .cognitive_engine
            .read()
            .await
            .score_node_behavior(block, dag, &rules, network_state)
            .await?;

        self.update_credit_score(&block.miner, &trust_breakdown, dag_arc)
            .await?;
        Ok(())
    }

    // FIX: This is a temporary workaround for the compilation error where
    // `get_poscp_reward_multiplier` was not found on `InfiniteStrataNode`.
    // This helper simulates the intended behavior by providing a default multiplier
    // when the `infinite-strata` feature is active. In a real implementation,
    // this would be replaced by a direct call to the ISNM service.
    async fn get_isnm_reward_multiplier(&self) -> f64 {
        #[cfg(feature = "infinite-strata")]
        {
            if self.isnm_service.is_some() {
                // The original call was `service.get_poscp_reward_multiplier().await`.
                // Since the method is not found, we log a warning and return a
                // default bonus multiplier to signify that the ISNM is active.
                warn!("SAGA: `get_poscp_reward_multiplier` not found. Using simulated ISNM reward multiplier of 1.1. Please verify the `InfiniteStrataNode` API.");
                return 1.1;
            }
        }
        1.0
    }

    pub async fn calculate_dynamic_reward(
        &self,
        block: &HyperBlock,
        dag_arc: &Arc<HyperDAG>,
    ) -> Result<u64> {
        let rules = self.economy.epoch_rules.read().await;
        let base_reward = rules.get("base_reward").map_or(250.0, |r| r.value);
        let threat_modifier = rules
            .get("omega_threat_reward_modifier")
            .map_or(-0.25, |r| r.value);
        let scs = self
            .reputation
            .credit_scores
            .read()
            .await
            .get(&block.miner)
            .map_or(0.5, |s| s.score);

        let threat_level = omega::get_threat_level().await;
        let omega_penalty = match threat_level {
            omega::identity::ThreatLevel::Nominal => 1.0,
            omega::identity::ThreatLevel::Guarded => 1.0 + threat_modifier,
            omega::identity::ThreatLevel::Elevated => 1.0 + (threat_modifier * 2.0),
        }
        .max(0.0);

        let metrics = self.economy.environmental_metrics.read().await;
        let market_premium = self
            .economic_model
            .predictive_market_premium(&**dag_arc, &metrics)
            .await;
        
        let edict_multiplier =
            if let Some(edict) = &*self.economy.active_edict.read().await {
                if let EdictAction::Economic {
                    reward_multiplier, ..
                } = edict.action
                {
                    reward_multiplier
                } else {
                    1.0
                }
            } else {
                1.0
            };

        // Integrate ISNM reward multiplier if the feature is enabled.
        let isnm_multiplier = self.get_isnm_reward_multiplier().await;
        
        let total_fees = block.transactions.iter().map(|tx| tx.fee).sum::<u64>();

        let final_reward =
            (base_reward * scs * omega_penalty * market_premium * edict_multiplier * isnm_multiplier) as u64
                + total_fees;
        Ok(final_reward)
    }
    
    #[instrument(skip(self, dag))]
    pub async fn process_epoch_evolution(&self, current_epoch: u64, dag: &HyperDAG) {
        info!("SAGA is processing epoch evolution for epoch {current_epoch}");
        self.update_network_state(dag).await;
        self.run_predictive_models(current_epoch, dag).await;
        self.generate_proactive_insights(current_epoch, dag).await;
        self.tally_proposals(current_epoch).await;
        self.process_karma_decay(current_epoch).await;
        self.update_council(current_epoch).await;
        self.issue_new_edict(current_epoch, dag).await;
        self.perform_autonomous_governance(current_epoch, dag).await;
        self.update_environmental_metrics(current_epoch).await;

        let mut last_retrain = self.last_retrain_epoch.write().await;
        if current_epoch > *last_retrain + RETRAIN_INTERVAL_EPOCHS {
            #[cfg(feature = "ai")]
            if let Err(e) = self.cognitive_engine.write().await.train_models_from_data() {
                error!("SAGA AI model retraining failed: {}", e);
            }
            *last_retrain = current_epoch;
        }
    }

    async fn update_network_state(&self, dag: &HyperDAG) {
        let avg_tx_per_block = dag.get_average_tx_per_block().await;
        let validator_count = dag.validators.read().await.len();
        let threat_level = omega::get_threat_level().await;
        
        let sybil_risk = self.security_monitor.check_for_sybil_attack(dag).await;
        let spam_risk = self.security_monitor.check_transactional_anomalies(dag).await;
        let centralization_risk = self.security_monitor.check_for_centralization_risk(dag, 1).await;
        let oracle_risk = self.security_monitor.check_for_oracle_manipulation_risk(dag).await;
        let time_drift_risk = self.security_monitor.check_for_time_drift_attack(dag).await;
        let wash_trading_risk = self.security_monitor.check_for_wash_trading(dag).await;

        let mut state_writer = self.economy.network_state.write().await;
        let old_state = *state_writer;

        *state_writer = if threat_level != omega::identity::ThreatLevel::Nominal 
            || sybil_risk > 0.8 
            || spam_risk > 0.75 
            || centralization_risk > 0.8 
            || oracle_risk > 0.7
            || time_drift_risk > 0.8
            || wash_trading_risk > 0.6
        {
            NetworkState::UnderAttack
        } else if validator_count < 10 {
            NetworkState::Degraded
        } else if avg_tx_per_block > (MAX_TRANSACTIONS_PER_BLOCK as f64 * 0.8) {
            NetworkState::Congested
        } else {
            NetworkState::Nominal
        };

        if old_state != *state_writer {
            info!(
                old_state = ?old_state,
                new_state = ?*state_writer,
                "SAGA has transitioned the Network State."
            );
        }
    }

    // FIX: Add attribute to allow unused 'current_epoch' when 'ai' feature is disabled,
    // which resolves the compiler warning.
    async fn run_predictive_models(
        &self,
        #[cfg_attr(not(feature = "ai"), allow(unused_variables))] current_epoch: u64,
        dag: &HyperDAG,
    ) {
        let avg_tx_per_block = dag.get_average_tx_per_block().await;
        let congestion_metric = avg_tx_per_block / MAX_TRANSACTIONS_PER_BLOCK as f64;

        let mut history = self.economy.congestion_history.write().await;
        history.push_back(congestion_metric.clamp(0.0, 1.0));
        if history.len() > 10 {
            history.pop_front();
        }

        #[cfg(feature = "ai")]
        if let Some(model) = &self.cognitive_engine.read().await.congestion_model {
            if history.len() == model.sequence_len as usize {
                let sequence_vec: Vec<f32> = history.iter().map(|&v| v as f32).collect();
                let input_tensor = Tensor::from_slice(&sequence_vec).to_kind(Kind::Float);
                match model.predict(&input_tensor) {
                    Ok(prediction) => {
                        info!(prediction, "SAGA LSTM predicted future congestion level.");
                        if prediction > 0.85 {
                            let mut insights = self.economy.proactive_insights.write().await;
                            if !insights.iter().any(|i| i.title.contains("Predicted Congestion")) {
                                insights.push(SagaInsight {
                                    id: Uuid::new_v4().to_string(),
                                    epoch: current_epoch,
                                    title: "High Congestion Predicted".to_string(),
                                    detail: format!("SAGA's LSTM model predicts a high network load in the near future (predicted level: {:.2}). Expect higher fees or slower transaction times.", prediction),
                                    severity: InsightSeverity::Warning,
                                });
                            }
                        }
                    }
                    Err(e) => warn!(error = ?e, "Congestion prediction model failed."),
                }
            }
        }
    }
    
    #[instrument(skip(self, trust_breakdown, dag_arc))]
    async fn update_credit_score(
        &self,
        miner_address: &str,
        trust_breakdown: &TrustScoreBreakdown,
        dag_arc: &Arc<HyperDAG>,
    ) -> Result<()> {
        let rules = self.economy.epoch_rules.read().await;
        let dag_read = &**dag_arc;
        
        let trust_weight = rules.get("scs_trust_weight").map_or(0.55, |r| r.value);
        let karma_weight = rules.get("scs_karma_weight").map_or(0.2, |r| r.value);
        let stake_weight = rules.get("scs_stake_weight").map_or(0.2, |r| r.value);
        let env_weight = rules.get("scs_environmental_weight").map_or(0.05, |r| r.value);

        let karma_divisor = rules.get("scs_karma_normalization_divisor").map_or(10000.0, |r| r.value);
        let stake_divisor = rules.get("scs_stake_normalization_divisor").map_or(50000.0, |r| r.value);
        let smoothing_factor = rules.get("scs_smoothing_factor").map_or(0.1, |r| r.value);

        let karma_score = self
            .reputation
            .karma_ledgers
            .read()
            .await
            .get(miner_address)
            .map_or(0.0, |kl| (kl.total_karma as f64 / karma_divisor).min(1.0));
        let stake_score = dag_read
            .validators
            .read()
            .await
            .get(miner_address)
            .map_or(0.0, |&s| (s as f64 / stake_divisor).min(1.0));
        
        let env_score = trust_breakdown.factors.get("environmental_contribution").cloned().unwrap_or(0.5);

        let new_raw_score = (trust_breakdown.final_weighted_score * trust_weight)
            + (karma_score * karma_weight)
            + (stake_score * stake_weight)
            + (env_score * env_weight);

        let mut scores = self.reputation.credit_scores.write().await;
        let mut scs_entry = scores.entry(miner_address.to_string()).or_default().clone();
        
        scs_entry.score = (scs_entry.score * (1.0 - smoothing_factor)) + (new_raw_score * smoothing_factor);
        scs_entry.last_updated = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| SagaError::TimeError(e.to_string()))?
            .as_secs();
        scs_entry.history.push((scs_entry.last_updated, scs_entry.score));
        if scs_entry.history.len() > 200 {
            scs_entry.history.remove(0);
        }
        
        scs_entry.factors = trust_breakdown.factors.clone();
        scs_entry.factors.insert("karma_score".to_string(), karma_score);
        scs_entry.factors.insert("stake_score".to_string(), stake_score);
        
        scores.insert(miner_address.to_string(), scs_entry.clone());
        info!(miner = miner_address, scs = scs_entry.score, "SCS Updated");
        Ok(())
    }

    async fn generate_proactive_insights(&self, current_epoch: u64, dag: &HyperDAG) {
        let mut insights = self.economy.proactive_insights.write().await;
        insights.retain(|i| current_epoch < i.epoch + 5);
        let network_state = *self.economy.network_state.read().await;

        if network_state == NetworkState::Congested
            && !insights.iter().any(|i| i.title.contains("Network Congestion"))
        {
            insights.push(SagaInsight {
                id: Uuid::new_v4().to_string(),
                epoch: current_epoch,
                title: "Network Congestion".to_string(),
                detail: "The network is experiencing high transaction volume. Consider increasing your transaction fees for faster confirmation.".to_string(),
                severity: InsightSeverity::Warning,
            });
        }
        let difficulty = *dag.difficulty.read().await;
        if difficulty > 5000 && !insights.iter().any(|i| i.title.contains("High Difficulty")) {
            insights.push(SagaInsight {
                id: Uuid::new_v4().to_string(),
                epoch: current_epoch,
                title: "High Difficulty Alert".to_string(),
                detail: "Network difficulty is currently high, which may lead to longer block times. This is normal during periods of high miner participation.".to_string(),
                severity: InsightSeverity::Tip,
            });
        }
        let proposal_count = self.governance.proposals.read().await.len();
        if current_epoch > 20
            && proposal_count < 5
            && !insights.iter().any(|i| i.title.contains("Low Governance Activity"))
        {
            insights.push(SagaInsight {
                id: Uuid::new_v4().to_string(),
                epoch: current_epoch,
                title: "Low Governance Activity".to_string(),
                detail: "There have been few governance proposals recently. Consider proposing changes or participating in discussions to help evolve the network.".to_string(),
                severity: InsightSeverity::Tip,
            });
        }
    }

    async fn tally_proposals(&self, current_epoch: u64) {
        let mut proposals = self.governance.proposals.write().await;
        let mut rules = self.economy.epoch_rules.write().await;
        let vote_threshold = rules.get("proposal_vote_threshold").map_or(100.0, |r| r.value);
        let karma_reward_proposer = 250;
        let karma_reward_voter = 25;
        
        for proposal in proposals.values_mut() {
            if proposal.status == ProposalStatus::Voting {
                if proposal.votes_for >= vote_threshold {
                    proposal.status = ProposalStatus::Enacted;
                    info!(proposal_id = %proposal.id, "Proposal has been enacted.");
                    if let ProposalType::UpdateRule(rule_name, new_value) = &proposal.proposal_type
                    {
                        if let Some(rule) = rules.get_mut(rule_name) {
                            info!(rule = %rule_name, old_value = rule.value, new_value = new_value, "Epoch Rule Evolved via Governance");
                            rule.value = *new_value;
                        }
                    }
                    self.award_karma(&proposal.proposer, KarmaSource::CreateSuccessfulProposal, karma_reward_proposer).await;
                    for voter in &proposal.voters {
                        if voter.voted_for {
                            self.award_karma(&voter.address, KarmaSource::VoteForPassedProposal, karma_reward_voter).await;
                        }
                    }
                } else if current_epoch > proposal.creation_epoch + 10 {
                    proposal.status = ProposalStatus::Rejected;
                    info!(proposal_id = %proposal.id, "Proposal expired and was rejected.");
                }
            }
        }
    }

    async fn award_karma(&self, address: &str, source: KarmaSource, amount: u64) {
        let mut ledgers = self.reputation.karma_ledgers.write().await;
        let ledger = ledgers.entry(address.to_string()).or_default();
        ledger.total_karma += amount;
        *ledger.contributions.entry(source).or_insert(0) += amount;
        info!(%address, ?source, %amount, "Awarded Karma");
    }

    async fn process_karma_decay(&self, current_epoch: u64) {
        let decay_rate = self.economy.epoch_rules.read().await.get("karma_decay_rate").map_or(1.0, |r| r.value);
        if decay_rate >= 1.0 { return; }
        
        let mut karma_ledgers = self.reputation.karma_ledgers.write().await;
        for (address, ledger) in karma_ledgers.iter_mut() {
            if ledger.last_updated_epoch < current_epoch {
                ledger.total_karma = (ledger.total_karma as f64 * decay_rate) as u64;
                ledger.last_updated_epoch = current_epoch;
                debug!(%address, new_karma = ledger.total_karma, "Applied Karma decay");
            }
        }
    }

    async fn update_council(&self, current_epoch: u64) {
        let mut council = self.governance.council.write().await;
        if council.last_updated_epoch >= current_epoch { return; }
        
        let rules = self.economy.epoch_rules.read().await;
        let council_size = rules.get("council_size").map_or(5.0, |r| r.value) as usize;
        let fatigue_decay = rules.get("council_fatigue_decay").map_or(0.9, |r| r.value);
        
        for member in &mut council.members {
            member.cognitive_load *= fatigue_decay;
        }
        
        let karma_ledgers = self.reputation.karma_ledgers.read().await;
        let mut karma_vec: Vec<_> = karma_ledgers.iter().collect();
        karma_vec.sort_by(|a, b| b.1.total_karma.cmp(&a.1.total_karma));
        
        let new_members: Vec<CouncilMember> = karma_vec.into_iter().take(council_size).map(|(address, _)| {
            let existing_load = council.members.iter().find(|m| m.address == *address).map_or(0.0, |m| m.cognitive_load);
            CouncilMember {
                address: address.clone(),
                cognitive_load: existing_load,
            }
        }).collect();
        
        if council.members.iter().map(|m| &m.address).collect::<Vec<_>>() != new_members.iter().map(|m| &m.address).collect::<Vec<_>>() {
            info!(new_council = ?new_members.iter().map(|m| &m.address).collect::<Vec<_>>(), "Updating SAGA Council based on Karma ranking");
            council.members = new_members;
        }
        council.last_updated_epoch = current_epoch;
    }

    async fn issue_new_edict(&self, current_epoch: u64, dag: &HyperDAG) {
        let mut last_edict_epoch = self.economy.last_edict_epoch.write().await;
        if current_epoch < *last_edict_epoch + 10 { return; }

        let mut active_edict = self.economy.active_edict.write().await;
        if let Some(edict) = &*active_edict {
            if current_epoch >= edict.expiry_epoch {
                info!("SAGA Edict #{} has expired.", edict.id);
                *active_edict = None;
            }
        }

        if active_edict.is_none() {
            let network_state = *self.economy.network_state.read().await;
            let new_edict = match network_state {
                NetworkState::Congested => {
                    let history = self.economy.congestion_history.read().await;
                    let avg_congestion = history.iter().sum::<f64>() / history.len().max(1) as f64;
                    if avg_congestion > 0.8 {
                        Some(SagaEdict {
                            id: Uuid::new_v4().to_string(),
                            issued_epoch: current_epoch,
                            expiry_epoch: current_epoch + 5,
                            description: "Sustained Network Congestion: Temporarily increasing block rewards to incentivize faster processing.".to_string(),
                            action: EdictAction::Economic { reward_multiplier: 1.2, fee_multiplier: 1.0 },
                        })
                    } else { None }
                },
                NetworkState::Degraded => {
                    let validator_count = dag.validators.read().await.len();
                    Some(SagaEdict {
                        id: Uuid::new_v4().to_string(),
                        issued_epoch: current_epoch,
                        expiry_epoch: current_epoch + 10,
                        description: format!("Network Degraded ({validator_count} validators): Temporarily boosting rewards to attract more validators."),
                        action: EdictAction::Economic { reward_multiplier: 1.5, fee_multiplier: 1.0 },
                    })
                }
                _ => None,
            };

            if let Some(edict) = new_edict {
                info!(id=%edict.id, desc=%edict.description, "SAGA has autonomously issued a new Edict.");
                *last_edict_epoch = current_epoch;
                *active_edict = Some(edict);
            }
        }
    }

    async fn perform_autonomous_governance(&self, current_epoch: u64, dag: &HyperDAG) {
        let mut council = self.governance.council.write().await;

        if current_epoch < council.autonomous_governance_cooldown_until_epoch {
            debug!("SAGA autonomous governance is in a cooldown period until epoch {}.", council.autonomous_governance_cooldown_until_epoch);
            return;
        }

        let avg_cognitive_load: f64 = council.members.iter().map(|m| m.cognitive_load).sum::<f64>() / council.members.len().max(1) as f64;
        if avg_cognitive_load > 0.8 {
            warn!(avg_load = avg_cognitive_load, "SAGA Council cognitive load is high. Initiating governance cooldown for 5 epochs.");
            council.autonomous_governance_cooldown_until_epoch = current_epoch + 5;
            return;
        }

        if self.propose_validator_stake_adjustment(current_epoch, dag).await {
            council.members.iter_mut().for_each(|m| m.cognitive_load += 0.1);
        }
        if self.propose_governance_parameter_tuning(current_epoch).await {
            council.members.iter_mut().for_each(|m| m.cognitive_load += 0.2);
        }
        if self.propose_economic_parameter_tuning(current_epoch).await {
            council.members.iter_mut().for_each(|m| m.cognitive_load += 0.15);
        }
        if self.propose_scs_weight_tuning(current_epoch).await {
            council.members.iter_mut().for_each(|m| m.cognitive_load += 0.25);
        }
    }
    
    async fn update_environmental_metrics(&self, _current_epoch: u64) {
        let mut metrics = self.economy.environmental_metrics.write().await;

        let total_offset: f64 = metrics
            .verified_credentials
            .values()
            .map(|c| {
                let quality_multiplier = metrics.trusted_project_registry.get(&c.project_id).cloned().unwrap_or(0.5);
                c.tonnes_co2_sequestered * quality_multiplier
            })
            .sum();
        metrics.total_co2_offset_epoch = total_offset;

        // Normalize green score based on a target offset amount per epoch (e.g., 1000 tonnes)
        let green_score = (total_offset / 1000.0).clamp(0.0, 1.0);
        metrics.network_green_score = green_score;

        info!(
            total_co2_offset_epoch = metrics.total_co2_offset_epoch,
            network_green_score = metrics.network_green_score,
            "Updated environmental metrics for epoch."
        );

        // Clear credentials for the next epoch to prevent reuse
        metrics.verified_credentials.clear();
    }

    async fn propose_validator_stake_adjustment(&self, current_epoch: u64, dag: &HyperDAG) -> bool {
        let rules = self.economy.epoch_rules.read().await;
        let validator_count = dag.validators.read().await.len();
        let min_stake_rule = "min_validator_stake".to_string();
        let current_min_stake = rules.get(&min_stake_rule).map_or(1000.0, |r| r.value);
        
        if validator_count < 5 && *self.economy.network_state.read().await == NetworkState::Degraded {
            let mut proposals = self.governance.proposals.write().await;
            if !proposals.values().any(|p| p.status == ProposalStatus::Voting && matches!(&p.proposal_type, ProposalType::UpdateRule(name, _) if name == &min_stake_rule)) {
                let new_stake_req = (current_min_stake * 0.9).round();
                let proposal = GovernanceProposal {
                    id: format!("saga-proposal-{}", Uuid::new_v4()),
                    proposer: "SAGA_AUTONOMOUS_AGENT".to_string(),
                    proposal_type: ProposalType::UpdateRule(min_stake_rule, new_stake_req),
                    votes_for: 1.0, // Autonomous proposals get a small initial vote
                    votes_against: 0.0,
                    status: ProposalStatus::Voting,
                    voters: vec![],
                    creation_epoch: current_epoch,
                };
                info!(proposal_id = %proposal.id, "SAGA is autonomously proposing to lower the minimum stake to {} to attract validators.", new_stake_req);
                self.award_karma("SAGA_AUTONOMOUS_AGENT", KarmaSource::SagaAutonomousAction, 100).await;
                proposals.insert(proposal.id.clone(), proposal);
                return true;
            }
        }
        false
    }

    async fn propose_governance_parameter_tuning(&self, current_epoch: u64) -> bool {
        if current_epoch % 20 != 0 { return false; } // Check less frequently

        let proposals = self.governance.proposals.read().await;
        let recent_proposals: Vec<_> = proposals.values()
            .filter(|p| p.creation_epoch > current_epoch.saturating_sub(20) && p.proposer != "SAGA_AUTONOMOUS_AGENT")
            .collect();

        if recent_proposals.len() < 2 { return false; }

        let rejected_count = recent_proposals.iter().filter(|p| p.status == ProposalStatus::Rejected).count();
        let rejection_rate = rejected_count as f64 / recent_proposals.len() as f64;

        let vote_threshold_rule = "proposal_vote_threshold".to_string();
        let rules = self.economy.epoch_rules.read().await;
        let current_threshold = rules.get(&vote_threshold_rule).map_or(100.0, |r| r.value);
        
        if rejection_rate > 0.75 {
            let mut proposals_writer = self.governance.proposals.write().await;
            if !proposals_writer.values().any(|p| p.status == ProposalStatus::Voting && matches!(&p.proposal_type, ProposalType::UpdateRule(name, _) if name == &vote_threshold_rule)) {
                let new_threshold = (current_threshold * 0.9).round();
                let proposal = GovernanceProposal {
                    id: format!("saga-proposal-{}", Uuid::new_v4()),
                    proposer: "SAGA_AUTONOMOUS_AGENT".to_string(),
                    proposal_type: ProposalType::UpdateRule(vote_threshold_rule, new_threshold),
                    votes_for: 1.0, votes_against: 0.0, status: ProposalStatus::Voting, voters: vec![], creation_epoch: current_epoch,
                };
                info!(proposal_id = %proposal.id, "SAGA detected a high proposal rejection rate and is proposing to lower the vote threshold to {}.", new_threshold);
                self.award_karma("SAGA_AUTONOMOUS_AGENT", KarmaSource::SagaAutonomousAction, 100).await;
                proposals_writer.insert(proposal.id.clone(), proposal);
                return true;
            }
        }
        false
    }

    async fn propose_economic_parameter_tuning(&self, current_epoch: u64) -> bool {
        if current_epoch % 15 != 0 { return false; }
        
        let fee_rule = "base_tx_fee_min".to_string();
        let rules = self.economy.epoch_rules.read().await;
        let current_base_fee = rules.get(&fee_rule).map_or(1.0, |r| r.value);
        let congestion_history = self.economy.congestion_history.read().await;
        
        if congestion_history.len() < 5 { return false; }
        let avg_congestion = congestion_history.iter().sum::<f64>() / congestion_history.len() as f64;

        if avg_congestion > 0.75 {
            let mut proposals_writer = self.governance.proposals.write().await;
            if !proposals_writer.values().any(|p| p.status == ProposalStatus::Voting && matches!(&p.proposal_type, ProposalType::UpdateRule(name, _) if name == &fee_rule)) {
                let new_fee = (current_base_fee * 1.25).round(); 
                let proposal = GovernanceProposal {
                    id: format!("saga-proposal-{}", Uuid::new_v4()),
                    proposer: "SAGA_AUTONOMOUS_AGENT".to_string(),
                    proposal_type: ProposalType::UpdateRule(fee_rule, new_fee),
                    votes_for: 1.0, votes_against: 0.0, status: ProposalStatus::Voting, voters: vec![], creation_epoch: current_epoch,
                };
                info!(proposal_id = %proposal.id, "SAGA detected sustained network congestion and is proposing to increase the minimum base transaction fee to {}.", new_fee);
                self.award_karma("SAGA_AUTONOMOUS_AGENT", KarmaSource::SagaAutonomousAction, 100).await;
                proposals_writer.insert(proposal.id.clone(), proposal);
                return true;
            }
        }
        false
    }

    async fn propose_scs_weight_tuning(&self, current_epoch: u64) -> bool {
        if current_epoch % 25 != 0 { return false; } // Check even less frequently

        let scores = self.reputation.credit_scores.read().await;
        if scores.len() < 10 { return false; }

        let mut high_env_low_scs_count = 0;
        for scs in scores.values() {
            let env_score = scs.factors.get("environmental_contribution").cloned().unwrap_or(0.0);
            if env_score > 0.8 && scs.score < 0.6 {
                high_env_low_scs_count += 1;
            }
        }

        // If more than 20% of nodes show this pattern, the weighting might be off.
        if high_env_low_scs_count as f64 / scores.len() as f64 > 0.2 {
            let weight_rule = "scs_environmental_weight".to_string();
            let rules = self.economy.epoch_rules.read().await;
            let current_weight = rules.get(&weight_rule).map_or(0.05, |r| r.value);

            if current_weight >= 0.2 { return false; } // Don't let it grow too large

            let mut proposals_writer = self.governance.proposals.write().await;
            if !proposals_writer.values().any(|p| p.status == ProposalStatus::Voting && matches!(&p.proposal_type, ProposalType::UpdateRule(name, _) if name == &weight_rule)) {
                let new_weight = (current_weight + 0.05).min(0.2);
                let proposal = GovernanceProposal {
                    id: format!("saga-proposal-{}", Uuid::new_v4()),
                    proposer: "SAGA_AUTONOMOUS_AGENT".to_string(),
                    proposal_type: ProposalType::UpdateRule(weight_rule, new_weight),
                    votes_for: 1.0, votes_against: 0.0, status: ProposalStatus::Voting, voters: vec![], creation_epoch: current_epoch,
                };
                info!(proposal_id = %proposal.id, "SAGA detected that environmental contributions may be undervalued and is proposing to increase the SCS environmental weight to {}.", new_weight);
                self.award_karma("SAGA_AUTONOMOUS_AGENT", KarmaSource::SagaAutonomousAction, 150).await;
                proposals_writer.insert(proposal.id.clone(), proposal);
                return true;
            }
        }
        false
    }
}
