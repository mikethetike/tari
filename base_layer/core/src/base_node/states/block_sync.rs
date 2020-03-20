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

use crate::{
    base_node::{
        base_node::BaseNodeStateMachine,
        states::{ListeningInfo, StateEvent},
    },
    blocks::BlockHash,
    chain_storage::{async_db, BlockchainBackend, ChainMetadata, ChainStorageError},
};
use log::*;
use rand::seq::SliceRandom;
use std::collections::VecDeque;
use tari_comms::peer_manager::NodeId;
use tari_crypto::tari_utilities::{hex::Hex, Hashable};
use crate::blocks::Block;

const LOG_TARGET: &str = "c::bn::states::block_sync";

// The maximum number of retry attempts a node can perform to request a particular block from remote nodes.
const MAX_HEADER_REQUEST_RETRY_ATTEMPTS: usize = 5;
const MAX_BLOCK_REQUEST_RETRY_ATTEMPTS: usize = 5;

/// Configuration for the Block Synchronization.
#[derive(Clone, Copy)]
pub struct BlockSyncConfig {
    pub max_header_request_retry_attempts: usize,
    pub max_block_request_retry_attempts: usize,
}

impl Default for BlockSyncConfig {
    fn default() -> Self {
        Self {
            max_header_request_retry_attempts: MAX_HEADER_REQUEST_RETRY_ATTEMPTS,
            max_block_request_retry_attempts: MAX_BLOCK_REQUEST_RETRY_ATTEMPTS,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BlockSyncInfo;

impl BlockSyncInfo {
    pub async fn next_event<B: BlockchainBackend + 'static>(
        &mut self,
        shared: &mut BaseNodeStateMachine<B>,
        network_tip: &ChainMetadata,
        sync_peers: &[NodeId],
    ) -> StateEvent
    {
        info!(target: LOG_TARGET, "Synchronizing missing blocks");

        match synchronize_blocks(shared, network_tip, sync_peers).await {
            Ok(StateEvent::BlocksSynchronized) => {
                info!(target: LOG_TARGET, "Block sync state has synchronised");
                StateEvent::BlocksSynchronized
            },
            Ok(StateEvent::MaxRequestAttemptsReached) => {
                warn!(
                    target: LOG_TARGET,
                    "Maximum unsuccessful header/block request attempts reached"
                );
                StateEvent::MaxRequestAttemptsReached
            },
            Ok(state_event) => state_event,
            Err(e) => StateEvent::FatalError(format!("Synchronizing blocks failed. {}", e)),
        }
    }
}

async fn synchronize_blocks<B: BlockchainBackend + 'static>(
    shared: &mut BaseNodeStateMachine<B>,
    network_metadata: &ChainMetadata,
    sync_peers: &[NodeId],
) -> Result<StateEvent, String>
{
    let local_metadata = shared.db.get_metadata().map_err(|e| e.to_string())?;
    let mut selected_sync_peer = select_sync_peer(sync_peers);

    if let Some(mut sync_block_hash) = network_metadata.best_block.clone() {
        // Find the missing block hashes of the strongest network chain.
        let mut attempts: usize = 0;
        let mut block_hashes = VecDeque::<BlockHash>::new();
        let mut linked_to_chain = false;
        while local_metadata.accumulated_difficulty.unwrap_or_else(|| 0.into()) <
            network_metadata.accumulated_difficulty.unwrap_or_else(|| 0.into())
        {
            debug!(target: LOG_TARGET, "Trying to sync header '{}' with peer:{}", sync_block_hash.to_hex(), selected_sync_peer.as_ref().map(|p| p.to_string()).unwrap_or("None".to_string()));
            debug!(target: LOG_TARGET, "Checking if we have '{}' in local best chain", sync_block_hash.to_hex());
            if async_db::fetch_header_with_block_hash(shared.db.clone(), sync_block_hash.clone())
                .await
                .is_ok()
            {
                debug!(target: LOG_TARGET, "Block '{}' is in local best chain, proceed to download block", sync_block_hash.to_hex());
                linked_to_chain = true;

                break;
            }


            debug!(target:LOG_TARGET, "Not in DB, Checking if '{}' is in the orphan DB", sync_block_hash.to_hex());
            if let Ok(block) = async_db::fetch_orphan(shared.db.clone(), sync_block_hash.clone()).await {
                debug!(target:LOG_TARGET, "'{}' is in the orphan DB, moving to check parent '{}", sync_block_hash.to_hex(),  block.header.prev_hash.to_hex());
                block_hashes.push_front(sync_block_hash.clone());
                sync_block_hash = block.header.prev_hash;
                continue;
            }

            debug!(target: LOG_TARGET, "Block '{}' is missing. Adding to download queue.", sync_block_hash.to_hex());
            block_hashes.push_front(sync_block_hash.clone());
            // Find the previous block hash by requesting the current header from the sync peer node.
            match shared
                .comms
                .request_headers_with_hashes_from_peer(vec![sync_block_hash.clone()], selected_sync_peer.clone())
                .await
            {
                Ok(headers) => {
                    debug!(target: LOG_TARGET, "Received {} headers from peer", headers.len());
                    if let Some(header) = headers.first() {
                        // TODO: Validate received headers and download larger set of headers with single request.
                        // TODO: ban peers that provided bad headers and blocks.

                        if header.hash() == sync_block_hash {
                            attempts = 0;
                            sync_block_hash = header.prev_hash.clone();
                            continue;
                        }
                    }
                },
                Err(e) => {
                    warn!(
                        target: LOG_TARGET,
                        "Failed to fetch header from peer:{:?}. Retrying.", e,
                    );
                },
            }
            // Attempt again to retrieve the correct header.
            attempts += 1;
            if attempts >= shared.config.block_sync_config.max_header_request_retry_attempts {
                return Ok(StateEvent::MaxRequestAttemptsReached);
            }
            // Select different sync peer
            selected_sync_peer = select_sync_peer(sync_peers);
        }

        debug!(target: LOG_TARGET, "Syncing missing blocks");
        if linked_to_chain {
            for sync_block_hash in block_hashes {
                debug!(target: LOG_TARGET, "Requesting block '{}' from orphan pool", sync_block_hash.to_hex());
                let mut block: Option<Block> = None;
                if let Ok(b) = async_db::fetch_orphan(shared.db.clone(), sync_block_hash.clone()).await {
                    block = Some(b);
                } else {
                    attempts = 0;
                    while attempts < shared.config.block_sync_config.max_block_request_retry_attempts {
                        debug!(target: LOG_TARGET, "Requesting block '{}' from sync node", sync_block_hash.to_hex());
                        match shared
                            .comms
                            .request_blocks_with_hashes_from_peer(vec![sync_block_hash.clone()], selected_sync_peer.clone())
                            .await
                        {
                            Ok(blocks) => {
                                debug!(target: LOG_TARGET, "Received {} blocks from peer", blocks.len());
                                if let Some(hist_block) = blocks.first() {
                                    let block_hash = hist_block.block().hash();

                                    if block_hash != sync_block_hash {
                                        warn!(
                                            target: LOG_TARGET,
                                            "Invalid block {} received from peer. Retrying",
                                            block_hash.to_hex(),
                                        );
                                    } else {
                                        block = Some(hist_block.block().clone());
                                        break;
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(
                                    target: LOG_TARGET,
                                    "Failed to fetch blocks from peer:{:?}. Retrying.", e,
                                );
                            },
                        }
                        // Attempt again to retrieve the correct block with different sync peer
                        attempts += 1;
                        selected_sync_peer = select_sync_peer(sync_peers);
                    }
                    if attempts >= shared.config.block_sync_config.max_block_request_retry_attempts {
                        return Ok(StateEvent::MaxRequestAttemptsReached);
                    }
                }
              // Should not ever have a None if we reach this point
                let block = block.unwrap();
                match shared.db.add_block(block) {
                    Ok(result) => {
                        info!(target: LOG_TARGET, "Added block {} to best chain:{}", sync_block_hash.to_hex(), result)
                    },
                    Err(ChainStorageError::InvalidBlock) => {
                        warn!(
                            target: LOG_TARGET,
                            "Invalid block {} received from peer. Retrying",
                            sync_block_hash.to_hex(),
                        );
                    },
                    Err(ChainStorageError::ValidationError(err)) => {
                        warn!(
                            target: LOG_TARGET,
                            "Validation on block {} from peer failed:{}. Retrying",
                            sync_block_hash.to_hex(),
                            err
                        );
                    },
                    Err(e) => return Err(e.to_string()),
                }
            }
        } else {
            warn!(target: LOG_TARGET, "Network fork chain not linked to local chain.",);
        }
    }

    Ok(StateEvent::BlocksSynchronized)
}

// Select a random peer from the set of sync peers that have the current network tip.
fn select_sync_peer(sync_peers: &[NodeId]) -> Option<NodeId> {
    sync_peers.choose(&mut rand::thread_rng()).map(Clone::clone)
}

/// State management for BlockSync -> Listening.
impl From<BlockSyncInfo> for ListeningInfo {
    fn from(_old_state: BlockSyncInfo) -> Self {
        ListeningInfo {}
    }
}

/// State management for Listening -> BlockSync. This change happens when a node has been temporarily disconnected
/// from the network, or a reorg has occurred.
impl From<ListeningInfo> for BlockSyncInfo {
    fn from(_old: ListeningInfo) -> Self {
        BlockSyncInfo {}
    }
}
