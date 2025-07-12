// src/omega.rs

use self::identity::{xor_entropy, DigitalIdentity, ThreatLevel};
use log::{info, warn};
use once_cell::sync::Lazy;
use sp_core::H256;
use tokio::sync::Mutex;

// --- Sub-module: identity ---
// This module defines the core data structures for the ΩMEGA protocol's
// Trueform Identity Algorithm (TIA), which models the system's operational state.
pub mod identity {
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Serialize};
    use sha3::{Digest, Sha3_256};
    use sp_core::H256;

    /// Represents the system's current perceived threat level. This state is used
    /// throughout the node to dynamically adjust operational parameters, forming
    /// the basis of Tactical Network Operations (TNO).
    #[derive(Serialize, Deserialize, Clone, Debug, Copy, PartialEq, Eq)]
    pub enum ThreatLevel {
        /// Normal operating conditions. Standard security protocols are in effect.
        Nominal,
        /// Potential threat detected. System parameters are tightened, and logging is increased.
        /// This could be triggered by anomalous network behavior that isn't overtly malicious.
        Guarded,
        /// Active threat or high-risk conditions detected. Non-essential operations may be
        /// suspended, and security measures are at their highest level.
        Elevated,
    }

    /// An implementation of the Trueform Identity Algorithm (TIA).
    /// This structure represents the evolving "identity" of the node's operational state.
    /// Each significant action causes the identity to "evolve," creating a cryptographic
    /// history that is computationally difficult to forge or predict.
    #[derive(Serialize, Deserialize, Clone, Debug)]
    pub struct DigitalIdentity {
        /// The core cryptographic identifier, evolved with each action.
        pub id: H256,
        /// Timestamp of the identity's creation.
        pub created: DateTime<Utc>,
        /// Timestamp of the last time the identity's entropy was updated.
        pub last_entropy_update: DateTime<Utc>,
        /// The version number, incremented with each evolution.
        pub version: u64,
        /// A simple counter for the number of interactions (evolutions).
        pub interaction_count: u64,
        /// A cryptographic hash representing the cumulative history of all actions taken.
        /// This provides a "path" of the system's state changes over time.
        pub historical_curvature: H256,
        /// The current threat level as assessed by the ΩMEGA protocol.
        pub threat_level: ThreatLevel,
    }

    impl DigitalIdentity {
        /// Creates a new, randomized Digital Identity.
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

        /// Evolves the Trueform Identity based on a new interaction (e.g., a new transaction).
        /// This is the core of the TIA, ensuring the system's state is constantly changing
        /// in a cryptographically secure and non-repudiable way.
        pub fn evolve(&mut self, action_hash: &H256) {
            let new_entropy = H256::random();
            // Evolve the primary ID
            let mut id_hasher = Sha3_256::new();
            id_hasher.update(self.id.as_bytes());
            id_hasher.update(new_entropy.as_bytes());
            self.id = H256::from_slice(id_hasher.finalize().as_slice());
            // Evolve the historical curvature
            let mut curve_hasher = Sha3_256::new();
            curve_hasher.update(self.historical_curvature.as_bytes());
            curve_hasher.update(action_hash.as_bytes());
            self.historical_curvature = H256::from_slice(curve_hasher.finalize().as_slice());
            // Update metadata
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

    /// A cryptographic helper function for the Zero-Attack Emission Horizon (ZAEH).
    /// It combines two entropy sources using XOR, a reversible and computationally
    /// cheap operation, to create a new "wave" for analysis.
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
    use sp_core::H256;
    use tokio::time::{sleep, Duration};

    /// Runs a simulation of the TIA identity evolution for demonstration and testing.
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
// A globally accessible, thread-safe instance of the system's Digital Identity.
static SYSTEM_IDENTITY: Lazy<Mutex<DigitalIdentity>> =
    Lazy::new(|| Mutex::new(DigitalIdentity::new()));

/// The primary entry point into the ΩMEGA protocol's reflective decision-making process.
/// Before a critical action (like processing a transaction) is committed, it is "reflected"
/// against the current system identity. This function determines if the action is safe to proceed.
pub async fn reflect_on_action(action_hash: H256) -> bool {
    let mut identity = SYSTEM_IDENTITY.lock().await;

    // First check: Reflective Danger Sense (RDS)
    // Simulates the action in a "mirror space" to see if it has properties associated with known attacks.
    if is_action_dangerous_in_mirror_space(&action_hash, &identity) {
        warn!("ΛΣ-ΩMEGA [RDS]: Action {action_hash:?} rejected. Unstable in mirror dimension. Elevating threat level.");
        identity.threat_level = ThreatLevel::Elevated;
        return false;
    }

    // Second check: Zero-Attack Emission Horizon (ZAEH)
    // Creates an "anti-entropy wave" by combining the action's hash with the system's current identity.
    // If the resulting wave has low complexity, it might be part of a simplistic, brute-force attack.
    let anti_entropy_wave = xor_entropy(&identity.id, &action_hash);
    if is_wave_collapsing_logic(&anti_entropy_wave) {
        warn!("ΛΣ-ΩMEGA [ZAEH]: Action {action_hash:?} rejected. Collapsed by anti-entropy wave. Setting threat level to Guarded.");
        identity.threat_level = ThreatLevel::Guarded;
        return false;
    }

    // If all checks pass, the action is approved, and the system identity evolves.
    identity.evolve(&action_hash);

    info!(
        "ΛΣ-ΩMEGA Reflex: Approved. Identity evolved to v{}. Curvature: {:?}. Threat Level: {:?}",
        identity.version, identity.historical_curvature, identity.threat_level
    );

    // After a period of calm (100 safe interactions), the threat level can return to Nominal.
    if identity.interaction_count % 100 == 0 && identity.threat_level != ThreatLevel::Nominal {
        info!("ΛΣ-ΩMEGA: System state calming. Returning threat level to Nominal.");
        identity.threat_level = ThreatLevel::Nominal;
    }

    true
}

/// Implements the logic for Reflective Danger Sense (RDS).
/// Here, "danger" is modeled as a hash with an unusually high number of leading zeros,
/// which can be a characteristic of certain types of algorithmic attacks.
/// The danger threshold becomes lower as the system's threat level increases.
fn is_action_dangerous_in_mirror_space(action_hash: &H256, identity: &DigitalIdentity) -> bool {
    let danger_threshold = match identity.threat_level {
        ThreatLevel::Nominal => 4,   // Requires a very unusual hash to trigger
        ThreatLevel::Guarded => 3,   // More sensitive
        ThreatLevel::Elevated => 2, // Highly sensitive
    };
    count_leading_zeros(action_hash) > danger_threshold
}

/// Implements the logic for the Zero-Attack Emission Horizon (ZAEH).
/// This function measures the "complexity" of the anti-entropy wave. A very low
/// complexity (e.g., low number of set bits) is considered suspicious.
fn is_wave_collapsing_logic(wave: &H256) -> bool {
    let complexity = wave.0.iter().map(|&byte| byte.count_ones()).sum::<u32>();
    // An arbitrary threshold. A wave with very few "1"s is considered low complexity.
    complexity < 80 
}

/// Public accessor to get the current system threat level.
pub async fn get_threat_level() -> ThreatLevel {
    SYSTEM_IDENTITY.lock().await.threat_level
}
