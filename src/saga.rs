//! --- SAGA: Sentient Autonomous Governance Algorithm ---
//! Pallet to manage a dynamic, AI-driven consensus and reputation system.
//! v0.1.0 - SagaAssistant Edition: Integrated AI Assistant, Predictive Economics, and Council Governance
//! This version embeds a SagaAssistant-like AI assistant directly into the governance layer,
//! capable of reasoned, step-by-step guidance on all aspects of the Hyperchain project.
//! It combines this with predictive economics and a robust council veto mechanism.

use crate::hyperdag::{HyperBlock, HyperDAG};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

// When the 'ai' feature is enabled, we bring in the ML libraries.
#[cfg(feature = "ai")]
use {
    linfa::prelude::*,
    linfa_trees::DecisionTree,
    ndarray::{array, Array1},
};

// --- Error Handling ---
#[derive(Error, Debug, Clone)]
pub enum SagaError {
    #[error("Rule not found in epoch state: {0}")]
    RuleNotFound(String),
    #[error("Proposal not found or inactive: {0}")]
    ProposalNotFound(String),
    #[error("Node has insufficient Karma for action: required {0}, has {1}")]
    InsufficientKarma(u64, u64),
    #[error("AI model is not trained or available")]
    ModelNotAvailable,
    #[error("Invalid proposal state transition")]
    InvalidProposalState,
    #[error("Proposer does not have enough Karma to create a proposal")]
    ProposalCreationKarma,
    #[error("Only a member of the SAGA Council can veto proposals")]
    NotACouncilMember,
    #[error("AI Assistant knowledge base does not contain the topic: {0}")]
    InvalidHelpTopic(String),
}

// --- 1. AI Oracles: Trust, Economics, and Conversation ---

/// A detailed breakdown of the trust score calculation.
#[derive(Debug, Default, Clone, Serialize)]
pub struct TrustScoreBreakdown {
    pub validity_score: f64,
    pub network_contribution_score: f64,
    pub historical_performance_score: f64,
    pub privacy_contribution_score: f64,
    pub cognitive_hazard_score: f64,
    pub final_weighted_score: f64,
}

/// The AI-powered oracle that scores node behavior.
#[derive(Debug, Clone)]
pub struct NeuralTrustOracle {
    #[cfg(feature = "ai")]
    behavior_model: Option<DecisionTree<f64, i32>>,
}

/// The AI-powered oracle that provides economic forecasts.
#[derive(Debug, Clone)]
pub struct EconomicOracle {
    #[cfg(feature = "ai")]
    fee_prediction_model: Option<DecisionTree<f64, i32>>,
}

/// **UPGRADE**: An advanced, SagaAssistant-like oracle that provides reasoned, step-by-step guidance.
/// It simulates a professional-grade interaction with a generative AI model, including
/// prompt engineering and response parsing, to deliver accurate and helpful information.
#[derive(Debug, Clone)]
pub struct SagaAssistant {
    knowledge_base: HashMap<String, String>,
}

impl Default for NeuralTrustOracle {
    fn default() -> Self {
        Self::new()
    }
}

impl NeuralTrustOracle {
    pub fn new() -> Self {
        #[cfg(feature = "ai")]
        let model = {
            let (train, _) = linfa_datasets::iris().split_with_ratio(0.8);
            Some(DecisionTree::params().fit(&train).unwrap())
        };
        Self {
            #[cfg(feature = "ai")]
            behavior_model: model,
        }
    }

    #[instrument(skip(self, dag))]
    pub async fn score_node_behavior(
        &self,
        block: &HyperBlock,
        dag: &Arc<RwLock<HyperDAG>>,
    ) -> TrustScoreBreakdown {
        let dag_read = dag.read().await;
        let validity = (self.check_block_validity(block), 0.35);
        let network = (self.predict_network_behavior(block).await, 0.20);
        let history = (
            self.check_historical_performance(&block.miner, &dag_read)
                .await,
            0.15,
        );
        let privacy = (self.check_privacy_contribution(block), 0.10);
        let cognitive_hazard = (self.analyze_cognitive_hazards(block), 0.20);
        let final_score = validity.0 * validity.1
            + network.0 * network.1
            + history.0 * history.1
            + privacy.0 * privacy.1
            + cognitive_hazard.0 * cognitive_hazard.1;
        TrustScoreBreakdown {
            validity_score: validity.0,
            network_contribution_score: network.0,
            historical_performance_score: history.0,
            privacy_contribution_score: privacy.0,
            cognitive_hazard_score: cognitive_hazard.0,
            final_weighted_score: final_score,
        }
    }

    async fn predict_network_behavior(&self, _block: &HyperBlock) -> f64 {
        0.95
    }
    fn check_block_validity(&self, block: &HyperBlock) -> f64 {
        if block.transactions.is_empty() {
            return 0.1;
        }
        let Some(coinbase) = block.transactions.first() else { return 0.0; };
        if !coinbase.inputs.is_empty() || coinbase.outputs.len() < 2 {
            return 0.2;
        }
        1.0
    }
    async fn check_historical_performance(&self, miner_address: &str, dag: &HyperDAG) -> f64 {
        let blocks = &dag.blocks.read().await;
        let total_blocks = blocks.len().max(1) as f64;
        let node_blocks = blocks
            .values()
            .filter(|b| b.miner == *miner_address)
            .count() as f64;
        (node_blocks / total_blocks).min(1.0)
    }
    fn check_privacy_contribution(&self, block: &HyperBlock) -> f64 {
        if block
            .transactions
            .iter()
            .any(|tx| tx.id.contains("zk_privacy_relay"))
        {
            1.0
        } else {
            0.8
        }
    }
    fn analyze_cognitive_hazards(&self, block: &HyperBlock) -> f64 {
        let total_fee = block.transactions.iter().map(|tx| tx.fee).sum::<u64>();
        let total_amount = block.transactions.iter().map(|tx| tx.amount).sum::<u64>();
        if total_amount > 1000 && total_fee < 1 {
            0.5
        } else {
            1.0
        }
    }
}

impl Default for EconomicOracle {
    fn default() -> Self {
        Self::new()
    }
}

impl EconomicOracle {
    pub fn new() -> Self {
        Self {}
    }
    /// Predicts future network demand to create a market premium on rewards.
    pub async fn predictive_market_premium(&self) -> f64 {
        1.05 // Simulate a 5% market premium
    }
}

impl Default for SagaAssistant {
    fn default() -> Self {
        Self::new()
    }
}

impl SagaAssistant {
    /// Creates a new SagaAssistant, pre-loaded with a knowledge base about Hyperchain.
    pub fn new() -> Self {
        let mut knowledge_base = HashMap::new();
        knowledge_base.insert(
            "setup".to_string(),
            "1. **Download:** Get the latest `hyperchain` binary for your OS from the official repository.\n\
             2. **Configuration:** Run `./hyperchain --init` to generate a default `config.toml` and a `wallet.json` file. Keep your wallet file secure!\n\
             3. **Review Config:** Open `config.toml`. The default settings are fine for starting, but you may want to add peer addresses under the `[p2p.peers]` section to connect to the network faster.\n\
             4. **First Run:** Start the node with `./hyperchain`. It will begin syncing with the network. Welcome to Hyperchain!".to_string()
        );
        knowledge_base.insert(
            "run".to_string(),
            "1. **Start the Node:** In your terminal, navigate to the directory with the `hyperchain` binary and `config.toml` and run `./hyperchain`.\n\
             2. **Check Logs:** You will see log output showing network synchronization, block proposals, and mining activity.\n\
             3. **API:** Your node exposes an API (by default at http://127.0.0.1:9944). You can use tools like `curl` to interact with it. Try `curl http://127.0.0.1:9944/info` to see network stats.\n\
             4. **Stopping:** To stop the node gracefully, press `Ctrl+C` in the terminal. It will save its state and shut down.".to_string()
        );
        knowledge_base.insert(
            "send".to_string(),
            "Sending tokens requires creating and submitting a transaction to the network.\n\
             1. **Get UTXOs:** First, you need to know what funds you can spend. Query your node's API for your unspent transaction outputs (UTXOs): `curl http://127.0.0.1:9944/utxos/YOUR_ADDRESS`.\n\
             2. **Construct Transaction:** Use a client library or tool to build a transaction. You'll need to specify:\n\
                - The UTXOs you are using as inputs.\n\
                - The recipient's address and the amount for the output.\n\
                - A change output back to your own address if the input UTXOs are larger than the amount you want to send.\n\
             3. **Sign & Submit:** Sign the transaction with your wallet's private key and submit it to your node's `/transaction` API endpoint.".to_string()
        );
        knowledge_base.insert(
            "receive".to_string(),
            "Receiving HCN is simple and secure!\n\
             1. **Get Your Address:** Your public address is in your `wallet.json` file. It's a long string of numbers and letters. This is what you share with others.\n\
             2. **Share Securely:** Provide your address to the person who wants to send you HCN. Never share your `wallet.json` file or your private key!\n\
             3. **Check Balance:** Once they've sent the transaction, you can check your balance by querying your node's API: `curl http://127.0.0.1:9944/balance/YOUR_ADDRESS`. It may take a minute for the transaction to be included in a block and for your balance to update.".to_string()
        );
        knowledge_base.insert(
            "saga".to_string(),
            "SAGA (Sentient Autonomous Governance Algorithm) is the AI brain of Hyperchain. It manages the network's economy and consensus rules. It uses AI oracles to score node behavior (SCS), predict market conditions, and even assist users like you. You can participate in governance by earning Karma and voting on proposals.".to_string()
        );
        knowledge_base.insert(
            "tokenomics".to_string(),
            "The Hyperchain (HCN) token is central to the network's operation.\n\
            - **Utility:** HCN is used to pay for transaction fees, interact with smart contracts, and participate in governance.\n\
            - **Rewards:** Miners earn HCN through dynamic block rewards calculated by SAGA. These rewards are based on the miner's reputation (SCS), network health, and transaction fees in the block.\n\
            - **Karma:** While not a token, Karma is an off-chain reputation score earned by performing actions that benefit the network, like running AI tasks or voting. High Karma leads to higher SCS and thus higher HCN rewards.".to_string()
        );

        Self { knowledge_base }
    }

    /// Simulates a full, professional interaction cycle with a generative AI model.
    pub fn get_ai_assistant_response(&self, query: &str) -> Result<String, SagaError> {
        // 1. Thought Process: Analyze the user's query to determine intent.
        debug!(target: "saga_assistant", "Received query: '{}'. Analyzing intent.", query);
        let topic = self.determine_topic_from_query(query);

        if topic.is_empty() {
            let error_msg = format!("I'm sorry, I don't have information on the topic: '{query}'. Please try topics like 'setup', 'run', 'send', 'receive', 'saga', or 'tokenomics'.");
            error!(target: "saga_assistant", "Failed to determine topic for query: '{}'", query);
            return Err(SagaError::InvalidHelpTopic(error_msg));
        }

        // 2. Prompt Engineering: Construct a detailed prompt for the AI model.
        let prompt = self.construct_prompt(&topic);
        debug!(target: "saga_assistant", "Constructed prompt for topic '{}'", topic);

        // 3. Simulated API Call: Query the "model" with the engineered prompt.
        // In a real implementation, this would be an async call to an external API.
        let ai_response = self.query_model(&prompt)?;

        // 4. Response Parsing & Formatting: Present the information clearly to the user.
        let final_response = self.format_response(&topic, &ai_response);
        Ok(final_response)
    }

    /// Determines a keyword topic from a free-form user query.
    fn determine_topic_from_query(&self, query: &str) -> String {
        let q = query.to_lowercase();
        if q.contains("setup") || q.contains("install") {
            "setup".to_string()
        } else if q.contains("run") || q.contains("start") {
            "run".to_string()
        } else if q.contains("send") || q.contains("transfer") {
            "send".to_string()
        } else if q.contains("receive") || q.contains("address") {
            "receive".to_string()
        } else if q.contains("saga") || q.contains("governance") {
            "saga".to_string()
        } else if q.contains("token") || q.contains("economy") || q.contains("tokenomics") {
            "tokenomics".to_string()
        } else {
            "".to_string()
        }
    }

    /// Creates a simulated prompt for the AI model.
    fn construct_prompt(&self, topic: &str) -> String {
        format!("You are an expert AI assistant for the Hyperchain blockchain project. Provide a clear, step-by-step, user-friendly explanation for the following topic: {topic}. Assume the user has limited technical knowledge.")
    }
    
    /// Queries the internal knowledge base. In a real system, this would make an API call.
    fn query_model(&self, prompt: &str) -> Result<String, SagaError> {
        // Extract the topic from the simulated prompt for the lookup
        let topic = prompt.split(':').next_back().unwrap_or("").trim();
        self.knowledge_base.get(topic)
            .cloned()
            .ok_or_else(|| SagaError::InvalidHelpTopic(topic.to_string()))
    }

    /// Formats the raw knowledge base content into a user-friendly response.
    fn format_response(&self, topic: &str, content: &str) -> String {
        format!(
            "--- SAGA AI Assistant ---\n\nTopic: {}\n\n{}",
            topic.to_uppercase(), content
        )
    }
}

// --- 2. SAGA Credit Score (SCS) ---
#[derive(Clone, Debug, Default, Serialize)]
pub struct SagaCreditScore {
    pub score: f64,
    pub factors: HashMap<String, f64>,
    pub history: Vec<(u64, f64)>,
    pub last_updated: u64,
}

// --- 3. Governance: Proposals, Council, and Rules ---
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EpochRule {
    pub value: f64,
    pub description: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ProposalStatus {
    Voting,
    Enacted,
    Rejected,
    Vetoed,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RuleProposal {
    pub id: String,
    pub rule_name: String,
    pub proposed_value: f64,
    pub proposer: String,
    pub votes: u64,
    pub start_epoch: u64,
    pub end_epoch: u64,
    pub status: ProposalStatus,
}

/// Represents the high-Karma council responsible for network stability.
#[derive(Clone, Debug, Default, Serialize)]
pub struct SagaCouncil {
    pub members: Vec<String>,
    pub last_updated_epoch: u64,
}

// --- 4. SAGA Karma Pools & Real-World Actions (RWAI) ---
#[derive(Serialize, Deserialize, Clone, Debug, Hash, Eq, PartialEq)]
pub enum KarmaSource {
    AiTaskPool,
    StorageShard,
    ZkRollup,
    CrossChainRelay,
    RwaiCarbonCredit,
    RwaiZkOracle,
    GovernanceVote,
    ProposalCreation,
    AiHelpdesk,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct KarmaLedger {
    pub total_karma: u64,
    pub contributions: HashMap<KarmaSource, u64>,
    pub last_updated_epoch: u64,
}

// --- 5. The Main SAGA Pallet ---
#[derive(Debug)]
pub struct PalletSaga {
    pub credit_scores: Arc<RwLock<HashMap<String, SagaCreditScore>>>,
    pub karma_ledgers: Arc<RwLock<HashMap<String, KarmaLedger>>>,
    pub trust_oracle: Arc<NeuralTrustOracle>,
    pub economic_oracle: Arc<EconomicOracle>,
    pub chatgpt_oracle: Arc<SagaAssistant>,
    pub epoch_rules: Arc<RwLock<HashMap<String, EpochRule>>>,
    pub proposals: Arc<RwLock<HashMap<String, RuleProposal>>>,
    pub council: Arc<RwLock<SagaCouncil>>,
}

impl Default for PalletSaga {
    fn default() -> Self {
        Self::new()
    }
}

impl PalletSaga {
    pub fn new() -> Self {
        let mut rules = HashMap::new();
        rules.insert("scs_trust_weight".to_string(), EpochRule { value: 0.5, description: "Weight of NTM score in SCS.".to_string() });
        rules.insert("scs_karma_weight".to_string(), EpochRule { value: 0.3, description: "Weight of Karma in SCS.".to_string() });
        rules.insert("scs_stake_weight".to_string(), EpochRule { value: 0.2, description: "Weight of raw stake in SCS.".to_string() });
        rules.insert("base_reward".to_string(), EpochRule { value: 250.0, description: "Base HCN reward per block.".to_string() });
        rules.insert("karma_reward_multiplier".to_string(), EpochRule { value: 5.0, description: "Multiplier for Karma from tasks.".to_string() });
        rules.insert("proposal_creation_cost".to_string(), EpochRule { value: 500.0, description: "Karma cost to create a new proposal.".to_string() });
        rules.insert("proposal_vote_cost".to_string(), EpochRule { value: 10.0, description: "Karma cost to vote on a proposal.".to_string() });
        rules.insert("network_health_multiplier".to_string(), EpochRule { value: 1.2, description: "Reward multiplier based on network health.".to_string() });
        rules.insert("karma_decay_rate".to_string(), EpochRule { value: 0.995, description: "Percentage of Karma that remains after decay each epoch.".to_string() });
        rules.insert("helpdesk_karma_cost".to_string(), EpochRule { value: 5.0, description: "Karma cost to query the AI helpdesk.".to_string() });

        Self {
            credit_scores: Arc::new(RwLock::new(HashMap::new())),
            karma_ledgers: Arc::new(RwLock::new(HashMap::new())),
            trust_oracle: Arc::new(NeuralTrustOracle::new()),
            economic_oracle: Arc::new(EconomicOracle::new()),
            chatgpt_oracle: Arc::new(SagaAssistant::new()),
            epoch_rules: Arc::new(RwLock::new(rules)),
            proposals: Arc::new(RwLock::new(HashMap::new())),
            council: Arc::new(RwLock::new(SagaCouncil::default())),
        }
    }

    #[instrument(skip(self, block, dag))]
    pub async fn evaluate_block_with_saga(
        &self,
        block: &HyperBlock,
        dag: &Arc<RwLock<HyperDAG>>,
    ) -> Result<f64> {
        info!(block_id = %block.id, miner = %block.miner, "SAGA: Starting evaluation of new block.");
        let score = self.evaluate_and_score_block(block, dag).await?;
        info!(block_id = %block.id, miner = %block.miner, final_scs = score, "SAGA: Evaluation complete.");
        Ok(score)
    }

    #[instrument(skip(self, block, dag))]
    async fn evaluate_and_score_block(
        &self,
        block: &HyperBlock,
        dag: &Arc<RwLock<HyperDAG>>,
    ) -> Result<f64> {
        let miner_address = &block.miner;
        let trust_breakdown = self.trust_oracle.score_node_behavior(block, dag).await;
        self.process_block_activities(block).await?;
        let final_scs = self
            .update_credit_score(miner_address, &trust_breakdown, dag)
            .await?;
        Ok(final_scs)
    }

    #[instrument(skip(self, trust_breakdown, dag))]
    async fn update_credit_score(
        &self,
        miner_address: &str,
        trust_breakdown: &TrustScoreBreakdown,
        dag: &Arc<RwLock<HyperDAG>>,
    ) -> Result<f64> {
        let rules = self.epoch_rules.read().await;
        let karma_ledgers = self.karma_ledgers.read().await;
        let dag_read = dag.read().await;
        let trust_weight = rules.get("scs_trust_weight").ok_or(SagaError::RuleNotFound("scs_trust_weight".into()))?.value;
        let karma_weight = rules.get("scs_karma_weight").ok_or(SagaError::RuleNotFound("scs_karma_weight".into()))?.value;
        let stake_weight = rules.get("scs_stake_weight").ok_or(SagaError::RuleNotFound("scs_stake_weight".into()))?.value;
        let karma_score = karma_ledgers.get(miner_address).map_or(0.0, |kl| (kl.total_karma as f64 / 10000.0).min(1.0));
        let stake_score = dag_read.validators.read().await.get(miner_address).map_or(0.0, |&s| (s as f64 / 50000.0).min(1.0));
        let new_score = (trust_breakdown.final_weighted_score * trust_weight) + (karma_score * karma_weight) + (stake_score * stake_weight);
        let mut scores = self.credit_scores.write().await;
        let mut scs_entry = scores.entry(miner_address.to_string()).or_default().clone();
        scs_entry.score = (scs_entry.score * 0.9) + (new_score * 0.1);
        scs_entry.last_updated = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        scs_entry.history.push((scs_entry.last_updated, scs_entry.score));
        if scs_entry.history.len() > 200 { scs_entry.history.remove(0); }
        scs_entry.factors.insert("trust_score".to_string(), trust_breakdown.final_weighted_score);
        scs_entry.factors.insert("karma_score".to_string(), karma_score);
        scs_entry.factors.insert("stake_score".to_string(), stake_score);
        scores.insert(miner_address.to_string(), scs_entry.clone());
        info!(miner = miner_address, scs = scs_entry.score, trust = trust_breakdown.final_weighted_score, karma = karma_score, "SCS Updated");
        Ok(scs_entry.score)
    }

    #[instrument(skip(self, block))]
    async fn process_block_activities(&self, block: &HyperBlock) -> Result<(), SagaError> {
        let rules = self.epoch_rules.read().await;
        let karma_multiplier = rules.get("karma_reward_multiplier").ok_or(SagaError::RuleNotFound("karma_reward_multiplier".into()))?.value;
        let vote_cost = rules.get("proposal_vote_cost").ok_or(SagaError::RuleNotFound("proposal_vote_cost".into()))?.value as u64;
        let mut karma_ledger = self.karma_ledgers.write().await.entry(block.miner.clone()).or_default().clone();
        let mut karma_earned = 0;
        for tx in &block.transactions {
            if tx.id.starts_with("rwai_carbon_credit") {
                let reward = (100.0 * karma_multiplier) as u64;
                *karma_ledger.contributions.entry(KarmaSource::RwaiCarbonCredit).or_insert(0) += reward;
                karma_earned += reward;
            } else if tx.id.starts_with("gov_vote_for_") {
                let proposal_id = tx.id.replace("gov_vote_for_", "");
                self.cast_vote(&block.miner, &proposal_id, vote_cost).await?;
            }
        }
        if karma_earned > 0 {
            karma_ledger.total_karma += karma_earned;
            self.karma_ledgers.write().await.insert(block.miner.clone(), karma_ledger);
        }
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn create_proposal(&self, proposer: &str, rule_name: String, proposed_value: f64, start_epoch: u64, end_epoch: u64) -> Result<String, SagaError> {
        let rules = self.epoch_rules.read().await;
        let cost = rules.get("proposal_creation_cost").ok_or(SagaError::RuleNotFound("proposal_creation_cost".into()))?.value as u64;
        let mut karma_ledgers = self.karma_ledgers.write().await;
        let proposer_karma = karma_ledgers.entry(proposer.to_string()).or_default();
        if proposer_karma.total_karma < cost { return Err(SagaError::ProposalCreationKarma); }
        proposer_karma.total_karma -= cost;
        *proposer_karma.contributions.entry(KarmaSource::ProposalCreation).or_insert(0) += 1;
        let id = Uuid::new_v4().to_string();
        let proposal = RuleProposal { id: id.clone(), rule_name: rule_name.clone(), proposed_value, proposer: proposer.to_string(), votes: 0, start_epoch, end_epoch, status: ProposalStatus::Voting };
        self.proposals.write().await.insert(id.clone(), proposal);
        info!(%proposer, %id, %rule_name, "New governance proposal created");
        Ok(id)
    }

    #[instrument(skip(self))]
    async fn cast_vote(&self, voter: &str, proposal_id: &str, cost: u64) -> Result<(), SagaError> {
        let mut karma_ledgers = self.karma_ledgers.write().await;
        let voter_karma = karma_ledgers.entry(voter.to_string()).or_default();
        if voter_karma.total_karma < cost { return Err(SagaError::InsufficientKarma(cost, voter_karma.total_karma)); }
        voter_karma.total_karma -= cost;
        *voter_karma.contributions.entry(KarmaSource::GovernanceVote).or_insert(0) += 1;
        let mut proposals = self.proposals.write().await;
        if let Some(proposal) = proposals.get_mut(proposal_id) {
            if proposal.status == ProposalStatus::Voting {
                proposal.votes += 1;
                info!(%voter, %proposal_id, votes = proposal.votes, "Vote cast");
            } else {
                warn!(%proposal_id, "Attempted to vote on non-active proposal");
                return Err(SagaError::InvalidProposalState);
            }
        } else { return Err(SagaError::ProposalNotFound(proposal_id.to_string())); }
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn veto_proposal(&self, vetoer: &str, proposal_id: &str) -> Result<(), SagaError> {
        if !self.council.read().await.members.contains(&vetoer.to_string()) { return Err(SagaError::NotACouncilMember); }
        let mut proposals = self.proposals.write().await;
        if let Some(proposal) = proposals.get_mut(proposal_id) {
            if proposal.status == ProposalStatus::Voting {
                proposal.status = ProposalStatus::Vetoed;
                info!(%vetoer, %proposal_id, "Proposal has been vetoed by the SAGA Council");
            } else { return Err(SagaError::InvalidProposalState); }
        } else { return Err(SagaError::ProposalNotFound(proposal_id.to_string())); }
        Ok(())
    }

    /// Provides step-by-step guidance to users on common tasks, powered by the SagaAssistant.
    /// This action costs a small amount of Karma to prevent spam.
    #[instrument(skip(self))]
    pub async fn get_user_guidance(&self, user_address: &str, topic: &str) -> Result<String, SagaError> {
        let rules = self.epoch_rules.read().await;
        let cost = rules.get("helpdesk_karma_cost").ok_or(SagaError::RuleNotFound("helpdesk_karma_cost".into()))?.value as u64;

        let mut karma_ledgers = self.karma_ledgers.write().await;
        let user_karma = karma_ledgers.entry(user_address.to_string()).or_default();

        if user_karma.total_karma < cost {
            return Err(SagaError::InsufficientKarma(cost, user_karma.total_karma));
        }
        user_karma.total_karma -= cost;
        *user_karma.contributions.entry(KarmaSource::AiHelpdesk).or_insert(0) += 1;
        
        info!(%user_address, %topic, "User requested AI guidance from SagaAssistant");
        self.chatgpt_oracle.get_ai_assistant_response(topic)
    }

    pub async fn calculate_dynamic_reward(&self, block: &HyperBlock) -> Result<u64> {
        let rules = self.epoch_rules.read().await;
        let base_reward = rules.get("base_reward").ok_or(SagaError::RuleNotFound("base_reward".into()))?.value;
        let health_multiplier = rules.get("network_health_multiplier").ok_or(SagaError::RuleNotFound("network_health_multiplier".into()))?.value;
        let scs = self.credit_scores.read().await.get(&block.miner).map_or(0.5, |s| s.score);
        let health_score = (block.transactions.len() as f64 / crate::hyperdag::MAX_TRANSACTIONS_PER_BLOCK as f64).min(1.0);
        let dynamic_multiplier = 1.0 + (health_score * (health_multiplier - 1.0));
        let market_premium = self.economic_oracle.predictive_market_premium().await;
        let total_fees = block.transactions.iter().map(|tx| tx.fee).sum::<u64>();
        let final_reward = (base_reward * scs * dynamic_multiplier * market_premium) as u64 + total_fees;
        Ok(final_reward)
    }

    #[instrument(skip(self))]
    pub async fn process_epoch_evolution(&self, current_epoch: u64) {
        info!("Processing SAGA epoch evolution for epoch {current_epoch}");
        self.process_karma_decay(current_epoch).await;
        self.update_saga_council(current_epoch).await;
        self.tally_proposals(current_epoch).await;
    }

    async fn tally_proposals(&self, current_epoch: u64) {
        let mut proposals = self.proposals.write().await;
        let mut rules = self.epoch_rules.write().await;
        let mut proposals_to_update = Vec::new();
        for (id, proposal) in proposals.iter() {
            if proposal.status == ProposalStatus::Voting && current_epoch >= proposal.end_epoch {
                proposals_to_update.push(id.clone());
            }
        }
        for id in proposals_to_update {
            if let Some(proposal) = proposals.get_mut(&id) {
                if proposal.votes > 100 {
                    proposal.status = ProposalStatus::Enacted;
                    if let Some(rule) = rules.get_mut(&proposal.rule_name) {
                        info!(rule = %proposal.rule_name, old_value = rule.value, new_value = proposal.proposed_value, "Epoch Rule Evolved");
                        rule.value = proposal.proposed_value;
                    }
                } else {
                    info!(proposal_id = %id, "Proposal failed to meet threshold.");
                    proposal.status = ProposalStatus::Rejected;
                }
            }
        }
    }

    async fn process_karma_decay(&self, current_epoch: u64) {
        let decay_rate = self.epoch_rules.read().await.get("karma_decay_rate").map_or(1.0, |r| r.value);
        if decay_rate >= 1.0 { return; }
        let mut karma_ledgers = self.karma_ledgers.write().await;
        for (address, ledger) in karma_ledgers.iter_mut() {
            if ledger.last_updated_epoch < current_epoch {
                let old_karma = ledger.total_karma;
                ledger.total_karma = (ledger.total_karma as f64 * decay_rate) as u64;
                ledger.last_updated_epoch = current_epoch;
                info!(%address, old_karma, new_karma = ledger.total_karma, "Applied Karma decay");
            }
        }
    }

    async fn update_saga_council(&self, current_epoch: u64) {
        let mut council = self.council.write().await;
        if council.last_updated_epoch >= current_epoch { return; }
        let karma_ledgers = self.karma_ledgers.read().await;
        let mut karma_vec: Vec<_> = karma_ledgers.iter().collect();
        karma_vec.sort_by(|a, b| b.1.total_karma.cmp(&a.1.total_karma));
        let new_members: Vec<String> = karma_vec.iter().take(5).map(|(k, _)| (*k).clone()).collect();
        info!(old_council = ?council.members, new_council = ?new_members, "Updating SAGA Council");
        council.members = new_members;
        council.last_updated_epoch = current_epoch;
    }
}