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
//

/// Utilities and helpers for building the base node instance
mod builder;
/// The command line interface definition and configuration
mod cli;
/// Application-specific constants
mod consts;
/// Miner lib Todo hide behind feature flag
mod miner;
/// Parser module used to control user commands
mod parser;

use crate::builder::{create_and_save_id, load_identity, BaseNodeContext};
use futures::stream::StreamExt;
use log::*;
use parser::Parser;
use rustyline::{config::OutputStreamType, error::ReadlineError, CompletionType, Config, EditMode, Editor};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use tari_common::{load_configuration, GlobalConfig};
use tokio::{runtime, runtime::Runtime};

pub const LOG_TARGET: &str = "base_node::app";

fn main() {
    cli::print_banner();
    // Create the tari data directory
    if let Err(e) = tari_common::dir_utils::create_data_directory() {
        println!(
            "We couldn't create a default Tari data directory and have to quit now. This makes us sad :(\n {}",
            e.to_string()
        );
        return;
    }

    // Parse and validate command-line arguments
    let arguments = cli::parse_cli_args();

    // Initialise the logger
    if !tari_common::initialize_logging(&arguments.bootstrap.log_config) {
        return;
    }

    // Load and apply configuration file
    let cfg = match load_configuration(&arguments.bootstrap) {
        Ok(cfg) => cfg,
        Err(s) => {
            error!(target: LOG_TARGET, "{}", s);
            return;
        },
    };

    // Populate the configuration struct
    let node_config = match GlobalConfig::convert_from(cfg) {
        Ok(c) => c,
        Err(e) => {
            error!(target: LOG_TARGET, "The configuration file has an error. {}", e);
            return;
        },
    };

    // Load or create the Node identity
    let node_id = match load_identity(&node_config.identity_file) {
        Ok(id) => id,
        Err(e) => {
            if !arguments.create_id {
                error!(
                    target: LOG_TARGET,
                    "Node identity information not found. {}. You can update the configuration file to point to a \
                     valid node identity file, or re-run the node with the --create_id flag to create anew identity.",
                    e
                );
                return;
            }
            debug!(target: LOG_TARGET, "Node id not found. {}. Creating new ID", e);
            match create_and_save_id(&node_config.identity_file, &node_config.address) {
                Ok(id) => {
                    info!(
                        target: LOG_TARGET,
                        "New node identity [{}] with public key {} has been created.",
                        id.node_id(),
                        id.public_key()
                    );
                    id
                },
                Err(e) => {
                    error!(target: LOG_TARGET, "Could not create new node id. {}.", e);
                    return;
                },
            }
        },
    };

    // Set up the Tokio runtime
    let mut rt = match setup_runtime(&node_config) {
        Ok(rt) => rt,
        Err(s) => {
            error!(target: LOG_TARGET, "{}", s);
            return;
        },
    };

    // Build, node, build!
    let (comms, node, mut miner, base_node_context) =
        match builder::configure_and_initialize_node(&node_config, node_id, &mut rt) {
            Ok(n) => n,
            Err(e) => {
                error!(target: LOG_TARGET, "Could not instantiate node instance. {}", e);
                return;
            },
        };
    let flag = node.get_flag();
    // lets run the miner
    let miner_handle = if true {
        let mut rx = miner.get_utxo_receiver_channel();
        let mut rx_events = node.get_state_change_event();
        miner.subscribe_to_state_change(rx_events);
        let mut wallet_output_handle = base_node_context.wallet_output_service.clone();
        rt.spawn(async move {
            while let Some(utxo) = rx.next().await {
                wallet_output_handle.add_output(utxo).await;
            }
        });
        Some(rt.spawn(async move {
            debug!(target: LOG_TARGET, "Starting miner");
            miner.mine().await;
            debug!(target: LOG_TARGET, "Miner has shutdown");
        }))
    } else {
        None
    };

    // Run, node, run!
    let main = async move {
        node.run().await;
        debug!(
            target: LOG_TARGET,
            "The node has finished all it's work. initiating Comms stack shutdown"
        );
        match comms.shutdown() {
            Ok(()) => info!(target: LOG_TARGET, "The comms stack reported a clean shutdown"),
            Err(e) => warn!(
                target: LOG_TARGET,
                "The comms stack did not shut down cleanly: {}",
                e.to_string()
            ),
        }
    };
    let base_node_handle = rt.spawn(main);

    cli_loop(flag, rt.handle().clone(), base_node_context);
    if let Some(miner) = miner_handle {
        rt.block_on(miner);
    }
    rt.block_on(base_node_handle);
    println!("Goodbye!");
}

fn setup_runtime(config: &GlobalConfig) -> Result<Runtime, String> {
    let num_core_threads = config.core_threads;
    let num_blocking_threads = config.blocking_threads;

    debug!(
        target: LOG_TARGET,
        "Configuring the node to run on {} core threads and {} blocking worker threads.",
        num_core_threads,
        num_blocking_threads
    );
    tokio::runtime::Builder::new()
        .threaded_scheduler()
        .enable_all()
        .max_threads(num_core_threads + num_blocking_threads)
        .core_threads(num_core_threads)
        .build()
        .map_err(|e| format!("There was an error while building the node runtime. {}", e.to_string()))
}

fn cli_loop(shutdown_flag: Arc<AtomicBool>, executor: runtime::Handle, base_node_context: BaseNodeContext) {
    let parser = Parser::new(executor, base_node_context, shutdown_flag.clone());
    let cli_config = Config::builder()
        .history_ignore_space(true)
        .completion_type(CompletionType::List)
        .edit_mode(EditMode::Emacs)
        .output_stream(OutputStreamType::Stdout)
        .build();
    let mut rustyline = Editor::with_config(cli_config);
    rustyline.set_helper(Some(parser));
    loop {
        let readline = rustyline.readline(">> ");
        match readline {
            Ok(line) => {
                rustyline.add_history_entry(line.as_str());
                rustyline.helper_mut().as_deref_mut().map(|p| p.handle_command(&line));
            },
            Err(ReadlineError::Interrupted) => {
                // shutdown section. Will shutdown all interfaces when ctrl-c was pressed
                println!("CTRL-C received");
                println!("Shutting down");
                info!(
                    target: LOG_TARGET,
                    "Termination signal received from user. Shutting node down."
                );
                shutdown_flag.store(true, Ordering::SeqCst);
                break;
            },
            Err(err) => {
                println!("Error: {:?}", err);
                break;
            },
        }
        if shutdown_flag.load(Ordering::Relaxed) {
            break;
        };
    }
}
