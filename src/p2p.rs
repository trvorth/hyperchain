use crate::hyperdag::{HyperBlock, HyperDAG, LatticeSignature, UTXO};
use crate::mempool::Mempool;
use crate::node::PeerCache;
use crate::saga::CarbonOffsetCredential;
use crate::transaction::Transaction;
use futures::stream::StreamExt;
use governor::{clock::DefaultClock, state::keyed::DashMapStateStore, Quota, RateLimiter};
use hmac::{Hmac, Mac};
use libp2p::{
    gossipsub::{self, IdentTopic, MessageAuthenticity, ValidationMode},
    identity,
    kad::{store::MemoryStore, Behaviour as KadBehaviour, Event as KadEvent},
    mdns::tokio::Behaviour as MdnsTokioBehaviour,
    mdns::Event as MdnsEvent,
    noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    yamux, Multiaddr, PeerId, Swarm, SwarmBuilder,
};
use log::{debug, error, info, warn};
use nonzero_ext::nonzero;
use prometheus::{register_int_counter, IntCounter};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::env;
use std::error::Error as StdError;
use std::fs;
use std::hash::Hash;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::{
    sync::{mpsc, RwLock},
    time::{interval, timeout},
};
use tracing::instrument;

const MAX_MESSAGE_SIZE: usize = 2_000_000;
const ADDRESS_REGEX: &str = r"^[0-9a-fA-F]{64}$";
const MAX_PROPOSALS: usize = 20_000;
const HEARTBEAT_INTERVAL_P2P: u64 = 10_000;
const MDNS_INTERVAL_SECS: u64 = 60;
const MIN_PEERS_FOR_MESH: usize = 1;
const DEFAULT_HMAC_SECRET: &str = "hyperledger_secret_key_for_p2p";

lazy_static::lazy_static! {
    static ref MESSAGES_SENT: IntCounter = register_int_counter!("p2p_messages_sent_total", "Total messages sent").unwrap();
    static ref MESSAGES_RECEIVED: IntCounter = register_int_counter!("p2p_messages_received_total", "Total messages received").unwrap();
    static ref PEERS_BLACKLISTED: IntCounter = register_int_counter!("p2p_peers_blacklisted_total", "Total peers blacklisted").unwrap();
}

#[derive(Error, Debug)]
pub enum P2PError {
    #[error("Invalid configuration: {0}")]
    Config(String),
    #[error("Io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Libp2p Core Transport error: {0}")]
    Libp2pTransport(#[from] libp2p::core::transport::TransportError<std::io::Error>),
    #[error("Noise protocol error: {0}")]
    Noise(#[from] libp2p::noise::Error),
    #[error("Multiaddr parsing error: {0}")]
    Multiaddr(#[from] libp2p::multiaddr::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("HMAC error")]
    Hmac,
    #[error("Invalid HMAC key length")]
    HmacKeyLength,
    #[error("Gossipsub configuration error: {0}")]
    GossipsubConfig(String),
    #[error("Gossipsub subscription error: {0}")]
    GossipsubSubscription(#[from] gossipsub::SubscriptionError),
    #[error("Timeout error: {0}")]
    Timeout(#[from] tokio::time::error::Elapsed),
    #[error("Broadcast error: {0}")]
    Broadcast(#[from] libp2p::gossipsub::PublishError),
    #[error("Lattice authentication error: {0}")]
    LatticeAuth(String),
    #[error("Swarm build error: {0}")]
    SwarmBuild(String),
    #[error("Boxed STD error: {0}")]
    BoxedStd(#[from] Box<dyn StdError + Send + Sync>),
    #[error("Infallible error (should not happen): {0}")]
    Infallible(#[from] Infallible),
    #[error("mDNS error: {0}")]
    Mdns(String),
}

#[derive(NetworkBehaviour)]
#[behaviour(to_swarm = "NodeBehaviourEvent")]
pub struct NodeBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub mdns: MdnsTokioBehaviour,
    pub kademlia: KadBehaviour<MemoryStore>,
}

#[derive(Debug)]
pub enum NodeBehaviourEvent {
    Gossipsub(gossipsub::Event),
    Mdns(MdnsEvent),
    Kademlia(KadEvent),
}

impl From<gossipsub::Event> for NodeBehaviourEvent {
    fn from(event: gossipsub::Event) -> Self {
        NodeBehaviourEvent::Gossipsub(event)
    }
}

impl From<MdnsEvent> for NodeBehaviourEvent {
    fn from(event: MdnsEvent) -> Self {
        NodeBehaviourEvent::Mdns(event)
    }
}
impl From<KadEvent> for NodeBehaviourEvent {
    fn from(event: KadEvent) -> Self {
        NodeBehaviourEvent::Kademlia(event)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NetworkMessage {
    data: NetworkMessageData,
    hmac: Vec<u8>,
    signature: LatticeSignature,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum NetworkMessageData {
    Block(HyperBlock),
    Transaction(Transaction),
    State(HashMap<String, HyperBlock>, HashMap<String, UTXO>),
    StateRequest,
    // EVOLVED: Add new message type for PoCO.
    CarbonOffsetCredential(CarbonOffsetCredential),
}

impl NetworkMessage {
    #[instrument]
    fn new(data: NetworkMessageData, lattice_key_material: &[u8]) -> Result<Self, P2PError> {
        let hmac_secret = Self::get_hmac_secret();
        let serialized_data = serde_json::to_vec(&data)?;
        let hmac = Self::compute_hmac(&serialized_data, &hmac_secret)?;

        let signature = LatticeSignature::sign(lattice_key_material, &serialized_data)
            .map_err(|e| P2PError::LatticeAuth(e.to_string()))?;

        Ok(Self {
            data,
            hmac,
            signature,
        })
    }

    fn get_hmac_secret() -> String {
        dotenvy::dotenv().ok();
        let secret = env::var("HMAC_SECRET").unwrap_or_else(|_| DEFAULT_HMAC_SECRET.to_string());
        if secret == DEFAULT_HMAC_SECRET {
            warn!("SECURITY: Using default HMAC secret. This is not secure for production. Please set the HMAC_SECRET environment variable.");
        }
        secret
    }

    fn compute_hmac(data: &[u8], secret: &str) -> Result<Vec<u8>, P2PError> {
        let mut hmac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
            .map_err(|_| P2PError::HmacKeyLength)?;
        hmac.update(data);
        Ok(hmac.finalize().into_bytes().to_vec())
    }
}

#[derive(Debug, Clone)]
pub enum P2PCommand {
    /// Broadcast a newly mined or received block to the network.
    BroadcastBlock(HyperBlock),
    /// Broadcast a new transaction to the network mempool.
    BroadcastTransaction(Transaction),
    /// Request the full network state from a peer, typically on startup.
    RequestState,
    /// Send a full state snapshot to the network in response to a request.
    BroadcastState(HashMap<String, HyperBlock>, HashMap<String, UTXO>),
    /// EVOLVED: Broadcast a verifiable carbon offset credential for the PoCO system.
    BroadcastCarbonCredential(CarbonOffsetCredential),
    /// A direct response to a state request, containing blocks and UTXOs.
    /// This is handled internally by the command processor.
    SyncResponse {
        blocks: Vec<HyperBlock>,
        utxos: HashMap<String, UTXO>,
    },
    /// Request a specific block by its ID from a specific peer.
    RequestBlock {
        block_id: String,
        peer_id: PeerId,
    },
    /// Send a specific block to a single peer that requested it.
    SendBlockToOnePeer {
        peer_id: PeerId,
        block: Box<HyperBlock>,
    },
}


type KeyedPeerRateLimiter = RateLimiter<PeerId, DashMapStateStore<PeerId>, DefaultClock>;

#[derive(Debug, Clone)]
pub struct P2PConfig<'a> {
    pub topic_prefix: &'a str,
    pub listen_addresses: Vec<String>,
    pub initial_peers: Vec<String>,
    pub dag: Arc<RwLock<HyperDAG>>,
    pub mempool: Arc<RwLock<Mempool>>,
    pub utxos: Arc<RwLock<HashMap<String, UTXO>>>,
    pub proposals: Arc<RwLock<Vec<HyperBlock>>>,
    pub local_keypair: identity::Keypair,
    pub node_signing_key_material: &'a [u8],
    pub peer_cache_path: String,
}

pub struct GossipMessageContext {
    pub blacklist: Arc<RwLock<HashSet<PeerId>>>,
    pub rate_limiter_block: Arc<KeyedPeerRateLimiter>,
    pub rate_limiter_tx: Arc<KeyedPeerRateLimiter>,
    pub rate_limiter_state: Arc<KeyedPeerRateLimiter>,
    // EVOLVED: Add a rate limiter for the new credential topic.
    pub rate_limiter_credential: Arc<KeyedPeerRateLimiter>,
    pub dag: Arc<RwLock<HyperDAG>>,
    pub mempool: Arc<RwLock<Mempool>>,
    pub utxos: Arc<RwLock<HashMap<String, UTXO>>>,
    pub proposals: Arc<RwLock<Vec<HyperBlock>>>,
    pub p2p_command_sender: mpsc::Sender<P2PCommand>,
}

pub struct P2PServer {
    swarm: Swarm<NodeBehaviour>,
    dag: Arc<RwLock<HyperDAG>>,
    mempool: Arc<RwLock<Mempool>>,
    utxos: Arc<RwLock<HashMap<String, UTXO>>>,
    proposals: Arc<RwLock<Vec<HyperBlock>>>,
    topics: Vec<IdentTopic>,
    rate_limiter_block: Arc<KeyedPeerRateLimiter>,
    rate_limiter_tx: Arc<KeyedPeerRateLimiter>,
    rate_limiter_state: Arc<KeyedPeerRateLimiter>,
    // EVOLVED: Add a rate limiter for the new credential topic.
    rate_limiter_credential: Arc<KeyedPeerRateLimiter>,
    blacklist: Arc<RwLock<HashSet<PeerId>>>,
    node_lattice_signing_key_bytes: Vec<u8>,
    known_peers: Arc<RwLock<HashSet<PeerId>>>,
    initial_peers_config: Vec<String>,
    peer_cache_path: String,
    p2p_command_sender: mpsc::Sender<P2PCommand>,
}

impl P2PServer {
    #[instrument(skip(config, p2p_command_sender))]
    pub async fn new(
        config: P2PConfig<'_>,
        p2p_command_sender: mpsc::Sender<P2PCommand>,
    ) -> Result<Self, P2PError> {
        let local_peer_id = PeerId::from(config.local_keypair.public());
        info!("P2PServer using Local P2P Peer ID: {}", local_peer_id);

        let store = MemoryStore::new(local_peer_id);
        let mut kademlia_behaviour = KadBehaviour::new(local_peer_id, store);

        for peer_addr_str in &config.initial_peers {
            if let Ok(multiaddr) = peer_addr_str.parse::<Multiaddr>() {
                if let Some(peer_id) = multiaddr.iter().find_map(|p| {
                    if let libp2p::multiaddr::Protocol::P2p(id) = p {
                        Some(id)
                    } else {
                        None
                    }
                }) {
                    kademlia_behaviour.add_address(&peer_id, multiaddr);
                }
            }
        }

        let gossipsub_behaviour = Self::build_gossipsub_behaviour(config.local_keypair.clone())?;

        let mdns_config = libp2p::mdns::Config {
            ttl: Duration::from_secs(MDNS_INTERVAL_SECS * 2),
            query_interval: Duration::from_secs(MDNS_INTERVAL_SECS),
            ..Default::default()
        };
        let mdns_behaviour = MdnsTokioBehaviour::new(mdns_config, local_peer_id)
            .map_err(|e| P2PError::Mdns(format!("Failed to create mDNS behaviour: {e}")))?;

        let behaviour = NodeBehaviour {
            gossipsub: gossipsub_behaviour,
            mdns: mdns_behaviour,
            kademlia: kademlia_behaviour,
        };

        let mut swarm = SwarmBuilder::with_existing_identity(config.local_keypair)
            .with_tokio()
            .with_tcp(
                libp2p::tcp::Config::default().nodelay(true),
                noise::Config::new,
                yamux::Config::default,
            )?
            .with_behaviour(|_key| Ok(behaviour))
            .map_err(|e| P2PError::SwarmBuild(format!("Behaviour setup error: {e:?}")))?
            .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
            .build();

        if !config.initial_peers.is_empty() {
            if let Err(e) = swarm.behaviour_mut().kademlia.bootstrap() {
                warn!("Failed to start Kademlia bootstrap: {e:?}");
            }
        }

        let topics =
            Self::subscribe_to_topics(config.topic_prefix, &mut swarm.behaviour_mut().gossipsub)?;

        Self::listen_on_addresses(&mut swarm, &config.listen_addresses, &local_peer_id)?;
        if !config.initial_peers.is_empty() {
            Self::dial_initial_peers(&mut swarm, &config.initial_peers).await;
        } else {
            info!("No initial peers configured. Relying on mDNS for local discovery.");
        }

        let quota_block = Quota::per_second(nonzero!(10u32));
        let quota_tx = Quota::per_second(nonzero!(50u32));
        let quota_state = Quota::per_second(nonzero!(5u32));
        // EVOLVED: Add a quota for the new credential topic.
        let quota_credential = Quota::per_second(nonzero!(20u32));

        Ok(Self {
            swarm,
            dag: config.dag,
            mempool: config.mempool,
            utxos: config.utxos,
            proposals: config.proposals,
            topics,
            rate_limiter_block: Arc::new(RateLimiter::keyed(quota_block)),
            rate_limiter_tx: Arc::new(RateLimiter::keyed(quota_tx)),
            rate_limiter_state: Arc::new(RateLimiter::keyed(quota_state)),
            rate_limiter_credential: Arc::new(RateLimiter::keyed(quota_credential)),
            blacklist: Arc::new(RwLock::new(HashSet::new())),
            node_lattice_signing_key_bytes: config.node_signing_key_material.to_vec(),
            known_peers: Arc::new(RwLock::new(HashSet::new())),
            initial_peers_config: config.initial_peers,
            peer_cache_path: config.peer_cache_path,
            p2p_command_sender,
        })
    }

    fn build_gossipsub_behaviour(
        local_key: identity::Keypair,
    ) -> Result<gossipsub::Behaviour, P2PError> {
        let message_id_fn = |message: &gossipsub::Message| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            message.data.hash(&mut hasher);
            if let Some(s) = message.source.as_ref() {
                s.hash(&mut hasher)
            }
            if let Some(s) = message.sequence_number {
                s.hash(&mut hasher)
            }
            gossipsub::MessageId::from(std::hash::Hasher::finish(&hasher).to_string())
        };

        let gossipsub_config = gossipsub::ConfigBuilder::default()
            .heartbeat_interval(Duration::from_millis(HEARTBEAT_INTERVAL_P2P))
            .validation_mode(ValidationMode::Strict)
            .max_transmit_size(MAX_MESSAGE_SIZE)
            .mesh_outbound_min(1)
            .mesh_n_low(2)
            .mesh_n(4)
            .mesh_n_high(6)
            .message_id_fn(message_id_fn)
            .build()
            .map_err(|e_str| {
                P2PError::GossipsubConfig(format!("Error building Gossipsub config: {e_str}"))
            })?;

        gossipsub::Behaviour::new(MessageAuthenticity::Signed(local_key), gossipsub_config).map_err(
            |e_str| {
                P2PError::GossipsubConfig(format!("Error creating Gossipsub behaviour: {e_str}"))
            },
        )
    }

    fn subscribe_to_topics(
        topic_prefix: &str,
        gossipsub: &mut gossipsub::Behaviour,
    ) -> Result<Vec<IdentTopic>, P2PError> {
        // EVOLVED: Add a new topic for carbon credentials.
        let topics_str = [
            format!("/hyperchain/{topic_prefix}/blocks"),
            format!("/hyperchain/{topic_prefix}/transactions"),
            format!("/hyperchain/{topic_prefix}/state_updates"),
            format!("/hyperchain/{topic_prefix}/carbon_credentials"),
        ];
        let mut topics = Vec::new();
        for topic_s in topics_str.iter() {
            let topic = IdentTopic::new(topic_s);
            gossipsub.subscribe(&topic)?;
            topics.push(topic);
        }
        Ok(topics)
    }

    fn listen_on_addresses(
        swarm: &mut Swarm<NodeBehaviour>,
        addresses: &[String],
        local_peer_id: &PeerId,
    ) -> Result<(), P2PError> {
        if addresses.is_empty() {
            warn!(
                "No explicit listen addresses provided. Node will attempt to listen on default OS-assigned addresses."
            );
        }
        for addr_str in addresses {
            let multiaddr: Multiaddr = addr_str.parse()?;
            match swarm.listen_on(multiaddr.clone()) {
                Ok(_) => {
                    info!("P2P Server attempting to listen on configured address: {multiaddr}")
                }
                Err(e) => warn!(
                    "Failed to listen on {multiaddr}: {e}. OS will assign address or mDNS might still work."
                ),
            }
        }
        info!("P2P Server initialized with Local Peer ID: {local_peer_id}");
        Ok(())
    }

    async fn dial_initial_peers(swarm: &mut Swarm<NodeBehaviour>, peers_addrs: &[String]) {
        if peers_addrs.is_empty() {
            return;
        }
        info!(
            "Attempting to dial {} initial peers from configuration.",
            peers_addrs.len()
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
        for peer_addr_str in peers_addrs {
            match peer_addr_str.parse::<Multiaddr>() {
                Ok(multiaddr) => {
                    info!("Dialing initial peer: {multiaddr}");
                    if let Err(e) = swarm.dial(multiaddr.clone()) {
                        warn!("Failed to initiate dial to peer {multiaddr}: {e}");
                    }
                }
                Err(e) => warn!("Invalid initial peer address format {peer_addr_str}: {e}"),
            }
        }
    }

    #[instrument(skip(self, rx))]
    pub async fn run(&mut self, mut rx: mpsc::Receiver<P2PCommand>) -> Result<(), P2PError> {
        let mut mesh_check_ticker = interval(Duration::from_secs(60));
        let mut peer_cache_ticker = interval(Duration::from_secs(300));

        loop {
            tokio::select! {
                event = self.swarm.select_next_some() => { self.handle_swarm_event(event).await; }
                Some(command) = rx.recv() => {
                    if let Err(e) = self.process_internal_command(command).await {
                        error!("Failed to process internal P2P command: {e}");
                    }
                }
                _ = mesh_check_ticker.tick() => {
                    self.check_mesh_peers().await;
                }
                _ = peer_cache_ticker.tick() => {
                    if let Err(e) = self.save_peers_to_cache().await {
                        warn!("Failed to save peer cache: {e}");
                    }
                }
            }
        }
    }

    async fn handle_swarm_event(&mut self, event: SwarmEvent<NodeBehaviourEvent>) {
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                info!(
                    "P2P Server listening on: {}/p2p/{}",
                    address,
                    self.swarm.local_peer_id()
                );
            }
            SwarmEvent::ConnectionEstablished {
                peer_id, endpoint, ..
            } => {
                self.known_peers.write().await.insert(peer_id);
                info!("Connection established with peer: {peer_id}");
                self.swarm
                    .behaviour_mut()
                    .gossipsub
                    .add_explicit_peer(&peer_id);
                let addr = endpoint.get_remote_address().clone();
                self.swarm
                    .behaviour_mut()
                    .kademlia
                    .add_address(&peer_id, addr);
            }
            SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                self.known_peers.write().await.remove(&peer_id);
                self.swarm
                    .behaviour_mut()
                    .gossipsub
                    .remove_explicit_peer(&peer_id);
                info!(
                    "Connection closed with peer: {}, cause: {:?}",
                    peer_id,
                    cause.map(|c| c.to_string())
                );
            }
            SwarmEvent::Behaviour(behaviour_event) => match behaviour_event {
                NodeBehaviourEvent::Gossipsub(gossipsub::Event::Message {
                    propagation_source,
                    message_id,
                    message,
                }) => {
                    debug!(
                        "GossipSub: Received message (ID: {message_id}) from peer: {propagation_source}"
                    );

                    let context = GossipMessageContext {
                        blacklist: self.blacklist.clone(),
                        rate_limiter_block: self.rate_limiter_block.clone(),
                        rate_limiter_tx: self.rate_limiter_tx.clone(),
                        rate_limiter_state: self.rate_limiter_state.clone(),
                        rate_limiter_credential: self.rate_limiter_credential.clone(),
                        dag: self.dag.clone(),
                        mempool: self.mempool.clone(),
                        utxos: self.utxos.clone(),
                        proposals: self.proposals.clone(),
                        p2p_command_sender: self.p2p_command_sender.clone(),
                    };

                    tokio::spawn(async move {
                        P2PServer::static_process_gossip_message(
                            message,
                            propagation_source,
                            context,
                        )
                        .await;
                    });
                }
                NodeBehaviourEvent::Mdns(MdnsEvent::Discovered(list)) => {
                    for (peer_id, multiaddr) in list {
                        info!("mDNS: Discovered peer {peer_id} at {multiaddr}");
                        self.swarm
                            .behaviour_mut()
                            .gossipsub
                            .add_explicit_peer(&peer_id);
                        self.swarm
                            .behaviour_mut()
                            .kademlia
                            .add_address(&peer_id, multiaddr);
                    }
                }
                NodeBehaviourEvent::Mdns(MdnsEvent::Expired(list)) => {
                    for (peer_id, multiaddr) in list {
                        debug!("mDNS: Expired peer {peer_id} at {multiaddr}");
                        if !self.swarm.is_connected(&peer_id) {
                            self.swarm
                                .behaviour_mut()
                                .gossipsub
                                .remove_explicit_peer(&peer_id);
                        }
                    }
                }
                NodeBehaviourEvent::Kademlia(kad_event) => {
                    debug!("Kademlia event: {kad_event:?}");
                }
                NodeBehaviourEvent::Gossipsub(other_gossip_event) => {
                    debug!("Other Gossipsub event: {other_gossip_event:?}");
                }
            },
            SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                warn!(
                    "Outgoing connection error to peer {:?}: {}",
                    peer_id
                        .map(|p| p.to_string())
                        .unwrap_or_else(|| "unknown".to_string()),
                    error
                );
            }
            other_event => {
                debug!("Unhandled swarm event: {other_event:?}");
            }
        }
    }

    async fn save_peers_to_cache(&mut self) -> Result<(), P2PError> {
        let mut peers = HashSet::new();
        for kbucket in self.swarm.behaviour_mut().kademlia.kbuckets() {
            for peer in kbucket.iter() {
                for addr in peer.node.value.iter() {
                    peers.insert(addr.to_string());
                }
            }
        }

        if peers.is_empty() {
            info!("No peers found in Kademlia routing table to cache.");
            return Ok(());
        }

        let cache = PeerCache {
            peers: peers.into_iter().collect(),
        };
        let cache_json = serde_json::to_string_pretty(&cache)?;

        fs::write(&self.peer_cache_path, cache_json)?;
        info!(
            "Successfully saved {} peers to cache file: {}",
            cache.peers.len(),
            self.peer_cache_path
        );

        Ok(())
    }

    #[instrument(skip(self, command))]
    async fn process_internal_command(&mut self, command: P2PCommand) -> Result<(), P2PError> {
        match command {
            P2PCommand::BroadcastBlock(block) => {
                self.broadcast_message(
                    NetworkMessageData::Block(block.clone()),
                    0, // Topic index for blocks
                    &format!("block {}", block.id),
                )
                .await
            }
            P2PCommand::BroadcastTransaction(tx) => {
                let add_result = {
                    let mempool_guard = self.mempool.read().await;
                    let utxos_guard = self.utxos.read().await;
                    let dag_guard = self.dag.read().await;
                    mempool_guard
                        .add_transaction(tx.clone(), &utxos_guard, &dag_guard)
                        .await
                };

                if let Err(e) = add_result {
                    warn!("Failed to add locally submitted transaction {} to mempool before broadcasting: {}", tx.id, e);
                } else {
                    self.broadcast_message(
                        NetworkMessageData::Transaction(tx.clone()),
                        1, // Topic index for transactions
                        &format!("transaction {}", tx.id),
                    )
                    .await?
                }
                Ok(())
            }
            P2PCommand::RequestState => {
                self.broadcast_message(NetworkMessageData::StateRequest, 2, "state request")
                    .await
            }
            P2PCommand::BroadcastState(blocks, utxos) => {
                self.broadcast_message(
                    NetworkMessageData::State(blocks, utxos),
                    2,
                    "state data",
                )
                .await
            }
            // EVOLVED: Handle broadcasting the new credential type.
            P2PCommand::BroadcastCarbonCredential(cred) => {
                self.broadcast_message(
                    NetworkMessageData::CarbonOffsetCredential(cred.clone()),
                    3, // Topic index for credentials
                    &format!("carbon credential {}", cred.id),
                )
                .await
            }
            P2PCommand::SyncResponse { .. } => {
                // This command is intended for the node's command processor, not for the P2P layer to act on.
                // It's an internal signal that a sync response has been received and needs processing.
                // No action is needed here in the P2P server's command loop.
                Ok(())
            }
            P2PCommand::RequestBlock { .. } => {
                // Similar to SyncResponse, this is an inbound command that the node processor handles.
                // The processor will then issue a `SendBlockToOnePeer` command if the block is found.
                Ok(())
            }
            // FIX: Correctly handle the borrow-after-move error (E0382).
            // The `block.id` is now accessed for the log message *before* the `block` value
            // is moved into `NetworkMessageData::Block`, satisfying the borrow checker.
            P2PCommand::SendBlockToOnePeer { peer_id, block } => {
                warn!("SendBlockToOnePeer is not yet efficiently implemented. Broadcasting block {} instead of direct sending to {}.", block.id, peer_id);
                let log_info = format!("block {} (for peer {})", block.id, peer_id);
                self.broadcast_message(
                    NetworkMessageData::Block(*block),
                    0, // blocks topic
                    &log_info,
                )
                .await
            }
        }
    }

    async fn static_process_gossip_message(
        message: gossipsub::Message,
        source: PeerId,
        context: GossipMessageContext,
    ) {
        if context.blacklist.read().await.contains(&source) {
            warn!("Ignoring message from blacklisted peer: {source}");
            return;
        }

        let topic_str = message.topic.as_str();
        // EVOLVED: Add the new credential topic to the rate-limiting logic.
        let (rate_limiter_to_use, message_type_str) = if topic_str.contains("blocks") {
            (context.rate_limiter_block, "block")
        } else if topic_str.contains("transactions") {
            (context.rate_limiter_tx, "transaction")
        } else if topic_str.contains("state_updates") {
            (context.rate_limiter_state, "state_update")
        } else if topic_str.contains("carbon_credentials") {
            (context.rate_limiter_credential, "carbon_credential")
        } else {
            warn!(
                "Message on unknown topic '{topic_str}', applying default (block) rate limiter"
            );
            (context.rate_limiter_block, "unknown_topic_block")
        };

        if rate_limiter_to_use.check_key(&source).is_err() {
            warn!("Rate limit exceeded for peer {source} on {message_type_str} messages");
            context.blacklist.write().await.insert(source);
            PEERS_BLACKLISTED.inc();
            return;
        }

        if message.data.len() > MAX_MESSAGE_SIZE {
            warn!(
                "Message from peer {} on topic {} exceeds MAX_MESSAGE_SIZE ({} > {})",
                source,
                topic_str,
                message.data.len(),
                MAX_MESSAGE_SIZE
            );
            return;
        }

        let msg_payload: NetworkMessage = match serde_json::from_slice(&message.data) {
            Ok(payload) => payload,
            Err(e) => {
                warn!("Failed to deserialize message from peer {source} on topic {topic_str}: {e}");
                return;
            }
        };

        let hmac_secret = NetworkMessage::get_hmac_secret();
        let serialized_data_for_verification = match serde_json::to_vec(&msg_payload.data) {
            Ok(d) => d,
            Err(e) => {
                warn!("Failed to serialize message data for verification from {source}: {e}");
                return;
            }
        };

        match NetworkMessage::compute_hmac(&serialized_data_for_verification, &hmac_secret) {
            Ok(computed_hmac) if computed_hmac == msg_payload.hmac => {
                debug!("HMAC verification passed for message from {source}");
            }
            _ => {
                warn!("HMAC verification failed for message from peer {source}");
                context.blacklist.write().await.insert(source);
                PEERS_BLACKLISTED.inc();
                return;
            }
        }

        if !msg_payload
            .signature
            .verify(&serialized_data_for_verification)
        {
            warn!("Lattice signature verification failed for message from peer {source}");
            context.blacklist.write().await.insert(source);
            PEERS_BLACKLISTED.inc();
            return;
        }

        MESSAGES_RECEIVED.inc();
        info!("Processing verified (HMAC & Sig) message data from {source} on topic {topic_str}");
        match msg_payload.data {
            NetworkMessageData::Block(block) => {
                P2PServer::static_process_block(
                    block,
                    source,
                    context.dag,
                    context.utxos,
                    context.proposals,
                )
                .await
            }
            NetworkMessageData::Transaction(tx) => {
                P2PServer::static_process_transaction(
                    tx,
                    source,
                    context.mempool,
                    context.dag,
                    context.utxos,
                )
                .await
            }
            NetworkMessageData::State(blocks_map, new_utxos_map) => {
                // This is a broadcasted state message. We should convert it into a `SyncResponse`
                // and send it to the command processor to handle it in a structured way.
                let blocks_vec = blocks_map.into_values().collect();
                let cmd = P2PCommand::SyncResponse {
                    blocks: blocks_vec,
                    utxos: new_utxos_map,
                };
                if let Err(e) = context.p2p_command_sender.send(cmd).await {
                    error!("Failed to forward state sync data to command processor: {e}");
                }
            }
            NetworkMessageData::StateRequest => {
                info!("Received StateRequest from peer {source}. Preparing to send current state.");
                let dag_guard = context.dag.read().await;

                let (blocks, current_utxos) = dag_guard.get_state_snapshot(0).await;
                drop(dag_guard);

                if let Err(e) = context
                    .p2p_command_sender
                    .send(P2PCommand::BroadcastState(blocks, current_utxos))
                    .await
                {
                    error!("Failed to send BroadcastState command: {e}");
                }
            }
            // EVOLVED: Add processing logic for the new credential message type.
            NetworkMessageData::CarbonOffsetCredential(cred) => {
                info!(
                    "Received CarbonOffsetCredential {} from peer {}",
                    cred.id, source
                );
                let dag = context.dag.read().await;
                // In a real implementation, we would add this to a separate pool
                // for SAGA to process during block evaluation.
                if let Err(e) = dag.saga.verify_and_store_credential(cred).await {
                    warn!("Invalid CarbonOffsetCredential received from {source}: {e}");
                }
            }
        }
    }

    async fn static_process_block(
        block: HyperBlock,
        source: PeerId,
        dag: Arc<RwLock<HyperDAG>>,
        utxos: Arc<RwLock<HashMap<String, UTXO>>>,
        proposals: Arc<RwLock<Vec<HyperBlock>>>,
    ) {
        debug!("Processing block (ID: {}) from peer {}", block.id, source);
        let address_regex = Regex::new(ADDRESS_REGEX).unwrap();
        if !address_regex.is_match(&block.validator) || !address_regex.is_match(&block.miner) {
            warn!(
                "Block from {} (ID: {}) has invalid validator/miner address format.",
                source, block.id
            );
            return;
        }

        let mut dag_write_lock = match timeout(Duration::from_millis(500), dag.write()).await {
            Ok(guard) => guard,
            Err(_) => {
                warn!(
                    "Timeout acquiring DAG write lock for block {} from {}",
                    block.id, source
                );
                return;
            }
        };

        let block_exists = dag_write_lock.blocks.read().await.contains_key(&block.id);

        if block_exists {
            debug!("Block {} from {} already known.", block.id, source);
            return;
        }

        let block_id_clone = block.id.clone();
        if let Err(e) = dag_write_lock.add_block(block.clone(), &utxos).await {
            warn!(
                "Failed to add block (id: {block_id_clone}) from {source}: {e}"
            );
        } else {
            info!(
                "Successfully processed and added block (id: {block_id_clone}) from {source}"
            );
            let mut proposals_lock = proposals.write().await;
            if proposals_lock.len() >= MAX_PROPOSALS && !proposals_lock.is_empty() {
                proposals_lock.remove(0);
            }
            proposals_lock.push(block);
        }
    }

    async fn static_process_transaction(
        tx: Transaction,
        source: PeerId,
        mempool: Arc<RwLock<Mempool>>,
        dag: Arc<RwLock<HyperDAG>>,
        utxos: Arc<RwLock<HashMap<String, UTXO>>>,
    ) {
        debug!("Processing transaction {} from peer {}", tx.id, source);
        let address_regex = Regex::new(ADDRESS_REGEX).unwrap();
        if !address_regex.is_match(&tx.sender)
            || !address_regex.is_match(&tx.receiver)
            || (tx.amount == 0 && !tx.inputs.is_empty())
        {
            warn!(
                "Transaction {} from {} has invalid sender/receiver/amount.",
                tx.id, source
            );
            return;
        }
        
        // FIX: Removed `mut` from `mempool_lock` as it's not mutated.
        let mempool_lock = mempool.write().await;
        let utxos_read_guard = utxos.read().await;
        let dag_read_guard = dag.read().await;

        let tx_id_for_log = tx.id.clone();
        if let Err(e) = mempool_lock
            .add_transaction(tx, &utxos_read_guard, &dag_read_guard)
            .await
        {
            warn!("Failed to add transaction {tx_id_for_log} from {source} to mempool: {e}");
        } else {
            info!("Added transaction {tx_id_for_log} from {source} to mempool.");
        }
    }

    async fn check_mesh_peers(&mut self) {
        for topic_instance in &self.topics {
            let topic_hash = topic_instance.hash();
            let mesh_peers: Vec<_> = self
                .swarm
                .behaviour()
                .gossipsub
                .mesh_peers(&topic_hash)
                .collect();
            let mesh_peer_count = mesh_peers.len();

            debug!(
                "Topic '{topic_instance}': {mesh_peer_count} mesh peers connected: {mesh_peers:?}"
            );
            if mesh_peer_count < MIN_PEERS_FOR_MESH && !self.initial_peers_config.is_empty() {
                warn!("Topic '{topic_instance}': Low number of mesh peers ({mesh_peer_count} < {MIN_PEERS_FOR_MESH}). Attempting to find more peers.");
                self.reconnect_to_initial_peers().await;
                break;
            }
        }
    }

    #[instrument(skip(self))]
    async fn reconnect_to_initial_peers(&mut self) {
        info!("Attempting to reconnect to initial peers from configuration.");
        Self::dial_initial_peers(&mut self.swarm, &self.initial_peers_config).await;
    }

    async fn broadcast_message(
        &mut self,
        data: NetworkMessageData,
        topic_index: usize,
        log_info: &str,
    ) -> Result<(), P2PError> {
        let topic = &self.topics[topic_index];
        let net_msg = NetworkMessage::new(data, &self.node_lattice_signing_key_bytes)?;
        let msg_bytes = serde_json::to_vec(&net_msg)?;

        self.swarm
            .behaviour_mut()
            .gossipsub
            .publish(topic.clone(), msg_bytes)
            .map(|msg_id| {
                MESSAGES_SENT.inc();
                info!("Broadcasted {log_info}: {msg_id}");
            })
            .map_err(P2PError::Broadcast)
    }
}