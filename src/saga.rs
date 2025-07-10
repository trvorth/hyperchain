//! --- SAGA: Sentient Autonomous Governance Algorithm ---
//! A pallet to manage a dynamic, AI-driven consensus and reputation system.
//! v0.4.1 - Compatibility and Bugfix Release. Aligns metadata analysis with
//! HashMap-based Transaction struct, resolves lifetime issues in the guidance
//! system, and fixes various type mismatches and warnings.

// SAGA integrates with the ΩMEGA protocol for a unified security posture.
use crate::omega;
use crate::hyperdag::{HyperBlock, HyperDAG, MAX_TRANSACTIONS_PER_BLOCK};
// FIX: The Transaction type is brought in via hyperdag, so this explicit import is unused.
// use crate::transaction::Transaction; 
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

// When the 'ai' feature is enabled, we bring in the ML libraries.
#[cfg(feature = "ai")]
use {
    linfa::prelude::*,
    linfa_trees::DecisionTree, // While we simulate a more complex model, this is used for the placeholder.
    ndarray::{array, Array1, Array2},
};

// --- Error Handling ---
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
}

// --- Core SAGA Data Structures ---

// NOTE: While a struct is ideal for type safety, the current `Transaction` type in the
// project uses a `HashMap` for metadata. This module is aligned to that reality.
// If `Transaction` is updated, this struct can be used.
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

#[derive(Debug, Clone, Default)]
pub struct CognitiveAnalyticsEngine {
    #[cfg(feature = "ai")]
    #[allow(dead_code)]
    // EVOLVED: Placeholder now explicitly references a more advanced model concept.
    // In a real implementation, this would be a loaded, pre-trained model object.
    behavior_model: Option<DecisionTree<f64, usize>>, // Using DecisionTree as a stand-in for a more complex Gradient Boosted model.
}

#[derive(Debug, Clone, Default)]
pub struct PredictiveEconomicModel;

// --- Saga Guidance System (Saga Assistant) Evolution ---

#[derive(Debug, Clone, PartialEq, Eq)]
enum QueryIntent {
    GetInfo,
    Compare,
    Troubleshoot,
    RequestAction,
    Unknown,
}

#[derive(Debug, Clone)]
struct AnalyzedQuery {
    intent: QueryIntent,
    primary_topic: String,
    entities: Vec<String>,
    original_query: String,
}

#[derive(Debug, Clone)]
pub struct SagaGuidanceSystem {
    knowledge_base: HashMap<String, String>,
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

// --- Implementation of SAGA's Cognitive Modules ---

impl CognitiveAnalyticsEngine {
    pub fn new() -> Self {
        #[cfg(feature = "ai")]
        {
            // --- EVOLVED: Advanced ML Model Placeholder ---
            // This is a more detailed placeholder for a real ML model training pipeline.
            // In a production environment, this model would be a Gradient Boosted Tree or a
            // small neural network, trained offline on a massive, labeled dataset from the
            // live network, and loaded from a serialized file on node startup.
            // The features are expanded to be more descriptive of actor behavior.
            let records = array![
                // Features: validity, contribution, history, hazard, temporal, dissonance, metadata_integrity
                [1.0, 0.9, 0.8, 1.0, 1.0, 1.0, 1.0], // Ideal good node
                [0.0, 0.1, 0.1, 0.1, 0.0, 0.2, 0.4], // Clear malicious node (e.g., temporal attack)
                [1.0, 0.1, 0.5, 0.0, 1.0, 1.0, 0.9], // Fee spammer (valid blocks, but low fees, high tx count)
                [1.0, 1.0, 0.9, 1.0, 1.0, 0.4, 0.2], // Sophisticated actor hiding malicious tx in valid block
                [0.8, 0.5, 0.6, 0.8, 0.9, 0.7, 0.6]  // Average, slightly sloppy node
            ];
            // Targets: 0 = Malicious, 1 = Good, 2 = Selfish/Spammer
            let targets = array![1, 0, 2, 0, 1];
            let dataset = Dataset::new(records, targets);
            let model = Some(DecisionTree::params().fit(&dataset).unwrap());
            Self {
                behavior_model: model,
            }
        }
        #[cfg(not(feature = "ai"))]
        {
            Self {}
        }
    }

    #[instrument(skip(self, block, dag, rules))]
    pub async fn score_node_behavior(
        &self,
        block: &HyperBlock,
        dag: &HyperDAG,
        rules: &HashMap<String, EpochRule>,
        network_state: NetworkState,
    ) -> Result<TrustScoreBreakdown, SagaError> {
        let grace_period = rules.get("temporal_grace_period_secs").map_or(120.0, |r| r.value) as u64;

        let mut factors = HashMap::new();
        factors.insert("validity".to_string(), self.check_block_validity(block));
        factors.insert("network_contribution".to_string(), self.analyze_network_contribution(block, dag).await);
        factors.insert("historical_performance".to_string(), self.check_historical_performance(&block.miner, dag).await);
        factors.insert("cognitive_hazard".to_string(), self.analyze_cognitive_hazards(block));
        factors.insert("temporal_consistency".to_string(), self.analyze_temporal_consistency(block, dag, grace_period).await?);
        factors.insert("cognitive_dissonance".to_string(), self.analyze_cognitive_dissonance(block));
        // EVOLVED: Now calls the deeply enhanced metadata analysis function.
        factors.insert("metadata_integrity".to_string(), self.analyze_metadata_integrity(block).await);

        let predicted_behavior_score = {
            #[cfg(feature = "ai")]
            {
                if let Some(model) = &self.behavior_model {
                    let features: Array1<f64> = factors.values().cloned().collect();
                    let prediction = model.predict(&features.into_shape((1,7)).unwrap());
                    // Translate multiclass prediction to a single risk score
                    // 1 (Good) -> 1.0, 2 (Spammer) -> 0.5, 0 (Malicious) -> 0.1
                    match prediction[0] {
                        1 => 1.0,
                        2 => 0.5,
                        _ => 0.1,
                    }
                } else { 0.5 } // Default if model is absent
            }
            #[cfg(not(feature = "ai"))]
            { 0.5 } // Default if AI feature is disabled
        };
        factors.insert("predicted_behavior".to_string(), predicted_behavior_score);

        let mut final_score = 0.0;
        let mut total_weight = 0.0;
        for (factor_name, factor_score) in &factors {
            let base_weight_key = format!("trust_{}_weight", factor_name);
            let mut weight = rules.get(&base_weight_key).map_or(0.1, |r| r.value);

            // Dynamically adjust weights based on network state. This is a form of attention.
            match network_state {
                NetworkState::UnderAttack => {
                    if factor_name == "temporal_consistency" || factor_name == "cognitive_hazard" || factor_name == "metadata_integrity" {
                        weight *= 2.5; // Focus on attack vectors
                    }
                }
                NetworkState::Congested => {
                     if factor_name == "network_contribution" {
                        weight *= 1.5; // Prioritize nodes that help clear congestion
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

    fn check_block_validity(&self, block: &HyperBlock) -> f64 {
        if block.transactions.is_empty() { return 0.1; }
        let Some(coinbase) = block.transactions.first() else { return 0.0; };
        // A valid coinbase has no inputs and at least one output for the miner reward.
        if !coinbase.inputs.is_empty() || coinbase.outputs.is_empty() { return 0.2; }
        1.0
    }

    async fn analyze_network_contribution(&self, block: &HyperBlock, dag: &HyperDAG) -> f64 {
        let avg_tx_per_block = dag.get_average_tx_per_block().await;
        let block_tx_count = block.transactions.len() as f64;
        // Use a Gaussian function to reward blocks near the average size, penalizing empty or stuffed blocks.
        let deviation = (block_tx_count - avg_tx_per_block) / avg_tx_per_block.max(1.0);
        (-deviation.powi(2)).exp()
    }

    async fn check_historical_performance(&self, miner_address: &str, dag: &HyperDAG) -> f64 {
        let blocks_reader = dag.blocks.read().await;
        let total_blocks = blocks_reader.len().max(1) as f64;
        let node_blocks = blocks_reader.values().filter(|b| b.miner == *miner_address).count() as f64;
        // Simple ratio of blocks produced. A more advanced version could incorporate past scores.
        (node_blocks / total_blocks).min(1.0)
    }

    fn analyze_cognitive_hazards(&self, block: &HyperBlock) -> f64 {
        let tx_count = block.transactions.len();
        if tx_count == 0 { return 1.0; } // Empty blocks are not hazardous, just unhelpful.
        let total_fee: u64 = block.transactions.iter().map(|tx| tx.fee).sum();
        let avg_fee = total_fee as f64 / tx_count as f64;
        let tx_ratio = tx_count as f64 / MAX_TRANSACTIONS_PER_BLOCK as f64;
        // Penalize blocks that are almost full but have suspiciously low average fees (fee spam).
        if tx_ratio > 0.9 && avg_fee < 1.0 { return 0.2; }
        1.0
    }

    async fn analyze_temporal_consistency(&self, block: &HyperBlock, dag: &HyperDAG, grace_period: u64) -> Result<f64, SagaError> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| SagaError::TimeError(e.to_string()))?.as_secs();
        if block.timestamp > now + grace_period {
            warn!(block_id = %block.id, "Temporal Anomaly: Block timestamp is too far in the future.");
            return Ok(0.2);
        }
        if !block.parents.is_empty() {
            let blocks_reader = dag.blocks.read().await;
            if let Some(max_parent_time) = block.parents.iter().filter_map(|p_id| blocks_reader.get(p_id).map(|p_block| p_block.timestamp)).max() {
                if block.timestamp < max_parent_time {
                    warn!(block_id = %block.id, "Temporal Anomaly: Block timestamp is before its parent.");
                    return Ok(0.0); // A severe violation.
                }
            }
        }
        Ok(1.0)
    }

    fn analyze_cognitive_dissonance(&self, block: &HyperBlock) -> f64 {
        // Look for contradictory economic signals within the same block.
        let high_fee_txs = block.transactions.iter().filter(|tx| tx.fee > 100).count();
        let zero_fee_txs = block.transactions.iter().filter(|tx| tx.fee == 0 && !tx.inputs.is_empty()).count();
        // A block containing both very high fee and numerous zero-fee transactions is suspicious.
        if high_fee_txs > 0 && zero_fee_txs > 5 { 0.3 } else { 1.0 }
    }

    /// FIX: This function now correctly handles metadata as a `HashMap` to align with the
    /// project's `Transaction` struct, resolving multiple compiler errors.
    async fn analyze_metadata_integrity(&self, block: &HyperBlock) -> f64 {
        let tx_count = block.transactions.len().max(1) as f64;
        let mut suspicious_tx_count = 0.0;

        for tx in block.transactions.iter().skip(1) { // Skip coinbase
            // Heuristic 1: Penalize generic, missing, or obfuscated metadata.
            if let Some(origin) = tx.metadata.get("origin_component") {
                if origin.is_empty() || origin == "unknown" {
                    suspicious_tx_count += 0.5;
                }
            } else {
                suspicious_tx_count += 0.5; // Missing origin is suspicious
            }

            let intent = tx.metadata.get("intent").map(|s| s.as_str());
            if intent.is_none() || intent == Some("") {
                suspicious_tx_count += 1.0;
            }

            // Heuristic 2: Contextual check of intent vs. transaction size.
            if let Some(intent_str) = intent {
                let tx_size_bytes = serde_json::to_vec(tx).unwrap_or_default().len();
                match intent_str {
                    "contract-deployment" if tx_size_bytes < 256 => suspicious_tx_count += 0.3,
                    "P2P Transfer" if tx_size_bytes > 2048 => suspicious_tx_count += 0.3,
                    "staking-lock" if tx_size_bytes > 1024 => suspicious_tx_count += 0.2,
                    _ => (),
                }
            }
            
            // Heuristic 3: Check for high-entropy (potentially randomized) strings.
            if let Some(intent_str) = intent {
                let intent_entropy = Self::calculate_shannon_entropy(intent_str);
                if intent_entropy > 3.5 {
                    suspicious_tx_count += 0.5;
                }
            }
        }

        // The score degrades as the proportion of suspicious transactions increases.
        (1.0 - (suspicious_tx_count / tx_count)).max(0.0)
    }

    // Helper function for metadata analysis.
    fn calculate_shannon_entropy(s: &str) -> f64 {
        if s.is_empty() { return 0.0; }
        let mut map = HashMap::new();
        for c in s.chars() {
            *map.entry(c).or_insert(0) += 1;
        }
        let len = s.len() as f64;
        map.values().map(|&count| {
            let p = count as f64 / len;
            -p * p.log2()
        }).sum()
    }
}

impl PredictiveEconomicModel {
    pub fn new() -> Self { Self {} }

    /// EVOLVED: The predictive model now considers more variables for a nuanced premium.
    /// FIX: `get_average_fee` is not on `HyperDAG`, so we compute it here from recent blocks.
    pub async fn predictive_market_premium(&self, dag: &HyperDAG) -> f64 {
        let avg_tx_per_block = dag.get_average_tx_per_block().await;
        let validator_count = dag.validators.read().await.len() as f64;

        // --- Start Local Fee Velocity Calculation ---
        let fee_velocity: f64 = {
            let blocks_reader = dag.blocks.read().await;
            let recent_blocks: Vec<_> = blocks_reader.values().take(100).collect(); // Look at last 100 blocks
            if recent_blocks.is_empty() {
                1.0 // Default fee
            } else {
                let total_fees: u64 = recent_blocks.iter().flat_map(|b| &b.transactions).map(|tx| tx.fee).sum();
                let total_txs: usize = recent_blocks.iter().map(|b| b.transactions.len()).sum();
                (total_fees as f64 / total_txs.max(1) as f64).max(1.0)
            }
        };
        // --- End Local Fee Velocity Calculation ---

        // Baseline premium for participation.
        let base_premium = 1.0;

        // Demand-based premium: increases as blocks get fuller and fees rise.
        let demand_factor = (avg_tx_per_block / MAX_TRANSACTIONS_PER_BLOCK as f64) * (1.0 + (fee_velocity / 100.0).min(1.0));

        // Security-based discount: premium is lower if the network is less decentralized.
        let security_factor = (1.0 - (10.0 / validator_count).min(1.0)).max(0.5);

        // Final premium is a blend of these factors.
        let premium = base_premium + demand_factor * security_factor;
        premium.clamp(0.8, 1.7) // Clamp to prevent extreme values.
    }
}

impl Default for SagaGuidanceSystem {
    fn default() -> Self { Self::new() }
}

impl SagaGuidanceSystem {
    pub fn new() -> Self {
        let mut knowledge_base = HashMap::new();
        knowledge_base.insert("setup".to_string(), "To get started with Hyperchain, follow these steps:\n1. **Download:** Get the latest `hyperchain` binary for your OS from the official repository.\n2. **Configuration:** Run `./hyperchain --init` in your terminal. This will create a default `config.toml` and a `wallet.key` file.\n3. **Review Config:** Open `config.toml` to review settings. You can add peer addresses to connect to the network.\n4. **First Run:** Start the node with `./hyperchain`. It will automatically connect to peers if specified.".to_string());
        knowledge_base.insert("staking".to_string(), "Staking is locking up HCN to act as a validator, securing the network and earning rewards.\n- **Become a Validator:** Stake at least the minimum HCN required by current epoch rules.\n- **Earn Rewards:** Proposing valid blocks earns rewards, boosted by your Saga Credit Score (SCS).\n- **Slashing Risk:** Malicious or offline behavior can cause your stake to be 'slashed' (partially forfeited).".to_string());
        knowledge_base.insert("send".to_string(), "To send tokens, you create and broadcast a transaction using your available funds (UTXOs).\n1. **Get UTXOs:** Use the API endpoint `/utxos/{your_address}` to see your funds.\n2. **Construct Transaction:** Specify inputs (UTXOs to spend), outputs (recipients and amounts), and any change to be returned to you.\n3. **Sign & Submit:** Sign the transaction with your private key and submit it to the `/transaction` API endpoint.".to_string());
        knowledge_base.insert("saga".to_string(), "SAGA is Hyperchain's AI core. It's a dynamic system that observes, learns, and adapts network parameters.\n- **Manages:** The economy, block rewards, and consensus rules.\n- **Scores:** Node behavior to calculate your Saga Credit Score (SCS), affecting rewards and influence.\n- **Assists:** You are interacting with its Guidance module right now.".to_string());
        knowledge_base.insert("tokenomics".to_string(), "HCN is the native token of Hyperchain.\n- **Utility:** Used for transaction fees, smart contract interaction, and governance proposals.\n- **Emission:** New HCN is created as block rewards for validators. The amount is dynamically calculated by SAGA based on validator reputation (SCS), network health, and transaction fees.".to_string());
        knowledge_base.insert("scs".to_string(), "Your Saga Credit Score (SCS) is your on-chain reputation, from 0.0 to 1.0.\n- **Calculation:** It's a weighted average of your trust score (from block analysis), Karma, and stake.\n- **Importance:** A higher SCS leads to greater block rewards and voting power. A low SCS reduces rewards.".to_string());
        knowledge_base.insert("slashing".to_string(), "Slashing is a penalty for validators who act maliciously or are consistently offline. A portion of the validator's staked HCN is forfeited. SAGA determines the severity of the slash based on the infraction, leveraging its analysis from the Cognitive Engine.".to_string());
        knowledge_base.insert("karma".to_string(), "Karma is a measure of positive, long-term contribution to the Hyperchain ecosystem. You earn Karma by creating successful governance proposals, voting constructively, and participating in the network's evolution. Unlike your SCS, which can fluctuate quickly, Karma is designed to decay very slowly.".to_string());
        Self { knowledge_base }
    }

    /// EVOLVED: This function is now the entry point to a sophisticated NLU pipeline.
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
            QueryIntent::GetInfo | QueryIntent::Troubleshoot => {
                self.knowledge_base.get(&analyzed_query.primary_topic)
                    .cloned()
                    .ok_or_else(|| SagaError::InvalidHelpTopic(analyzed_query.primary_topic.clone()))?
            },
            QueryIntent::Compare => {
                // FIX: Use let bindings to fix temporary value lifetime errors.
                let not_found_str = "Topic not found.".to_string();
                let topic1_content = self.knowledge_base.get(&analyzed_query.primary_topic).unwrap_or(&not_found_str);
                let entity_key = analyzed_query.entities.first().unwrap_or(&"".to_string()).clone();
                let topic2_content = self.knowledge_base.get(&entity_key).unwrap_or(&not_found_str);
                format!("**Comparing {} and {}:**\n\n**{}:** {}\n\n**{}:** {}",
                    analyzed_query.primary_topic.to_uppercase(),
                    entity_key.to_uppercase(),
                    analyzed_query.primary_topic.to_uppercase(), topic1_content,
                    entity_key.to_uppercase(), topic2_content
                )
            },
            _ => "I understand you want to perform an action, but this interface is for guidance only. Please use the appropriate API endpoints.".to_string()
        };

        let insight_text = if let Some(insight) = proactive_insight {
            format!("\n\n**SAGA Proactive Insight:**\n*[{:?}] {}: {}*", insight.severity, insight.title, insight.detail)
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

    /// EVOLVED: Replaced simple keyword matching with a more robust NLU function.
    /// This simulates intent recognition and entity extraction.
    fn analyze_query_intent(&self, query: &str) -> Result<AnalyzedQuery, SagaError> {
        let q = query.to_lowercase();
        // FIX: Remove unused 'tokens' variable.
        // let tokens: Vec<&str> = q.split_whitespace().collect();

        // --- Intent Recognition ---
        let intent = if q.contains("what is") || q.contains("explain") || q.contains("tell me about") {
            QueryIntent::GetInfo
        } else if q.contains("vs") || q.contains("difference between") {
            QueryIntent::Compare
        } else if q.contains("fix") || q.contains("problem") || q.contains("error") {
            QueryIntent::Troubleshoot
        } else if q.contains("how to") || q.contains("can i") || q.contains("send") || q.contains("stake") {
            // "how to send" is informational, not a command.
            QueryIntent::GetInfo
        } else {
            QueryIntent::GetInfo // Default to GetInfo
        };

        // --- Entity Extraction & Topic Matching ---
        let known_topics: Vec<String> = self.knowledge_base.keys().cloned().collect();
        let mut found_topics = Vec::new();
        for topic in &known_topics {
            if q.contains(topic) {
                found_topics.push(topic.clone());
            }
        }
        
        // Add synonyms
        if q.contains("score") || q.contains("reputation") { found_topics.push("scs".to_string()); }
        if q.contains("token") || q.contains("economy") || q.contains("reward") { found_topics.push("tokenomics".to_string()); }
        if q.contains("validator") { found_topics.push("staking".to_string()); }
        if q.contains("transaction") { found_topics.push("send".to_string()); }
        if q.contains("ai") || q.contains("governance") { found_topics.push("saga".to_string()); }
        if q.contains("penalty") { found_topics.push("slashing".to_string()); }
        
        // Deduplicate topics
        found_topics.sort();
        found_topics.dedup();

        if found_topics.is_empty() {
            Err(SagaError::AmbiguousQuery(vec![]))
        } else if found_topics.len() > 1 && intent != QueryIntent::Compare {
            // If multiple topics are found and it's not a comparison, the query is ambiguous.
            Err(SagaError::AmbiguousQuery(found_topics))
        } else {
            Ok(AnalyzedQuery {
                intent,
                primary_topic: found_topics[0].clone(),
                entities: found_topics.into_iter().skip(1).collect(),
                original_query: query.to_string(),
            })
        }
    }
}


// --- Governance, Reputation & Economic State Management ---
// EVOLVED: These types are now defined here, making the pallet self-contained.

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
    pub cognitive_fatigue: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SagaCouncil {
    pub members: Vec<CouncilMember>,
    pub last_updated_epoch: u64,
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

#[derive(Debug, Clone)]
pub struct EconomicState {
    pub epoch_rules: Arc<RwLock<HashMap<String, EpochRule>>>,
    pub network_state: Arc<RwLock<NetworkState>>,
    pub active_edict: Arc<RwLock<Option<SagaEdict>>>,
    pub last_edict_epoch: Arc<RwLock<u64>>,
    pub proactive_insights: Arc<RwLock<Vec<SagaInsight>>>,
}

#[derive(Debug, Clone)]
pub struct PalletSaga {
    pub reputation: ReputationState,
    pub governance: GovernanceState,
    pub economy: EconomicState,
    pub cognitive_engine: Arc<CognitiveAnalyticsEngine>,
    pub economic_model: Arc<PredictiveEconomicModel>,
    pub guidance_system: Arc<SagaGuidanceSystem>,
}

impl Default for PalletSaga {
    fn default() -> Self { Self::new() }
}

impl PalletSaga {
    pub fn new() -> Self {
        let mut rules = HashMap::new();
        // Consensus Rules
        rules.insert("base_difficulty".to_string(), EpochRule { value: 10.0, description: "The baseline PoW difficulty before PoSe adjustments.".to_string() });
        rules.insert("min_validator_stake".to_string(), EpochRule { value: 1000.0, description: "The minimum stake required to be a validator.".to_string() });

        // SCS Weights
        rules.insert("scs_trust_weight".to_string(), EpochRule { value: 0.6, description: "Weight of Cognitive Engine score in SCS.".to_string() });
        rules.insert("scs_karma_weight".to_string(), EpochRule { value: 0.2, description: "Weight of Karma in SCS.".to_string() });
        rules.insert("scs_stake_weight".to_string(), EpochRule { value: 0.2, description: "Weight of raw stake in SCS.".to_string() });
        // Trust Score Component Weights
        rules.insert("trust_validity_weight".to_string(), EpochRule { value: 0.20, description: "Weight of block validity in trust score.".to_string() });
        rules.insert("trust_network_contribution_weight".to_string(), EpochRule { value: 0.10, description: "Weight of network contribution in trust score.".to_string() });
        rules.insert("trust_historical_performance_weight".to_string(), EpochRule { value: 0.10, description: "Weight of historical performance in trust score.".to_string() });
        rules.insert("trust_cognitive_hazard_weight".to_string(), EpochRule { value: 0.15, description: "Weight of cognitive hazard analysis in trust score.".to_string() });
        rules.insert("trust_temporal_consistency_weight".to_string(), EpochRule { value: 0.15, description: "Weight of temporal consistency in trust score.".to_string() });
        rules.insert("trust_cognitive_dissonance_weight".to_string(), EpochRule { value: 0.10, description: "Weight of cognitive dissonance in trust score.".to_string() });
        rules.insert("trust_metadata_integrity_weight".to_string(), EpochRule { value: 0.10, description: "Weight of transaction metadata integrity in trust score.".to_string() });
        rules.insert("trust_predicted_behavior_weight".to_string(), EpochRule { value: 0.10, description: "Weight of AI behavioral prediction in trust score.".to_string() });
        // Economic Parameters
        rules.insert("base_reward".to_string(), EpochRule { value: 250.0, description: "Base HCN reward per block before modifiers.".to_string() });
        rules.insert("base_tx_fee".to_string(), EpochRule { value: 1.0, description: "Base transaction fee that can be dynamically adjusted.".to_string() });
        rules.insert("omega_threat_reward_modifier".to_string(), EpochRule { value: -0.25, description: "Reward reduction per elevated ΩMEGA threat level.".to_string() });
        // Governance & Karma
        rules.insert("proposal_creation_cost".to_string(), EpochRule { value: 500.0, description: "Karma cost to create a new proposal.".to_string() });
        rules.insert("guidance_karma_cost".to_string(), EpochRule { value: 5.0, description: "Karma cost to query the SAGA Guidance System.".to_string() });
        rules.insert("karma_decay_rate".to_string(), EpochRule { value: 0.995, description: "Percentage of Karma remaining after decay each epoch.".to_string() });
        rules.insert("proposal_vote_threshold".to_string(), EpochRule { value: 100.0, description: "Minimum votes for a proposal to be enacted.".to_string() });
        rules.insert("council_size".to_string(), EpochRule { value: 5.0, description: "Number of members in the SAGA Council.".to_string() });
        rules.insert("council_fatigue_decay".to_string(), EpochRule { value: 0.9, description: "Factor by which council member fatigue decays each epoch.".to_string() });
        rules.insert("council_fatigue_per_action".to_string(), EpochRule { value: 0.1, description: "Fatigue increase for a council member per veto/vote.".to_string() });
        // Technical Parameters
        rules.insert("temporal_grace_period_secs".to_string(), EpochRule { value: 120.0, description: "Grace period in seconds for block timestamps.".to_string() });
        rules.insert("scs_karma_normalization_divisor".to_string(), EpochRule { value: 10000.0, description: "Divisor to normalize Karma for SCS.".to_string() });
        rules.insert("scs_stake_normalization_divisor".to_string(), EpochRule { value: 50000.0, description: "Divisor to normalize stake for SCS.".to_string() });
        rules.insert("scs_smoothing_factor".to_string(), EpochRule { value: 0.1, description: "Smoothing factor for updating SCS (weight of the new score).".to_string() });

        Self {
            reputation: ReputationState { credit_scores: Arc::new(RwLock::new(HashMap::new())), karma_ledgers: Arc::new(RwLock::new(HashMap::new())) },
            governance: GovernanceState { proposals: Arc::new(RwLock::new(HashMap::new())), council: Arc::new(RwLock::new(SagaCouncil::default())) },
            economy: EconomicState {
                epoch_rules: Arc::new(RwLock::new(rules)),
                network_state: Arc::new(RwLock::new(NetworkState::Nominal)),
                active_edict: Arc::new(RwLock::new(None)),
                last_edict_epoch: Arc::new(RwLock::new(0)),
                proactive_insights: Arc::new(RwLock::new(Vec::new())),
            },
            cognitive_engine: Arc::new(CognitiveAnalyticsEngine::new()),
            economic_model: Arc::new(PredictiveEconomicModel::new()),
            guidance_system: Arc::new(SagaGuidanceSystem::new()),
        }
    }

    #[instrument(skip(self, block, dag_arc))]
    pub async fn evaluate_block_with_saga(&self, block: &HyperBlock, dag_arc: &Arc<RwLock<HyperDAG>>) -> Result<()> {
        info!(block_id = %block.id, miner = %block.miner, "SAGA: Starting evaluation of new block.");
        self.evaluate_and_score_block(block, dag_arc).await?;
        info!(block_id = %block.id, "SAGA: Evaluation complete.");
        Ok(())
    }

    pub async fn calculate_dynamic_reward(&self, block: &HyperBlock, dag_arc: &Arc<RwLock<HyperDAG>>) -> Result<u64> {
        let rules = self.economy.epoch_rules.read().await;
        let base_reward = rules.get("base_reward").map_or(250.0, |r| r.value);
        let threat_modifier = rules.get("omega_threat_reward_modifier").map_or(-0.25, |r| r.value);
        let scs = self.reputation.credit_scores.read().await.get(&block.miner).map_or(0.5, |s| s.score);

        let threat_level = omega::get_threat_level().await;
        let omega_penalty = match threat_level {
            omega::identity::ThreatLevel::Nominal => 1.0,
            omega::identity::ThreatLevel::Guarded => 1.0 + threat_modifier,
            omega::identity::ThreatLevel::Elevated => 1.0 + (threat_modifier * 2.0),
        }.max(0.0); // Ensure penalty doesn't create negative rewards

        let market_premium = self.economic_model.predictive_market_premium(&*dag_arc.read().await).await;
        let edict_multiplier = if let Some(edict) = &*self.economy.active_edict.read().await {
            if let EdictAction::Economic { reward_multiplier, .. } = edict.action { reward_multiplier } else { 1.0 }
        } else { 1.0 };
        let total_fees = block.transactions.iter().map(|tx| tx.fee).sum::<u64>();

        let final_reward = (base_reward * scs * omega_penalty * market_premium * edict_multiplier) as u64 + total_fees;
        Ok(final_reward)
    }

    #[instrument(skip(self, dag))]
    pub async fn process_epoch_evolution(&self, current_epoch: u64, dag: &HyperDAG) {
        info!("Processing SAGA epoch evolution for epoch {current_epoch}");
        self.update_network_state(dag).await;
        self.generate_proactive_insights(current_epoch, dag).await;
        self.tally_proposals(current_epoch).await;
        self.process_karma_decay(current_epoch).await;
        self.update_council(current_epoch).await;
        self.issue_new_edict(current_epoch, dag).await;
        self.perform_autonomous_governance(current_epoch, dag).await;
    }

    #[instrument(skip(self, block, dag_arc))]
    async fn evaluate_and_score_block(&self, block: &HyperBlock, dag_arc: &Arc<RwLock<HyperDAG>>) -> Result<()> {
        let miner_address = &block.miner;
        let rules = self.economy.epoch_rules.read().await;
        let network_state = *self.economy.network_state.read().await;
        let trust_breakdown = self.cognitive_engine.score_node_behavior(block, &*dag_arc.read().await, &rules, network_state).await?;
        self.update_credit_score(miner_address, &trust_breakdown, dag_arc).await?;
        Ok(())
    }

    #[instrument(skip(self, trust_breakdown, dag_arc))]
    async fn update_credit_score(&self, miner_address: &str, trust_breakdown: &TrustScoreBreakdown, dag_arc: &Arc<RwLock<HyperDAG>>) -> Result<()> {
        let rules = self.economy.epoch_rules.read().await;
        let dag_read = dag_arc.read().await;
        let trust_weight = rules.get("scs_trust_weight").map_or(0.6, |r| r.value);
        let karma_weight = rules.get("scs_karma_weight").map_or(0.2, |r| r.value);
        let stake_weight = rules.get("scs_stake_weight").map_or(0.2, |r| r.value);
        let karma_divisor = rules.get("scs_karma_normalization_divisor").map_or(10000.0, |r| r.value);
        let stake_divisor = rules.get("scs_stake_normalization_divisor").map_or(50000.0, |r| r.value);
        let smoothing_factor = rules.get("scs_smoothing_factor").map_or(0.1, |r| r.value);
        let karma_score = self.reputation.karma_ledgers.read().await.get(miner_address).map_or(0.0, |kl| (kl.total_karma as f64 / karma_divisor).min(1.0));
        let stake_score = dag_read.validators.read().await.get(miner_address).map_or(0.0, |&s| (s as f64 / stake_divisor).min(1.0));
        let new_raw_score = (trust_breakdown.final_weighted_score * trust_weight) + (karma_score * karma_weight) + (stake_score * stake_weight);
        let mut scores = self.reputation.credit_scores.write().await;
        let mut scs_entry = scores.entry(miner_address.to_string()).or_default().clone();
        scs_entry.score = (scs_entry.score * (1.0 - smoothing_factor)) + (new_raw_score * smoothing_factor);
        scs_entry.last_updated = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| SagaError::TimeError(e.to_string()))?.as_secs();
        scs_entry.history.push((scs_entry.last_updated, scs_entry.score));
        if scs_entry.history.len() > 200 { scs_entry.history.remove(0); }
        scs_entry.factors = trust_breakdown.factors.clone();
        scs_entry.factors.insert("karma_score".to_string(), karma_score);
        scs_entry.factors.insert("stake_score".to_string(), stake_score);
        scores.insert(miner_address.to_string(), scs_entry.clone());
        info!(miner = miner_address, scs = scs_entry.score, "SCS Updated");
        Ok(())
    }

    async fn update_network_state(&self, dag: &HyperDAG) {
        let avg_tx_per_block = dag.get_average_tx_per_block().await;
        let validator_count = dag.validators.read().await.len();
        let threat_level = omega::get_threat_level().await;
        let mut state_writer = self.economy.network_state.write().await;
        *state_writer = if threat_level != omega::identity::ThreatLevel::Nominal { NetworkState::UnderAttack }
            else if validator_count < 10 { NetworkState::Degraded }
            else if avg_tx_per_block > (MAX_TRANSACTIONS_PER_BLOCK as f64 * 0.8) { NetworkState::Congested }
            else { NetworkState::Nominal };
        info!("SAGA has assessed the Network State as: {:?}", *state_writer);
    }

    async fn generate_proactive_insights(&self, current_epoch: u64, dag: &HyperDAG) {
        let mut insights = self.economy.proactive_insights.write().await;
        insights.retain(|i| current_epoch < i.epoch + 5);
        let network_state = *self.economy.network_state.read().await;
        if network_state == NetworkState::Congested && !insights.iter().any(|i| i.title.contains("Congestion")) {
            insights.push(SagaInsight {
                id: Uuid::new_v4().to_string(), epoch: current_epoch, title: "Network Congestion".to_string(),
                detail: "The network is experiencing high transaction volume. Consider increasing your transaction fees for faster confirmation.".to_string(),
                severity: InsightSeverity::Warning,
            });
        }
        let difficulty = *dag.difficulty.read().await;
        // FIX: Compare u64 with u64, not f64.
        if difficulty > 50 && !insights.iter().any(|i| i.title.contains("High Difficulty")) {
             insights.push(SagaInsight {
                id: Uuid::new_v4().to_string(), epoch: current_epoch, title: "High Difficulty Alert".to_string(),
                detail: "Network difficulty is currently high, which may lead to longer block times. This is normal during periods of high miner participation.".to_string(),
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
                    if let ProposalType::UpdateRule(rule_name, new_value) = &proposal.proposal_type {
                        if let Some(rule) = rules.get_mut(rule_name) {
                            info!(rule = %rule_name, old_value = rule.value, new_value = new_value, "Epoch Rule Evolved via Governance");
                            rule.value = *new_value;
                        }
                    }
                    self.award_karma(&proposal.proposer, KarmaSource::CreateSuccessfulProposal, karma_reward_proposer).await;
                    for voter in &proposal.voters {
                        if voter.voted_for { self.award_karma(&voter.address, KarmaSource::VoteForPassedProposal, karma_reward_voter).await; }
                    }
                } else if current_epoch > proposal.creation_epoch + 10 { // 10-epoch voting period
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
        for member in &mut council.members { member.cognitive_fatigue *= fatigue_decay; }
        let karma_ledgers = self.reputation.karma_ledgers.read().await;
        let mut karma_vec: Vec<_> = karma_ledgers.iter().collect();
        karma_vec.sort_by(|a, b| b.1.total_karma.cmp(&a.1.total_karma));
        let new_members: Vec<CouncilMember> = karma_vec.into_iter().take(council_size).map(|(address, _)| {
            let existing_fatigue = council.members.iter().find(|m| m.address == *address).map_or(0.0, |m| m.cognitive_fatigue);
            CouncilMember { address: address.clone(), cognitive_fatigue: existing_fatigue }
        }).collect();
        if council.members.iter().map(|m| &m.address).collect::<Vec<_>>() != new_members.iter().map(|m| &m.address).collect::<Vec<_>>() {
             info!(new_council = ?new_members.iter().map(|m| &m.address).collect::<Vec<_>>(), "Updating SAGA Council based on Karma ranking");
             council.members = new_members;
        }
        council.last_updated_epoch = current_epoch;
    }

    async fn issue_new_edict(&self, current_epoch: u64, dag: &HyperDAG) {
        let mut last_edict_epoch = self.economy.last_edict_epoch.write().await;
        if current_epoch < *last_edict_epoch + 10 { return; } // Cooldown period for edicts
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
                NetworkState::Congested => Some(SagaEdict {
                    id: Uuid::new_v4().to_string(), issued_epoch: current_epoch, expiry_epoch: current_epoch + 5,
                    description: "Network Congestion: Temporarily increasing block rewards to incentivize faster processing.".to_string(),
                    action: EdictAction::Economic { reward_multiplier: 1.2, fee_multiplier: 1.0 },
                }),
                NetworkState::Degraded => {
                    let validator_count = dag.validators.read().await.len();
                    Some(SagaEdict {
                        id: Uuid::new_v4().to_string(), issued_epoch: current_epoch, expiry_epoch: current_epoch + 10,
                        description: format!("Network Degraded ({} validators): Temporarily boosting rewards to attract more validators.", validator_count),
                        action: EdictAction::Economic { reward_multiplier: 1.5, fee_multiplier: 1.0 },
                    })
                },
                _ => None,
            };
            if let Some(edict) = new_edict {
                info!(id=%edict.id, desc=%edict.description, "SAGA has autonomously issued a new Edict.");
                *last_edict_epoch = current_epoch;
                *active_edict = Some(edict);
            }
        }
    }

    /// EVOLVED: SAGA's autonomous functions are now grouped and more diverse.
    async fn perform_autonomous_governance(&self, current_epoch: u64, dag: &HyperDAG) {
        self.propose_validator_stake_adjustment(current_epoch, dag).await;
        self.propose_governance_parameter_tuning(current_epoch).await;
        self.propose_economic_parameter_tuning(current_epoch, dag).await;
    }

    async fn propose_validator_stake_adjustment(&self, current_epoch: u64, dag: &HyperDAG) {
        let rules = self.economy.epoch_rules.read().await;
        let validator_count = dag.validators.read().await.len();
        let min_stake_rule = "min_validator_stake".to_string();
        let current_min_stake = rules.get(&min_stake_rule).map_or(1000.0, |r| r.value);
        if validator_count < 5 && *self.economy.network_state.read().await == NetworkState::Degraded {
            let mut proposals = self.governance.proposals.write().await;
            if !proposals.values().any(|p| matches!(&p.proposal_type, ProposalType::UpdateRule(name, _) if name == &min_stake_rule)) {
                let new_stake_req = (current_min_stake * 0.9).round();
                let proposal = GovernanceProposal {
                    id: format!("saga-proposal-{}", Uuid::new_v4()), proposer: "SAGA_AUTONOMOUS_AGENT".to_string(),
                    proposal_type: ProposalType::UpdateRule(min_stake_rule, new_stake_req),
                    votes_for: 1.0, votes_against: 0.0, status: ProposalStatus::Voting, // Pre-seeded with SAGA's own vote
                    voters: vec![], creation_epoch: current_epoch,
                };
                info!(proposal_id = %proposal.id, "SAGA is autonomously proposing to lower the minimum stake to {} to attract validators.", new_stake_req);
                self.award_karma("SAGA_AUTONOMOUS_AGENT", KarmaSource::SagaAutonomousAction, 100).await;
                proposals.insert(proposal.id.clone(), proposal);
            }
        }
    }

    async fn propose_governance_parameter_tuning(&self, current_epoch: u64) {
        if current_epoch % 20 != 0 { return; } // Gather data over 20 epochs.

        let proposals = self.governance.proposals.read().await;
        let recent_proposals: Vec<_> = proposals.values()
            .filter(|p| p.creation_epoch > current_epoch.saturating_sub(20) && p.proposer != "SAGA_AUTONOMOUS_AGENT")
            .collect();

        if recent_proposals.len() < 2 { return; } // Not enough data.

        let rejected_count = recent_proposals.iter().filter(|p| p.status == ProposalStatus::Rejected).count();
        let rejection_rate = rejected_count as f64 / recent_proposals.len() as f64;

        let vote_threshold_rule = "proposal_vote_threshold".to_string();
        let rules = self.economy.epoch_rules.read().await;
        let current_threshold = rules.get(&vote_threshold_rule).map_or(100.0, |r| r.value);

        if rejection_rate > 0.75 { // If >75% of proposals fail, participation may be too difficult.
            let mut proposals_writer = self.governance.proposals.write().await;
            if !proposals_writer.values().any(|p| matches!(&p.proposal_type, ProposalType::UpdateRule(name, _) if name == &vote_threshold_rule)) {
                 let new_threshold = (current_threshold * 0.9).round(); // Propose a 10% reduction.
                 let proposal = GovernanceProposal {
                    id: format!("saga-proposal-{}", Uuid::new_v4()), proposer: "SAGA_AUTONOMOUS_AGENT".to_string(),
                    proposal_type: ProposalType::UpdateRule(vote_threshold_rule, new_threshold),
                    votes_for: 1.0, votes_against: 0.0, status: ProposalStatus::Voting,
                    voters: vec![], creation_epoch: current_epoch,
                };
                info!(proposal_id = %proposal.id, "SAGA detected a high proposal rejection rate and is proposing to lower the vote threshold to {}.", new_threshold);
                self.award_karma("SAGA_AUTONOMOUS_AGENT", KarmaSource::SagaAutonomousAction, 100).await;
                proposals_writer.insert(proposal.id.clone(), proposal);
            }
        }
    }

    /// EVOLVED: New autonomous function to tune economic policy based on network conditions.
    /// FIX: Mark `dag` as unused to resolve warning.
    async fn propose_economic_parameter_tuning(&self, current_epoch: u64, _dag: &HyperDAG) {
        if current_epoch % 15 != 0 { return; } // Run every 15 epochs.

        let network_state = *self.economy.network_state.read().await;
        let fee_rule = "base_tx_fee".to_string();
        let rules = self.economy.epoch_rules.read().await;
        let current_base_fee = rules.get(&fee_rule).map_or(1.0, |r| r.value);

        if network_state == NetworkState::Congested {
            // Check if there is sustained congestion over time (simple check here).
            // A real implementation would look at a moving average of `avg_tx_per_block`.
            let mut proposals_writer = self.governance.proposals.write().await;
            // Avoid spamming proposals if one for the same rule already exists.
            if !proposals_writer.values().any(|p| matches!(&p.proposal_type, ProposalType::UpdateRule(name, _) if name == &fee_rule)) {
                let new_fee = (current_base_fee * 1.25).round(); // Propose a 25% fee increase to manage demand.
                let proposal = GovernanceProposal {
                    id: format!("saga-proposal-{}", Uuid::new_v4()), proposer: "SAGA_AUTONOMOUS_AGENT".to_string(),
                    proposal_type: ProposalType::UpdateRule(fee_rule, new_fee),
                    votes_for: 1.0, votes_against: 0.0, status: ProposalStatus::Voting,
                    voters: vec![], creation_epoch: current_epoch,
                };
                info!(proposal_id = %proposal.id, "SAGA detected sustained network congestion and is proposing to increase the base transaction fee to {}.", new_fee);
                self.award_karma("SAGA_AUTONOMOUS_AGENT", KarmaSource::SagaAutonomousAction, 100).await;
                proposals_writer.insert(proposal.id.clone(), proposal);
            }
        }
    }
}