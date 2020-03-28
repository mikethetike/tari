// Copyright 2019. The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
// disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
// following disclaimer in the documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
// products derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
// WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
// USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use crate::miner;
use futures::future;
use log::*;
use rand::rngs::OsRng;
use std::{
    fs,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tari_common::{CommsTransport, DatabaseType, GlobalConfig, Network, SocksAuthentication, TorControlAuthentication};
use tari_comms::{
    multiaddr::{Multiaddr, Protocol},
    peer_manager::{NodeId, NodeIdentity, Peer, PeerFeatures, PeerFlags},
    socks,
    tor,
    tor::TorIdentity,
    transports::SocksConfig,
    utils::multiaddr::multiaddr_to_socketaddr,
    CommsNode,
    ConnectionManagerEvent,
    PeerManager,
};
use tari_comms_dht::Dht;
use tari_core::{
    base_node::{
        chain_metadata_service::{ChainMetadataHandle, ChainMetadataServiceInitializer},
        service::{BaseNodeServiceConfig, BaseNodeServiceInitializer},
        BaseNodeStateMachine,
        BaseNodeStateMachineConfig,
        LocalNodeCommsInterface,
        OutboundNodeCommsInterface,
    },
    chain_storage::{
        create_lmdb_database,
        BlockchainBackend,
        BlockchainDatabase,
        LMDBDatabase,
        MemoryDatabase,
        Validators,
    },
    consensus::{ConsensusManager, ConsensusManagerBuilder, Network as NetworkType},
    mempool::{Mempool, MempoolConfig, MempoolServiceConfig, MempoolServiceInitializer, MempoolValidators},
    mining::Miner,
    proof_of_work::DiffAdjManager,
    tari_utilities::{hex::Hex, message_format::MessageFormat},
    transactions::{
        crypto::keys::SecretKey as SK,
        types::{CryptoFactories, HashDigest, PrivateKey, PublicKey},
    },
    validation::{
        block_validators::{FullConsensusValidator, StatelessBlockValidator},
        transaction_validators::{FullTxValidator, TxInputAndMaturityValidator},
    },
};
use tari_mmr::MmrCacheConfig;
use tari_p2p::{
    comms_connector::{pubsub_connector, PubsubDomainConnector, SubscriptionFactory},
    initialization::{initialize_comms, CommsConfig},
    services::{
        comms_outbound::CommsOutboundServiceInitializer,
        liveness::{LivenessConfig, LivenessInitializer},
    },
    transport::{TorConfig, TransportType},
};
use tari_service_framework::{handles::ServiceHandles, StackBuilder};
use tari_shutdown::ShutdownSignal;
use tari_wallet::{
    output_manager_service::{
        config::OutputManagerServiceConfig,
        handle::OutputManagerHandle,
        storage::sqlite_db::OutputManagerSqliteDatabase,
        OutputManagerServiceInitializer,
    },
    storage::connection_manager::{run_migration_and_create_sqlite_connection, WalletDbConnection},
    transaction_service::{
        config::TransactionServiceConfig,
        handle::TransactionServiceHandle,
        storage::sqlite_db::TransactionServiceSqliteDatabase,
        TransactionServiceInitializer,
    },
};
use tokio::{runtime, stream::StreamExt, sync::broadcast, task};

const LOG_TARGET: &str = "c::bn::initialization";

#[macro_export]
macro_rules! using_backend {
    ($self:expr, $i: ident, $cmd: expr) => {
        match $self {
            NodeContainer::LMDB($i) => $cmd,
            NodeContainer::Memory($i) => $cmd,
        }
    };
}

/// The type of DB is configured dynamically in the config file, but the state machine struct has static dispatch;
/// and so we have to use an enum wrapper to hold the various acceptable types.
pub enum NodeContainer {
    LMDB(BaseNodeContext<LMDBDatabase<HashDigest>>),
    Memory(BaseNodeContext<MemoryDatabase<HashDigest>>),
}

impl NodeContainer {
    /// Starts the node container. This entails starting the miner and wallet (if `mining_enabled` is true) and then
    /// starting the base node state machine. This call consumes the NodeContainer instance.
    pub async fn run(self, rt: runtime::Handle) {
        using_backend!(self, ctx, NodeContainer::run_impl(ctx, rt).await)
    }

    /// Returns a handle to the wallet output manager service. This function panics if it has not been registered
    /// with the comms service
    pub fn output_manager(&self) -> OutputManagerHandle {
        using_backend!(self, ctx, ctx.output_manager())
    }

    /// Returns a handle to the local node communication service. This function panics if it has not been registered
    /// with the comms service
    pub fn local_node(&self) -> LocalNodeCommsInterface {
        using_backend!(self, ctx, ctx.local_node())
    }

    /// Returns the CommsNode.
    pub fn base_node_comms(&self) -> &CommsNode {
        using_backend!(self, ctx, &ctx.base_node_comms)
    }

    /// Returns this node's identity.
    pub fn base_node_identity(&self) -> Arc<NodeIdentity> {
        using_backend!(self, ctx, ctx.base_node_comms.node_identity())
    }

    /// Returns the base node DHT
    pub fn base_node_dht(&self) -> &Dht {
        using_backend!(self, ctx, &ctx.base_node_dht)
    }

    /// Returns this node's wallet identity.
    pub fn wallet_node_identity(&self) -> Arc<NodeIdentity> {
        using_backend!(self, ctx, ctx.wallet_comms.node_identity())
    }

    /// Returns this node's miner enabled flag.
    pub fn miner_enabled(&self) -> Arc<AtomicBool> {
        using_backend!(self, ctx, ctx.miner_enabled.clone())
    }

    /// Returns a handle to the wallet transaction service. This function panics if it has not been registered
    /// with the comms service
    pub fn wallet_transaction_service(&self) -> TransactionServiceHandle {
        using_backend!(self, ctx, ctx.wallet_transaction_service())
    }

    async fn run_impl<B: BlockchainBackend + 'static>(mut ctx: BaseNodeContext<B>, rt: runtime::Handle) {
        info!(target: LOG_TARGET, "Tari base node has STARTED");
        let mut wallet_output_handle = ctx.output_manager();
        // Start wallet & miner
        let mut miner = ctx.miner.take().expect("Miner was not constructed");
        let mut rx = miner.get_utxo_receiver_channel();
        rt.spawn(async move {
            debug!(target: LOG_TARGET, "Mining wallet ready to receive coins.");
            while let Some(utxo) = rx.next().await {
                match wallet_output_handle.add_output(utxo).await {
                    Ok(_) => info!(
                        target: LOG_TARGET,
                        "🤑💰🤑 Newly mined coinbase output added to wallet 🤑💰🤑"
                    ),
                    Err(e) => warn!(target: LOG_TARGET, "Error adding output: {}", e),
                }
            }
        });
        rt.spawn(async move {
            debug!(target: LOG_TARGET, "Starting miner");
            miner.mine().await;
            debug!(target: LOG_TARGET, "Miner has shutdown");
        });
        info!(
            target: LOG_TARGET,
            "Starting node - It will run until a fatal error occurs or until the stop flag is activated."
        );
        ctx.node.run().await;
        info!(target: LOG_TARGET, "Initiating communications stack shutdown");
        future::join(ctx.base_node_comms.shutdown(), ctx.wallet_comms.shutdown()).await;
    }
}

pub struct BaseNodeContext<B: BlockchainBackend> {
    pub base_node_comms: CommsNode,
    pub base_node_dht: Dht,
    pub wallet_comms: CommsNode,
    pub wallet_dht: Dht,
    pub base_node_handles: Arc<ServiceHandles>,
    pub wallet_handles: Arc<ServiceHandles>,
    pub node: BaseNodeStateMachine<B>,
    pub miner: Option<Miner>,
    pub miner_enabled: Arc<AtomicBool>,
}

impl<B: BlockchainBackend> BaseNodeContext<B> {
    pub fn output_manager(&self) -> OutputManagerHandle {
        self.wallet_handles
            .get_handle::<OutputManagerHandle>()
            .expect("Problem getting wallet output manager handle")
    }

    pub fn local_node(&self) -> LocalNodeCommsInterface {
        self.base_node_handles
            .get_handle::<LocalNodeCommsInterface>()
            .expect("Could not get local comms interface handle")
    }

    pub fn wallet_transaction_service(&self) -> TransactionServiceHandle {
        self.wallet_handles
            .get_handle::<TransactionServiceHandle>()
            .expect("Could not get wallet transaction service handle")
    }
}

/// Tries to construct a node identity by loading the secret key and other metadata from disk and calculating the
/// missing fields from that information.
pub fn load_identity(path: &Path) -> Result<NodeIdentity, String> {
    if !path.exists() {
        return Err(format!("Identity file, {}, does not exist.", path.to_str().unwrap()));
    }

    let id_str = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "The node identity file, {}, could not be read. {}",
            path.to_str().unwrap_or("?"),
            e.to_string()
        )
    })?;
    let id = NodeIdentity::from_json(&id_str).map_err(|e| {
        format!(
            "The node identity file, {}, has an error. {}",
            path.to_str().unwrap_or("?"),
            e.to_string()
        )
    })?;
    info!(
        "Node ID loaded with public key {} and Node id {}",
        id.public_key().to_hex(),
        id.node_id().to_hex()
    );
    Ok(id)
}

/// Create a new node id and save it to disk
pub fn create_new_base_node_identity<P: AsRef<Path>>(
    path: P,
    public_addr: Multiaddr,
    features: PeerFeatures,
) -> Result<NodeIdentity, String>
{
    let private_key = PrivateKey::random(&mut OsRng);
    let node_identity = NodeIdentity::new(private_key, public_addr, features)
        .map_err(|e| format!("We were unable to construct a node identity. {}", e.to_string()))?;
    save_as_json(path, &node_identity)?;
    Ok(node_identity)
}

pub fn load_from_json<P: AsRef<Path>, T: MessageFormat>(path: P) -> Result<T, String> {
    if !path.as_ref().exists() {
        return Err(format!(
            "Identity file, {}, does not exist.",
            path.as_ref().to_str().unwrap()
        ));
    }

    let contents = fs::read_to_string(path).map_err(|err| err.to_string())?;
    let object = T::from_json(&contents).map_err(|err| err.to_string())?;
    Ok(object)
}

pub fn save_as_json<P: AsRef<Path>, T: MessageFormat>(path: P, object: &T) -> Result<(), String> {
    let json = object.to_json().unwrap();
    if let Some(p) = path.as_ref().parent() {
        if !p.exists() {
            fs::create_dir_all(p).map_err(|e| format!("Could not save json to data folder. {}", e.to_string()))?;
        }
    }
    fs::write(path.as_ref(), json.as_bytes()).map_err(|e| {
        format!(
            "Error writing json file, {}. {}",
            path.as_ref().to_str().unwrap_or("<invalid UTF-8>"),
            e.to_string()
        )
    })?;

    Ok(())
}

pub async fn configure_and_initialize_node(
    config: &GlobalConfig,
    node_identity: Arc<NodeIdentity>,
    wallet_node_identity: Arc<NodeIdentity>,
    interrupt_signal: ShutdownSignal,
) -> Result<NodeContainer, String>
{
    let network = match &config.network {
        Network::MainNet => NetworkType::MainNet,
        Network::Rincewind => NetworkType::Rincewind,
    };
    let result = match &config.db_type {
        DatabaseType::Memory => {
            let backend = MemoryDatabase::<HashDigest>::default();
            let ctx = build_node_context(
                backend,
                network,
                node_identity,
                wallet_node_identity,
                config,
                interrupt_signal,
            )
            .await?;
            NodeContainer::Memory(ctx)
        },
        DatabaseType::LMDB(p) => {
            let backend = create_lmdb_database(&p, MmrCacheConfig::default()).map_err(|e| e.to_string())?;
            let ctx = build_node_context(
                backend,
                network,
                node_identity,
                wallet_node_identity,
                config,
                interrupt_signal,
            )
            .await?;
            NodeContainer::LMDB(ctx)
        },
    };
    Ok(result)
}

async fn build_node_context<B>(
    backend: B,
    network: NetworkType,
    base_node_identity: Arc<NodeIdentity>,
    wallet_node_identity: Arc<NodeIdentity>,
    config: &GlobalConfig,
    interrupt_signal: ShutdownSignal,
) -> Result<BaseNodeContext<B>, String>
where
    B: BlockchainBackend + 'static,
{
    //---------------------------------- Blockchain --------------------------------------------//

    let rules = ConsensusManagerBuilder::new(network).build();
    let factories = CryptoFactories::default();
    let validators = Validators::new(
        FullConsensusValidator::new(rules.clone(), factories.clone()),
        StatelessBlockValidator::new(&rules.consensus_constants()),
    );
    let db = BlockchainDatabase::new(backend, &rules, validators).map_err(|e| e.to_string())?;
    let mempool_validator =
        MempoolValidators::new(FullTxValidator::new(factories.clone()), TxInputAndMaturityValidator {});
    let mempool = Mempool::new(db.clone(), MempoolConfig::default(), mempool_validator);
    let diff_adj_manager = DiffAdjManager::new(&rules.consensus_constants()).map_err(|e| e.to_string())?;
    rules.set_diff_manager(diff_adj_manager).map_err(|e| e.to_string())?;
    let handle = runtime::Handle::current();

    //---------------------------------- Base Node --------------------------------------------//

    let (publisher, base_node_subscriptions) = pubsub_connector(handle.clone(), 100);
    let base_node_subscriptions = Arc::new(base_node_subscriptions);
    create_peer_db_folder(&config.peer_db_path)?;
    let (base_node_comms, base_node_dht) = setup_base_node_comms(base_node_identity, config, publisher).await?;

    debug!(target: LOG_TARGET, "Registering base node services");
    let base_node_handles = register_base_node_services(
        &base_node_comms,
        &base_node_dht,
        db.clone(),
        base_node_subscriptions.clone(),
        mempool,
        rules.clone(),
    )
    .await;
    debug!(target: LOG_TARGET, "Base node service registration complete.");

    //---------------------------------- Wallet --------------------------------------------//
    let (publisher, wallet_subscriptions) = pubsub_connector(handle.clone(), 100);
    let wallet_subscriptions = Arc::new(wallet_subscriptions);
    create_peer_db_folder(&config.wallet_peer_db_path)?;
    let (wallet_comms, wallet_dht) = setup_wallet_comms(
        wallet_node_identity,
        config,
        publisher,
        base_node_comms.node_identity().to_peer(),
    )
    .await?;

    task::spawn(sync_peers(
        base_node_comms.subscribe_connection_manager_events(),
        base_node_comms.peer_manager(),
        wallet_comms.peer_manager(),
    ));

    create_wallet_folder(
        &config
            .wallet_db_file
            .parent()
            .expect("wallet_db_file cannot be set to a root directory"),
    )?;
    let wallet_conn = run_migration_and_create_sqlite_connection(&config.wallet_db_file)
        .map_err(|e| format!("Could not create wallet: {:?}", e))?;

    let wallet_handles = register_wallet_services(
        &wallet_comms,
        &wallet_dht,
        &wallet_conn,
        wallet_subscriptions,
        factories,
    )
    .await;

    // Set the base node for the wallet to the 'local' base node
    let base_node_public_key = base_node_comms.node_identity().public_key().clone();
    wallet_handles
        .get_handle::<TransactionServiceHandle>()
        .expect("TransactionService is not registered")
        .set_base_node_public_key(base_node_public_key.clone())
        .await
        .expect("Problem setting local base node public key for transaction service.");
    wallet_handles
        .get_handle::<OutputManagerHandle>()
        .expect("OutputManagerService is not registered")
        .set_base_node_public_key(base_node_public_key)
        .await
        .expect("Problem setting local base node public key for output manager service.");

    //---------------------------------- Base Node State Machine --------------------------------------------//
    let outbound_interface = base_node_handles
        .get_handle::<OutboundNodeCommsInterface>()
        .expect("Problem getting node interface handle.");
    let chain_metadata_service = base_node_handles
        .get_handle::<ChainMetadataHandle>()
        .expect("Problem getting chain metadata interface handle.");
    debug!(target: LOG_TARGET, "Creating base node state machine.");
    let mut state_machine_config = BaseNodeStateMachineConfig::default();
    state_machine_config.block_sync_config.sync_strategy = config
        .block_sync_strategy
        .parse()
        .expect("Problem reading block sync strategy from config");

    let node = BaseNodeStateMachine::new(
        &db,
        &outbound_interface,
        base_node_comms.peer_manager(),
        base_node_comms.connection_manager(),
        chain_metadata_service.get_event_stream(),
        state_machine_config,
        interrupt_signal,
    );

    //---------------------------------- Mining --------------------------------------------//

    let event_stream = node.get_state_change_event_stream();
    let miner = miner::build_miner(
        &base_node_handles,
        node.get_interrupt_signal(),
        event_stream,
        rules,
        config.num_mining_threads,
    );
    if config.enable_mining {
        debug!(target: LOG_TARGET, "Enabling solo miner");
        miner.enable_mining_flag().store(true, Ordering::Relaxed);
    } else {
        debug!(
            target: LOG_TARGET,
            "Mining is disabled in the config file. This node will not mine for Tari unless enabled in the UI"
        );
    };

    let miner_enabled = miner.enable_mining_flag();
    Ok(BaseNodeContext {
        base_node_comms,
        base_node_dht,
        wallet_comms,
        wallet_dht,
        base_node_handles,
        wallet_handles,
        node,
        miner: Some(miner),
        miner_enabled,
    })
}

async fn sync_peers(
    mut events_rx: broadcast::Receiver<Arc<ConnectionManagerEvent>>,
    base_node_peer_manager: Arc<PeerManager>,
    wallet_peer_manager: Arc<PeerManager>,
)
{
    while let Some(Ok(event)) = events_rx.next().await {
        if let ConnectionManagerEvent::PeerConnected(conn) = &*event {
            if !wallet_peer_manager.exists_node_id(conn.peer_node_id()).await {
                match base_node_peer_manager.find_by_node_id(conn.peer_node_id()).await {
                    Ok(mut peer) => {
                        peer.unset_id();
                        if let Err(err) = wallet_peer_manager.add_peer(peer).await {
                            error!(target: LOG_TARGET, "Failed to add peer to wallet: {:?}", err);
                        }
                    },
                    Err(err) => {
                        error!(target: LOG_TARGET, "Failed to find peer in base node: {:?}", err);
                    },
                }
            }
        }
    }
}

fn parse_peer_seeds(seeds: &[String]) -> Vec<Peer> {
    info!("Adding {} peers to the peer database", seeds.len());
    let mut result = Vec::with_capacity(seeds.len());
    for s in seeds {
        let parts: Vec<&str> = s.split("::").map(|s| s.trim()).collect();
        if parts.len() != 2 {
            warn!(target: LOG_TARGET, "Invalid peer seed: {}", s);
            continue;
        }
        let pub_key = match PublicKey::from_hex(parts[0]) {
            Err(e) => {
                warn!(
                    target: LOG_TARGET,
                    "{} is not a valid peer seed. The public key is incorrect. {}",
                    s,
                    e.to_string()
                );
                continue;
            },
            Ok(p) => p,
        };
        let addr = match parts[1].parse::<Multiaddr>() {
            Err(e) => {
                warn!(
                    target: LOG_TARGET,
                    "{} is not a valid peer seed. The address is incorrect. {}",
                    s,
                    e.to_string()
                );
                continue;
            },
            Ok(a) => a,
        };
        let node_id = match NodeId::from_key(&pub_key) {
            Err(e) => {
                warn!(
                    target: LOG_TARGET,
                    "{} is not a valid peer seed. A node id couldn't be derived from the public key. {}",
                    s,
                    e.to_string()
                );
                continue;
            },
            Ok(id) => id,
        };
        let peer = Peer::new(
            pub_key,
            node_id,
            addr.into(),
            PeerFlags::default(),
            PeerFeatures::COMMUNICATION_NODE,
            &[],
        );
        result.push(peer);
    }
    result
}

fn setup_transport_type(config: &GlobalConfig) -> TransportType {
    debug!(target: LOG_TARGET, "Transport is set to '{:?}'", config.comms_transport);

    match config.comms_transport.clone() {
        CommsTransport::Tcp {
            listener_address,
            tor_socks_address,
            tor_socks_auth,
        } => TransportType::Tcp {
            listener_address,
            tor_socks_config: tor_socks_address.map(|proxy_address| SocksConfig {
                proxy_address,
                authentication: tor_socks_auth.map(into_socks_authentication).unwrap_or_default(),
            }),
        },
        CommsTransport::TorHiddenService {
            control_server_address,
            socks_address_override,
            forward_address,
            auth,
            onion_port,
        } => {
            let tor_identity_path = Path::new(&config.tor_identity_file);
            let identity = if tor_identity_path.exists() {
                // If this fails, we can just use another address
                load_from_json::<_, TorIdentity>(&tor_identity_path).ok()
            } else {
                None
            };
            info!(
                target: LOG_TARGET,
                "Tor identity at path '{}' {:?}",
                tor_identity_path.to_string_lossy(),
                identity
                    .as_ref()
                    .map(|ident| format!("loaded for address '{}.onion'", ident.service_id))
                    .or_else(|| Some("not found".to_string()))
                    .unwrap()
            );

            let forward_addr = multiaddr_to_socketaddr(&forward_address).expect("Invalid tor forward address");
            TransportType::Tor(TorConfig {
                control_server_addr: control_server_address,
                control_server_auth: {
                    match auth {
                        TorControlAuthentication::None => tor::Authentication::None,
                        TorControlAuthentication::Password(password) => tor::Authentication::HashedPassword(password),
                    }
                },
                identity: identity.map(Box::new),
                port_mapping: (onion_port, forward_addr).into(),
                // TODO: make configurable
                socks_address_override,
                socks_auth: socks::Authentication::None,
            })
        },
        CommsTransport::Socks5 {
            proxy_address,
            listener_address,
            auth,
        } => TransportType::Socks {
            socks_config: SocksConfig {
                proxy_address,
                authentication: into_socks_authentication(auth),
            },
            listener_address,
        },
    }
}

fn setup_wallet_transport_type(config: &GlobalConfig) -> TransportType {
    debug!(
        target: LOG_TARGET,
        "Wallet transport is set to '{:?}'", config.comms_transport
    );

    let add_to_port = |addr: Multiaddr, n| -> Multiaddr {
        addr.iter()
            .map(|p| match p {
                Protocol::Tcp(port) => Protocol::Tcp(port + n),
                p => p,
            })
            .collect()
    };

    match config.comms_transport.clone() {
        CommsTransport::Tcp {
            listener_address,
            tor_socks_address,
            tor_socks_auth,
        } => TransportType::Tcp {
            listener_address: add_to_port(listener_address, 1),
            tor_socks_config: tor_socks_address.map(|proxy_address| SocksConfig {
                proxy_address,
                authentication: tor_socks_auth.map(into_socks_authentication).unwrap_or_default(),
            }),
        },
        CommsTransport::TorHiddenService {
            control_server_address,
            socks_address_override,
            forward_address,
            auth,
            onion_port,
        } => {
            let tor_identity_path = Path::new(&config.wallet_tor_identity_file);
            let identity = if tor_identity_path.exists() {
                // If this fails, we can just use another address
                load_from_json::<_, TorIdentity>(&tor_identity_path).ok()
            } else {
                None
            };
            info!(
                target: LOG_TARGET,
                "Wallet tor identity at path '{}' {:?}",
                tor_identity_path.to_string_lossy(),
                identity
                    .as_ref()
                    .map(|ident| format!("loaded for address '{}.onion'", ident.service_id))
                    .or_else(|| Some("not found".to_string()))
                    .unwrap()
            );

            let mut forward_addr = multiaddr_to_socketaddr(&forward_address).expect("Invalid tor forward address");
            forward_addr.set_port(forward_addr.port() + 1);
            TransportType::Tor(TorConfig {
                control_server_addr: control_server_address,
                control_server_auth: {
                    match auth {
                        TorControlAuthentication::None => tor::Authentication::None,
                        TorControlAuthentication::Password(password) => tor::Authentication::HashedPassword(password),
                    }
                },
                identity: identity.map(Box::new),

                port_mapping: (onion_port.get() + 1, forward_addr).into(),
                // TODO: make configurable
                socks_address_override,
                socks_auth: socks::Authentication::None,
            })
        },
        CommsTransport::Socks5 {
            proxy_address,
            listener_address,
            auth,
        } => TransportType::Socks {
            socks_config: SocksConfig {
                proxy_address,
                authentication: into_socks_authentication(auth),
            },
            listener_address: add_to_port(listener_address, 1),
        },
    }
}

fn into_socks_authentication(auth: SocksAuthentication) -> socks::Authentication {
    match auth {
        SocksAuthentication::None => socks::Authentication::None,
        SocksAuthentication::UsernamePassword(username, password) => {
            socks::Authentication::Password(username, password)
        },
    }
}

fn create_wallet_folder<P: AsRef<Path>>(wallet_path: P) -> Result<(), String> {
    let path = wallet_path.as_ref();
    match fs::create_dir_all(path) {
        Ok(_) => {
            info!(
                target: LOG_TARGET,
                "Wallet directory has been created at {}",
                path.display()
            );
            Ok(())
        },
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            info!(
                target: LOG_TARGET,
                "Wallet directory already exists in {}",
                path.display()
            );
            Ok(())
        },
        Err(e) => Err(format!("Could not create wallet directory: {}", e)),
    }
}

fn create_peer_db_folder<P: AsRef<Path>>(peer_db_path: P) -> Result<(), String> {
    let path = peer_db_path.as_ref();
    match fs::create_dir_all(path) {
        Ok(_) => {
            info!(
                target: LOG_TARGET,
                "Peer database directory has been created in {}",
                path.display()
            );
            Ok(())
        },
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            info!(target: LOG_TARGET, "Peer database already exists in {}", path.display());
            Ok(())
        },
        Err(e) => Err(format!("could not create peer db path: {}", e)),
    }
}

async fn setup_base_node_comms(
    node_identity: Arc<NodeIdentity>,
    config: &GlobalConfig,
    publisher: PubsubDomainConnector,
) -> Result<(CommsNode, Dht), String>
{
    let comms_config = CommsConfig {
        node_identity,
        transport_type: setup_transport_type(&config),
        datastore_path: config.peer_db_path.clone(),
        peer_database_name: "peers".to_string(),
        max_concurrent_inbound_tasks: 100,
        outbound_buffer_size: 100,
        // TODO - make this configurable
        dht: Default::default(),
        // TODO: This should be false unless testing locally - make this configurable
        allow_test_addresses: true,
    };
    let (comms, dht) = initialize_comms(comms_config, publisher)
        .await
        .map_err(|e| format!("Could not create comms layer: {:?}", e))?;

    // Save final node identity after comms has initialized. This is required because the public_address can be changed
    // by comms during initialization when using tor.
    save_as_json(&config.identity_file, &*comms.node_identity())
        .map_err(|e| format!("Failed to save node identity: {:?}", e))?;
    if let Some(hs) = comms.hidden_service() {
        save_as_json(&config.tor_identity_file, hs.tor_identity())
            .map_err(|e| format!("Failed to save tor identity: {:?}", e))?;
    }

    add_peers_to_comms(&comms, parse_peer_seeds(&config.peer_seeds)).await?;

    Ok((comms, dht))
}

async fn setup_wallet_comms(
    node_identity: Arc<NodeIdentity>,
    config: &GlobalConfig,
    publisher: PubsubDomainConnector,
    base_node_peer: Peer,
) -> Result<(CommsNode, Dht), String>
{
    let comms_config = CommsConfig {
        node_identity,
        transport_type: setup_wallet_transport_type(&config),
        datastore_path: config.wallet_peer_db_path.clone(),
        peer_database_name: "peers".to_string(),
        max_concurrent_inbound_tasks: 100,
        outbound_buffer_size: 100,
        // TODO - make this configurable
        dht: Default::default(),
        // TODO: This should be false unless testing locally - make this configurable
        allow_test_addresses: true,
    };
    let (comms, dht) = initialize_comms(comms_config, publisher)
        .await
        .map_err(|e| format!("Could not create comms layer: {:?}", e))?;

    // Save final node identity after comms has initialized. This is required because the public_address can be changed
    // by comms during initialization when using tor.
    save_as_json(&config.wallet_identity_file, &*comms.node_identity())
        .map_err(|e| format!("Failed to save node identity: {:?}", e))?;
    if let Some(hs) = comms.hidden_service() {
        save_as_json(&config.wallet_tor_identity_file, hs.tor_identity())
            .map_err(|e| format!("Failed to save tor identity: {:?}", e))?;
    }

    add_peers_to_comms(&comms, vec![base_node_peer]).await?;

    Ok((comms, dht))
}

async fn add_peers_to_comms(comms: &CommsNode, peers: Vec<Peer>) -> Result<(), String> {
    for p in peers {
        let peer_desc = p.to_string();
        info!(target: LOG_TARGET, "Adding seed peer [{}]", peer_desc);
        comms
            .peer_manager()
            .add_peer(p)
            .await
            .map_err(|e| format!("Could not add peer {} to comms layer: {}", peer_desc, e))?;
    }
    Ok(())
}

async fn register_base_node_services<B>(
    comms: &CommsNode,
    dht: &Dht,
    db: BlockchainDatabase<B>,
    subscription_factory: Arc<SubscriptionFactory>,
    mempool: Mempool<B>,
    consensus_manager: ConsensusManager,
) -> Arc<ServiceHandles>
where
    B: BlockchainBackend + 'static,
{
    let node_config = BaseNodeServiceConfig::default(); // TODO - make this configurable
    let mempool_config = MempoolServiceConfig::default(); // TODO - make this configurable
    StackBuilder::new(runtime::Handle::current(), comms.shutdown_signal())
        .add_initializer(CommsOutboundServiceInitializer::new(dht.outbound_requester()))
        .add_initializer(BaseNodeServiceInitializer::new(
            subscription_factory.clone(),
            db,
            mempool.clone(),
            consensus_manager,
            node_config,
        ))
        .add_initializer(MempoolServiceInitializer::new(
            subscription_factory.clone(),
            mempool,
            mempool_config,
        ))
        .add_initializer(LivenessInitializer::new(
            LivenessConfig {
                auto_ping_interval: Some(Duration::from_secs(30)),
                enable_auto_join: true,
                enable_auto_stored_message_request: true,
                refresh_neighbours_interval: Duration::from_secs(3 * 60),
            },
            subscription_factory,
            dht.dht_requester(),
        ))
        .add_initializer(ChainMetadataServiceInitializer)
        .finish()
        .await
        .expect("Service initialization failed")
}

async fn register_wallet_services(
    wallet_comms: &CommsNode,
    wallet_dht: &Dht,
    wallet_db_conn: &WalletDbConnection,
    subscription_factory: Arc<SubscriptionFactory>,
    factories: CryptoFactories,
) -> Arc<ServiceHandles>
{
    StackBuilder::new(runtime::Handle::current(), wallet_comms.shutdown_signal())
        .add_initializer(CommsOutboundServiceInitializer::new(wallet_dht.outbound_requester()))
        .add_initializer(LivenessInitializer::new(
            LivenessConfig{
                auto_ping_interval: None,
                enable_auto_join: true,
                enable_auto_stored_message_request: true,
                ..Default::default()
            },
            subscription_factory.clone(),
            wallet_dht.dht_requester()

        ))
        // Wallet services
        .add_initializer(OutputManagerServiceInitializer::new(
            OutputManagerServiceConfig::default(),
            subscription_factory.clone(),
            OutputManagerSqliteDatabase::new(wallet_db_conn.clone()),
            factories.clone(),
        ))
        .add_initializer(TransactionServiceInitializer::new(
            TransactionServiceConfig::default(),
            subscription_factory,
            wallet_comms.subscribe_messaging_events(),
            TransactionServiceSqliteDatabase::new(wallet_db_conn.clone()),
            wallet_comms.node_identity(),
            factories,
        ))
        .finish()
        .await
        .expect("Service initialization failed")
}
