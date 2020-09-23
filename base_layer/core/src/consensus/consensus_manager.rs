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
    blocks::{
        genesis_block::{
            get_mainnet_block_hash,
            get_mainnet_genesis_block,
            get_rincewind_block_hash,
            get_rincewind_genesis_block,
        },
        Block,
    },
    chain_storage::ChainStorageError,
    consensus::{emission::EmissionSchedule, network::Network, ConsensusConstants},
    proof_of_work::DifficultyAdjustmentError,
    transactions::tari_amount::MicroTari,
};
use std::sync::Arc;
use tari_crypto::tari_utilities::hash::Hashable;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConsensusManagerError {
    #[error("Difficulty adjustment encountered an error: `{0}`")]
    DifficultyAdjustmentError(#[from] DifficultyAdjustmentError),
    #[error("Problem with the DB backend storage: `{0}`")]
    ChainStorageError(#[from] ChainStorageError),
    #[error("There is no blockchain to query")]
    EmptyBlockchain,
    #[error("RwLock access broken: `{0}`")]
    PoisonedAccess(String),
    #[error("No Difficulty adjustment manager present")]
    MissingDifficultyAdjustmentManager,
}

/// Container struct for consensus rules. This can be cheaply cloned.
#[derive(Debug, Clone)]
pub struct ConsensusManager {
    inner: Arc<ConsensusManagerInner>,
}

impl ConsensusManager {
    /// Returns the genesis block for the selected network.
    pub fn get_genesis_block(&self) -> Block {
        match self.inner.network {
            Network::MainNet => get_mainnet_genesis_block(),
            Network::Rincewind => get_rincewind_genesis_block(),
            Network::LocalNet => self.inner.gen_block.clone().unwrap_or_else(get_rincewind_genesis_block),
        }
    }

    /// Returns the genesis block hash for the selected network.
    pub fn get_genesis_block_hash(&self) -> Vec<u8> {
        match self.inner.network {
            Network::MainNet => get_mainnet_block_hash(),
            Network::Rincewind => get_rincewind_block_hash(),
            Network::LocalNet => self
                .inner
                .gen_block
                .clone()
                .unwrap_or_else(get_rincewind_genesis_block)
                .hash(),
        }
    }

    /// Get a pointer to the emission schedule
    pub fn emission_schedule(&self) -> &EmissionSchedule {
        &self.inner.emission
    }

    /// Get a pointer to the consensus constants
    pub fn consensus_constants(&self, height: u64) -> &ConsensusConstants {
        let mut constants = &self.inner.consensus_constants[0];
        for c in self.inner.consensus_constants.iter() {
            if c.effective_from_height() > height {
                break;
            }
            constants = &c
        }
       constants
    }

    /// Creates a total_coinbase offset containing all fees for the validation from block
    pub fn calculate_coinbase_and_fees(&self, block: &Block) -> MicroTari {
        let coinbase = self.emission_schedule().block_reward(block.header.height);
        coinbase + block.calculate_fees()
    }

    /// This is the currently configured chain network.
    pub fn network(&self) -> Network {
        self.inner.network
    }
}

/// This is the used to control all consensus values.
#[derive(Debug)]
struct ConsensusManagerInner {
    /// This is the inner struct used to control all consensus values.
    pub consensus_constants: Vec<ConsensusConstants>,
    /// The configured chain network.
    pub network: Network,
    /// The configuration for the emission schedule.
    pub emission: EmissionSchedule,
    /// This allows the user to set a custom Genesis block
    pub gen_block: Option<Block>,
}

/// Constructor for the consensus manager struct
pub struct ConsensusManagerBuilder {
    /// This is the inner struct used to control all consensus values.
    pub consensus_constants: Vec<ConsensusConstants>,
    /// The configured chain network.
    pub network: Network,
    /// This allows the user to set a custom Genesis block
    pub gen_block: Option<Block>,
}

impl ConsensusManagerBuilder {
    /// Creates a new ConsensusManagerBuilder with the specified network
    pub fn new(network: Network) -> Self {
        ConsensusManagerBuilder {
            consensus_constants: vec![],
            network,
            gen_block: None,
        }
    }

    /// Adds in a custom consensus constants to be used
    pub fn with_consensus_constants(mut self, consensus_constants: ConsensusConstants) -> Self {
        self.consensus_constants.push(consensus_constants);
        self
    }

    /// Adds in a custom block to be used. This will be overwritten if the network is anything else than localnet
    pub fn with_block(mut self, block: Block) -> Self {
        self.gen_block = Some(block);
        self
    }

    /// Builds a consensus manager
    pub fn build(mut self) -> ConsensusManager {
        if self.consensus_constants.is_empty() {
            self.consensus_constants =self.network.create_consensus_constants();
        }
        // TODO: Check that constants is not empty

        // Use the first constants for now.
        let emission = EmissionSchedule::new(
            self.consensus_constants[0].emission_initial,
            self.consensus_constants[0].emission_decay,
            self.consensus_constants[0].emission_tail,
        );
        let inner = ConsensusManagerInner {
            consensus_constants: self.consensus_constants,
            network: self.network,
            emission,
            gen_block: self.gen_block,
        };
        ConsensusManager { inner: Arc::new(inner) }
    }
}
