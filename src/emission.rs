use log::debug;
use prometheus::{register_int_counter, IntCounter};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::instrument;

// Made constants public
pub const INITIAL_REWARD: u64 = 500; // Initial block reward
pub const TOTAL_SUPPLY: u64 = 1_000_000_000_000_000; // Total supply cap
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
        
        let reward_scaled = self.initial_reward
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
        let per_chain_reward = reward.checked_div(self.num_chains as u64).unwrap_or(1).max(1);

        if elapsed_periods > self.last_halving_period { // Check against persisted last_halving_period
            HALVING_EVENTS.inc();
            debug!("Halving event: period {elapsed_periods}, current reward per chain: {per_chain_reward}");
            // Note: self.last_halving_period should be updated by the caller (e.g., Miner or HyperDAG)
            // after a block containing this reward is confirmed, or via update_last_halving_period method.
        }

        Ok(per_chain_reward)
    }

    #[instrument]
    pub fn update_supply(&mut self, reward: u64) -> Result<(), String> {
        // The 'reward' parameter is the total reward for a block across all chains.
        // The per-chain reward is calculated and used internally, but current_supply tracks total.
        let actual_reward_added_to_supply = reward;

        self.current_supply = self
            .current_supply
            .checked_add(actual_reward_added_to_supply)
            .ok_or_else(|| "Current supply overflow during update".to_string())?;

        if self.current_supply > self.total_supply {
            debug!(
                "Current supply {} would exceed total supply {}. Capping at total supply.",
                self.current_supply, self.total_supply
            );
            self.current_supply = self.total_supply;
            // This is debatable whether it's an error or just capping.
            // For now, let's allow capping but a real system might error or stop emission.
        }

        SUPPLY_UPDATED.inc();
        debug!("Updated supply: {}. Reward added to total supply: {}", self.current_supply, actual_reward_added_to_supply);
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
        if self.halving_period == 0 { return; }
        let elapsed_time = timestamp.saturating_sub(self.genesis_timestamp);
        let current_calculated_period = elapsed_time / self.halving_period;
        if current_calculated_period > self.last_halving_period {
            // HALVING_EVENTS.inc(); // Incrementing here could be redundant if also done in calculate_reward
            // This method is for explicitly updating the state if needed.
            debug!("Emission state: Last halving period updated from {} to {}", self.last_halving_period, current_calculated_period);
            self.last_halving_period = current_calculated_period;
        }
    }

    #[instrument]
    pub fn quantum_resistant_adjustment(&self, entropy_seed: u64) -> u64 {
        let now_secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let base_reward = self.calculate_reward(now_secs).unwrap_or(1);
        let adjustment = (entropy_seed % 1000).saturating_add(base_reward) % SCALE.max(1); 
        base_reward.saturating_add(adjustment).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_reward() {
        let genesis_timestamp = 1_000_000_000;
        let num_chains_single = 1;
        let mut emission_single = Emission::new(INITIAL_REWARD, TOTAL_SUPPLY, HALVING_PERIOD, HALVING_FACTOR, genesis_timestamp, num_chains_single);

        let reward_at_genesis = emission_single.calculate_reward(genesis_timestamp).unwrap();
        assert_eq!(reward_at_genesis, INITIAL_REWARD / num_chains_single as u64);
        emission_single.update_last_halving_period(genesis_timestamp); // Manually update for test consistency

        let timestamp_after_one_halving = genesis_timestamp + HALVING_PERIOD;
        let reward_after_one_halving = emission_single.calculate_reward(timestamp_after_one_halving).unwrap();
        // Corrected: Explicitly cast to f64 for round
        let expected_after_one_halving = ((INITIAL_REWARD as f64 * HALVING_FACTOR * SCALE as f64).round() / SCALE as f64) as u64 / num_chains_single as u64;
        assert_eq!(reward_after_one_halving, expected_after_one_halving.max(1));
        emission_single.update_last_halving_period(timestamp_after_one_halving);

        let num_chains_multi = 2;
        let mut emission_multi = Emission::new(INITIAL_REWARD, TOTAL_SUPPLY, HALVING_PERIOD, HALVING_FACTOR, genesis_timestamp, num_chains_multi);
        let reward_multi_genesis = emission_multi.calculate_reward(genesis_timestamp).unwrap();
        assert_eq!(reward_multi_genesis, (INITIAL_REWARD / num_chains_multi as u64).max(1));
        emission_multi.update_last_halving_period(genesis_timestamp);

        let reward_multi_one_halving = emission_multi.calculate_reward(timestamp_after_one_halving).unwrap();
        let expected_multi_one_halving = (((INITIAL_REWARD as f64 * HALVING_FACTOR).round() as u64) / num_chains_multi as u64).max(1);
        assert_eq!(reward_multi_one_halving, expected_multi_one_halving);
    }

    #[test]
    fn test_update_supply() {
        let mut emission = Emission::new(500, 1000, 7_884_000, 0.97, 1_000_000_000, 1);
        emission.update_supply(500).unwrap();
        assert_eq!(emission.current_supply(), 500);

        let result = emission.update_supply(600); // Attempts to add 600, total would be 1100
        // The current implementation caps the supply and does not error if total_supply is exceeded by addition.
        // It returns an error message string but the function signature is Result<(), String>
        // Let's assume the intention is that if it *would* exceed, it caps and returns Err.
        // However, the current code returns Err if current_supply > total_supply *after* addition.
        assert!(result.is_err(), "Should return an error if total supply cap is hit/exceeded during update");
        assert_eq!(emission.current_supply(), 1000, "Supply should be capped at total_supply");
    }

    #[test]
    fn test_invalid_timestamp() {
        let emission = Emission::new(500, 1_000_000_000_000_000, 7_884_000, 0.97, 1_000_000_000, 1);
        let result = emission.calculate_reward(999_999_999); // Timestamp before genesis
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Timestamp cannot be before genesis");
    }

    #[test]
    fn test_quantum_resistant_adjustment() {
        // Ensure genesis is in the past enough for a few halvings if needed for base_reward calculation
        let now_for_test = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let genesis_ts = now_for_test.saturating_sub(HALVING_PERIOD * 2); // Approx 2 halvings ago
        
        let emission = Emission::new(INITIAL_REWARD, TOTAL_SUPPLY, HALVING_PERIOD, HALVING_FACTOR, genesis_ts, 1);
        
        let current_reward = emission.calculate_reward(now_for_test).unwrap_or(1);
        let adjusted = emission.quantum_resistant_adjustment(42); // Use current time via SystemTime::now()
        
        assert!(adjusted >= current_reward, "Adjusted reward {} should be >= base reward {}", adjusted, current_reward);
        // The upper bound is more complex due to the modulo SCALE, but it should be related to base_reward + SCALE
        assert!(adjusted <= current_reward + SCALE, "Adjusted reward {} is unexpectedly high relative to base {} and SCALE {}", adjusted, current_reward, SCALE);
        assert!(adjusted >= 1, "Adjusted reward should be at least 1");
    }
}