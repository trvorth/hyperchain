//! --- INFINITE STRATA NODE MINING (ISNM) Add-on ---
//! Cloud-Sustained, Resource-Absorbing Mining Protocol
//! v4.0.0 - Passport System, Predictive Analysis & Dynamic Economy

use anyhow::{anyhow, Result};
use rand::{thread_rng, Rng};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use sysinfo::System;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

// --- Public Constants ---
pub const MIN_UPTIME_HEARTBEAT_SECS: u64 = 300; // 5 minutes

// --- Private Constants ---
const REQUIRED_CPU_LOAD_PERCENT: f32 = 0.60;
const REQUIRED_MEM_USAGE_PERCENT: f32 = 0.60;
const PROBE_SUCCESS_RATE_THRESHOLD: f32 = 0.9;
const DECAY_RATE_PER_FAILURE: f64 = 0.95;

// Tiered Reputation Thresholds
const WARN_THRESHOLD: f64 = 0.75;
const PROBATION_THRESHOLD: f64 = 0.5;
const PROBATION_RECOVERY_CLOCKS: u32 = 12; // 1 hour of successful heartbeats

// Predictive Analysis
const SCORE_HISTORY_LENGTH: usize = 6; // Track the last 30 minutes of scores

/// A verifiable, soulbound record of a node's contributions and reputation.
/// This data could be minted as an NFT to represent the node's identity.
#[derive(Debug, Clone)]
pub struct StrataPassport {
    pub node_id: String,
    pub total_uptime_hours: u64,
    pub reputation_tier: NodeStatus,
    pub total_slashed_funds_contributed: u64,
    pub merit_badges: Vec<String>,
}

/// Represents the node's operational status within the ISNM system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum NodeStatus {
    Active,
    Warned,
    Probation,
}

/// Represents the state of a node's sustained cloud presence.
#[derive(Debug, Clone)]
pub struct CloudPresence {
    pub last_heartbeat: u64,
    pub cumulative_uptime: u64,
    pub consecutive_heartbeats_ok: u32,
    // Monitored Resources & Network Health
    pub cpu_load_avg: f32,
    pub mem_usage_avg: f32,
    pub probe_success_rate: f32,
    // Economic State & Predictive Analysis
    pub decay_score: f64,
    pub status: NodeStatus,
    pub probation_recovery_clocks: u32,
    pub score_history: VecDeque<f64>,
}

impl Default for CloudPresence {
    fn default() -> Self {
        Self {
            last_heartbeat: 0,
            cumulative_uptime: 0,
            consecutive_heartbeats_ok: 0,
            cpu_load_avg: 0.0,
            mem_usage_avg: 0.0,
            probe_success_rate: 1.0,
            decay_score: 1.0,
            status: NodeStatus::Active,
            probation_recovery_clocks: 0,
            score_history: VecDeque::with_capacity(SCORE_HISTORY_LENGTH),
        }
    }
}

/// A self-regulating mechanism to ensure the node consumes enough resources
/// to comply with free-tier terms of service without exceeding them.
#[derive(Debug, Default)]
struct ResourceGovernor;

impl ResourceGovernor {
    fn adjust_workload(&self, current_presence: &CloudPresence) {
        if current_presence.status == NodeStatus::Probation {
            warn!("[ISNM Governor] Node is on probation. Halting synthetic workloads.");
            return;
        }

        if current_presence.cpu_load_avg < REQUIRED_CPU_LOAD_PERCENT - 0.1 {
            info!("[ISNM Governor] Resource usage is low. Spawning synthetic workload to meet target.");
        } else if current_presence.cpu_load_avg > 0.95 {
            info!("[ISNM Governor] Resource usage is high. Throttling synthetic workload to avoid exceeding free tier limits.");
        }
    }
}

/// The main ISNM service that plugs into the Hyperchain node.
#[derive(Debug)]
pub struct InfiniteStrataNode {
    cloud_state: Arc<RwLock<CloudPresence>>,
    sys: Arc<RwLock<System>>,
    sentry_nodes: Vec<String>,
    governor: ResourceGovernor,
    slashed_fund_pool: Arc<RwLock<u64>>,
    dynamic_distribution_rate: Arc<RwLock<f64>>,
    passport: Arc<RwLock<StrataPassport>>,
}

impl InfiniteStrataNode {
    pub fn new() -> Self {
        let node_id = format!("strata-node-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        info!("[ISNM] Initializing Infinite Strata Node Mining add-on (v4.0)...");
        info!("[ISNM] Generated Passport ID: {}", node_id);
        Self {
            cloud_state: Arc::new(RwLock::new(CloudPresence::default())),
            sys: Arc::new(RwLock::new(System::new_all())),
            sentry_nodes: vec![
                "peer_A".to_string(),
                "peer_B".to_string(),
                "peer_C".to_string(),
            ],
            governor: ResourceGovernor::default(),
            slashed_fund_pool: Arc::new(RwLock::new(0)),
            dynamic_distribution_rate: Arc::new(RwLock::new(0.05)), // Start at 5%
            passport: Arc::new(RwLock::new(StrataPassport {
                node_id,
                total_uptime_hours: 0,
                reputation_tier: NodeStatus::Active,
                total_slashed_funds_contributed: 0,
                merit_badges: Vec::new(),
            })),
        }
    }

    /// The main periodic task called by the node's orchestrator.
    pub async fn run_periodic_check(&self) -> Result<()> {
        self.update_system_metrics().await?;
        self.run_network_probes().await?;
        self.perform_poscp_heartbeat().await?;
        self.update_dynamic_distribution_rate().await;

        let state = self.cloud_state.read().await;
        self.governor.adjust_workload(&state);
        Ok(())
    }

    /// Simulates SAGA penalizing a node and adding its funds to the redistribution pool.
    pub async fn add_to_slashed_pool(&self, amount: u64) {
        let mut pool = self.slashed_fund_pool.write().await;
        *pool += amount;
        let mut passport = self.passport.write().await;
        passport.total_slashed_funds_contributed += amount;
    }

    /// Updates recorded system metrics.
    async fn update_system_metrics(&self) -> Result<()> {
        let mut sys_guard = self.sys.write().await;
        sys_guard.refresh_cpu();
        sys_guard.refresh_memory();
        let mut state = self.cloud_state.write().await;
        state.cpu_load_avg = sys_guard.global_cpu_info().cpu_usage();
        state.mem_usage_avg = sys_guard.used_memory() as f32 / sys_guard.total_memory() as f32;
        Ok(())
    }

    /// Simulates probing network sentry nodes to verify connectivity.
    async fn run_network_probes(&self) -> Result<()> {
        let successful_probes = self
            .sentry_nodes
            .iter()
            .filter(|_| thread_rng().gen_bool(0.98))
            .count();
        let success_rate = successful_probes as f32 / self.sentry_nodes.len() as f32;
        self.cloud_state.write().await.probe_success_rate = success_rate;
        Ok(())
    }

    /// The core heartbeat logic for updating node status, decay, and passport.
    async fn perform_poscp_heartbeat(&self) -> Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| anyhow!("Time error: {}", e))?
            .as_secs();
        let mut state = self.cloud_state.write().await;

        if state.last_heartbeat > 0 {
            state.cumulative_uptime += now.saturating_sub(state.last_heartbeat);
        }
        state.last_heartbeat = now;

        let is_compliant = state.cpu_load_avg >= REQUIRED_CPU_LOAD_PERCENT
            && state.mem_usage_avg >= REQUIRED_MEM_USAGE_PERCENT
            && state.probe_success_rate >= PROBE_SUCCESS_RATE_THRESHOLD;

        if is_compliant {
            self.handle_compliant_heartbeat(&mut state).await;
        } else {
            self.handle_non_compliant_heartbeat(&mut state).await;
        }

        // FIX: Resolve the E0502 borrow checker error.
        // Read the value needed before the mutable borrow for the history push.
        let current_decay_score = state.decay_score;
        state.score_history.push_back(current_decay_score);

        if state.score_history.len() > SCORE_HISTORY_LENGTH {
            state.score_history.pop_front();
        }
        if let (Some(first), Some(last)) = (state.score_history.front(), state.score_history.back())
        {
            if (first - last) > 0.15 {
                // A drop of > 15% in 30 mins
                warn!("[ISNM Predictive] Rapid decay detected! Node is at risk of entering a penalized state. Check system load and network connectivity.");
            }
        }

        self.update_passport(&state).await;
        debug!(score = state.decay_score, status = ?state.status, "[ISNM] Heartbeat processed.");
        Ok(())
    }

    async fn handle_compliant_heartbeat(
        &self,
        state: &mut tokio::sync::RwLockWriteGuard<'_, CloudPresence>,
    ) {
        state.consecutive_heartbeats_ok += 1;
        match state.status {
            NodeStatus::Probation => {
                state.probation_recovery_clocks -= 1;
                if state.probation_recovery_clocks == 0 {
                    state.status = NodeStatus::Warned;
                    info!("[ISNM] Node has recovered from Probation to Warned status.");
                }
            }
            NodeStatus::Warned if state.consecutive_heartbeats_ok >= 12 => {
                state.status = NodeStatus::Active;
                info!("[ISNM] Node has recovered to Active status.");
            }
            NodeStatus::Active => {
                let recovery_factor = (state.consecutive_heartbeats_ok as f64 / 100.0).min(1.0);
                state.decay_score = (state.decay_score + (0.01 * recovery_factor)).min(1.0);
            }
            _ => {}
        }
    }

    async fn handle_non_compliant_heartbeat(
        &self,
        state: &mut tokio::sync::RwLockWriteGuard<'_, CloudPresence>,
    ) {
        warn!(
            cpu = state.cpu_load_avg,
            mem = state.mem_usage_avg,
            probe_rate = state.probe_success_rate,
            "[ISNM] Compliance check failed. Applying decay."
        );
        state.consecutive_heartbeats_ok = 0;
        let previous_score = state.decay_score;
        state.decay_score *= DECAY_RATE_PER_FAILURE;

        let score_lost = previous_score - state.decay_score;
        let slashed_amount = (score_lost * 100.0) as u64; // Simulate 100 HCN per full point of score lost
        self.add_to_slashed_pool(slashed_amount).await;

        if state.decay_score < PROBATION_THRESHOLD {
            if state.status != NodeStatus::Probation {
                warn!("[ISNM] Node decay score is critical. Entering Probation.");
                state.status = NodeStatus::Probation;
                state.probation_recovery_clocks = PROBATION_RECOVERY_CLOCKS;
            }
        } else if state.decay_score < WARN_THRESHOLD && state.status == NodeStatus::Active {
            warn!("[ISNM] Node decay score has dropped. Entering Warned status.");
            state.status = NodeStatus::Warned;
        }
    }

    /// Updates the node's permanent passport with new achievements.
    async fn update_passport(&self, state: &CloudPresence) {
        let mut passport = self.passport.write().await;
        passport.reputation_tier = state.status;
        let total_hours = state.cumulative_uptime / 3600;

        if total_hours > passport.total_uptime_hours {
            passport.total_uptime_hours = total_hours;
            // Award badges for uptime milestones
            let badge_name = format!("{}-Hour Vigil", total_hours);
            if !passport.merit_badges.contains(&badge_name)
                && [24, 100, 500, 1000].contains(&(total_hours as i32))
            {
                info!(
                    "[ISNM Passport] Node merit badge '{}' earned. Passport updated.",
                    badge_name
                );
                passport.merit_badges.push(badge_name);
            }
        }
    }

    /// Dynamically adjusts the reward pool distribution rate based on network health.
    async fn update_dynamic_distribution_rate(&self) {
        let state = self.cloud_state.read().await;
        let mut rate = self.dynamic_distribution_rate.write().await;
        *rate = (0.01 + (state.decay_score * 0.09)).clamp(0.0, 0.1); // Rate scales from 1% to 10%
    }

    /// Calculates the final reward tuple: (Base Reward Multiplier, Redistributed Reward).
    pub async fn get_rewards(&self) -> (f64, u64) {
        let state = self.cloud_state.read().await;
        let uptime_bonus = (state.cumulative_uptime as f64 / 3600.0).min(10.0) * 0.01;
        let mut base_multiplier = (state.decay_score + uptime_bonus).clamp(0.0, 1.5);

        match state.status {
            NodeStatus::Warned => base_multiplier *= 0.75,
            NodeStatus::Probation => base_multiplier = 0.1,
            NodeStatus::Active => {}
        };

        let mut pool = self.slashed_fund_pool.write().await;
        let distribution_rate = *self.dynamic_distribution_rate.read().await;
        let distributable_amount = (*pool as f64 * distribution_rate) as u64;

        let redistributed_reward = if state.status == NodeStatus::Active {
            let reward_share = (distributable_amount as f64 * state.decay_score) as u64;
            *pool = pool.saturating_sub(reward_share);
            reward_share
        } else {
            0
        };

        (base_multiplier, redistributed_reward)
    }
}

impl Default for InfiniteStrataNode {
    fn default() -> Self {
        Self::new()
    }
}
