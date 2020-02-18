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

use crate::support::utils::random_string;
use rand::rngs::OsRng;
use tari_core::transactions::types::PublicKey;
use tari_crypto::keys::PublicKey as PublicKeyTrait;
use tari_service_framework::StackBuilder;
use tari_shutdown::Shutdown;
use tari_wallet::{
    contacts_service::{
        error::{ContactsServiceError, ContactsServiceStorageError},
        handle::ContactsServiceHandle,
        storage::{
            database::{Contact, ContactsBackend, ContactsDatabase, DbKey},
            memory_db::ContactsServiceMemoryDatabase,
            sqlite_db::ContactsServiceSqliteDatabase,
        },
        ContactsServiceInitializer,
    },
    storage::connection_manager::run_migration_and_create_connection_pool,
};
use tempdir::TempDir;
use tokio::runtime::Runtime;
use tari_crypto::ristretto::{RistrettoSecretKey, RistrettoPublicKey};
use tari_comms::peer_manager::NodeId;

pub fn setup_contacts_service<T: ContactsBackend + 'static>(
    runtime: &mut Runtime,
    backend: T,
) -> (ContactsServiceHandle, Shutdown)
{
    let shutdown = Shutdown::new();
    let fut = StackBuilder::new(runtime.handle().clone(), shutdown.to_signal())
        .add_initializer(ContactsServiceInitializer::new(backend))
        .finish();

    let handles = runtime.block_on(fut).expect("Service initialization failed");

    let contacts_api = handles.get_handle::<ContactsServiceHandle>().unwrap();

    (contacts_api, shutdown)
}

#[test]
pub fn test_memory_database_crud() {
    let mut runtime = Runtime::new().unwrap();

    let db = ContactsDatabase::new(ContactsServiceMemoryDatabase::new());
    let mut contacts = Vec::new();
    for i in 0..5 {
        let (_secret_key, public_key): (RistrettoSecretKey,RistrettoPublicKey) = PublicKey::random_keypair(&mut OsRng);
        let node_id = NodeId::from_key(&public_key).unwrap();
        contacts.push(Contact {
            alias: random_string(8),
            node_id,
        });

        runtime.block_on(db.upsert_contact(contacts[i].clone())).unwrap();
    }

    let got_contacts = runtime.block_on(db.get_contacts()).unwrap();
    assert_eq!(contacts, got_contacts);

    let contact = runtime
        .block_on(db.get_contact(contacts[0].node_id.clone()))
        .unwrap();
    assert_eq!(contact, contacts[0]);

    let (_secret_key, public_key) = PublicKey::random_keypair(&mut OsRng);
    let node_id = NodeId::from_key(&public_key).unwrap();
    let contact = runtime.block_on(db.get_contact(node_id.clone()));
    assert_eq!(
        contact,
        Err(ContactsServiceStorageError::ValueNotFound(DbKey::Contact(
            node_id.clone()
        )))
    );
    assert_eq!(
        runtime.block_on(db.remove_contact(node_id.clone())),
        Err(ContactsServiceStorageError::ValueNotFound(DbKey::Contact(
            node_id.clone()
        )))
    );

    let _ = runtime
        .block_on(db.remove_contact(contacts[0].node_id.clone()))
        .unwrap();
    contacts.remove(0);
    let got_contacts = runtime.block_on(db.get_contacts()).unwrap();

    assert_eq!(contacts, got_contacts);
}

pub fn test_contacts_service<T: ContactsBackend + 'static>(backend: T) {
    let mut runtime = Runtime::new().unwrap();
    let (mut contacts_service, _shutdown) = setup_contacts_service(&mut runtime, backend);

    let mut contacts = Vec::new();
    for i in 0..5 {
        let (_secret_key, public_key): (RistrettoSecretKey,RistrettoPublicKey) = PublicKey::random_keypair(&mut OsRng);
        let node_id = NodeId::from_key(&public_key).unwrap();
        contacts.push(Contact {
            alias: random_string(8),
            node_id,
        });

        runtime
            .block_on(contacts_service.upsert_contact(contacts[i].clone()))
            .unwrap();
    }

    let got_contacts = runtime.block_on(contacts_service.get_contacts()).unwrap();
    assert_eq!(contacts, got_contacts);

    let contact = runtime
        .block_on(contacts_service.get_contact(contacts[0].node_id.clone()))
        .unwrap();
    assert_eq!(contact, contacts[0]);

    let (_secret_key, public_key): (RistrettoSecretKey,RistrettoPublicKey) = PublicKey::random_keypair(&mut OsRng);
    let node_id = NodeId::from_key(&public_key).unwrap();
    let contact = runtime.block_on(contacts_service.get_contact(node_id.clone()));
    assert_eq!(
        contact,
        Err(ContactsServiceError::ContactsServiceStorageError(
            ContactsServiceStorageError::ValueNotFound(DbKey::Contact(node_id.clone()))
        ))
    );
    assert_eq!(
        runtime.block_on(contacts_service.remove_contact(node_id.clone())),
        Err(ContactsServiceError::ContactsServiceStorageError(
            ContactsServiceStorageError::ValueNotFound(DbKey::Contact(node_id.clone()))
        ))
    );

    let _ = runtime
        .block_on(contacts_service.remove_contact(contacts[0].node_id.clone()))
        .unwrap();
    contacts.remove(0);
    let got_contacts = runtime.block_on(contacts_service.get_contacts()).unwrap();

    assert_eq!(contacts, got_contacts);

    let mut updated_contact = contacts[1].clone();
    updated_contact.alias = "Fred".to_string();

    runtime
        .block_on(contacts_service.upsert_contact(updated_contact.clone()))
        .unwrap();
    let new_contact = runtime
        .block_on(contacts_service.get_contact(updated_contact.node_id))
        .unwrap();

    assert_eq!(new_contact.alias, updated_contact.alias);
}

#[test]
fn contacts_service_memory_db() {
    test_contacts_service(ContactsServiceMemoryDatabase::new());
}

#[test]
fn contacts_service_sqlite_db() {
    let db_name = format!("{}.sqlite3", random_string(8).as_str());
    let temp_dir = TempDir::new(random_string(8).as_str()).unwrap();
    let db_folder = temp_dir.path().to_str().unwrap().to_string();
    let connection_pool =
        run_migration_and_create_connection_pool(format!("{}/{}", db_folder, db_name).to_string()).unwrap();
    test_contacts_service(ContactsServiceSqliteDatabase::new(connection_pool));
}
