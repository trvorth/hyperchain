use crate::config::{Config, ConfigError};
use crate::hyperdag::{HyperBlock, HyperDAG};
use crate::mempool::Mempool;
use crate::miner::{Miner, MinerConfig, MiningError};
use crate::p2p::{P2PCommand, P2PConfig, P2PError, P2PServer};
use crate::transaction::{Transaction, UTXO};
use crate::wallet::HyperWallet;
use anyhow;
use axum::{
    body::Body,
    extract::{Path as AxumPath, State, State as MiddlewareState},
    http::{Request as HttpRequest, StatusCode},
    middleware::{self, Next},
    routing::{get, post},
    Json, Router,
};
use backoff::future::retry;
use backoff::{Error as BackoffError, ExponentialBackoff};
use governor::clock::QuantaClock;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};
use libp2p::identity;
use libp2p::PeerId;
use log::{debug, error, info, warn};
use nonzero_ext::nonzero;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::signal;
use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinSet;
use tokio::time::{self, Instant};
use tracing::instrument;

const MAX_UTXOS: usize = 1_000_000;
const MAX_PROPOSALS: usize = 10_000;
const ADDRESS_REGEX: &str = r"^[0-9a-fA-F]{64}$";

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
    Join(#[from] tokio::task::JoinError),
    #[error("P2P specific error: {0}")]
    P2PSpecific(#[from] P2PError),
    #[error("P2P Identity error: {0}")]
    P2PIdentity(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Serialize, Debug)]
struct DagInfo {
    block_count: usize,
    tip_count: usize,
    difficulty: u64,
    target_block_time: u64,
    validator_count: usize,
    num_chains: u32,
}

#[derive(Serialize, Debug)]
struct PublishReadiness {
    is_ready: bool,
    block_count: usize,
    utxo_count: usize,
    peer_count: usize,
    mempool_size: usize,
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
    wallet: Arc<HyperWallet>,
    pub mempool: Arc<RwLock<Mempool>>,
    pub utxos: Arc<RwLock<HashMap<String, UTXO>>>,
    pub proposals: Arc<RwLock<Vec<HyperBlock>>>,
    _mining_chain_id: u32,
    peer_cache_path: String,
}

type DirectApiRateLimiter = RateLimiter<NotKeyed, InMemoryState, QuantaClock>;

impl Node {
    #[instrument(skip(config, wallet))]
    pub async fn new(
        mut config: Config,
        config_path: String,
        wallet: Arc<HyperWallet>,
        p2p_identity_path: &str,
        peer_cache_path: String,
    ) -> Result<Self, NodeError> {
        config.validate()?;

        let local_keypair = match fs::read(p2p_identity_path) {
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
                    fs::create_dir_all(p)?;
                }
                let new_key = identity::Keypair::generate_ed25519();
                fs::write(
                    p2p_identity_path,
                    new_key.to_protobuf_encoding().map_err(|e| {
                        NodeError::P2PIdentity(format!("Failed to encode P2P key: {e:?}"))
                    })?,
                )?;
                info!("New P2P identity key saved to {p2p_identity_path}");
                Ok(new_key)
            }
            Err(e) => Err(NodeError::P2PIdentity(format!(
                "Failed to read P2P identity key file '{p2p_identity_path}': {e}"
            ))),
        }?;

        let local_peer_id = PeerId::from(local_keypair.public());
        info!("Node Local P2P Peer ID: {}", local_peer_id);

        let full_local_p2p_address = format!("{}/p2p/{}", config.p2p_address, local_peer_id);

        if config.local_full_p2p_address.as_deref() != Some(&full_local_p2p_address) {
            info!("Updating config file '{config_path}' with local full P2P address: {full_local_p2p_address}");
            config.local_full_p2p_address = Some(full_local_p2p_address.clone());
            config.save(&config_path)?;
        }

        let initial_validator = wallet.get_address().trim().to_lowercase();
        if initial_validator != config.genesis_validator.trim().to_lowercase() {
            return Err(NodeError::Config(ConfigError::Validation(
                "Wallet address does not match genesis validator".to_string(),
            )));
        }

        let signing_key_dalek = wallet.get_signing_key()?;
        let node_signing_key_bytes = signing_key_dalek.to_bytes().to_vec();

        let dag = Arc::new(RwLock::new(
            HyperDAG::new(
                &initial_validator,
                config.target_block_time,
                config.difficulty,
                config.num_chains,
                &node_signing_key_bytes,
            )
            .await
            .map_err(|e| NodeError::DAG(e.to_string()))?,
        ));

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
                        amount: 100,
                        tx_id: genesis_id_convention,
                        output_index: 0,
                        explorer_link: format!("https://hyperblockexplorer.org/utxo/{utxo_id}"),
                    },
                );
            }
        }

        let miner_config = MinerConfig {
            address: wallet.get_address(),
            dag: dag.clone(),
            mempool: mempool.clone(),
            difficulty_hex: format!("{:x}", config.difficulty),
            target_block_time: config.target_block_time,
            use_gpu: config.use_gpu,
            zk_enabled: config.zk_enabled,
            threads: config.mining_threads,
            num_chains: config.num_chains,
        };
        let miner_instance = Miner::new(miner_config)?;
        let miner = Arc::new(miner_instance);

        info!(
            "Node initialized: Configured P2P Address Base={}, API Address={}, Chains={}",
            config.p2p_address, config.api_address, config.num_chains
        );

        Ok(Self {
            _config_path: config_path,
            config: config.clone(),
            p2p_identity_keypair: local_keypair,
            dag,
            miner,
            wallet,
            mempool,
            utxos,
            proposals,
            _mining_chain_id: config.mining_chain_id,
            peer_cache_path,
        })
    }

    pub fn api_address(&self) -> &str {
        &self.config.api_address
    }

    pub async fn start(&self) -> Result<(), NodeError> {
        let (tx_p2p_commands, rx_p2p_commands_for_p2p_task) = mpsc::channel::<P2PCommand>(100);
        let mut join_set: JoinSet<Result<(), NodeError>> = JoinSet::new();

        info!("Initializing P2P server task for the Node.");
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
            let current_rx = rx_p2p_commands_for_p2p_task;
            loop {
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
                match P2PServer::new(p2p_config).await {
                    Ok(mut p2p_server) => {
                        if !p2p_initial_peers_config_clone.is_empty() {
                            if let Err(e) = p2p_command_sender_clone
                                .send(P2PCommand::RequestState)
                                .await
                            {
                                error!("Failed to send initial RequestState P2P command: {e}");
                            }
                        }
                        if let Err(e) = p2p_server.run(current_rx).await {
                            error!("Node's internal P2P server run error: {e}. Node will attempt to restart P2P server.");
                            return Err(NodeError::P2PSpecific(e));
                        }
                        info!("Node's internal P2P server task finished cleanly.");
                        return Ok(());
                    }
                    Err(e) => {
                        warn!("Node's internal P2P server creation failed: {e}. Retrying in 5 seconds...");
                        time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        };
        join_set.spawn(p2p_task_fut);

        let mining_task_fut = {
            let miner_clone = self.miner.clone();
            let dag_clone_miner = self.dag.clone();
            let mempool_clone_miner = self.mempool.clone();
            let utxos_clone_miner = self.utxos.clone();
            let mining_tx_channel = tx_p2p_commands.clone();
            let peers_present = !self.config.peers.is_empty();
            let proposals_clone_miner = self.proposals.clone();
            let wallet_clone_miner = self.wallet.clone();
            async move {
                let mut last_aggregation = Instant::now();
                loop {
                    {
                        let guard = mempool_clone_miner.write().await;
                        guard.prune_expired().await;
                    }
                    match miner_clone.mine(&utxos_clone_miner).await {
                        Ok(Some(block)) => {
                            info!("Block mined: hash={}", block.id);
                            let wallet_address = wallet_clone_miner.get_address();
                            let balance = {
                                let utxos_read_guard = utxos_clone_miner.read().await;
                                utxos_read_guard
                                    .iter()
                                    .filter(|(_, utxo)| utxo.address == wallet_address)
                                    .map(|(_, utxo)| utxo.amount)
                                    .sum::<u64>()
                            };
                            info!("Wallet balance: address={wallet_address}, balance={balance}");

                            let proposals_result = time::timeout(
                                Duration::from_secs(2),
                                proposals_clone_miner.write(),
                            )
                            .await;
                            if let Ok(mut proposals_guard) = proposals_result {
                                if proposals_guard.len() >= MAX_PROPOSALS
                                    && !proposals_guard.is_empty()
                                {
                                    proposals_guard.remove(0);
                                }
                                proposals_guard.push(block.clone());
                                if peers_present {
                                    if let Err(e_send) = mining_tx_channel
                                        .try_send(P2PCommand::BroadcastBlock(block))
                                    {
                                        debug!(
                                            "Failed to send mined block to P2P channel: {e_send}"
                                        );
                                    }
                                } else {
                                    let mut dag_write_guard = dag_clone_miner.write().await;
                                    if let Err(e) =
                                        dag_write_guard.add_block(block, &utxos_clone_miner).await
                                    {
                                        warn!("Failed to add self-mined block in single-node mode: {e}");
                                    } else {
                                        info!("Added self-mined block to DAG in single-node mode.");
                                    }
                                }
                            } else if let Err(_timeout_err) = proposals_result {
                                error!("Mining loop proposals lock timeout on mined block");
                            }
                        }
                        Ok(None) => debug!("No block mined in this round"),
                        Err(e) => {
                            error!("Mining error: {e:?}");
                        }
                    }

                    if last_aggregation.elapsed() >= Duration::from_secs(5) {
                        let proposals_result =
                            time::timeout(Duration::from_secs(2), proposals_clone_miner.write())
                                .await;
                        if let Ok(mut proposals_guard) = proposals_result {
                            if !proposals_guard.is_empty() {
                                let aggregated_block_result = {
                                    let dag_read_guard = dag_clone_miner.read().await;
                                    dag_read_guard
                                        .aggregate_blocks(
                                            proposals_guard.clone(),
                                            &utxos_clone_miner,
                                        )
                                        .await
                                };
                                if let Ok(Some(block_to_add)) = aggregated_block_result {
                                    let mut dag_write_guard = dag_clone_miner.write().await;
                                    if dag_write_guard
                                        .add_block(block_to_add.clone(), &utxos_clone_miner)
                                        .await
                                        .unwrap_or(false)
                                    {
                                        info!("Added aggregated block {} to DAG", block_to_add.id);
                                        if peers_present {
                                            if let Err(e_send) = mining_tx_channel
                                                .try_send(P2PCommand::BroadcastBlock(block_to_add))
                                            {
                                                debug!("Failed to send aggregated block to P2P channel: {e_send}");
                                            }
                                        }
                                    }
                                    proposals_guard.clear();
                                } else if let Err(e) = aggregated_block_result {
                                    error!("Error during block aggregation: {e:?}");
                                }
                            }
                        } else if let Err(_timeout_err) = proposals_result {
                            error!("Mining loop proposals lock error on aggregation");
                        }
                        last_aggregation = Instant::now();
                    }

                    if let Err(e) = dag_clone_miner.read().await.adjust_difficulty().await {
                        error!("Failed to adjust difficulty: {e:?}");
                    }
                    time::sleep(Duration::from_millis(500)).await;
                }
            }
        };

        let server_task_fut = {
            let app_state = AppState {
                dag: self.dag.clone(),
                mempool: self.mempool.clone(),
                utxos: self.utxos.clone(),
                api_address: self.config.api_address.clone(),
            };
            async move {
                let backoff = ExponentialBackoff {
                    max_elapsed_time: Some(Duration::from_secs(30)),
                    ..Default::default()
                };
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

                match retry(backoff, || async {
                    let addr: SocketAddr = app_state.api_address.parse().map_err(|e| {
                        BackoffError::Permanent(NodeError::Config(ConfigError::Validation(
                            format!("Invalid API address: {e}"),
                        )))
                    })?;
                    info!("API server starting on {addr}");
                    axum_server::bind(addr)
                        .serve(app.clone().into_make_service())
                        .await
                        .map_err(|e| BackoffError::Transient {
                            err: NodeError::ServerExecution(format!("API server failed: {e}")),
                            retry_after: None,
                        })?;
                    Ok(())
                })
                .await
                {
                    Ok(_) => {
                        info!("API server task completed (this is unexpected for an indefinite server).");
                        Ok(())
                    }
                    Err(e) => match e {
                        BackoffError::Permanent(ne) | BackoffError::Transient { err: ne, .. } => {
                            error!("API server failed permanently after retries: {ne:?}");
                            Err(ne)
                        }
                    },
                }
            }
        };

        let mining_task_wrapper = async move {
            mining_task_fut.await;
            Ok(())
        };
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
}

async fn rate_limit_layer(
    MiddlewareState(limiter): MiddlewareState<Arc<DirectApiRateLimiter>>,
    req: HttpRequest<Body>,
    next: Next<Body>,
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

    let is_ready = issues.is_empty();
    Ok(Json(PublishReadiness {
        is_ready,
        block_count: blocks_read_guard.len(),
        utxo_count: utxos_read_guard.len(),
        peer_count: 0,
        mempool_size: mempool_read_guard.size().await,
        issues,
    }))
}

async fn get_balance(
    State(state): State<AppState>,
    AxumPath(address): AxumPath<String>,
) -> Result<Json<u64>, StatusCode> {
    if !Regex::new(ADDRESS_REGEX).unwrap().is_match(&address) {
        warn!("Invalid address format for balance check: {address}");
        return Err(StatusCode::BAD_REQUEST);
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
) -> Result<Json<HashMap<String, UTXO>>, StatusCode> {
    if !Regex::new(ADDRESS_REGEX).unwrap().is_match(&address) {
        warn!("Invalid address format for UTXO fetch: {address}");
        return Err(StatusCode::BAD_REQUEST);
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
) -> Result<Json<String>, StatusCode> {
    if !Regex::new(ADDRESS_REGEX).unwrap().is_match(&tx_data.sender)
        || !Regex::new(ADDRESS_REGEX)
            .unwrap()
            .is_match(&tx_data.receiver)
    {
        warn!(
            "Invalid sender or receiver address in transaction: {}",
            tx_data.id
        );
        return Err(StatusCode::BAD_REQUEST);
    }
    if tx_data.amount == 0 && !tx_data.inputs.is_empty() {
        warn!(
            "Invalid transaction amount (0 for non-coinbase): {}",
            tx_data.id
        );
        return Err(StatusCode::BAD_REQUEST);
    }

    let mempool_write_guard = state.mempool.write().await;
    let utxos_read_guard = state.utxos.read().await;
    let dag_read_guard = state.dag.read().await;

    if tx_data.verify(&state.dag, &state.utxos).await.is_err() {
        warn!(
            "Transaction {} failed full verification via API",
            tx_data.id
        );
        return Err(StatusCode::BAD_REQUEST);
    }

    match mempool_write_guard
        .add_transaction(tx_data.clone(), &utxos_read_guard, &dag_read_guard)
        .await
    {
        Ok(_) => {
            info!("Transaction {} added to mempool via API", tx_data.id);
            Ok(Json(tx_data.id.clone()))
        }
        Err(mempool_err) => {
            warn!(
                "Failed to add transaction {} to mempool: {}",
                tx_data.id, mempool_err
            );
            Err(StatusCode::BAD_REQUEST)
        }
    }
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

    Ok(Json(DagInfo {
        block_count: blocks_read_guard.len(),
        tip_count: tips_read_guard.values().map(|t_set| t_set.len()).sum(),
        difficulty: difficulty_val,
        target_block_time: dag_read_guard.target_block_time,
        validator_count: validators_read_guard.len(),
        num_chains: num_chains_val,
    }))
}

async fn health_check() -> Result<Json<serde_json::Value>, StatusCode> {
    Ok(Json(serde_json::json!({ "status": "healthy" })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, LoggingConfig, P2pConfig};
    use rand::Rng;
    use serial_test::serial; // Added
    use std::fs as std_fs;
    use std::sync::Arc;

    #[tokio::test]
    #[serial] // Added to ensure serial execution
    async fn test_node_creation_and_config_save() {
        // Clean up the database directory before the test
        if std::path::Path::new("hyperdag_db").exists() {
            std::fs::remove_dir_all("hyperdag_db").unwrap();
        }

        let _ = env_logger::try_init();
        let wallet = HyperWallet::new().expect("Failed to create new wallet for test");
        let wallet_arc = Arc::new(wallet);
        let genesis_validator_addr = wallet_arc.get_address();

        let rand_id: u32 = rand::thread_rng().gen();
        let temp_config_path = format!("./temp_test_node_config_{}.toml", rand_id);
        let temp_identity_path = format!("./temp_p2p_identity_{}.key", rand_id);
        let temp_peer_cache_path = format!("./temp_peer_cache_{}.json", rand_id);

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
        assert_eq!(node_instance._mining_chain_id, 0);

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
