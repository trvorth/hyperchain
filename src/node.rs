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
use governor::{Quota, RateLimiter};
use libp2p::identity;
use libp2p::PeerId;
use nonzero_ext::nonzero;
use regex::Regex;
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
use tokio::task::{self, JoinError, JoinSet};
use tokio::time::{self, timeout};
use tracing::{debug, error, info, instrument, warn};

const MAX_UTXOS: usize = 1_000_000;
const MAX_PROPOSALS: usize = 10_000;
const ADDRESS_REGEX: &str = r"^[0-9a-fA-F]{64}$";
const MAX_SYNC_AGE_SECONDS: u64 = 3600; // 1 hour

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
}

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

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct PeerCache {
    pub peers: Vec<String>,
}

pub struct Node {
    _config_path: String,
    config: Config,
    p2p_identity_keypair: identity::Keypair,
    pub dag: Arc<RwLock<HyperDAG>>,
    pub miner: Arc<Miner>,
    wallet: Arc<Wallet>,
    pub mempool: Arc<RwLock<Mempool>>,
    pub utxos: Arc<RwLock<HashMap<String, UTXO>>>,
    pub proposals: Arc<RwLock<Vec<HyperBlock>>>,
    mining_chain_id: u32,
    peer_cache_path: String,
    pub saga_pallet: Arc<PalletSaga>,
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
            info!("Updating config file '{config_path}' with local full P2P address: {full_local_p2p_address}");
            config.local_full_p2p_address = Some(full_local_p2p_address.clone());
            config.save(&config_path)?;
        }

        let initial_validator = wallet.address().trim().to_lowercase();
        if initial_validator != config.genesis_validator.trim().to_lowercase() {
            return Err(NodeError::Config(ConfigError::Validation(
                "Wallet address does not match genesis validator".to_string(),
            )));
        }

        let signing_key_dalek = wallet.get_signing_key()?;
        let node_signing_key_bytes = signing_key_dalek.to_bytes().to_vec();

        info!("Initializing HyperDAG (loading database)...");
        let saga_pallet = Arc::new(PalletSaga::new());

        let dag_instance = HyperDAG::new(
            &initial_validator,
            config.target_block_time,
            config.difficulty,
            config.num_chains,
            &node_signing_key_bytes,
            saga_pallet.clone(),
        )
        .await
        .map_err(|e| NodeError::DAG(e.to_string()))?;

        let dag_arc = Arc::new(RwLock::new(dag_instance));

        dag_arc.write().await.init_self_arc(dag_arc.clone());

        info!("HyperDAG initialized.");

        task::yield_now().await;
        time::sleep(Duration::from_secs(1)).await;

        let mempool = Arc::new(RwLock::new(Mempool::new(3600, 10_000_000, 10_000)));
        let utxos = Arc::new(RwLock::new(HashMap::with_capacity(MAX_UTXOS)));
        let proposals = Arc::new(RwLock::new(Vec::with_capacity(MAX_PROPOSALS)));

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
                        amount: 100, // Genesis amount
                        tx_id: genesis_id_convention,
                        output_index: 0,
                        explorer_link: format!("[https://hyperblockexplorer.org/utxo/](https://hyperblockexplorer.org/utxo/){utxo_id}"),
                    },
                );
            }
        }

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
            mining_chain_id: config.mining_chain_id,
            peer_cache_path,
            saga_pallet,
        })
    }

    pub fn api_address(&self) -> &str {
        &self.config.api_address
    }

    pub async fn start(&self) -> Result<(), NodeError> {
        let (tx_p2p_commands, mut rx_p2p_commands) = mpsc::channel::<P2PCommand>(100);
        let mut join_set: JoinSet<Result<(), NodeError>> = JoinSet::new();

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
                            debug!("Command processor received block {}", block.id);
                            // This atomic operation prevents race conditions between adding a block and updating DAG state.
                            let mut dag = dag_clone.write().await;
                            let add_result = dag.add_block(block, &utxos_clone).await;

                            if matches!(add_result, Ok(true)) {
                                info!("Successfully added new block to DAG. Running maintenance.");
                                if let Err(e) = dag.run_periodic_maintenance().await {
                                    warn!("Periodic maintenance failed after adding block: {}", e);
                                }
                            } else if let Err(e) = add_result {
                                warn!("Block failed validation or processing: {}", e);
                            }
                        }
                        P2PCommand::BroadcastTransaction(tx) => {
                            debug!("Command processor received transaction {}", tx.id);
                            let mempool_writer = mempool_clone.write().await;
                            let dag_reader = dag_clone.read().await;
                            let utxos_reader = utxos_clone.read().await;
                            if let Err(e) = mempool_writer
                                .add_transaction(tx, &utxos_reader, &dag_reader)
                                .await
                            {
                                warn!("Failed to add transaction to mempool: {}", e);
                            }
                        }
                        // This process is broken down into safer, atomic steps to prevent deadlocks and hanging.
                        P2PCommand::SyncResponse { blocks, utxos } => {
                            info!("Received state sync response with {} blocks and {} UTXOs.", blocks.len(), utxos.len());
                            
                            // Step 1: Update UTXO set atomically.
                            {
                                let mut utxos_writer = utxos_clone.write().await;
                                let initial_utxo_count = utxos_writer.len();
                                utxos_writer.extend(utxos);
                                info!("UTXO set updated with {} new entries from sync.", utxos_writer.len() - initial_utxo_count);
                            }

                            // Step 2: Add blocks sequentially.
                            if let Ok(sorted_blocks) = Self::topological_sort_blocks(blocks, &dag_clone).await {
                                let mut added_count = 0;
                                let mut failed_count = 0;
                                for b in sorted_blocks {
                                    // Use a write lock per block to avoid holding the lock for too long.
                                    let mut dag_writer = dag_clone.write().await;
                                    match dag_writer.add_block(b, &utxos_clone).await {
                                        Ok(true) => added_count += 1,
                                        Ok(false) => { /* block already exists */ },
                                        Err(e) => {
                                            warn!("Failed to add block from sync response: {}", e);
                                            failed_count += 1;
                                        }
                                    }
                                }
                                info!("Block sync complete. Added: {}, Failed: {}.", added_count, failed_count);
                            } else {
                                error!("Failed to topologically sort blocks from sync response. Discarding batch.");
                            }
                            
                            // Step 3: Run maintenance after all blocks are added.
                            {
                                let dag_writer = dag_clone.write().await;
                                if let Err(e) = dag_writer.run_periodic_maintenance().await {
                                    warn!("Periodic maintenance failed after state sync: {}", e);
                                }
                            }
                        },
                        P2PCommand::RequestState => {
                            info!("Command processor noted a RequestState command. The P2P layer is responsible for acting on this.");
                        },
                        P2PCommand::RequestBlock { block_id, peer_id } => {
                            info!("Received request for block {} from peer {}", block_id, peer_id);
                            let dag_reader = dag_clone.read().await;
                            let blocks_reader = dag_reader.blocks.read().await;
                            if let Some(block) = blocks_reader.get(&block_id) {
                                info!("Found block {}, sending to peer {}", block_id, peer_id);
                                let cmd = P2PCommand::SendBlockToOnePeer { peer_id, block: Box::new(block.clone()) };
                                if let Err(e) = p2p_tx_clone.send(cmd).await {
                                    error!("Failed to send SendBlockToOnePeer command: {}", e);
                                }
                            } else {
                                warn!("Peer {} requested block {} which we don't have.", peer_id, block_id);
                            }
                        }
                        P2PCommand::SendBlockToOnePeer { .. } => {
                            // This command is outbound, so the processor doesn't handle it directly.
                        }
                        P2PCommand::BroadcastState(_, _) => {
                            warn!("Received unhandled P2PCommand: BroadcastState. This feature is not yet implemented.");
                        }
                        P2PCommand::BroadcastCarbonCredential(cred) => {
                            // When a credential is received from the network, verify it and add it to SAGA's pool for the current epoch.
                            if let Err(e) = saga_clone.verify_and_store_credential(cred.clone()).await {
                                warn!(cred_id=%cred.id, "Received invalid CarbonOffsetCredential from network: {}", e);
                            } else {
                                info!(cred_id=%cred.id, "Successfully verified and stored CarbonOffsetCredential from network.");
                            }
                        }
                    }
                }
                Ok(())
            }
        };
        join_set.spawn(command_processor_task);


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
                        node_signing_key_material: &node_signing_key_bytes_for_p2p,
                        peer_cache_path: peer_cache_path_clone.clone(),
                    };
                    info!("Attempting to initialize P2P server (attempt {})...", attempts + 1);
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
                    let backoff_duration = Duration::from_secs(2u64.pow(attempts.min(6))); // Exponential backoff up to ~1min
                    warn!("Retrying P2P initialization in {:?}", backoff_duration);
                    time::sleep(backoff_duration).await;
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

                p2p_server
                    .run(p2p_rx)
                    .await
                    .map_err(NodeError::P2PSpecific)
            };
            join_set.spawn(p2p_task_fut);
        } else {
            info!("No peers found. Running in single-node mode.");
        }

        let mining_task_fut = {
            let miner_clone = self.miner.clone();
            let dag_clone_miner = self.dag.clone();
            let mempool_clone_miner = self.mempool.clone();
            let utxos_clone_miner = self.utxos.clone();
            let mining_tx_channel = tx_p2p_commands.clone();
            let proposals_clone_miner = self.proposals.clone();
            let wallet_clone_miner = self.wallet.clone();
            let mining_chain_id = self.mining_chain_id;
            let saga_pallet_clone = self.saga_pallet.clone();

            async move {
                loop {
                    time::sleep(Duration::from_millis(500)).await;
                    debug!("MINING_LOOP_START: Preparing to mine a new block.");

                    let is_synced = {
                        let dag_reader = dag_clone_miner.read().await;
                        let blocks_reader = dag_reader.blocks.read().await;
                        let latest_timestamp = blocks_reader.values().map(|b| b.timestamp).max().unwrap_or(0);
                        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                        now.saturating_sub(latest_timestamp) < MAX_SYNC_AGE_SECONDS
                    };

                    if !is_synced {
                        warn!("Node is not synced (latest block is too old). Pausing mining.");
                        time::sleep(Duration::from_secs(30)).await;
                        continue;
                    }

                    mempool_clone_miner.write().await.prune_expired().await;
                    debug!("MINING_LOOP_STEP_1: Mempool pruned.");

                    let candidate_block_result = dag_clone_miner.read().await.create_candidate_block(
                        &wallet_clone_miner.get_signing_key()?.to_bytes(),
                        &wallet_clone_miner.address(),
                        &mempool_clone_miner,
                        &utxos_clone_miner,
                        mining_chain_id
                    ).await;

                    let mut candidate_block = match candidate_block_result {
                        Ok(block) => block,
                        Err(e) => {
                            debug!("Could not create candidate block: {}. Retrying...", e);
                            time::sleep(Duration::from_secs(2)).await;
                            continue;
                        }
                    };

                    debug!(
                        "MINING_LOOP_STEP_2: Candidate block {} created with {} txs.",
                        candidate_block.id,
                        candidate_block.transactions.len()
                    );

                    let mining_result = {
                        let miner_instance = miner_clone.clone();
                        tokio::task::spawn_blocking(move || {
                            miner_instance.solve_pow(&mut candidate_block).map(|_| candidate_block)
                        })
                        .await?
                    };

                    debug!("MINING_LOOP_STEP_3: Mining (PoW) attempt finished.");

                    match mining_result {
                        Ok(mut solved_block) => {
                            let signing_key = wallet_clone_miner.get_signing_key()?;
                            let signing_data = crate::hyperdag::SigningData {
                                parents: &solved_block.parents,
                                transactions: &solved_block.transactions,
                                timestamp: solved_block.timestamp,
                                nonce: solved_block.nonce,
                                difficulty: solved_block.difficulty,
                                validator: &solved_block.validator,
                                miner: &solved_block.miner,
                                chain_id: solved_block.chain_id,
                                merkle_root: &solved_block.merkle_root,
                            };

                            let pre_signature_data = crate::hyperdag::HyperBlock::serialize_for_signing(&signing_data)?;

                            solved_block.lattice_signature = crate::hyperdag::LatticeSignature::sign(
                                &signing_key.to_bytes(),
                                &pre_signature_data
                            )?;

                            if let Err(e) = saga_pallet_clone
                                .evaluate_block_with_saga(&solved_block, &dag_clone_miner)
                                .await
                            {
                                warn!("SAGA evaluation for block {} failed: {}", solved_block.id, e);
                            }

                            let mut proposals_guard = proposals_clone_miner.write().await;
                            if proposals_guard.len() >= MAX_PROPOSALS && !proposals_guard.is_empty()
                            {
                                proposals_guard.remove(0);
                            }
                            proposals_guard.push(solved_block.clone());

                            info!("Mined block {}, broadcasting...", solved_block.id);
                            if let Err(e_send) =
                                mining_tx_channel.send(P2PCommand::BroadcastBlock(solved_block)).await
                            {
                                error!("Failed to send mined block to command channel: {e_send}");
                            }
                        }
                        Err(e) => error!("An error occurred during mining: {e:?}"),
                    }
                    time::sleep(Duration::from_millis(1200)).await;
                }
            }
        };

        let server_task_fut = {
            let app_state = AppState {
                dag: self.dag.clone(),
                mempool: self.mempool.clone(),
                utxos: self.utxos.clone(),
                api_address: self.config.api_address.clone(),
                p2p_command_sender: tx_p2p_commands.clone(),
            };
            async move {
                let rate_limiter_info: Arc<DirectApiRateLimiter> =
                    Arc::new(RateLimiter::direct(Quota::per_second(nonzero!(20u32))));
                let rate_limiter_balance: Arc<DirectApiRateLimiter> =
                    Arc::new(RateLimiter::direct(Quota::per_second(nonzero!(30u32))));
                let rate_limiter_tx: Arc<DirectApiRateLimiter> =
                    Arc::new(RateLimiter::direct(Quota::per_second(nonzero!(15u32))));

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
                        rate_limiter_info.clone(),
                        |s, r, n| rate_limit_layer(s, r, n, "/info"),
                    ))
                    .layer(middleware::from_fn_with_state(
                        rate_limiter_balance.clone(),
                        |s, r, n| rate_limit_layer(s, r, n, "/balance"),
                    ))
                    .layer(middleware::from_fn_with_state(
                        rate_limiter_tx.clone(),
                        |s, r, n| rate_limit_layer(s, r, n, "/transaction"),
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

        let mining_task_wrapper = mining_task_fut;
        join_set.spawn(mining_task_wrapper);
        join_set.spawn(server_task_fut);

        tokio::select! {
            biased;

            _ = signal::ctrl_c() => {
                info!("Received Ctrl+C, initiating shutdown...");
                join_set.shutdown().await;
                info!("Node shutdown complete after Ctrl+C.");
            },
            Some(res) = join_set.join_next() => {
                match res {
                    Ok(task_result_inner) => {
                        if let Err(node_err) = task_result_inner {
                             error!("A critical node task completed with error: {node_err:?}. Shutting down.");
                             join_set.shutdown().await;
                             return Err(node_err);
                        }
                        info!("A critical node task completed successfully. Shutting down remaining tasks.");
                    }
                    Err(join_err) => {
                        error!("A critical node task failed (panicked or cancelled): {join_err:?}. Shutting down.");
                        if !join_err.is_cancelled() {
                            join_set.shutdown().await;
                            return Err(NodeError::Join(join_err));
                        }
                        info!("A task was cancelled, likely as part of shutdown sequence.");
                    }
                }
                join_set.shutdown().await;
                info!("Node shutdown due to task completion or error.");
            },
        }
        info!("Node::start() is exiting.");
        Ok(())
    }

    async fn topological_sort_blocks(blocks: Vec<HyperBlock>, dag_arc: &Arc<RwLock<HyperDAG>>) -> Result<Vec<HyperBlock>, NodeError> {
        if blocks.is_empty() { return Ok(vec![]); }
    
        let block_map: HashMap<String, HyperBlock> = blocks.into_iter().map(|b| (b.id.clone(), b)).collect();
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut children_map: HashMap<String, Vec<String>> = HashMap::new();
        let dag_reader = dag_arc.read().await;
        let local_blocks = dag_reader.blocks.read().await;
    
        for (id, block) in &block_map {
            let mut degree = 0;
            for parent_id in &block.parents {
                if block_map.contains_key(parent_id) {
                    degree += 1;
                    children_map.entry(parent_id.clone()).or_default().push(id.clone());
                } else if !local_blocks.contains_key(parent_id) {
                    warn!("Sync Error: Block {} has a missing parent {} that is not in the local DAG or the sync batch.", id, parent_id);
                    return Err(NodeError::SyncError(format!("Missing parent {parent_id} for block {id} in sync batch")));
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
            error!("Cycle detected in block sync batch or missing parent. Sorted {} of {} blocks.", sorted_blocks.len(), block_map.len());
            Err(NodeError::SyncError(
                "Cycle detected or missing parent in block sync batch.".to_string(),
            ))
        } else {
            Ok(sorted_blocks)
        }
    }
}

async fn rate_limit_layer(
    MiddlewareState(limiter): MiddlewareState<Arc<DirectApiRateLimiter>>,
    req: HttpRequest<Body>,
    next: Next,
    route_name: &str,
) -> Result<axum::response::Response, StatusCode> {
    if limiter.check().is_err() {
        warn!(
            "API rate limit exceeded for {}: {:?}",
            route_name,
            req.uri()
        );
        Err(StatusCode::TOO_MANY_REQUESTS)
    } else {
        Ok(next.run(req).await)
    }
}

#[derive(Clone)]
struct AppState {
    dag: Arc<RwLock<HyperDAG>>,
    mempool: Arc<RwLock<Mempool>>,
    utxos: Arc<RwLock<HashMap<String, UTXO>>>,
    api_address: String,
    p2p_command_sender: mpsc::Sender<P2PCommand>,
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

async fn ask_saga(
    State(state): State<AppState>,
    Json(payload): Json<SagaQuery>,
) -> Result<Json<String>, ApiError> {
    let dag_read = state.dag.read().await;
    let saga = &dag_read.saga;

    let network_state = *saga.economy.network_state.read().await;
    let threat_level = crate::omega::get_threat_level().await;
    let proactive_insight = saga.economy.proactive_insights.read().await.first().cloned();

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
            let error_message = format!("SAGA query is too ambiguous. Please be more specific. Possible topics: {topics:?}");
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
    let dag_read_guard = time::timeout(Duration::from_secs(2), state.dag.read())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mempool_read_guard = state.mempool.read().await;
    let utxos_read_guard = state.utxos.read().await;

    let tips_read_guard = dag_read_guard.tips.read().await;
    let selected_tip = tips_read_guard
        .get(&0)
        .and_then(|tips_set| tips_set.iter().next().cloned())
        .unwrap_or_else(|| "none".to_string());

    let blocks_read_guard = dag_read_guard.blocks.read().await;
    let num_chains_val = *dag_read_guard.num_chains.read().await;

    Ok(Json(serde_json::json!({
        "block_count": blocks_read_guard.len(),
        "tip_count": tips_read_guard.values().map(|t_set| t_set.len()).sum::<usize>(),
        "mempool_size": mempool_read_guard.size().await,
        "mempool_total_fees": mempool_read_guard.total_fees().await,
        "utxo_count": utxos_read_guard.len(),
        "selected_tip": selected_tip,
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
    let dag_read_guard = state.dag.read().await;
    let mempool_read_guard = state.mempool.read().await;
    let utxos_read_guard = state.utxos.read().await;
    let blocks_read_guard = dag_read_guard.blocks.read().await;
    
    let mut issues = vec![];
    if blocks_read_guard.len() < 2 {
        issues.push("Insufficient blocks in DAG".to_string());
    }
    if utxos_read_guard.is_empty() {
        issues.push("No UTXOs available".to_string());
    }

    let latest_timestamp = blocks_read_guard.values().map(|b| b.timestamp).max().unwrap_or(0);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let is_synced = now.saturating_sub(latest_timestamp) < MAX_SYNC_AGE_SECONDS;

    if !is_synced {
        issues.push(format!("Node is out of sync. Last block is {} seconds old.", now.saturating_sub(latest_timestamp)));
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
        return Err(ApiError { code: 400, message: "Invalid address format".to_string(), details: None });
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
        return Err(ApiError { code: 400, message: "Invalid address format".to_string(), details: None });
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
        Err(_) => return Err(ApiError { code: 400, message: "Invalid transaction ID format".to_string(), details: None }),
    };
    let tx_hash = H256::from_slice(&tx_hash_bytes);

    if !reflect_on_action(tx_hash).await {
        error!(
            "ΛΣ-ΩMEGA Reflex Rejection: Transaction {} halted due to unstable system state.",
            tx_data.id
        );
        return Err(ApiError { code: 503, message: "System unstable, transaction rejected".to_string(), details: None });
    }
    info!(
        "ΛΣ-ΩMEGA Passed: Transaction {} approved for processing.",
        tx_data.id
    );

    let utxos_read_guard = state.utxos.read().await;
    let dag_read_guard = state.dag.read().await;

    if let Err(e) = tx_data.verify(&dag_read_guard, &utxos_read_guard).await {
        warn!("Transaction {} failed verification via API: {}", tx_data.id, e);
        return Err(ApiError { code: 400, message: "Transaction verification failed".to_string(), details: Some(e.to_string()) });
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
        return Err(ApiError { code: 500, message: "Internal server error".to_string(), details: None });
    }

    info!(
        "Transaction {} submitted to mempool and broadcasted via API",
        tx_id
    );
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
    let dag_read_guard = state.dag.read().await;
    let blocks_read_guard = dag_read_guard.blocks.read().await;
    let block_data = blocks_read_guard
        .get(&id_str)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(block_data))
}

async fn get_dag(State(state): State<AppState>) -> Result<Json<DagInfo>, StatusCode> {
    let dag_read_guard = state.dag.read().await;
    let blocks_read_guard = dag_read_guard.blocks.read().await;
    let tips_read_guard = dag_read_guard.tips.read().await;
    let validators_read_guard = dag_read_guard.validators.read().await;
    let difficulty_val = *dag_read_guard.difficulty.read().await;
    let num_chains_val = *dag_read_guard.num_chains.read().await;
    let latest_block_timestamp = blocks_read_guard.values().map(|b| b.timestamp).max().unwrap_or(0);

    Ok(Json(DagInfo {
        block_count: blocks_read_guard.len(),
        tip_count: tips_read_guard.values().map(|t_set| t_set.len()).sum(),
        difficulty: difficulty_val,
        target_block_time: dag_read_guard.target_block_time,
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
    use rand::Rng;
    use serial_test::serial;
    use std::fs as std_fs;

    #[tokio::test]
    #[serial]
    async fn test_node_creation_and_config_save() {
        let db_path = "hyperdag_db_test_node_creation";
        if std::path::Path::new(db_path).exists() {
            std::fs::remove_dir_all(db_path).unwrap();
        }

        let _ = tracing_subscriber::fmt::try_init();
        let wallet = Wallet::new().expect("Failed to create new wallet for test");
        let wallet_arc = Arc::new(wallet);
        let genesis_validator_addr = wallet_arc.address();

        let rand_id: u32 = rand::thread_rng().gen();
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
            difficulty: 10,
            max_amount: 10_000_000_000,
            use_gpu: false,
            zk_enabled: false,
            mining_threads: 1,
            num_chains: 1,
            mining_chain_id: 0,
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
            std::fs::remove_dir_all(db_path).unwrap();
        }

        assert!(
            node_instance_result.is_ok(),
            "Node::new failed: {:?}",
            node_instance_result.err()
        );
        let node_instance = node_instance_result.unwrap();

        let updated_config =
            Config::load(&temp_config_path).expect("Failed to load updated temp config");
        assert!(updated_config.local_full_p2p_address.is_some());
        let full_addr = updated_config.local_full_p2p_address.unwrap();
        assert!(full_addr.contains(
            &node_instance
                .p2p_identity_keypair
                .public()
                .to_peer_id()
                .to_string()
        ));
        assert!(full_addr.starts_with(&updated_config.p2p_address));

        assert!(node_instance
            .config
            .p2p_address
            .contains("/ip4/127.0.0.1/tcp/"));
        assert!(node_instance.config.api_address.contains("127.0.0.1:"));
        assert_eq!(node_instance.mining_chain_id, 0);

        let utxos_guard = node_instance.utxos.read().await;
        assert_eq!(utxos_guard.len(), 1);
        let expected_genesis_tx_id = "genesis_placeholder_tx_id_for_chain_0".to_string();
        let utxo_key_expected = "genesis_utxo_for_chain_0".to_string();

        let genesis_utxo_entry = utxos_guard
            .get(&utxo_key_expected)
            .expect("Genesis UTXO not found");
        assert_eq!(genesis_utxo_entry.amount, 100);
        assert_eq!(
            genesis_utxo_entry.address.to_lowercase(),
            genesis_validator_addr.to_lowercase()
        );
        assert_eq!(genesis_utxo_entry.tx_id, expected_genesis_tx_id);

        let _ = std_fs::remove_file(&temp_config_path);
        let _ = std_fs::remove_file(&temp_identity_path);
        let _ = std_fs::remove_file(&temp_peer_cache_path);
    }
}