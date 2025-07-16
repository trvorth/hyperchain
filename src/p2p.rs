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
use nonzero_ext::nonzero;
use prometheus::{register_int_counter, IntCounter};
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
    time::interval,
};
use tracing::{error, info, instrument, warn};

const MAX_MESSAGE_SIZE: usize = 2_000_000;
const HEARTBEAT_INTERVAL_P2P: u64 = 10_000;
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
    BroadcastBlock(HyperBlock),
    BroadcastTransaction(Transaction),
    RequestState,
    BroadcastState(HashMap<String, HyperBlock>, HashMap<String, UTXO>),
    BroadcastCarbonCredential(CarbonOffsetCredential),
    SyncResponse {
        blocks: Vec<HyperBlock>,
        utxos: HashMap<String, UTXO>,
    },
    RequestBlock { block_id: String, peer_id: PeerId },
    SendBlockToOnePeer {
        peer_id: PeerId,
        block: Box<HyperBlock>,
    },
}

type KeyedPeerRateLimiter = RateLimiter<PeerId, DashMapStateStore<PeerId>, DefaultClock>;

#[derive(Clone)]
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

pub struct P2PServer {
    swarm: Swarm<NodeBehaviour>,
    topics: Vec<IdentTopic>,
    node_lattice_signing_key_bytes: Vec<u8>,
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
        let mdns_behaviour = MdnsTokioBehaviour::new(Default::default(), local_peer_id)
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
        }

        Ok(Self {
            swarm,
            topics,
            node_lattice_signing_key_bytes: config.node_signing_key_material.to_vec(),
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
            gossipsub::MessageId::from(std::hash::Hasher::finish(&hasher).to_string())
        };

        let gossipsub_config = gossipsub::ConfigBuilder::default()
            .heartbeat_interval(Duration::from_millis(HEARTBEAT_INTERVAL_P2P))
            .validation_mode(ValidationMode::Strict)
            .max_transmit_size(MAX_MESSAGE_SIZE)
            .mesh_n(4)
            .message_id_fn(message_id_fn)
            .build()
            .map_err(|e_str| {
                P2PError::GossipsubConfig(format!("Error building Gossipsub config: {e_str}"))
            })?;

        gossipsub::Behaviour::new(MessageAuthenticity::Signed(local_key), gossipsub_config)
            .map_err(|e_str| {
                P2PError::GossipsubConfig(format!("Error creating Gossipsub behaviour: {e_str}"))
            })
    }

    fn subscribe_to_topics(
        topic_prefix: &str,
        gossipsub: &mut gossipsub::Behaviour,
    ) -> Result<Vec<IdentTopic>, P2PError> {
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
        for addr_str in addresses {
            let multiaddr: Multiaddr = addr_str.parse()?;
            swarm.listen_on(multiaddr)?;
        }
        info!("P2P Server initialized with Local Peer ID: {local_peer_id}");
        Ok(())
    }

    async fn dial_initial_peers(swarm: &mut Swarm<NodeBehaviour>, peers_addrs: &[String]) {
        for peer_addr_str in peers_addrs {
            if let Ok(multiaddr) = peer_addr_str.parse::<Multiaddr>() {
                if let Err(e) = swarm.dial(multiaddr.clone()) {
                    warn!("Failed to dial peer {multiaddr}: {e}");
                }
            }
        }
    }

    #[instrument(skip(self, rx))]
    pub async fn run(&mut self, mut rx: mpsc::Receiver<P2PCommand>) -> Result<(), P2PError> {
        let mut mesh_check_ticker = interval(Duration::from_secs(60));
        let mut peer_cache_ticker = interval(Duration::from_secs(300));
        let blacklist = Arc::new(RwLock::new(HashSet::new()));

        let rate_limiter_block = Arc::new(RateLimiter::keyed(Quota::per_second(nonzero!(10u32))));
        let rate_limiter_tx = Arc::new(RateLimiter::keyed(Quota::per_second(nonzero!(50u32))));
        let rate_limiter_state = Arc::new(RateLimiter::keyed(Quota::per_second(nonzero!(5u32))));
        let rate_limiter_credential =
            Arc::new(RateLimiter::keyed(Quota::per_second(nonzero!(20u32))));

        loop {
            tokio::select! {
                event = self.swarm.select_next_some() => {
                    if let SwarmEvent::Behaviour(NodeBehaviourEvent::Gossipsub(gossipsub::Event::Message { propagation_source, message, .. })) = event {
                        tokio::spawn({
                            let blacklist = blacklist.clone();
                            let p2p_sender = self.p2p_command_sender.clone();
                            let rate_limiter_block = rate_limiter_block.clone();
                            let rate_limiter_tx = rate_limiter_tx.clone();
                            let rate_limiter_state = rate_limiter_state.clone();
                            let rate_limiter_credential = rate_limiter_credential.clone();
                            async move {
                                Self::static_process_gossip_message(
                                    message,
                                    propagation_source,
                                    blacklist,
                                    p2p_sender,
                                    rate_limiter_block,
                                    rate_limiter_tx,
                                    rate_limiter_state,
                                    rate_limiter_credential,
                                )
                                .await;
                            }
                        });
                    } else {
                        self.handle_swarm_event(event).await;
                    }
                }
                Some(command) = rx.recv() => {
                    if let Err(e) = self.process_internal_command(command).await {
                        error!("Failed to process internal P2P command: {e}");
                    }
                }
                _ = mesh_check_ticker.tick() => { self.check_mesh_peers().await; }
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
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                info!("Connection established with peer: {peer_id}");
            }
            _ => {}
        }
    }

    async fn save_peers_to_cache(&mut self) -> Result<(), P2PError> {
        let mut cache_peers = HashSet::new();
        for kbucket in self.swarm.behaviour_mut().kademlia.kbuckets() {
            for entry in kbucket.iter() {
                // FIX: Use .iter() to correctly iterate over the addresses.
                for addr in entry.node.value.iter() {
                    cache_peers.insert(addr.to_string());
                }
            }
        }

        if !cache_peers.is_empty() {
            let cache = PeerCache {
                peers: cache_peers.into_iter().collect(),
            };
            let cache_json = serde_json::to_string_pretty(&cache)?;
            fs::write(&self.peer_cache_path, cache_json)?;
        }
        Ok(())
    }

    #[instrument(skip(self, command))]
    async fn process_internal_command(&mut self, command: P2PCommand) -> Result<(), P2PError> {
        match command {
            P2PCommand::BroadcastBlock(block) => {
                self.broadcast_message(
                    NetworkMessageData::Block(block.clone()),
                    0,
                    &format!("block {}", block.id),
                )
                .await
            }
            P2PCommand::BroadcastTransaction(tx) => {
                self.broadcast_message(
                    NetworkMessageData::Transaction(tx.clone()),
                    1,
                    &format!("transaction {}", tx.id),
                )
                .await
            }
            P2PCommand::RequestState => {
                self.broadcast_message(NetworkMessageData::StateRequest, 2, "state request")
                    .await
            }
            P2PCommand::BroadcastState(blocks, utxos) => {
                self.broadcast_message(NetworkMessageData::State(blocks, utxos), 2, "state data")
                    .await
            }
            P2PCommand::BroadcastCarbonCredential(cred) => {
                self.broadcast_message(
                    NetworkMessageData::CarbonOffsetCredential(cred.clone()),
                    3,
                    &format!("carbon credential {}", cred.id),
                )
                .await
            }
            _ => Ok(()),
        }
    }

    async fn static_process_gossip_message(
        message: gossipsub::Message,
        source: PeerId,
        blacklist: Arc<RwLock<HashSet<PeerId>>>,
        p2p_command_sender: mpsc::Sender<P2PCommand>,
        rate_limiter_block: Arc<KeyedPeerRateLimiter>,
        rate_limiter_tx: Arc<KeyedPeerRateLimiter>,
        rate_limiter_state: Arc<KeyedPeerRateLimiter>,
        rate_limiter_credential: Arc<KeyedPeerRateLimiter>,
    ) {
        // FIX: Reordered checks and locking to prevent deadlocks (hanging issue).
        // First, perform a quick read-only check to see if the peer is already blacklisted.
        if blacklist.read().await.contains(&source) {
            return;
        }

        let topic_str = message.topic.as_str();
        let rate_limiter_to_use = if topic_str.contains("blocks") {
            rate_limiter_block
        } else if topic_str.contains("transactions") {
            rate_limiter_tx
        } else if topic_str.contains("state_updates") {
            rate_limiter_state
        } else if topic_str.contains("carbon_credentials") {
            rate_limiter_credential
        } else {
            warn!("Received message on unknown topic: {}", topic_str);
            return;
        };
        
        // Second, check the rate limiter. If it fails, acquire a write lock to update the blacklist.
        if rate_limiter_to_use.check_key(&source).is_err() {
            let mut blacklist_writer = blacklist.write().await;
            // The `insert` method returns true if the peer was not already in the set.
            if blacklist_writer.insert(source) {
                warn!("Peer {} exceeded rate limit. Blacklisting.", source);
                PEERS_BLACKLISTED.inc();
            }
            return;
        }

        if let Ok(msg_payload) = serde_json::from_slice::<NetworkMessage>(&message.data) {
            let cmd = match msg_payload.data {
                NetworkMessageData::Block(block) => P2PCommand::BroadcastBlock(block),
                NetworkMessageData::Transaction(tx) => P2PCommand::BroadcastTransaction(tx),
                NetworkMessageData::State(blocks, utxos) => P2PCommand::SyncResponse {
                    blocks: blocks.into_values().collect(),
                    utxos,
                },
                NetworkMessageData::StateRequest => P2PCommand::RequestState,
                NetworkMessageData::CarbonOffsetCredential(cred) => {
                    P2PCommand::BroadcastCarbonCredential(cred)
                }
            };
            if p2p_command_sender.send(cmd).await.is_err() {
                error!("Failed to forward message to command processor");
            }
        }
    }

    async fn check_mesh_peers(&mut self) {
        for topic_instance in &self.topics {
            let mesh_peers: Vec<_> = self
                .swarm
                .behaviour()
                .gossipsub
                .mesh_peers(&topic_instance.hash())
                .collect();
            if mesh_peers.len() < MIN_PEERS_FOR_MESH {
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