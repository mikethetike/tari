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

use super::LOG_TARGET;
use crate::builder::BaseNodeContext;
use log::*;
use rustyline::{
    completion::Completer,
    error::ReadlineError,
    hint::{Hinter, HistoryHinter},
    line_buffer::LineBuffer,
    Context,
};
use rustyline_derive::{Helper, Highlighter, Validator};
use std::{
    str::FromStr,
    string::ToString,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use strum::IntoEnumIterator;
use strum_macros::{Display, EnumIter, EnumString};
use tari_core::{
    tari_utilities::hex::Hex,
    transactions::tari_amount::{uT, MicroTari},
};
use tokio::runtime;
use tari_comms::peer_manager::NodeId;

/// Enum representing commands used by the basenode
#[derive(Clone, PartialEq, Debug, Display, EnumIter, EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum BaseNodeCommand {
    Help,
    GetBalance,
    SendTari,
    GetChainMetadata,
    Quit,
    Exit,
}

/// This is used to parse commands from the user and execute them
#[derive(Helper, Validator, Highlighter)]
pub struct Parser {
    executor: runtime::Handle,
    base_node_context: BaseNodeContext,
    shutdown_flag: Arc<AtomicBool>,
    commands: Vec<String>,
    hinter: HistoryHinter,
}

// This will go through all instructions and look for potential matches
impl Completer for Parser {
    type Candidate = String;

    fn complete(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> Result<(usize, Vec<String>), ReadlineError> {
        let mut completions: Vec<String> = Vec::new();
        for command in &self.commands {
            if command.starts_with(line) {
                completions.push(command.to_string());
            }
        }

        Ok((pos, completions))
    }

    fn update(&self, line: &mut LineBuffer, start: usize, elected: &str) {
        line.update(elected, start);
    }
}

// This allows us to make hints based on historic inputs
impl Hinter for Parser {
    fn hint(&self, line: &str, pos: usize, ctx: &rustyline::Context<'_>) -> Option<String> {
        self.hinter.hint(line, pos, ctx)
    }
}

impl Parser {
    /// creates a new parser struct
    pub fn new(executor: runtime::Handle, base_node_context: BaseNodeContext, shutdown_flag: Arc<AtomicBool>) -> Self {
        Parser {
            executor,
            base_node_context,
            shutdown_flag,
            commands: BaseNodeCommand::iter().map(|x| x.to_string()).collect(),
            hinter: HistoryHinter {},
        }
    }

    /// This will parse the provided command and execute the task
    pub fn handle_command(&mut self, command_str: &str) {
        let commands: Vec<&str> = command_str.split(' ').collect();
        let command = BaseNodeCommand::from_str(commands[0]);
        if command.is_err() {
            println!(
                "Received: {}, this is not a valid command, please enter a valid command",
                command_str
            );
            println!("Enter help or press tab for available commands");
            return;
        }
        let command = command.unwrap();
        let help_command = if commands.len() == 2 {
            Some(BaseNodeCommand::from_str(commands[1]).unwrap_or(BaseNodeCommand::Help))
        } else {
            None
        };
        if help_command != Some(BaseNodeCommand::Help) {
            return self.process_command(command, commands);
        }
        match command {
            BaseNodeCommand::Help => {
                println!("Available commands are: ");
                let joined = self.commands.join(", ");
                println!("{}", joined);
            },
            BaseNodeCommand::GetBalance => {
                println!("This command gets your balance");
            },
            BaseNodeCommand::SendTari => {
                println!("This command sends an amount of Tari to a address call this command via:");
                println!("send_tari [amount of tari to send] [public key to send to]");
            },
            BaseNodeCommand::GetChainMetadata => {
                println!("This command gets your base node chain meta data");
            },
            BaseNodeCommand::Exit | BaseNodeCommand::Quit => {
                println!("This command exits the base node");
            },
        }
    }

    // Function to process commands
    fn process_command(&mut self, command: BaseNodeCommand, command_arg: Vec<&str>) {
        match command {
            BaseNodeCommand::Help => {
                println!("Available commands are: ");
                let joined = self.commands.join(", ");
                println!("{}", joined);
            },
            BaseNodeCommand::GetBalance => {
                self.process_get_balance();
            },
            BaseNodeCommand::SendTari => {
                self.process_send_tari(command_arg);
            },
            BaseNodeCommand::GetChainMetadata => {
                self.process_get_chain_meta();
            },
            BaseNodeCommand::Exit | BaseNodeCommand::Quit => {
                println!("quit received");
                println!("Shutting down");
                info!(
                    target: LOG_TARGET,
                    "Termination signal received from user. Shutting node down."
                );
                self.shutdown_flag.store(true, Ordering::SeqCst);
            },
        }
    }

    // Function to process  the get balance command
    fn process_get_balance(&mut self) {
        let mut handler = self.base_node_context.wallet_output_service.clone();
        self.executor.spawn(async move {
            match handler.get_balance().await {
                Err(e) => {
                    println!("Something went wrong");
                    warn!(target: LOG_TARGET, "Error communicating with wallet: {}", e.to_string(),);
                    return;
                },
                Ok(data) => println!("Current balance is: {}", data),
            };
        });
    }

    // Function to process  the get chain meta data
    fn process_get_chain_meta(&mut self) {
        let mut handler = self.base_node_context.node_service.clone();
        self.executor.spawn(async move {
            match handler.get_metadata().await {
                Err(e) => {
                    println!("Something went wrong");
                    warn!(
                        target: LOG_TARGET,
                        "Error communicating with base node: {}",
                        e.to_string(),
                    );
                    return;
                },
                Ok(data) => println!("Current meta data is is: {}", data),
            };
        });
    }

    // Function to process  the send transaction function
    fn process_send_tari(&mut self, command_arg: Vec<&str>) {
        if command_arg.len() != 3 {
            println!("Command entered wrong, please enter in the following format: ");
            println!("send_tari [amount of tari to send] [public key to send to]");
            return;
        }
        let amount = command_arg[1].parse::<u64>();
        if amount.is_err() {
            println!("please enter a valid amount of tari");
            return;
        }
        let amount: MicroTari = amount.unwrap().into();
        let mut dest_node_id = NodeId::from_hex(command_arg[2]);
        if dest_node_id.is_err() {
            println!("please enter a valid destination pub_key");
            return;
        }
        let node_id = dest_node_id.unwrap();
        let fee_per_gram = 25 * uT;
        let mut handler = self.base_node_context.wallet_transaction_service.clone();
        self.executor.spawn(async move {
            match handler
                .send_transaction(
                    node_id.clone(),
                    amount,
                    fee_per_gram,
                    "coinbase reward from mining".into(),
                )
                .await
            {
                Err(e) => {
                    println!("Something went wrong sending funds");
                    println!("{:?}", e);
                    warn!(target: LOG_TARGET, "Error communicating with wallet: {}", e.to_string(),);
                    return;
                },
                Ok(_) => println!("Send {} Tari to {} ", amount, node_id.clone()),
            };
        });
    }
}
