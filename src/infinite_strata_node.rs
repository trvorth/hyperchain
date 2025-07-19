// src/infinite_strata_node.rs

//! --- INFINITE STRATA NODE (ISN) ---
//! v14.0.3 - "SAGA Titan" Edition: Code Cleanup
//! This version resolves a compiler warning by removing the unused `is_stateless` field.

use anyhow::Result;
use rand::{thread_rng, Rng};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use sysinfo::System;
use thiserror::Error;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{error, info, instrument, warn};
use uuid::Uuid;

// --- Public Constants ---
pub const MIN_UPTIME_HEARTBEAT_SECS: u64 = 300; // 5 minutes

// ---
// # 1. Professional Error Handling & Configuration
// ---

#[derive(Error, Debug)]
pub enum NodeError {
    #[error("Failed to produce block after winning auction")]
    BlockProductionFailure,
    #[error("Verification of ZK-SNARK snapshot failed")]
    ZkSnapshotVerificationError,
    #[error("Heartbeat cycle encountered an unrecoverable error")]
    HeartbeatCycleFailed,
}

#[derive(Clone, Debug)]
pub struct NodeConfig {
    pub heartbeat_interval: Duration,
    pub max_decay_score: f64,
    pub compliance_failure_decay_rate: f64,
    pub bpf_audit_failure_decay_rate: f64,
    pub regeneration_rate: f64,
    pub decay_history_len: usize,
    // --- Auction Parameters ---
    pub auction_fee: f64,
    pub max_bid_leverage: f64,
    // --- Resource Governor Parameters ---
    pub optimal_utilization_target: f32,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            heartbeat_interval: Duration::from_secs(MIN_UPTIME_HEARTBEAT_SECS),
            max_decay_score: 1.0,
            compliance_failure_decay_rate: 0.05,
            bpf_audit_failure_decay_rate: 0.25,
            regeneration_rate: 0.01,
            decay_history_len: 12,
            auction_fee: 0.1,
            max_bid_leverage: 5.0,
            optimal_utilization_target: 0.80, // Target 80% usage for max rewards
        }
    }
}

// ---
// # 2. Core Types & State
// ---
type NodeId = Uuid;
type HeartbeatChallenge = [u8; 32];
type Bid = (f64, u64); // (Bid Score, Locked Collateral)

#[derive(Debug, Clone, Copy, PartialEq)]
enum NodeStatus {
    Active,
    Warned,
    Probation,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ResourceUsage {
    cpu: f32,
    memory: f32,
    bandwidth_mbps: f32,
}

#[derive(Debug, Clone)]
struct NodeState {
    id: NodeId,
    status: NodeStatus,
    total_uptime_ticks: u64,
    decay_score: f64,
    decay_score_history: VecDeque<f64>,
    last_slash_amount: u64,
    resources: Arc<RwLock<ResourceUsage>>,
    last_won_auction: bool,
}

// ---
// # 3. Security & Communication Primitives
// ---
#[derive(Debug)]
struct SecureChannel<T> {
    inner: T,
}
impl<T> SecureChannel<T> {
    fn new(inner: T) -> Self {
        Self { inner }
    }
    async fn receive(&self) -> &T {
        &self.inner
    }
}

fn solve_challenge(challenge: &HeartbeatChallenge, node_id: &NodeId) -> [u8; 32] {
    Sha256::new()
        .chain_update(challenge)
        .chain_update(node_id.as_bytes())
        .finalize()
        .into()
}

// ---
// # 4. Decentralized & Autonomous Services
// ---

#[derive(Clone, Debug)]
pub struct DecentralizedOracleAggregator {
    total_slashed_pool: Arc<RwLock<u64>>,
}
impl DecentralizedOracleAggregator {
    pub fn new() -> Self {
        Self {
            total_slashed_pool: Arc::new(RwLock::new(500)),
        }
    }
    async fn issue_heartbeat_challenge(&self) -> HeartbeatChallenge {
        Sha256::digest(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
                .to_le_bytes(),
        )
        .into()
    }
    #[instrument(skip(self))]
    async fn get_network_health_factors(&self, last_pool_size: u64) -> (u32, u64, f64) {
        let slashed_pool_size = *self.total_slashed_pool.read().await;
        let volatility = (slashed_pool_size as f64 - last_pool_size as f64).abs() / 500.0;
        (1, slashed_pool_size, volatility.clamp(0.0, 5.0))
    }
}

impl Default for DecentralizedOracleAggregator {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
struct SagaGovernanceClient;
impl SagaGovernanceClient {
    fn predictive_failure_analysis(&self, history: &VecDeque<f64>) -> f64 {
        if history.len() < 4 {
            return 0.0;
        }
        let recent_avg = history.iter().rev().take(4).sum::<f64>() / 4.0;
        let older_avg = history.iter().rev().skip(4).take(4).sum::<f64>() / 4.0;
        let velocity = recent_avg - older_avg;
        if velocity < -0.05 {
            velocity.abs() * 2.0
        } else {
            0.0
        }
    }
}

#[derive(Clone, Debug)]
struct AutonomousEconomicCalibrator;
impl AutonomousEconomicCalibrator {
    #[instrument(skip(self, config))]
    fn recalibrate_config(&self, config: &mut NodeConfig, network_health: (u32, u64, f64)) {
        let (_, _, volatility) = network_health;
        if volatility > 1.5 {
            info!("AEC: High volatility detected. Entering stabilization mode.");
            config.auction_fee = 0.05;
            config.optimal_utilization_target = 0.70;
        } else {
            info!("AEC: Network stable. Operating under standard parameters.");
            let default = NodeConfig::default();
            config.auction_fee = default.auction_fee;
            config.optimal_utilization_target = default.optimal_utilization_target;
        }
    }
}

// ---
// # 5. Core Logic Modules
// ---

mod resource_governor {
    use super::*;
    pub async fn govern_workload(resources: &ResourceUsage) {
        if resources.cpu > 0.95 || resources.memory > 0.95 {
            warn!(cpu = ?resources.cpu, memory = ?resources.memory, "Approaching resource limits. Throttling work.");
            sleep(Duration::from_millis(500)).await;
        }
    }
}

mod block_auctioneer {
    use super::*;
    pub fn place_bid(state: &NodeState, config: &NodeConfig, potential_reward: u64) -> Bid {
        let collateral = (potential_reward as f64 * 0.25 * state.decay_score) as u64;
        let leverage = 1.0 + (config.max_bid_leverage * state.decay_score);
        let bid_score = (collateral as f64 * leverage) - config.auction_fee;
        (bid_score.max(0.0), collateral)
    }

    pub fn run_auction(_our_bid: Bid) -> bool {
        thread_rng().gen_bool(0.8)
    }
}

mod reward_calculator {
    use super::*;
    #[instrument(skip(state, config, network_health))]
    pub async fn calculate(
        state: &NodeState,
        config: &NodeConfig,
        network_health: (u32, u64, f64),
        risk_factor: f64,
    ) -> (f64, u64, u64) {
        let (_, slashed_pool, volatility) = network_health;
        let uptime_bonus = (state.total_uptime_ticks as f64 / 288.0).min(0.5);
        let stability_bonus = if state.status == NodeStatus::Active {
            volatility * 0.1
        } else {
            0.0
        };

        let utilization_diff =
            (state.resources.read().await.cpu - config.optimal_utilization_target).abs();
        let utilization_multiplier = (1.0 - utilization_diff as f64 * 2.0).clamp(0.5, 1.0);

        let base_multiplier =
            (1.0 + uptime_bonus + stability_bonus) * (1.0 - risk_factor) * utilization_multiplier;
        let final_reward_multiplier = state.decay_score * base_multiplier;

        let base_reward = 20.0 * final_reward_multiplier;
        let general_pool_share = if state.status == NodeStatus::Active && slashed_pool > 0 {
            (slashed_pool as f64 * 0.5 * state.decay_score * utilization_multiplier) as u64
        } else {
            0
        };
        let winner_bonus = if state.last_won_auction && slashed_pool > 0 {
            (slashed_pool as f64 * 0.5) as u64
        } else {
            0
        };

        (
            base_reward,
            general_pool_share.min(slashed_pool),
            winner_bonus.min(slashed_pool),
        )
    }
}

mod verification {
    use super::*;
    pub async fn run_bpf_audit(resources: &ResourceUsage) -> (bool, [u8; 32]) {
        let mut hasher = Sha256::new();
        hasher.update(&resources.cpu.to_le_bytes());
        hasher.update(&resources.memory.to_le_bytes());
        let proof = hasher.finalize().into();
        (!thread_rng().gen_bool(0.05), proof)
    }
    pub async fn verify_zk_snapshot() -> Result<(), NodeError> {
        if thread_rng().gen_bool(0.99) {
            Ok(())
        } else {
            Err(NodeError::ZkSnapshotVerificationError)
        }
    }
}

// ---
// # 6. The Infinite Strata Node
// ---

#[derive(Debug)]
pub struct InfiniteStrataNode {
    config: RwLock<NodeConfig>,
    state: RwLock<NodeState>,
    system: Arc<RwLock<System>>,
    governance_channel: SecureChannel<SagaGovernanceClient>,
    oracle_aggregator: DecentralizedOracleAggregator,
    aec: AutonomousEconomicCalibrator,
}

impl InfiniteStrataNode {
    pub fn new(config: NodeConfig, oracle_aggregator: DecentralizedOracleAggregator) -> Self {
        let initial_state = NodeState {
            id: Uuid::new_v4(),
            status: NodeStatus::Active,
            total_uptime_ticks: 0,
            decay_score: config.max_decay_score,
            decay_score_history: VecDeque::with_capacity(config.decay_history_len),
            last_slash_amount: 0,
            resources: Arc::new(RwLock::new(ResourceUsage::default())),
            last_won_auction: false,
        };
        info!(node_id = %initial_state.id, "SAGA Titan Node Initialized.");
        Self {
            config: RwLock::new(config),
            state: RwLock::new(initial_state),
            system: Arc::new(RwLock::new(System::new_all())),
            governance_channel: SecureChannel::new(SagaGovernanceClient),
            oracle_aggregator,
            aec: AutonomousEconomicCalibrator,
        }
    }

    pub async fn start(self: Arc<Self>) -> Result<()> {
        let interval_duration = self.config.read().await.heartbeat_interval;
        let mut interval = tokio::time::interval(interval_duration);
        loop {
            interval.tick().await;
            let self_clone = self.clone();
            tokio::spawn(async move {
                if let Err(e) = self_clone.perform_heartbeat_cycle().await {
                    error!(error = %e, "Heartbeat cycle failed");
                }
            });
        }
    }

    #[instrument(skip(self), fields(node_id = %self.state.read().await.id))]
    async fn perform_heartbeat_cycle(&self) -> Result<(), NodeError> {
        let last_pool_size = *self.oracle_aggregator.total_slashed_pool.read().await;
        let network_health = self
            .oracle_aggregator
            .get_network_health_factors(last_pool_size)
            .await;
        self.aec
            .recalibrate_config(&mut *self.config.write().await, network_health);

        let _response = solve_challenge(
            &self.oracle_aggregator.issue_heartbeat_challenge().await,
            &self.state.read().await.id,
        );
        self.measure_resources().await;
        let is_compliant = self.verify_poscp().await;
        let saga_client = self.governance_channel.receive().await;

        let (bpf_audit_passed, _bpf_proof) = {
            let resources = *self.state.read().await.resources.read().await;
            verification::run_bpf_audit(&resources).await
        };

        self.update_decay_score(is_compliant, bpf_audit_passed)
            .await;

        let history = self.state.read().await.decay_score_history.clone();
        let risk_factor = saga_client.predictive_failure_analysis(&history);

        self.update_node_status().await;

        let potential_reward = network_health.1 / 2;
        let bid = {
            let state = self.state.read().await;
            let config = self.config.read().await;
            block_auctioneer::place_bid(&state, &config, potential_reward)
        };
        let we_won_auction = block_auctioneer::run_auction(bid);
        self.state.write().await.last_won_auction = we_won_auction;

        let (base, general, winner) = {
            let state = self.state.read().await;
            let config = self.config.read().await;
            reward_calculator::calculate(&state, &config, network_health, risk_factor).await
        };
        {
            let mut pool = self.oracle_aggregator.total_slashed_pool.write().await;
            *pool = pool.saturating_sub(general).saturating_sub(winner);
        }
        info!(
            total_reward = base + (general + winner) as f64,
            "Heartbeat cycle complete."
        );

        {
            let resources = *self.state.read().await.resources.read().await;
            resource_governor::govern_workload(&resources).await;
        }

        if we_won_auction {
            if let Err(e) = verification::verify_zk_snapshot().await {
                warn!("Auction winner failed to produce block! Forfeiting collateral.");
                self.slash_node(bid.1).await;
                return Err(e);
            }
        }
        Ok(())
    }

    async fn measure_resources(&self) {
        let mut sys = self.system.write().await;
        sys.refresh_cpu();
        sys.refresh_memory();

        let cpu_usage = sys.global_cpu_info().cpu_usage();
        let mem_usage = sys.used_memory() as f32 / sys.total_memory() as f32;
        let bandwidth = 0.5 + thread_rng().gen::<f32>() * 2.0;

        let state_guard = self.state.read().await;
        let mut res = state_guard.resources.write().await;
        res.cpu = cpu_usage;
        res.memory = mem_usage;
        res.bandwidth_mbps = bandwidth;
    }

    async fn verify_poscp(&self) -> bool {
        let state_guard = self.state.read().await;
        let resources_guard = state_guard.resources.read().await;
        let res = *resources_guard;
        drop(resources_guard);
        drop(state_guard);

        res.cpu >= 0.60
            && res.cpu <= 1.00
            && res.memory >= 0.60
            && res.memory <= 1.00
            && res.bandwidth_mbps >= 1.0
    }

    async fn update_decay_score(&self, is_compliant: bool, bpf_audit_passed: bool) {
        let mut state = self.state.write().await;
        let config = self.config.read().await;
        let mut score = state.decay_score;

        if !bpf_audit_passed {
            score -= config.bpf_audit_failure_decay_rate;
        } else if is_compliant {
            score += config.regeneration_rate;
            state.total_uptime_ticks += 1;
        } else {
            score -= config.compliance_failure_decay_rate;
        }
        state.decay_score = score.clamp(0.0, config.max_decay_score);

        if state.decay_score_history.len() == config.decay_history_len {
            state.decay_score_history.pop_front();
        }
        let current_score = state.decay_score;
        state.decay_score_history.push_back(current_score);
    }

    async fn update_node_status(&self) {
        let mut state = self.state.write().await;
        let old_status = state.status;
        state.status = if state.decay_score < 0.2 {
            NodeStatus::Probation
        } else if state.decay_score < 0.6 {
            NodeStatus::Warned
        } else {
            NodeStatus::Active
        };

        if state.status == NodeStatus::Probation && old_status != NodeStatus::Probation {
            let state_ro = state.clone();
            let reputation_shield = (state_ro.total_uptime_ticks as f64 / 2880.0).min(0.5);
            let pool_size = *self.oracle_aggregator.total_slashed_pool.read().await;
            let base_penalty = 50.0 + (pool_size as f64 * 0.05);
            self.slash_node((base_penalty * (1.0 - reputation_shield)) as u64)
                .await;
        }
    }

    async fn slash_node(&self, amount: u64) {
        let mut state = self.state.write().await;
        state.last_slash_amount = amount;
        let mut pool = self.oracle_aggregator.total_slashed_pool.write().await;
        *pool += amount;
        error!(
            slashed_amount = amount,
            new_pool_size = *pool,
            "Node slashed!"
        );
    }

    pub async fn run_periodic_check(self: &Arc<Self>) -> Result<(), NodeError> {
        self.perform_heartbeat_cycle().await
    }

    pub async fn get_rewards(&self) -> (f64, u64) {
        let state = self.state.read().await;
        let uptime_bonus = (state.total_uptime_ticks as f64 / 3600.0).min(10.0) * 0.01;
        let base_multiplier = (state.decay_score + uptime_bonus).clamp(0.0, 1.5);
        (base_multiplier, 0)
    }
}