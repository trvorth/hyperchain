// src/omega.rs

use self::identity::{xor_entropy, DigitalIdentity, ThreatLevel};
use log::{info, warn};
use once_cell::sync::Lazy;
use sp_core::H256;
use tokio::sync::Mutex;

// --- Sub-module: identity (This module is correct and unchanged) ---
pub mod identity {
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Serialize};
    use sha3::{Digest, Sha3_256};
    use sp_core::H256;

    /// Represents the system's current perceived threat level, enabling TNO.
    #[derive(Serialize, Deserialize, Clone, Debug, Copy, PartialEq, Eq)]
    pub enum ThreatLevel {
        Nominal,
        Guarded,
        Elevated,
    }

    /// An implementation of the Trueform Identity Algorithm (TIA).
    #[derive(Serialize, Deserialize, Clone, Debug)]
    pub struct DigitalIdentity {
        pub id: H256,
        pub created: DateTime<Utc>,
        pub last_entropy_update: DateTime<Utc>,
        pub version: u64,
        pub interaction_count: u64,
        pub historical_curvature: H256,
        pub threat_level: ThreatLevel,
    }

    impl DigitalIdentity {
        pub fn new() -> Self {
            let initial_entropy = H256::random();
            Self {
                id: initial_entropy,
                created: Utc::now(),
                last_entropy_update: Utc::now(),
                version: 1,
                interaction_count: 0,
                historical_curvature: H256::zero(),
                threat_level: ThreatLevel::Nominal,
            }
        }

        /// Evolves the Trueform Identity based on a new interaction.
        pub fn evolve(&mut self, action_hash: &H256) {
            let new_entropy = H256::random();
            let mut id_hasher = Sha3_256::new();
            id_hasher.update(self.id.as_bytes());
            id_hasher.update(new_entropy.as_bytes());
            self.id = H256::from_slice(id_hasher.finalize().as_slice());
            let mut curve_hasher = Sha3_256::new();
            curve_hasher.update(self.historical_curvature.as_bytes());
            curve_hasher.update(action_hash.as_bytes());
            self.historical_curvature = H256::from_slice(curve_hasher.finalize().as_slice());
            self.version += 1;
            self.interaction_count += 1;
            self.last_entropy_update = Utc::now();
        }
    }

    impl Default for DigitalIdentity {
        fn default() -> Self {
            Self::new()
        }
    }

    /// Helper for Zero-Attack Emission Horizon (ZAEH)
    pub fn xor_entropy(a: &H256, b: &H256) -> H256 {
        let mut res = [0u8; 32];
        for i in 0..32 {
            res[i] = a[i] ^ b[i];
        }
        H256::from(res)
    }
}

// --- Sub-module: simulation ---
pub mod simulation {
    use super::identity::DigitalIdentity;
    use log::info;
    // **FIX**: Import the H256 type to resolve the compiler error.
    use sp_core::H256;
    use tokio::time::{sleep, Duration};

    /// Runs a simulation of the TIA identity evolution.
    pub async fn run_simulation() {
        info!("--- [ΛΣ-ΩMEGA SIMULATION: IDENTITY EVOLUTION] ---");
        let mut identity = DigitalIdentity::new();
        info!(
            "[Time: {}] New Identity created. Version: {}, ID: {}",
            identity.created,
            identity.version,
            hex::encode(identity.id)
        );

        for i in 0..5 {
            sleep(Duration::from_millis(500)).await;
            // Evolve with a random hash to simulate an interaction
            identity.evolve(&H256::random());
            info!(
                "[Time: {}] Identity evolved after interaction #{}. Version: {}, ID: {}",
                identity.last_entropy_update,
                i + 1,
                identity.version,
                hex::encode(identity.id)
            );
        }
        info!("--- [SIMULATION COMPLETE] ---");
    }
}

// --- Helper function for TIA/RDS ---
fn count_leading_zeros(hash: &H256) -> u32 {
    let mut count = 0;
    for &byte in hash.as_bytes() {
        if byte == 0 {
            count += 8;
        } else {
            count += byte.leading_zeros();
            break;
        }
    }
    count
}

// --- Main Logic for the omega module ---
static SYSTEM_IDENTITY: Lazy<Mutex<DigitalIdentity>> =
    Lazy::new(|| Mutex::new(DigitalIdentity::new()));

pub async fn reflect_on_action(action_hash: H256) -> bool {
    let mut identity = SYSTEM_IDENTITY.lock().await;

    if is_action_dangerous_in_mirror_space(&action_hash, &identity) {
        warn!("ΛΣ-ΩMEGA [RDS]: Action {action_hash:?} rejected. Unstable in mirror dimension.");
        identity.threat_level = ThreatLevel::Elevated;
        return false;
    }

    let anti_entropy_wave = xor_entropy(&identity.id, &action_hash);
    if is_wave_collapsing_logic(&anti_entropy_wave) {
        warn!("ΛΣ-ΩMEGA [ZAEH]: Action {action_hash:?} rejected. Collapsed by anti-entropy wave.");
        identity.threat_level = ThreatLevel::Guarded;
        return false;
    }

    identity.evolve(&action_hash);

    info!(
        "ΛΣ-ΩMEGA Reflex: Approved. Identity evolved to v{}. Curvature: {:?}. Threat Level: {:?}",
        identity.version, identity.historical_curvature, identity.threat_level
    );

    // Slowly return to nominal state after a period of calm
    if identity.interaction_count % 100 == 0 {
        identity.threat_level = ThreatLevel::Nominal;
    }

    true
}

fn is_action_dangerous_in_mirror_space(action_hash: &H256, identity: &DigitalIdentity) -> bool {
    let danger_threshold = match identity.threat_level {
        ThreatLevel::Nominal => 4,
        ThreatLevel::Guarded => 3,
        ThreatLevel::Elevated => 2,
    };
    count_leading_zeros(action_hash) > danger_threshold
}

fn is_wave_collapsing_logic(wave: &H256) -> bool {
    // A simple complexity measure. Real logic would be more sophisticated.
    let complexity = wave.0.iter().map(|&byte| byte.count_ones()).sum::<u32>();
    complexity < 80 // Arbitrary threshold for "low complexity"
}

pub async fn get_threat_level() -> ThreatLevel {
    SYSTEM_IDENTITY.lock().await.threat_level
}