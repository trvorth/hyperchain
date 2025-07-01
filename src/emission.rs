use log::debug;
use prometheus::{register_int_counter, IntCounter};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::instrument;

// Made constants public
pub const INITIAL_REWARD: u64 = 500; // Initial block reward
pub const TOTAL_SUPPLY: u64 = 10_000_000_000_000_000; // Total supply cap set to 10 Billion
pub const HALVING_PERIOD: u64 = 7_884_000; // 3 months (~30.42 days/month * 86,400 seconds/day)
pub const HALVING_FACTOR: f64 = 0.97; // 3% reduction per halving
pub const SCALE: u64 = 1_000_000; // Fixed-point scale for precision

lazy_static::lazy_static! {
    static ref HALVING_EVENTS: IntCounter = register_int_counter!(
        "emission_halving_events_total",
        "Total number of halving events"
    ).unwrap();
    static ref SUPPLY_UPDATED: IntCounter = register_int_counter!(
        "emission_supply_updated_total",
        "Total number of supply updates"
    ).unwrap();
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Emission {
    initial_reward: u64,
    total_supply: u64,
    halving_period: u64,
    halving_factor: f64,
    genesis_timestamp: u64,
    current_supply: u64,
    num_chains: u32,
    last_halving_period: u64,
}

impl Emission {
    #[instrument]
    pub fn new(
        initial_reward: u64,
        total_supply: u64,
        halving_period: u64,
        halving_factor: f64,
        genesis_timestamp: u64,
        num_chains: u32,
    ) -> Self {
        Self {
            initial_reward: initial_reward.max(1),
            total_supply,
            halving_period: halving_period.max(1), // Ensure halving_period is not zero
            halving_factor: halving_factor.clamp(0.0, 1.0),
            genesis_timestamp,
            current_supply: 0,
            num_chains: num_chains.max(1),
            last_halving_period: 0,
        }
    }

    #[instrument]
    pub fn default_with_timestamp(genesis_timestamp: u64, num_chains: u32) -> Self {
        Self::new(
            INITIAL_REWARD,
            TOTAL_SUPPLY,
            HALVING_PERIOD,
            HALVING_FACTOR,
            genesis_timestamp,
            num_chains,
        )
    }

    #[instrument]
    pub fn calculate_reward(&self, timestamp: u64) -> Result<u64, String> {
        if timestamp < self.genesis_timestamp {
            return Err("Timestamp cannot be before genesis".into());
        }

        let elapsed_time = timestamp.saturating_sub(self.genesis_timestamp);
        let elapsed_periods = elapsed_time / self.halving_period; // Already validated halving_period > 0 in new()

        let reward_scaled = self
            .initial_reward
            .checked_mul(SCALE)
            .ok_or("Reward scale overflow")?;

        let factor = self.halving_factor.powi(elapsed_periods as i32);
        // Ensure SCALE is not zero before division, though it's a const.
        let reward_f64 = (reward_scaled as f64 * factor) / (SCALE as f64).max(1.0);

        if !reward_f64.is_finite() {
            return Err("Reward calculation resulted in non-finite number".into());
        }

        let reward = reward_f64.round() as u64;
        // num_chains is validated to be at least 1 in new()
        let per_chain_reward = reward
            .checked_div(self.num_chains as u64)
            .unwrap_or(1)
            .max(1);

        if elapsed_periods > self.last_halving_period {
            // Check against persisted last_halving_period
            HALVING_EVENTS.inc();
            debug!("Halving event: period {elapsed_periods}, current reward per chain: {per_chain_reward}");
            // Note: self.last_halving_period should be updated by the caller (e.g., Miner or HyperDAG)
            // after a block containing this reward is confirmed, or via update_last_halving_period method.
        }

        Ok(per_chain_reward)
    }

    #[instrument]
    pub fn update_supply(&mut self, reward: u64) -> Result<(), String> {
        let new_supply = self.current_supply.saturating_add(reward);

        if new_supply > self.total_supply {
            self.current_supply = self.total_supply;
            return Err("Total supply cap reached or exceeded".to_string());
        }

        self.current_supply = new_supply;
        SUPPLY_UPDATED.inc();
        debug!(
            "Updated supply: {}. Reward added to total supply: {}",
            self.current_supply, reward
        );
        Ok(())
    }

    #[instrument]
    pub fn current_supply(&self) -> u64 {
        self.current_supply
    }

    #[instrument]
    pub fn total_supply(&self) -> u64 {
        self.total_supply
    }

    #[instrument]
    pub fn update_last_halving_period(&mut self, timestamp: u64) {
        if self.halving_period == 0 {
            return;
        }
        let elapsed_time = timestamp.saturating_sub(self.genesis_timestamp);
        let current_calculated_period = elapsed_time / self.halving_period;
        if current_calculated_period > self.last_halving_period {
            // HALVING_EVENTS.inc(); // Incrementing here could be redundant if also done in calculate_reward
            // This method is for explicitly updating the state if needed.
            debug!(
                "Emission state: Last halving period updated from {} to {}",
                self.last_halving_period, current_calculated_period
            );
            self.last_halving_period = current_calculated_period;
        }
    }

    #[instrument]
    pub fn quantum_resistant_adjustment(&self, entropy_seed: u64) -> u64 {
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let base_reward = self.calculate_reward(now_secs).unwrap_or(1);
        let adjustment = (entropy_seed % 1000).saturating_add(base_reward) % SCALE.max(1);
        base_reward.saturating_add(adjustment).max(1)
    }
}
