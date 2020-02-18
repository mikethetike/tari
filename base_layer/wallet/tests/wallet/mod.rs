//  Copyright 2019 The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use crate::support::utils::{make_input, random_string};
use rand::rngs::OsRng;
use std::{sync::Arc, time::Duration};
use tari_comms::{
    control_service::ControlServiceConfig,
    multiaddr::Multiaddr,
    peer_manager::{peer::PeerFlags, NodeId, NodeIdentity, Peer, PeerFeatures},
    types::CommsPublicKey,
};
#[cfg(feature = "test_harness")]
use tari_comms_dht::DhtConfig;
use tari_core::transactions::{tari_amount::MicroTari, types::CryptoFactories};
use tari_crypto::keys::{PublicKey, SecretKey};
use tari_p2p::initialization::CommsConfig;
use tari_test_utils::{collect_stream, paths::with_temp_dir};

use tari_core::transactions::{tari_amount::uT, transaction::UnblindedOutput, types::PrivateKey};
use tari_wallet::{
    contacts_service::storage::{database::Contact, memory_db::ContactsServiceMemoryDatabase},
    output_manager_service::storage::memory_db::OutputManagerMemoryDatabase,
    storage::memory_db::WalletMemoryDatabase,
    transaction_service::{handle::TransactionEvent, storage::memory_db::TransactionMemoryDatabase},
    wallet::WalletConfig,
    Wallet,
};
use tempdir::TempDir;
use tokio::runtime::Runtime;
use tari_crypto::ristretto::{RistrettoSecretKey, RistrettoPublicKey};

fn create_peer(public_key: CommsPublicKey, net_address: Multiaddr) -> Peer {
    Peer::new(
        public_key.clone(),
        NodeId::from_key(&public_key).unwrap(),
        net_address.into(),
        PeerFlags::empty(),
        PeerFeatures::COMMUNICATION_NODE,
    )
}

#[test]
fn test_wallet() {
    with_temp_dir(|dir_path| {
        let mut runtime = Runtime::new().unwrap();
        let factories = CryptoFactories::default();
        let alice_identity = NodeIdentity::random(
            &mut OsRng,
            "/ip4/127.0.0.1/tcp/22523".parse().unwrap(),
            PeerFeatures::COMMUNICATION_NODE,
        )
        .unwrap();
        let bob_identity = NodeIdentity::random(
            &mut OsRng,
            "/ip4/127.0.0.1/tcp/22145".parse().unwrap(),
            PeerFeatures::COMMUNICATION_NODE,
        )
        .unwrap();

        let base_node_identity = NodeIdentity::random(
            &mut OsRng,
            "/ip4/127.0.0.1/tcp/54225".parse().unwrap(),
            PeerFeatures::COMMUNICATION_NODE,
        )
        .unwrap();

        let comms_config1 = CommsConfig {
            node_identity: Arc::new(alice_identity.clone()),
            peer_connection_listening_address: "/ip4/127.0.0.1/tcp/0".parse().unwrap(),
            socks_proxy_address: None,
            control_service: ControlServiceConfig {
                listening_address: alice_identity.public_address(),
                socks_proxy_address: None,
                public_peer_address: None,
                requested_connection_timeout: Duration::from_millis(2000),
            },
            datastore_path: dir_path.to_str().unwrap().to_string(),
            establish_connection_timeout: Duration::from_secs(10),
            peer_database_name: random_string(8),
            inbound_buffer_size: 100,
            outbound_buffer_size: 100,
            dht: Default::default(),
        };
        let comms_config2 = CommsConfig {
            node_identity: Arc::new(bob_identity.clone()),
            peer_connection_listening_address: "/ip4/127.0.0.1/tcp/0".parse().unwrap(),
            socks_proxy_address: None,
            control_service: ControlServiceConfig {
                listening_address: bob_identity.public_address(),
                socks_proxy_address: None,
                public_peer_address: None,
                requested_connection_timeout: Duration::from_millis(2000),
            },
            datastore_path: dir_path.to_str().unwrap().to_string(),
            establish_connection_timeout: Duration::from_secs(10),
            peer_database_name: random_string(8),
            inbound_buffer_size: 100,
            outbound_buffer_size: 100,
            dht: Default::default(),
        };
        let config1 = WalletConfig {
            comms_config: comms_config1,
            logging_path: None,
            factories: factories.clone(),
        };
        let config2 = WalletConfig {
            comms_config: comms_config2,
            logging_path: None,
            factories: factories.clone(),
        };
        let runtime_node1 = Runtime::new().unwrap();
        let runtime_node2 = Runtime::new().unwrap();
        let mut alice_wallet = Wallet::new(
            config1,
            runtime_node1,
            WalletMemoryDatabase::new(),
            TransactionMemoryDatabase::new(),
            OutputManagerMemoryDatabase::new(),
            ContactsServiceMemoryDatabase::new(),
        )
        .unwrap();
        let bob_wallet = Wallet::new(
            config2,
            runtime_node2,
            WalletMemoryDatabase::new(),
            TransactionMemoryDatabase::new(),
            OutputManagerMemoryDatabase::new(),
            ContactsServiceMemoryDatabase::new(),
        )
        .unwrap();

        alice_wallet
            .comms
            .peer_manager()
            .add_peer(create_peer(
                bob_identity.public_key().clone(),
                bob_identity.public_address(),
            ))
            .unwrap();

        bob_wallet
            .comms
            .peer_manager()
            .add_peer(create_peer(
                alice_identity.public_key().clone(),
                alice_identity.public_address(),
            ))
            .unwrap();

        alice_wallet
            .set_base_node_peer(
                (*base_node_identity.public_key()).clone(),
                "/ip4/127.0.0.1/tcp/54225".to_string(),
            )
            .unwrap();

        let alice_event_stream = alice_wallet.transaction_service.get_event_stream_fused();

        let value = MicroTari::from(1000);
        let (_utxo, uo1) = make_input(&mut OsRng, MicroTari(2500), &factories.commitment);

        runtime
            .block_on(alice_wallet.output_manager_service.add_output(uo1))
            .unwrap();

        runtime
            .block_on(alice_wallet.transaction_service.send_transaction(
                bob_identity.node_id().clone(),
                value,
                MicroTari::from(20),
                "".to_string(),
            ))
            .unwrap();

        assert_eq!(
            runtime
                .block_on(async {
                    collect_stream!(
                        alice_event_stream.map(|i| (*i).clone()),
                        take = 1,
                        timeout = Duration::from_secs(10)
                    )
                })
                .iter()
                .fold(0, |acc, x| match x {
                    TransactionEvent::ReceivedTransactionReply(_) => acc + 1,
                    _ => acc,
                }),
            1
        );

        let mut contacts = Vec::new();
        for i in 0..2 {
            let (_secret_key, public_key) : (RistrettoSecretKey, RistrettoPublicKey) = PublicKey::random_keypair(&mut OsRng);
            let node_id = NodeId::from_key(&public_key).unwrap();
            contacts.push(Contact {
                alias: random_string(8),
                node_id,
            });

            runtime
                .block_on(alice_wallet.contacts_service.upsert_contact(contacts[i].clone()))
                .unwrap();
        }

        let got_contacts = runtime.block_on(alice_wallet.contacts_service.get_contacts()).unwrap();
        assert_eq!(contacts, got_contacts);
    });
}

#[test]
fn test_import_utxo() {
    let factories = CryptoFactories::default();
    let alice_identity = NodeIdentity::random(
        &mut OsRng,
        "/ip4/127.0.0.1/tcp/24521".parse().unwrap(),
        PeerFeatures::COMMUNICATION_NODE,
    )
    .unwrap();
    let base_node_identity = NodeIdentity::random(
        &mut OsRng,
        "/ip4/127.0.0.1/tcp/24522".parse().unwrap(),
        PeerFeatures::COMMUNICATION_NODE,
    )
    .unwrap();
    let temp_dir = TempDir::new(random_string(8).as_str()).unwrap();
    let comms_config = CommsConfig {
        node_identity: Arc::new(alice_identity.clone()),
        peer_connection_listening_address: "/ip4/127.0.0.1/tcp/0".parse().unwrap(),
        socks_proxy_address: None,
        control_service: ControlServiceConfig {
            listening_address: alice_identity.public_address(),
            socks_proxy_address: None,
            public_peer_address: None,
            requested_connection_timeout: Duration::from_millis(2000),
        },
        datastore_path: temp_dir.path().to_str().unwrap().to_string(),
        establish_connection_timeout: Duration::from_secs(10),
        peer_database_name: random_string(8),
        inbound_buffer_size: 100,
        outbound_buffer_size: 100,
        dht: Default::default(),
    };
    let config = WalletConfig {
        comms_config,
        logging_path: None,
        factories: factories.clone(),
    };
    let runtime_node = Runtime::new().unwrap();
    let mut alice_wallet = Wallet::new(
        config,
        runtime_node,
        WalletMemoryDatabase::new(),
        TransactionMemoryDatabase::new(),
        OutputManagerMemoryDatabase::new(),
        ContactsServiceMemoryDatabase::new(),
    )
    .unwrap();

    let utxo = UnblindedOutput::new(20000 * uT, PrivateKey::default(), None);

    let tx_id = alice_wallet
        .import_utxo(
            &utxo.value,
            &utxo.spending_key,
            base_node_identity.node_id(),
            "Testing".to_string(),
        )
        .unwrap();

    let balance = alice_wallet
        .runtime
        .block_on(alice_wallet.output_manager_service.get_balance())
        .unwrap();

    assert_eq!(balance.available_balance, 20000 * uT);

    let completed_tx = alice_wallet
        .runtime
        .block_on(alice_wallet.transaction_service.get_completed_transactions())
        .unwrap()
        .remove(&tx_id)
        .expect("Tx should be in collection");

    assert_eq!(completed_tx.amount, 20000 * uT);
}

#[cfg(feature = "test_harness")]
#[test]
fn test_data_generation() {
    use tari_wallet::testnet_utils::generate_wallet_test_data;
    let runtime = Runtime::new().unwrap();
    let factories = CryptoFactories::default();
    let node_id = NodeIdentity::random(
        &mut OsRng,
        "/ip4/127.0.0.1/tcp/22712".parse().unwrap(),
        PeerFeatures::COMMUNICATION_NODE,
    )
    .unwrap();
    let temp_dir = TempDir::new(random_string(8).as_str()).unwrap();
    let comms_config = CommsConfig {
        node_identity: Arc::new(node_id.clone()),
        peer_connection_listening_address: "/ip4/127.0.0.1/tcp/0".parse().unwrap(),
        socks_proxy_address: None,
        control_service: ControlServiceConfig {
            listening_address: node_id.public_address(),
            socks_proxy_address: None,
            public_peer_address: None,
            requested_connection_timeout: Duration::from_millis(2000),
        },
        establish_connection_timeout: Duration::from_secs(10),
        datastore_path: temp_dir.path().to_str().unwrap().to_string(),
        peer_database_name: random_string(8),
        inbound_buffer_size: 100,
        outbound_buffer_size: 100,
        dht: DhtConfig {
            discovery_request_timeout: Duration::from_millis(500),
            ..Default::default()
        },
    };

    let config = WalletConfig {
        comms_config,
        factories,
        logging_path: None,
    };

    let transaction_backend = TransactionMemoryDatabase::new();

    let mut wallet = Wallet::new(
        config,
        runtime,
        WalletMemoryDatabase::new(),
        transaction_backend.clone(),
        OutputManagerMemoryDatabase::new(),
        ContactsServiceMemoryDatabase::new(),
    )
    .unwrap();

    generate_wallet_test_data(&mut wallet, temp_dir.path().to_str().unwrap(), transaction_backend).unwrap();

    let contacts = wallet.runtime.block_on(wallet.contacts_service.get_contacts()).unwrap();
    assert!(contacts.len() > 0);

    let balance = wallet
        .runtime
        .block_on(wallet.output_manager_service.get_balance())
        .unwrap();
    assert!(balance.available_balance > MicroTari::from(0));

    let outbound_tx = wallet
        .runtime
        .block_on(wallet.transaction_service.get_pending_outbound_transactions())
        .unwrap();
    assert!(outbound_tx.len() > 0);

    let completed_tx = wallet
        .runtime
        .block_on(wallet.transaction_service.get_completed_transactions())
        .unwrap();
    assert!(completed_tx.len() > 0);

    wallet.shutdown().unwrap();
}
