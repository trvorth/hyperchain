use hex;
use libp2p::Multiaddr;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Write;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML parsing error: {0}")]
    TomlDe(#[from] toml::de::Error),
    #[error("TOML serialization error: {0}")]
    TomlSer(#[from] toml::ser::Error),
    #[error("Invalid address: {0}")]
    InvalidAddress(String),
    #[error("Invalid validator: {0}")]
    InvalidValidator(String),
    #[error("Invalid log level: {0}, must be trace, debug, info, warn, or error")]
    InvalidLogLevel(String),
    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),
    #[error("Validation error: {0}")]
    Validation(String),
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Config {
    pub p2p_address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_full_p2p_address: Option<String>,
    pub api_address: String,
    pub network_id: String,
    pub peers: Vec<String>,
    pub genesis_validator: String,
    pub target_block_time: u64,
    pub difficulty: u64,
    pub max_amount: u64,
    pub use_gpu: bool,
    pub zk_enabled: bool,
    pub mining_threads: usize,
    pub num_chains: u32,
    pub mining_chain_id: u32,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub p2p: P2pConfig,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct LoggingConfig {
    pub level: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string()),
        }
    }
}

impl LoggingConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        match self.level.as_str() {
            "trace" | "debug" | "info" | "warn" | "error" => Ok(()),
            _ => Err(ConfigError::InvalidLogLevel(self.level.clone())),
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct P2pConfig {
    pub heartbeat_interval: u64,
    pub mesh_n: usize,
    pub mesh_n_low: usize,
    pub mesh_n_high: usize,
}

impl Default for P2pConfig {
    fn default() -> Self {
        Self {
            heartbeat_interval: std::env::var("P2P_HEARTBEAT")
                .unwrap_or_else(|_| "10000".to_string())
                .parse()
                .unwrap_or(10000),
            mesh_n: 4,
            mesh_n_low: 1,
            mesh_n_high: 8,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

impl Config {
    pub fn new() -> Self {
        Config {
            p2p_address: std::env::var("P2P_ADDRESS")
                .unwrap_or_else(|_| "/ip4/0.0.0.0/tcp/8000".to_string()),
            local_full_p2p_address: None,
            api_address: std::env::var("API_ADDRESS")
                .unwrap_or_else(|_| "0.0.0.0:9000".to_string()),
            network_id: "hyperdag-mainnet".to_string(),
            peers: std::env::var("PEERS")
                .unwrap_or_else(|_| "".to_string())
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            genesis_validator: std::env::var("GENESIS_VALIDATOR").unwrap_or_else(|_| {
                "2119707c4caf16139cfb5c09c4dcc9bf9cfe6808b571c108d739f49cc14793b9".to_string()
            }),
            target_block_time: std::env::var("TARGET_BLOCK_TIME")
                .unwrap_or_else(|_| "60000".to_string())
                .parse()
                .unwrap_or(60000),
            difficulty: std::env::var("DIFFICULTY")
                .unwrap_or_else(|_| "100".to_string())
                .parse()
                .unwrap_or(100),
            max_amount: std::env::var("MAX_AMOUNT")
                .unwrap_or_else(|_| "10000000000".to_string())
                .parse()
                .unwrap_or(10_000_000_000),
            use_gpu: std::env::var("USE_GPU")
                .unwrap_or_else(|_| "false".to_string())
                .parse()
                .unwrap_or(false),
            zk_enabled: std::env::var("ZK_ENABLED")
                .unwrap_or_else(|_| "false".to_string())
                .parse()
                .unwrap_or(false),
            mining_threads: std::env::var("MINING_THREADS")
                .unwrap_or_else(|_| "1".to_string())
                .parse()
                .unwrap_or(1),
            num_chains: std::env::var("NUM_CHAINS")
                .unwrap_or_else(|_| "1".to_string())
                .parse()
                .unwrap_or(1),
            mining_chain_id: std::env::var("MINING_CHAIN_ID")
                .unwrap_or_else(|_| "0".to_string())
                .parse()
                .unwrap_or(0),
            logging: LoggingConfig::default(),
            p2p: P2pConfig::default(),
        }
    }

    pub fn load(path: &str) -> Result<Self, ConfigError> {
        let contents = fs::read_to_string(path)?;
        let config: Config = toml::from_str(&contents).map_err(ConfigError::TomlDe)?;
        config.logging.validate()?;
        config.validate()?;
        Ok(config)
    }

    pub fn save(&self, path: &str) -> Result<(), ConfigError> {
        let toml_string = toml::to_string_pretty(self).map_err(ConfigError::TomlSer)?;
        let mut file = File::create(path)?;
        file.write_all(toml_string.as_bytes())?;
        Ok(())
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.p2p_address.is_empty() {
            return Err(ConfigError::InvalidAddress(
                "P2P address cannot be empty".to_string(),
            ));
        }
        if let Some(full_addr) = &self.local_full_p2p_address {
            if full_addr.parse::<Multiaddr>().is_err() {
                return Err(ConfigError::InvalidAddress(format!(
                    "Invalid local_full_p2p_address format: {full_addr}"
                )));
            }
        }
        if self.api_address.is_empty() {
            return Err(ConfigError::InvalidAddress(
                "API address cannot be empty".to_string(),
            ));
        }
        if self.genesis_validator.is_empty() {
            return Err(ConfigError::InvalidValidator(
                "Genesis validator cannot be empty".to_string(),
            ));
        }
        self.p2p_address.parse::<Multiaddr>().map_err(|e| {
            ConfigError::InvalidAddress(format!("Invalid P2P address {}: {}", self.p2p_address, e))
        })?;
        for peer in &self.peers {
            peer.parse::<Multiaddr>().map_err(|e| {
                ConfigError::InvalidAddress(format!("Invalid peer address {peer}: {e}"))
            })?;
        }
        if self.genesis_validator.len() != 64 || hex::decode(&self.genesis_validator).is_err() {
            return Err(ConfigError::InvalidValidator(
                "Genesis validator must be a 64-character hex string representing 32 bytes"
                    .to_string(),
            ));
        }
        if self.target_block_time == 0 {
            return Err(ConfigError::InvalidParameter(
                "Target block time must be positive".to_string(),
            ));
        }
        if self.difficulty == 0 {
            return Err(ConfigError::InvalidParameter(
                "Difficulty must be positive".to_string(),
            ));
        }
        if self.max_amount == 0 {
            return Err(ConfigError::InvalidParameter(
                "Max amount must be positive".to_string(),
            ));
        }
        if self.num_chains == 0 {
            return Err(ConfigError::InvalidParameter(
                "Number of chains must be positive".to_string(),
            ));
        }
        if self.mining_chain_id >= self.num_chains {
            return Err(ConfigError::InvalidParameter(format!(
                "Mining chain ID {} must be less than number of chains {}",
                self.mining_chain_id, self.num_chains
            )));
        }
        if self.mining_threads == 0 || self.mining_threads > 128 {
            return Err(ConfigError::InvalidParameter(
                "Mining threads must be between 1 and 128".to_string(),
            ));
        }
        if self.p2p.mesh_n_low > self.p2p.mesh_n || self.p2p.mesh_n > self.p2p.mesh_n_high {
            return Err(ConfigError::InvalidParameter(
                "Invalid mesh parameters: must satisfy mesh_n_low <= mesh_n <= mesh_n_high"
                    .to_string(),
            ));
        }
        if self.p2p.heartbeat_interval < 100 {
            return Err(ConfigError::InvalidParameter(
                "P2P heartbeat interval must be at least 100ms".to_string(),
            ));
        }
        if self.network_id.is_empty() {
            return Err(ConfigError::Validation(
                "network_id cannot be empty".to_string(),
            ));
        }
        Ok(())
    }
}
