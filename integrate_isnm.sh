#!/bin/bash

# This script integrates the Infinite Strata Node Mining (ISNM) add-on.

# 1. Create the new module file
echo "Creating src/infinite_strata_node.rs..."
cat > src/infinite_strata_node.rs <<'EOF'
//! --- INFINITE STRATA NODE MINING (ISNM) Add-on ---
//! v1.0.0 - Cloud-Sustained, Resource-Absorbing Mining Protocol

use crate::hyperdag::{HyperDAG, HyperBlock};
use crate::saga::{KarmaSource, PalletSaga};
use anyhow::{anyhow, Result};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use sysinfo::{System, SystemExt};
use tokio::sync::RwLock;
use tracing::{info, warn};

// --- Constants ---
const MIN_UPTIME_HEARTBEAT_SECS: u64 = 300; // 5 minutes
const REQUIRED_CPU_LOAD_PERCENT: f32 = 0.60; // 60% of free tier capacity
const REQUIRED_MEM_USAGE_PERCENT: f32 = 0.60; // 60% of free tier capacity
const DECAY_RATE_PER_EPOCH: f64 = 0.95; // 5% decay for inactivity

/// Represents the state of a node's sustained cloud presence.
#[derive(Debug, Clone, Default)]
pub struct CloudPresence {
    pub last_heartbeat: u64,
    pub cumulative_uptime: u64,
    pub cpu_load_avg: f32,
    pub mem_usage_avg: f32,
    pub decay_score: f64, // 0.0 (fully decayed) to 1.0 (fully active)
}

/// The main ISNM service that plugs into the Hyperchain node.
#[derive(Debug)]
pub struct InfiniteStrataNode {
    dag: Arc<HyperDAG>,
    saga: Arc<PalletSaga>,
    cloud_state: Arc<RwLock<CloudPresence>>,
    sys: Arc<RwLock<System>>,
}

impl InfiniteStrataNode {
    /// Creates a new instance of the ISNM service.
    pub fn new(dag: Arc<HyperDAG>, saga: Arc<PalletSaga>) -> Self {
        info!("[ISNM] Initializing Infinite Strata Node Mining add-on...");
        Self {
            dag,
            saga,
            cloud_state: Arc::new(RwLock::new(CloudPresence {
                decay_score: 1.0, // Start with a perfect score
                ..Default::default()
            })),
            sys: Arc::new(RwLock::new(System::new_all())),
        }
    }

    /// This function would be called periodically by the node's main loop.
    pub async fn run_periodic_check(&self) -> Result<()> {
        self.update_system_metrics().await?;
        self.perform_poscp_heartbeat().await?;
        Ok(())
    }

    /// Updates the recorded CPU and memory usage.
    async fn update_system_metrics(&self) -> Result<()> {
        let mut sys_guard = self.sys.write().await;
        sys_guard.refresh_cpu();
        sys_guard.refresh_memory();

        let mut state = self.cloud_state.write().await;
        state.cpu_load_avg = sys_guard.global_cpu_info().cpu_usage();
        state.mem_usage_avg =
            (sys_guard.used_memory() as f32 / sys_guard.total_memory() as f32);

        Ok(())
    }

    /// Simulates a heartbeat to prove uptime and check for resource decay.
    async fn perform_poscp_heartbeat(&self) -> Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| anyhow!("System time error: {}", e))?
            .as_secs();

        let mut state = self.cloud_state.write().await;

        if state.last_heartbeat > 0 {
            state.cumulative_uptime += now.saturating_sub(state.last_heartbeat);
        }
        state.last_heartbeat = now;

        // Apply Cloudbound Memory Decay (CMDO) if resource usage is too low.
        if state.cpu_load_avg < REQUIRED_CPU_LOAD_PERCENT
            || state.mem_usage_avg < REQUIRED_MEM_USAGE_PERCENT
        {
            state.decay_score *= DECAY_RATE_PER_EPOCH;
            warn!("[ISNM] Cloud resource usage is below threshold. Decay score is now: {:.2}", state.decay_score);
        } else {
            // Gradually recover score if usage is back to normal
            state.decay_score = (state.decay_score + 0.01).min(1.0);
        }
        Ok(())
    }

    /// Adjusts the mining reward based on the node's PoSCP score.
    /// This hooks into the existing SAGA reward calculation.
    pub async fn get_poscp_reward_multiplier(&self) -> f64 {
        let state = self.cloud_state.read().await;
        let uptime_bonus = (state.cumulative_uptime as f64 / 3600.0).min(10.0) * 0.01; // 1% bonus per hour of uptime, capped
        let final_multiplier = state.decay_score + uptime_bonus;

        final_multiplier.clamp(0.1, 1.5) // Ensure multiplier is within a reasonable range
    }
}
EOF

# 2. Update Cargo.toml
echo "Updating Cargo.toml..."
if ! grep -q "sysinfo" Cargo.toml; then
  sed -i '' '/chrono = .*/a\
sysinfo = "0.30.12"
' Cargo.toml
fi
if ! grep -q "infinite-strata" Cargo.toml; then
  echo 'infinite-strata = []' >> Cargo.toml
fi


# 3. Update src/lib.rs
echo "Updating src/lib.rs..."
if ! grep -q "infinite_strata_node" src/lib.rs; then
  echo -e '\n#[cfg(feature = "infinite-strata")]\npub mod infinite_strata_node;' >> src/lib.rs
fi

# 4. Update src/node.rs
# This is more complex and might require manual intervention if the file structure has changed.
# This script uses simple `sed` commands that should work if the file is as expected.

echo "Updating src/node.rs..."

# Add import
sed -i '' '/use tokio::sync::{mpsc, RwLock};/a\
#[cfg(feature = "infinite-strata")]\
use crate::infinite_strata_node::InfiniteStrataNode;\
use tokio::time::interval;
' src/node.rs

# Add field to Node struct
sed -i '' '/pub saga_pallet: Arc<PalletSaga>,/a\
    #[cfg(feature = "infinite-strata")]\
    isnm_service: Arc<InfiniteStrataNode>,
' src/node.rs

# Add initialization logic
sed -i '' '/let miner = Arc::new(miner_instance);/a\
\
        #[cfg(feature = "infinite-strata")]\
        let isnm_service = {\
            info!("[ISNM] Infinite Strata feature is enabled. Initializing service.");\
            Arc::new(InfiniteStrataNode::new(dag_arc.clone(), saga_pallet.clone()))\
        };\
' src/node.rs

# Add service to Ok(Self { ... })
sed -i '' '/peer_cache_path,/a\
            saga_pallet,\
            #[cfg(feature = "infinite-strata")]\
            isnm_service,
' src/node.rs

# Add periodic check task
sed -i '' '/--- API Server Task ---/i\
\
        // --- ISNM Periodic Check Task ---\
        #[cfg(feature = "infinite-strata")]\
        {\
            let isnm_clone = self.isnm_service.clone();\
            join_set.spawn(async move {\
                let mut isnm_ticker = interval(Duration::from_secs(MIN_UPTIME_HEARTBEAT_SECS));\
                loop {\
                    isnm_ticker.tick().await;\
                    info!("[ISNM] Running periodic Proof-of-Sustained-Cloud-Presence check.");\
                    if let Err(e) = isnm_clone.run_periodic_check().await {\
                        warn!("[ISNM] Periodic check failed: {}", e);\
                    }\
                }\
            });\
        }\
        // --- End ISNM Task ---\
' src/node.rs


echo "Integration complete. Please review the changes."
echo "You will need to manually integrate the reward multiplier in src/saga.rs."
echo "Then, build with: cargo build --release --features infinite-strata"
