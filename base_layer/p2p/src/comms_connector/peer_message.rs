// Copyright 2019, The Tari Project
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

use log::*;
use tari_comms::{peer_manager::Peer, types::CommsPublicKey};
use tari_comms_dht::{domain_message::MessageHeader, envelope::DhtMessageHeader};

const LOG_TARGET: &str = "comms::dht::requests::inbound";

/// A domain-level message
pub struct PeerMessage {
    /// The message envelope header
    pub dht_header: DhtMessageHeader,
    /// The connected peer which sent this message
    pub source_peer: Peer,
    /// Domain message header
    pub message_header: MessageHeader,
    /// This messages authenticated origin, otherwise None
    pub authenticated_origin: Option<CommsPublicKey>,
    /// Serialized message data
    pub body: Vec<u8>,
}

impl PeerMessage {
    pub fn decode_message<T>(&self) -> Result<T, prost::DecodeError>
    where T: prost::Message + Default {
        let msg = T::decode(self.body.as_slice())?;
        if cfg!(debug_assertions) {
            trace!(
                target: LOG_TARGET,
                "Inbound message: Peer:{}, DhtHeader:{},  {:?}",
                self.source_peer,
                self.dht_header,
                msg
            );
        }
        Ok(msg)
    }
}
