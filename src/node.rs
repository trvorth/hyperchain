//! --- Hyperchain Node Orchestrator ---
//! v1.5.0 - SAGA Integration Hardened
//! Resolved ISNM feature integration bug by correctly wiring the service
//! into the SAGA pallet during node initialization.

use crate::config::{Config, ConfigError};
use crate::hyperdag::{HyperBlock, HyperDAG, HyperDAGError, UTXO};
use crate::mempool::Mempool;
use crate::miner::{Miner, MinerConfig, MiningError};
use crate::omega::reflect_on_action;
use crate::p2p::{P2PCommand, P2PConfig, P2PError, P2PServer};
use crate::saga::{PalletSaga, SagaError};
use crate::transaction::Transaction;
use crate::wallet::Wallet;
use anyhow;
use axum::{
    body::Body,
    extract::{Path as AxumPath, State, State as MiddlewareState},
    http::{Request as HttpRequest, StatusCode},
    middleware::{self, Next},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use governor::clock::QuantaClock;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter}; // FIX: Use Quota explicitly
use libp2p::identity;
use libp2p::PeerId;
use nonzero_ext::nonzero;
use regex::Regex;
use rocksdb::{Options, DB};
use serde::{Deserialize, Serialize};
use sp_core::H256;
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::fs;
use tokio::signal;
use tokio::sync::{mpsc, RwLock};
use tokio::task::{JoinError, JoinSet};
use tokio::time::timeout;
use tracing::{debug, error, info, instrument, warn};

// Import ISNM components when the 'infinite-strata' feature is enabled.
#[cfg(feature = "infinite-strata")]
use crate::infinite_strata_node::{InfiniteStrataNode, MIN_UPTIME_HEARTBEAT_SECS};

// Constants defining node limits and validation rules.
const MAX_UTXOS: usize = 1_000_000;
const MAX_PROPOSALS: usize = 10_000;
const ADDRESS_REGEX: &str = r"^[0-9a-fA-F]{64}$";
const MAX_SYNC_AGE_SECONDS: u64 = 3600; // 1 hour
const DEFAULT_MINING_INTERVAL_SECS: u64 = 15; // Time between mining attempts

// Comprehensive error enumeration for the node.
#[derive(Error, Debug)]
pub enum NodeError {
    #[error("DAG error: {0}")]
    DAG(String),
    #[error("P2P error: {0}")]
    P2P(String),
    #[error("Mempool error: {0}")]
    Mempool(String),
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),
    #[error("Wallet error: {0}")]
    Wallet(#[from] crate::wallet::WalletError),
    #[error("Mining error: {0}")]
    Mining(#[from] MiningError),
    #[error("Timeout error: {0}")]
    Timeout(String),
    #[error("Anyhow error: {0}")]
    Anyhow(#[from] anyhow::Error),
    #[error("Server execution error: {0}")]
    ServerExecution(String),
    #[error("Task join error: {0}")]
    Join(#[from] JoinError),
    #[error("P2P specific error: {0}")]
    P2PSpecific(#[from] P2PError),
    #[error("P2P Identity error: {0}")]
    P2PIdentity(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("HyperDAG error: {0}")]
    HyperDAG(#[from] HyperDAGError),
    #[error("Sync error: {0}")]
    SyncError(String),
    #[error("Database error: {0}")]
    Database(#[from] rocksdb::Error),
    #[error("Node initialization error: {0}")]
    NodeInitialization(String),
}

// Convert specific errors to the top-level NodeError
impl From<NodeError> for String {
    fn from(err: NodeError) -> Self {
        err.to_string()
    }
}

// Structs for API responses, providing a clear view of the node's state.
#[derive(Serialize, Debug)]
struct DagInfo {
    block_count: usize,
    tip_count: usize,
    difficulty: u64,
    target_block_time: u64,
    validator_count: usize,
    num_chains: u32,
    latest_block_timestamp: u64,
}

#[derive(Serialize, Debug)]
struct PublishReadiness {
    is_ready: bool,
    block_count: usize,
    utxo_count: usize,
    peer_count: usize,
    mempool_size: usize,
    is_synced: bool,
    issues: Vec<String>,
}

// Peer cache for persisting known peers between sessions.
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct PeerCache {
    pub peers: Vec<String>,
}

// The main Node struct, holding the state of all core components.
pub struct Node {
    _config_path: String,
    config: Config,
    p2p_identity_keypair: identity::Keypair,
    pub dag: Arc<HyperDAG>,
    pub miner: Arc<Miner>,
    wallet: Arc<Wallet>,
    pub mempool: Arc<RwLock<Mempool>>,
    pub utxos: Arc<RwLock<HashMap<String, UTXO>>>,
    pub proposals: Arc<RwLock<Vec<HyperBlock>>>,
    peer_cache_path: String,
    pub saga_pallet: Arc<PalletSaga>,
    // The ISNM service, conditionally compiled with the 'infinite-strata' feature.
    #[cfg(feature = "infinite-strata")]
    isnm_service: Arc<InfiniteStrataNode>,
}

type DirectApiRateLimiter = RateLimiter<NotKeyed, InMemoryState, QuantaClock>;

impl Node {
    #[instrument(skip(config, wallet))]
    pub async fn new(
        mut config: Config,
        config_path: String,
        wallet: Arc<Wallet>,
        p2p_identity_path: &str,
        peer_cache_path: String,
    ) -> Result<Self, NodeError> {
        config.validate()?;

        // --- P2P Identity Initialization ---
        let local_keypair = match fs::read(p2p_identity_path).await {
            Ok(key_bytes) => {
                info!("Loading P2P identity from file: {p2p_identity_path}");
                identity::Keypair::from_protobuf_encoding(&key_bytes).map_err(|e| {
                    NodeError::P2PIdentity(format!("Failed to decode P2P identity key: {e}"))
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                info!(
                    "P2P identity key file not found at {p2p_identity_path}, generating a new one."
                );
                if let Some(p) = Path::new(p2p_identity_path).parent() {
                    fs::create_dir_all(p).await?;
                }
                let new_key = identity::Keypair::generate_ed25519();
                let new_key_bytes = new_key.to_protobuf_encoding().map_err(|e| {
                    NodeError::P2PIdentity(format!("Failed to encode P2P key: {e:?}"))
                })?;
                fs::write(p2p_identity_path, new_key_bytes).await?;
                info!("New P2P identity key saved to {p2p_identity_path}");
                Ok(new_key)
            }
            Err(e) => Err(NodeError::P2PIdentity(format!(
                "Failed to read P2P identity key file '{p2p_identity_path}': {e}"
            ))),
        }?;
        let local_peer_id = PeerId::from(local_keypair.public());
        info!("Node Local P2P Peer ID: {local_peer_id}");
        let full_local_p2p_address = format!("{}/p2p/{}", config.p2p_address, local_peer_id);
        if config.local_full_p2p_address.as_deref() != Some(&full_local_p2p_address) {
            info!(
                "Updating config file '{config_path}' with local full P2P address: {full_local_p2p_address}"
            );
            config.local_full_p2p_address = Some(full_local_p2p_address.clone());
            config.save(&config_path)?;
        }
        // --- End P2P Identity Initialization ---

        let initial_validator = wallet.address().trim().to_lowercase();
        if initial_validator != config.genesis_validator.trim().to_lowercase() {
            return Err(NodeError::Config(ConfigError::Validation(
                "Wallet address does not match genesis validator".to_string(),
            )));
        }

        let signing_key_dalek = wallet.get_signing_key()?;
        let node_signing_key_bytes = signing_key_dalek.to_bytes().to_vec();

        info!("Initializing SAGA and dependent services...");

        // --- FIX: Correctly initialize ISNM service and SAGA pallet ---
        // Conditionally initialize the ISNM service.
        #[cfg(feature = "infinite-strata")]
        let isnm_service = {
            info!("[ISNM] Infinite Strata feature enabled, initializing service.");
            Arc::new(InfiniteStrataNode::new())
        };

        // Initialize the SAGA pallet, conditionally passing the ISNM service.
        // This corrected logic resolves the build error.
        let saga_pallet = {
            #[cfg(feature = "infinite-strata")]
            {
                Arc::new(PalletSaga::new(Some(isnm_service.clone())))
            }
            #[cfg(not(feature = "infinite-strata"))]
            {
                Arc::new(PalletSaga::new())
            }
        };
        // --- End FIX ---

        info!("Initializing HyperDAG (loading database)...");
        let db = {
            let mut opts = Options::default();
            opts.create_if_missing(true);
            DB::open(&opts, "hyperdag_db_evolved")?
        };

        let dag_arc = HyperDAG::new(
            &initial_validator,
            config.target_block_time,
            config.difficulty,
            config.num_chains,
            &node_signing_key_bytes,
            saga_pallet.clone(),
            db,
        )
        .map_err(|e| NodeError::DAG(e.to_string()))?;

        info!("HyperDAG initialized.");

        let mempool = Arc::new(RwLock::new(Mempool::new(3600, 10_000_000, 10_000)));
        let utxos = Arc::new(RwLock::new(HashMap::with_capacity(MAX_UTXOS)));
        let proposals = Arc::new(RwLock::new(Vec::with_capacity(MAX_PROPOSALS)));

        // --- Genesis UTXO initialization ---
        {
            let mut utxos_lock = utxos.write().await;
            for chain_id_val in 0..config.num_chains {
                let genesis_id_convention =
                    format!("genesis_placeholder_tx_id_for_chain_{chain_id_val}");
                let utxo_id = format!("genesis_utxo_for_chain_{chain_id_val}");
                utxos_lock.insert(
                    utxo_id.clone(),
                    UTXO {
                        address: initial_validator.clone(),
                        amount: 100,
                        tx_id: genesis_id_convention,
                        output_index: 0,
                        explorer_link: format!(
                            "[https://hyperblockexplorer.org/utxo/](https://hyperblockexplorer.org/utxo/){utxo_id}"
                        ),
                    },
                );
            }
        }
        // --- End Genesis UTXO Initialization ---

        let miner_config = MinerConfig {
            address: wallet.address(),
            dag: dag_arc.clone(),
            difficulty_hex: format!("{:x}", config.difficulty),
            target_block_time: config.target_block_time,
            use_gpu: config.use_gpu,
            zk_enabled: config.zk_enabled,
            threads: config.mining_threads,
            num_chains: config.num_chains,
        };
        let miner_instance = Miner::new(miner_config)?;
        let miner = Arc::new(miner_instance);

        Ok(Self {
            _config_path: config_path,
            config: config.clone(),
            p2p_identity_keypair: local_keypair,
            dag: dag_arc,
            miner,
            wallet,
            mempool,
            utxos,
            proposals,
            peer_cache_path,
            saga_pallet,
            // Add the isnm_service field to the initializer, gated by the feature flag.
            #[cfg(feature = "infinite-strata")]
            isnm_service,
        })
    }

    pub async fn start(&self) -> Result<(), NodeError> {
        let (tx_p2p_commands, mut rx_p2p_commands) = mpsc::channel::<P2PCommand>(100);
        let mut join_set: JoinSet<Result<(), NodeError>> = JoinSet::new();

        // --- ISNM Periodic Check Task ---
        // If the feature is enabled, spawn a background task to run the ISNM heartbeat.
        #[cfg(feature = "infinite-strata")]
        {
            info!("[ISNM] Spawning periodic cloud presence check task.");
            let isnm_service_clone = self.isnm_service.clone();
            join_set.spawn(async move {
                // Give a small delay at startup to avoid resource contention.
                tokio::time::sleep(Duration::from_secs(10)).await;
                let mut interval =
                    tokio::time::interval(Duration::from_secs(MIN_UPTIME_HEARTBEAT_SECS));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

                loop {
                    interval.tick().await;
                    debug!("[ISNM] Running periodic cloud presence check...");
                    if let Err(e) = isnm_service_clone.run_periodic_check().await {
                        warn!("[ISNM] Periodic check failed: {}", e);
                    }
                }
                #[allow(unreachable_code)]
                Ok(())
            });
        }
        // --- End ISNM Task ---

        // --- Command Processor Task ---
        let command_processor_task = {
            let dag_clone = self.dag.clone();
            let mempool_clone = self.mempool.clone();
            let utxos_clone = self.utxos.clone();
            let p2p_tx_clone = tx_p2p_commands.clone();
            let saga_clone = self.saga_pallet.clone();

            async move {
                while let Some(command) = rx_p2p_commands.recv().await {
                    match command {
                        P2PCommand::BroadcastBlock(block) => {
                            info!("\n{}", block);
                            let add_result = dag_clone.add_block(block, &utxos_clone).await;

                            if matches!(add_result, Ok(true)) {
                                debug!("Running periodic maintenance after adding new block.");
                                dag_clone.run_periodic_maintenance().await;
                            } else if let Err(e) = add_result {
                                warn!("Block failed validation or processing: {}", e);
                            }
                        }
                        P2PCommand::BroadcastTransaction(tx) => {
                            debug!("Command processor received transaction {}", tx.id);
                            let mempool_writer = mempool_clone.write().await;
                            let utxos_reader = utxos_clone.read().await;
                            if let Err(e) = mempool_writer
                                .add_transaction(tx, &utxos_reader, &dag_clone)
                                .await
                            {
                                warn!("Failed to add transaction to mempool: {}", e);
                            }
                        }
                        P2PCommand::SyncResponse { blocks, utxos } => {
                            info!(
                                "Received state sync response with {} blocks and {} UTXOs.",
                                blocks.len(),
                                utxos.len()
                            );
                            {
                                let mut utxos_writer = utxos_clone.write().await;
                                let initial_utxo_count = utxos_writer.len();
                                utxos_writer.extend(utxos);
                                info!(
                                    "UTXO set updated with {} new entries from sync.",
                                    utxos_writer.len() - initial_utxo_count
                                );
                            }
                            if let Ok(sorted_blocks) =
                                Self::topological_sort_blocks(blocks, &dag_clone).await
                            {
                                let mut added_count = 0;
                                let mut failed_count = 0;
                                for b in sorted_blocks {
                                    match dag_clone.add_block(b, &utxos_clone).await {
                                        Ok(true) => added_count += 1,
                                        Ok(false) => { /* block already exists */ }
                                        Err(e) => {
                                            warn!("Failed to add block from sync response: {}", e);
                                            failed_count += 1;
                                        }
                                    }
                                }
                                info!(
                                    "Block sync complete. Added: {}, Failed: {}.",
                                    added_count, failed_count
                                );
                                dag_clone.run_periodic_maintenance().await;
                            } else {
                                error!(
                                    "Failed to topologically sort blocks from sync response. Discarding batch."
                                );
                            }
                        }
                        P2PCommand::RequestBlock { block_id, peer_id } => {
                            info!(
                                "Received request for block {} from peer {}",
                                block_id, peer_id
                            );
                            let blocks_reader = dag_clone.blocks.read().await;
                            if let Some(block) = blocks_reader.get(&block_id) {
                                info!("Found block {}, sending to peer {}", block_id, peer_id);
                                let cmd = P2PCommand::SendBlockToOnePeer {
                                    peer_id,
                                    block: Box::new(block.clone()),
                                };
                                if let Err(e) = p2p_tx_clone.send(cmd).await {
                                    error!("Failed to send SendBlockToOnePeer command: {}", e);
                                }
                            } else {
                                warn!(
                                    "Peer {} requested block {} which we don't have.",
                                    peer_id, block_id
                                );
                            }
                        }
                        P2PCommand::BroadcastCarbonCredential(cred) => {
                            if let Err(e) =
                                saga_clone.verify_and_store_credential(cred.clone()).await
                            {
                                warn!(cred_id=%cred.id, "Received invalid CarbonOffsetCredential from network: {}", e);
                            } else {
                                info!(cred_id=%cred.id, "Successfully verified and stored CarbonOffsetCredential from network.");
                            }
                        }
                        _ => {}
                    }
                }
                Ok(())
            }
        };
        join_set.spawn(command_processor_task);
        // --- End Command Processor Task ---

        // --- P2P Server Task ---
        if !self.config.peers.is_empty() {
            info!("Peers detected in config, initializing P2P server task...");
            let p2p_dag_clone = self.dag.clone();
            let p2p_mempool_clone = self.mempool.clone();
            let p2p_utxos_clone = self.utxos.clone();
            let p2p_proposals_clone = self.proposals.clone();
            let p2p_command_sender_clone = tx_p2p_commands.clone();
            let p2p_identity_keypair_clone = self.p2p_identity_keypair.clone();
            let p2p_listen_address_config_clone = self.config.p2p_address.clone();
            let p2p_initial_peers_config_clone = self.config.peers.clone();
            let p2p_settings_clone = self.config.p2p.clone();
            let node_signing_key_bytes_for_p2p = self.wallet.get_signing_key()?.to_bytes().to_vec();
            let peer_cache_path_clone = self.peer_cache_path.clone();
            let network_id_clone = self.config.network_id.clone();
            let p2p_task_fut = async move {
                let mut attempts = 0;
                let mut p2p_server = loop {
                    let p2p_config = P2PConfig {
                        topic_prefix: &network_id_clone,
                        listen_addresses: vec![p2p_listen_address_config_clone.clone()],
                        initial_peers: p2p_initial_peers_config_clone.clone(),
                        dag: p2p_dag_clone.clone(),
                        mempool: p2p_mempool_clone.clone(),
                        utxos: p2p_utxos_clone.clone(),
                        proposals: p2p_proposals_clone.clone(),
                        local_keypair: p2p_identity_keypair_clone.clone(),
                        p2p_settings: p2p_settings_clone.clone(),
                        node_signing_key_material: &node_signing_key_bytes_for_p2p,
                        peer_cache_path: peer_cache_path_clone.clone(),
                    };
                    info!(
                        "Attempting to initialize P2P server (attempt {})...",
                        attempts + 1
                    );
                    match timeout(
                        Duration::from_secs(15),
                        P2PServer::new(p2p_config, p2p_command_sender_clone.clone()),
                    )
                    .await
                    {
                        Ok(Ok(server)) => {
                            info!("P2P server initialized successfully.");
                            break server;
                        }
                        Ok(Err(e)) => {
                            warn!("P2P server failed to initialize: {e}.");
                        }
                        Err(_) => {
                            warn!("P2P server initialization timed out.");
                        }
                    }
                    attempts += 1;
                    let backoff_duration = Duration::from_secs(2u64.pow(attempts.min(6)));
                    warn!("Retrying P2P initialization in {:?}", backoff_duration);
                    tokio::time::sleep(backoff_duration).await;
                };
                if !p2p_initial_peers_config_clone.is_empty() {
                    if let Err(e) = p2p_command_sender_clone
                        .send(P2PCommand::RequestState)
                        .await
                    {
                        error!("Failed to send initial RequestState P2P command: {e}");
                    }
                }
                let (_p2p_tx, p2p_rx) = mpsc::channel::<P2PCommand>(100);
                p2p_server.run(p2p_rx).await.map_err(NodeError::P2PSpecific)
            };
            join_set.spawn(p2p_task_fut);
        } else {
            info!("No peers found. Running in single-node mode. Spawning solo miner...");
            let miner_dag_clone = self.dag.clone();
            let miner_wallet_clone = self.wallet.clone();
            let miner_mempool_clone = self.mempool.clone();
            let miner_utxos_clone = self.utxos.clone();
            let miner_clone = self.miner.clone();
            join_set.spawn(async move {
                miner_dag_clone
                    .run_solo_miner(
                        miner_wallet_clone,
                        miner_mempool_clone,
                        miner_utxos_clone,
                        miner_clone,
                        DEFAULT_MINING_INTERVAL_SECS,
                    )
                    .await;
                Ok(())
            });
        }
        // --- End P2P Server Task ---

        // --- API Server Task ---
        let server_task_fut = {
            let app_state = AppState {
                dag: self.dag.clone(),
                mempool: self.mempool.clone(),
                utxos: self.utxos.clone(),
                api_address: self.config.api_address.clone(),
                p2p_command_sender: tx_p2p_commands.clone(),
                saga: self.saga_pallet.clone(),
            };
            async move {
                let rate_limiter: Arc<DirectApiRateLimiter> =
                    // FIX: Construct Quota explicitly
                    Arc::new(RateLimiter::direct(Quota::per_second(nonzero!(50u32))));
                let app = Router::new()
                    .route("/info", get(info_handler))
                    .route("/balance/:address", get(get_balance))
                    .route("/utxos/:address", get(get_utxos))
                    .route("/transaction", post(submit_transaction))
                    .route("/block/:id", get(get_block))
                    .route("/dag", get(get_dag))
                    .route("/health", get(health_check))
                    .route("/mempool", get(mempool_handler))
                    .route("/publish-readiness", get(publish_readiness_handler))
                    .route("/saga/ask", post(ask_saga))
                    .layer(middleware::from_fn_with_state(
                        rate_limiter,
                        rate_limit_layer,
                    ))
                    .with_state(app_state.clone());
                let addr: SocketAddr = app_state.api_address.parse().map_err(|e| {
                    NodeError::Config(ConfigError::Validation(format!("Invalid API address: {e}")))
                })?;
                let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
                    NodeError::ServerExecution(format!("Failed to bind to API address {addr}: {e}"))
                })?;
                info!("API server listening on {}", listener.local_addr().unwrap());
                if let Err(e) = axum::serve(listener, app.into_make_service()).await {
                    error!("API server failed: {e}");
                    return Err(NodeError::ServerExecution(format!(
                        "API server failed: {e}"
                    )));
                }
                Ok(())
            }
        };
        join_set.spawn(server_task_fut);
        // --- End API Server Task ---

        // --- Graceful Shutdown Logic ---
        tokio::select! {
            biased;
            _ = signal::ctrl_c() => {
                info!("Received Ctrl+C, initiating shutdown...");
            },
            Some(res) = join_set.join_next() => {
                match res {
                    Ok(Err(e)) => error!("A critical node task failed: {}", e),
                    Err(e) => error!("A critical node task panicked: {}", e),
                    _ => {}
                }
            },
        }
        join_set.shutdown().await;
        info!("Node shutdown complete.");
        Ok(())
    }

    // --- Topological Sort Helper ---
    async fn topological_sort_blocks(
        blocks: Vec<HyperBlock>,
        dag: &HyperDAG,
    ) -> Result<Vec<HyperBlock>, NodeError> {
        if blocks.is_empty() {
            return Ok(vec![]);
        }
        let block_map: HashMap<String, HyperBlock> =
            blocks.into_iter().map(|b| (b.id.clone(), b)).collect();
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut children_map: HashMap<String, Vec<String>> = HashMap::new();
        let local_blocks = dag.blocks.read().await;
        for (id, block) in &block_map {
            let mut degree = 0;
            for parent_id in &block.parents {
                if block_map.contains_key(parent_id) {
                    degree += 1;
                    children_map
                        .entry(parent_id.clone())
                        .or_default()
                        .push(id.clone());
                } else if !local_blocks.contains_key(parent_id) {
                    warn!(
                        "Sync Error: Block {} has a missing parent {} that is not in the local DAG or the sync batch.",
                        id, parent_id
                    );
                    return Err(NodeError::SyncError(format!(
                        "Missing parent {parent_id} for block {id} in sync batch"
                    )));
                }
            }
            in_degree.insert(id.clone(), degree);
        }
        let mut queue: VecDeque<String> = in_degree
            .iter()
            .filter(|(_, &degree)| degree == 0)
            .map(|(id, _)| id.clone())
            .collect();
        let mut sorted_blocks = Vec::with_capacity(block_map.len());
        while let Some(id) = queue.pop_front() {
            if let Some(children) = children_map.get(&id) {
                for child_id in children {
                    if let Some(degree) = in_degree.get_mut(child_id) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push_back(child_id.clone());
                        }
                    }
                }
            }
            sorted_blocks.push(block_map.get(&id).unwrap().clone());
        }
        if sorted_blocks.len() != block_map.len() {
            error!(
                "Cycle detected in block sync batch or missing parent. Sorted {} of {} blocks.",
                sorted_blocks.len(),
                block_map.len()
            );
            Err(NodeError::SyncError(
                "Cycle detected or missing parent in block sync batch.".to_string(),
            ))
        } else {
            Ok(sorted_blocks)
        }
    }
}

// --- Rate Limiting Layer ---
async fn rate_limit_layer(
    MiddlewareState(limiter): MiddlewareState<Arc<DirectApiRateLimiter>>,
    req: HttpRequest<Body>,
    next: Next,
) -> Result<axum::response::Response, StatusCode> {
    if limiter.check().is_err() {
        warn!("API rate limit exceeded for: {:?}", req.uri());
        Err(StatusCode::TOO_MANY_REQUESTS)
    } else {
        Ok(next.run(req).await)
    }
}

// The AppState for the API server.
#[derive(Clone)]
struct AppState {
    dag: Arc<HyperDAG>,
    mempool: Arc<RwLock<Mempool>>,
    utxos: Arc<RwLock<HashMap<String, UTXO>>>,
    api_address: String,
    p2p_command_sender: mpsc::Sender<P2PCommand>,
    saga: Arc<PalletSaga>,
}

#[derive(Deserialize)]
struct SagaQuery {
    query: String,
}
#[derive(Serialize)]
struct ApiError {
    code: u16,
    message: String,
    details: Option<String>,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let status = StatusCode::from_u16(self.code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(self)).into_response()
    }
}

// --- API Handlers ---

async fn ask_saga(
    State(state): State<AppState>,
    Json(payload): Json<SagaQuery>,
) -> Result<Json<String>, ApiError> {
    let saga = &state.saga;
    let network_state = *saga.economy.network_state.read().await;
    let threat_level = crate::omega::get_threat_level().await;
    let proactive_insight = saga
        .economy
        .proactive_insights
        .read()
        .await
        .first()
        .cloned();
    match saga
        .guidance_system
        .get_guidance_response(
            &payload.query,
            network_state,
            threat_level,
            proactive_insight.as_ref(),
        )
        .await
    {
        Ok(response) => Ok(Json(response)),
        Err(SagaError::AmbiguousQuery(topics)) => {
            let error_message = format!(
                "SAGA query is too ambiguous. Please be more specific. Possible topics: {topics:?}"
            );
            error!("{}", error_message);
            Err(ApiError {
                code: 400,
                message: "Ambiguous query".to_string(),
                details: Some(error_message),
            })
        }
        Err(e) => {
            warn!("SAGA guidance query failed: {}", e);
            Err(ApiError {
                code: 500,
                message: "Internal SAGA error".to_string(),
                details: None,
            })
        }
    }
}

async fn info_handler(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mempool_read_guard = state.mempool.read().await;
    let utxos_read_guard = state.utxos.read().await;
    let tips_read_guard = state.dag.tips.read().await;
    let blocks_read_guard = state.dag.blocks.read().await;
    let num_chains_val = *state.dag.num_chains.read().await;
    Ok(Json(serde_json::json!({
        "block_count": blocks_read_guard.len(),
        "tip_count": tips_read_guard.values().map(|t_set| t_set.len()).sum::<usize>(),
        "mempool_size": mempool_read_guard.size().await,
        "utxo_count": utxos_read_guard.len(),
        "num_chains": num_chains_val,
    })))
}

async fn mempool_handler(
    State(state): State<AppState>,
) -> Result<Json<HashMap<String, Transaction>>, StatusCode> {
    let mempool_read_guard = state.mempool.read().await;
    Ok(Json(mempool_read_guard.get_transactions().await))
}

async fn publish_readiness_handler(
    State(state): State<AppState>,
) -> Result<Json<PublishReadiness>, StatusCode> {
    let mempool_read_guard = state.mempool.read().await;
    let utxos_read_guard = state.utxos.read().await;
    let blocks_read_guard = state.dag.blocks.read().await;
    let mut issues = vec![];
    if blocks_read_guard.len() < 2 {
        issues.push("Insufficient blocks in DAG".to_string());
    }
    if utxos_read_guard.is_empty() {
        issues.push("No UTXOs available".to_string());
    }
    let latest_timestamp = blocks_read_guard
        .values()
        .map(|b| b.timestamp)
        .max()
        .unwrap_or(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let is_synced = now.saturating_sub(latest_timestamp) < MAX_SYNC_AGE_SECONDS;
    if !is_synced {
        issues.push(format!(
            "Node is out of sync. Last block is {} seconds old.",
            now.saturating_sub(latest_timestamp)
        ));
    }
    let is_ready = issues.is_empty();
    Ok(Json(PublishReadiness {
        is_ready,
        block_count: blocks_read_guard.len(),
        utxo_count: utxos_read_guard.len(),
        peer_count: 0,
        mempool_size: mempool_read_guard.size().await,
        is_synced,
        issues,
    }))
}

async fn get_balance(
    State(state): State<AppState>,
    AxumPath(address): AxumPath<String>,
) -> Result<Json<u64>, ApiError> {
    if !Regex::new(ADDRESS_REGEX).unwrap().is_match(&address) {
        warn!("Invalid address format for balance check: {address}");
        return Err(ApiError {
            code: 400,
            message: "Invalid address format".to_string(),
            details: None,
        });
    }
    let utxos_read_guard = state.utxos.read().await;
    let balance = utxos_read_guard
        .values()
        .filter(|utxo_item| utxo_item.address == address)
        .map(|utxo_item| utxo_item.amount)
        .sum();
    Ok(Json(balance))
}

async fn get_utxos(
    State(state): State<AppState>,
    AxumPath(address): AxumPath<String>,
) -> Result<Json<HashMap<String, UTXO>>, ApiError> {
    if !Regex::new(ADDRESS_REGEX).unwrap().is_match(&address) {
        warn!("Invalid address format for UTXO fetch: {address}");
        return Err(ApiError {
            code: 400,
            message: "Invalid address format".to_string(),
            details: None,
        });
    }
    let utxos_read_guard = state.utxos.read().await;
    let filtered_utxos_map = utxos_read_guard
        .iter()
        .filter(|(_, utxo_item)| utxo_item.address == address)
        .map(|(key_str, value_utxo)| (key_str.clone(), value_utxo.clone()))
        .collect();
    Ok(Json(filtered_utxos_map))
}

async fn submit_transaction(
    State(state): State<AppState>,
    Json(tx_data): Json<Transaction>,
) -> Result<Json<String>, ApiError> {
    let tx_hash_bytes = match hex::decode(&tx_data.id) {
        Ok(bytes) => bytes,
        Err(_) => {
            return Err(ApiError {
                code: 400,
                message: "Invalid transaction ID format".to_string(),
                details: None,
            })
        }
    };
    let tx_hash = H256::from_slice(&tx_hash_bytes);
    if !reflect_on_action(tx_hash).await {
        error!("ΛΣ-ΩMEGA rejected transaction {}", tx_data.id);
        return Err(ApiError {
            code: 503,
            message: "System unstable, transaction rejected".to_string(),
            details: None,
        });
    }
    let utxos_read_guard = state.utxos.read().await;
    if let Err(e) = tx_data.verify(&state.dag, &utxos_read_guard).await {
        warn!(
            "Transaction {} failed verification via API: {}",
            tx_data.id, e
        );
        return Err(ApiError {
            code: 400,
            message: "Transaction verification failed".to_string(),
            details: Some(e.to_string()),
        });
    }
    let tx_id = tx_data.id.clone();
    if let Err(e) = state
        .p2p_command_sender
        .send(P2PCommand::BroadcastTransaction(tx_data))
        .await
    {
        error!(
            "Failed to broadcast transaction {} to P2P task: {}",
            tx_id, e
        );
        return Err(ApiError {
            code: 500,
            message: "Internal server error".to_string(),
            details: None,
        });
    }
    info!("Transaction {} submitted via API", tx_id);
    Ok(Json(tx_id))
}

async fn get_block(
    State(state): State<AppState>,
    AxumPath(id_str): AxumPath<String>,
) -> Result<Json<HyperBlock>, StatusCode> {
    if id_str.len() > 128 || id_str.is_empty() {
        warn!("Invalid block ID length: {id_str}");
        return Err(StatusCode::BAD_REQUEST);
    }
    let blocks_read_guard = state.dag.blocks.read().await;
    let block_data = blocks_read_guard
        .get(&id_str)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(block_data))
}

async fn get_dag(State(state): State<AppState>) -> Result<Json<DagInfo>, StatusCode> {
    let blocks_read_guard = state.dag.blocks.read().await;
    let tips_read_guard = state.dag.tips.read().await;
    let validators_read_guard = state.dag.validators.read().await;
    let difficulty_val = *state.dag.difficulty.read().await;
    let num_chains_val = *state.dag.num_chains.read().await;
    let latest_block_timestamp = blocks_read_guard
        .values()
        .map(|b| b.timestamp)
        .max()
        .unwrap_or(0);
    Ok(Json(DagInfo {
        block_count: blocks_read_guard.len(),
        tip_count: tips_read_guard.values().map(|t_set| t_set.len()).sum(),
        difficulty: difficulty_val,
        target_block_time: state.dag.target_block_time,
        validator_count: validators_read_guard.len(),
        num_chains: num_chains_val,
        latest_block_timestamp,
    }))
}

async fn health_check() -> Result<Json<serde_json::Value>, StatusCode> {
    Ok(Json(serde_json::json!({ "status": "healthy" })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{LoggingConfig, P2pConfig};
    use crate::wallet::Wallet;
    use rand::Rng; // Import Rng trait
    use serial_test::serial;
    use std::fs as std_fs;

    #[tokio::test]
    #[serial]
    async fn test_node_creation_and_config_save() {
        let db_path = "hyperdag_db_test_node_creation";
        if std::path::Path::new(db_path).exists() {
            std_fs::remove_dir_all(db_path).unwrap();
        }
        let _ = tracing_subscriber::fmt::try_init();
        let wallet = Wallet::new().expect("Failed to create new wallet for test");
        let wallet_arc = Arc::new(wallet);
        let genesis_validator_addr = wallet_arc.address();
        let rand_id: u32 = rand::thread_rng().gen(); // Use thread_rng()
        let temp_config_path = format!("./temp_test_config_{rand_id}.toml");
        let temp_identity_path = format!("./temp_p2p_identity_{rand_id}.key");
        let temp_peer_cache_path = format!("./temp_peer_cache_{rand_id}.json");
        let test_config = Config {
            p2p_address: "/ip4/127.0.0.1/tcp/0".to_string(),
            local_full_p2p_address: None,
            api_address: "127.0.0.1:0".to_string(),
            peers: vec![],
            genesis_validator: genesis_validator_addr.clone(),
            target_block_time: 60,
            difficulty: 1, // Lower difficulty for faster testing
            max_amount: 10_000_000_000,
            use_gpu: false,
            zk_enabled: false,
            mining_threads: 1,
            num_chains: 1,
            logging: LoggingConfig {
                level: "debug".to_string(),
            },
            p2p: P2pConfig::default(),
            network_id: "testnet".to_string(),
        };
        test_config
            .save(&temp_config_path)
            .expect("Failed to save initial temp config for test");

        let node_instance_result = Node::new(
            test_config,
            temp_config_path.clone(),
            wallet_arc.clone(),
            &temp_identity_path,
            temp_peer_cache_path.clone(),
        )
        .await;

        if std::path::Path::new(db_path).exists() {
            std_fs::remove_dir_all(db_path).unwrap();
        }

        assert!(
            node_instance_result.is_ok(),
            "Node::new failed: {:?}",
            node_instance_result.err()
        );

        let _ = std_fs::remove_file(&temp_config_path);
        let _ = std_fs::remove_file(&temp_identity_path);
        let _ = std_fs::remove_file(&temp_peer_cache_path);
    }
}
