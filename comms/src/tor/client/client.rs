// Copyright 2020, The Tari Project
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

use super::{
    commands,
    commands::{AddOnionFlag, AddOnionResponse, TorCommand},
    error::TorClientError,
    parsers,
    response::{ResponseLine, EVENT_CODE},
    types::{KeyBlob, KeyType, PortMapping},
    PrivateKey,
};
use crate::{
    compat::IoCompat,
    multiaddr::Multiaddr,
    transports::{TcpTransport, Transport},
};
use futures::{AsyncRead, AsyncWrite, SinkExt, StreamExt};
use std::{borrow::Cow, num::NonZeroU16};
use tokio_util::codec::{Framed, LinesCodec};

/// Client for the Tor control port.
///
/// See the [Tor Control Port Spec](https://gitweb.torproject.org/torspec.git/tree/control-spec.txt) for more details.
pub struct TorControlPortClient<TSocket> {
    framed: Framed<IoCompat<TSocket>, LinesCodec>,
}

impl TorControlPortClient<<TcpTransport as Transport>::Output> {
    /// Connect using TCP to the given address.
    pub async fn connect(addr: Multiaddr) -> Result<Self, TorClientError> {
        let mut tcp = TcpTransport::new();
        tcp.set_nodelay(true);
        let socket = tcp.dial(addr)?.await?;
        Ok(Self::new(socket))
    }
}

/// Represents tor control port authentication mechanisms
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Authentication {
    /// No control port authentication required
    None,
    /// A hashed password will be sent to authenticate
    HashedPassword(String),
}
impl Default for Authentication {
    fn default() -> Self {
        Authentication::None
    }
}

impl<TSocket> TorControlPortClient<TSocket>
where TSocket: AsyncRead + AsyncWrite + Unpin
{
    /// Create a new TorControlPortClient using the given socket
    pub fn new(socket: TSocket) -> Self {
        Self {
            framed: Framed::new(IoCompat::new(socket), LinesCodec::new()),
        }
    }

    /// Authenticate with the tor control port
    pub async fn authenticate(&mut self, authentication: &Authentication) -> Result<(), TorClientError> {
        match authentication {
            Authentication::None => {
                self.send_line("AUTHENTICATE".to_string()).await?;
            },
            Authentication::HashedPassword(passwd) => {
                self.send_line(format!("AUTHENTICATE \"{}\"", passwd.replace("\"", "\\\"")))
                    .await?;
            },
        }

        self.recv_ok().await?;

        Ok(())
    }

    /// The GETCONF command. Returns configuration keys matching the `conf_name`.
    pub async fn get_conf<'a>(&mut self, conf_name: &'a str) -> Result<Vec<Cow<'a, str>>, TorClientError> {
        let command = commands::get_conf(conf_name);
        self.request_response(command).await
    }

    /// The GETINFO command. Returns configuration keys matching the `conf_name`.
    pub async fn get_info<'a>(&mut self, key_name: &'a str) -> Result<Cow<'a, str>, TorClientError> {
        let command = commands::get_info(key_name);
        let mut response = self.request_response(command).await?;
        if response.len() == 0 {
            return Err(TorClientError::ServerNoResponse);
        }
        Ok(response.remove(0))
    }

    /// The ADD_ONION command, used to create onion hidden services.
    pub async fn add_onion_custom<P: Into<PortMapping>>(
        &mut self,
        key_type: KeyType,
        key_blob: KeyBlob<'_>,
        flags: Vec<AddOnionFlag>,
        port: P,
        num_streams: Option<NonZeroU16>,
    ) -> Result<AddOnionResponse, TorClientError>
    {
        let command = commands::AddOnion::new(key_type, key_blob, flags, port.into(), num_streams);
        self.request_response(command).await
    }

    /// The ADD_ONION command using a v2 key
    pub async fn add_onion_v2<P: Into<PortMapping>>(
        &mut self,
        flags: Vec<AddOnionFlag>,
        port: P,
        num_streams: Option<NonZeroU16>,
    ) -> Result<AddOnionResponse, TorClientError>
    {
        self.add_onion_custom(KeyType::New, KeyBlob::Rsa1024, flags, port, num_streams)
            .await
    }

    /// The ADD_ONION command using the 'best' key. The 'best' key is determined by the tor proxy. At the time of
    /// writing tor will select a Ed25519 key.
    pub async fn add_onion<P: Into<PortMapping>>(
        &mut self,
        flags: Vec<AddOnionFlag>,
        port: P,
        num_streams: Option<NonZeroU16>,
    ) -> Result<AddOnionResponse, TorClientError>
    {
        self.add_onion_custom(KeyType::New, KeyBlob::Best, flags, port, num_streams)
            .await
    }

    /// The ADD_ONION command using the given `PrivateKey`.
    pub async fn add_onion_from_private_key<P: Into<PortMapping>>(
        &mut self,
        private_key: &PrivateKey,
        flags: Vec<AddOnionFlag>,
        port: P,
        num_streams: Option<NonZeroU16>,
    ) -> Result<AddOnionResponse, TorClientError>
    {
        let (key_type, key_blob) = match private_key {
            PrivateKey::Rsa1024(key) => (KeyType::Rsa1024, KeyBlob::String(key)),
            PrivateKey::Ed25519V3(key) => (KeyType::Ed25519V3, KeyBlob::String(key)),
        };
        self.add_onion_custom(key_type, key_blob, flags, port, num_streams)
            .await
    }

    /// The DEL_ONION command.
    pub async fn del_onion(&mut self, service_id: &str) -> Result<(), TorClientError> {
        let command = commands::DelOnion::new(service_id);
        self.request_response(command).await
    }

    async fn request_response<T: TorCommand>(&mut self, command: T) -> Result<T::Output, TorClientError>
    where T::Error: Into<TorClientError> {
        self.send_line(command.to_command_string().map_err(Into::into)?).await?;
        let responses = self.recv_next_responses().await?;
        if responses.len() == 0 {
            return Err(TorClientError::ServerNoResponse);
        }
        let response = command.parse_responses(responses).map_err(Into::into)?;
        Ok(response)
    }

    async fn send_line(&mut self, line: String) -> Result<(), TorClientError> {
        self.framed.send(line).await.map_err(Into::into)
    }

    async fn recv_ok(&mut self) -> Result<(), TorClientError> {
        let line = self.receive_line().await?;
        let resp = parsers::response_line(&line)?;
        if resp.is_ok() {
            Ok(())
        } else {
            Err(TorClientError::TorCommandFailed(resp.value.into_owned()))
        }
    }

    async fn recv_next_responses(&mut self) -> Result<Vec<ResponseLine<'_>>, TorClientError> {
        let mut msgs = Vec::new();
        loop {
            let line = self.receive_line().await?;
            let mut msg = parsers::response_line(&line)?;
            // Ignore event codes (for now)
            if msg.code == EVENT_CODE {
                continue;
            }
            if msg.is_multiline {
                let lines = self.receive_multiline().await?;
                msg.value = Cow::from(format!("{}\n{}", msg.value, lines.join("\n")));
            }

            let has_more = msg.has_more();
            msgs.push(msg.into_owned());
            if !has_more {
                break;
            }
        }

        Ok(msgs)
    }

    async fn receive_line(&mut self) -> Result<String, TorClientError> {
        let line = self
            .framed
            .next()
            .await
            .ok_or_else(|| TorClientError::UnexpectedEof)??;

        Ok(line)
    }

    async fn receive_multiline(&mut self) -> Result<Vec<String>, TorClientError> {
        let mut lines = Vec::new();
        loop {
            let line = self.receive_line().await?;
            let trimmed = line.trim();
            if trimmed == "." {
                break;
            }
            lines.push(trimmed.to_string());
        }

        Ok(lines)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{
        memsocket::MemorySocket,
        tor::client::{test_server, test_server::canned_responses, types::PrivateKey},
    };
    use futures::future;
    use std::net::SocketAddr;
    use tari_test_utils::unpack_enum;

    async fn setup_test() -> (TorControlPortClient<MemorySocket>, test_server::State) {
        let (_, mock_state, socket) = test_server::spawn().await;
        let tor = TorControlPortClient::new(socket);
        (tor, mock_state)
    }

    #[tokio_macros::test]
    async fn connect() {
        let (mut listener, addr) = TcpTransport::default()
            .listen("/ip4/127.0.0.1/tcp/0".parse().unwrap())
            .unwrap()
            .await
            .unwrap();
        let (result_out, result_in) = future::join(TorControlPortClient::connect(addr), listener.next()).await;

        // Check that the connection is successfully made
        result_out.unwrap();
        result_in.unwrap().unwrap().0.await.unwrap();
    }

    #[tokio_macros::test]
    async fn authenticate() {
        let (mut tor, mock_state) = setup_test().await;

        tor.authenticate(&Authentication::None).await.unwrap();
        let mut req = mock_state.take_requests().await;
        assert_eq!(req.len(), 1);
        assert_eq!(req.remove(0), "AUTHENTICATE");

        tor.authenticate(&Authentication::HashedPassword("ab\"cde".to_string()))
            .await
            .unwrap();
        let mut req = mock_state.take_requests().await;
        assert_eq!(req.len(), 1);
        assert_eq!(req.remove(0), "AUTHENTICATE \"ab\\\"cde\"");
    }

    #[tokio_macros::test]
    async fn get_conf_ok() {
        let (mut tor, mock_state) = setup_test().await;

        mock_state
            .set_canned_response(canned_responses::GET_CONF_HIDDEN_SERVICE_PORT_OK)
            .await;

        let results = tor.get_conf("HiddenServicePort").await.unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0], "8080");
        assert_eq!(results[1], "8081 127.0.0.1:9000");
        assert_eq!(results[2], "8082 127.0.0.1:9001");
    }

    #[tokio_macros::test]
    async fn get_conf_err() {
        let (mut tor, mock_state) = setup_test().await;

        mock_state.set_canned_response(canned_responses::ERR_552).await;

        let err = tor.get_conf("HiddenServicePort").await.unwrap_err();
        unpack_enum!(TorClientError::TorCommandFailed(_s) = err);
    }

    #[tokio_macros::test]
    async fn get_info_multiline_kv_ok() {
        let (mut tor, mock_state) = setup_test().await;

        mock_state
            .set_canned_response(canned_responses::GET_INFO_NET_LISTENERS_OK)
            .await;

        let value = tor.get_info("net/listeners/socks").await.unwrap();
        assert_eq!(value, "127.0.0.1:9050");
    }

    #[tokio_macros::test]
    async fn get_info_kv_multiline_value_ok() {
        let (mut tor, mock_state) = setup_test().await;

        mock_state
            .set_canned_response(canned_responses::GET_INFO_ONIONS_DETACHED_OK)
            .await;

        let value = tor.get_info("onions/detached").await.unwrap();
        assert_eq!(value.split('\n').collect::<Vec<_>>(), [
            "mochz2xppfziim5olr5f6q27poc4vfob2xxxxxxxxxxxxxxxxxxxxxxx",
            "nhqdqym6j35rk7tdou4cdj4gjjqagimutxxxxxxxxxxxxxxxxxxxxxxx"
        ]);
    }

    #[tokio_macros::test]
    async fn get_info_err() {
        let (mut tor, mock_state) = setup_test().await;

        mock_state.set_canned_response(canned_responses::ERR_552).await;

        let err = tor.get_info("net/listeners/socks").await.unwrap_err();
        unpack_enum!(TorClientError::TorCommandFailed(_s) = err);
    }

    #[tokio_macros::test]
    async fn add_onion_from_private_key_ok() {
        let (mut tor, mock_state) = setup_test().await;

        mock_state
            .set_canned_response(canned_responses::ADD_ONION_RSA1024_OK)
            .await;

        let private_key = PrivateKey::Rsa1024("dummy-key".into());
        let response = tor
            .add_onion_from_private_key(&private_key, vec![], 8080, None)
            .await
            .unwrap();

        assert_eq!(response.service_id, "62q4tswkxp74dtn7");
        assert!(response.private_key.is_none());

        let request = mock_state.take_requests().await.pop().unwrap();
        assert_eq!(request, "ADD_ONION RSA1024:dummy-key Port=8080,127.0.0.1:8080");
    }

    #[tokio_macros::test]
    async fn add_onion_ok() {
        let (mut tor, mock_state) = setup_test().await;

        mock_state.set_canned_response(canned_responses::ADD_ONION_OK).await;

        let response = tor
            .add_onion_custom(
                KeyType::New,
                KeyBlob::Best,
                vec![],
                8080,
                Some(NonZeroU16::new(10u16).unwrap()),
            )
            .await
            .unwrap();

        assert_eq!(
            response.service_id,
            "qigbgbs4ue3ghbupsotgh73cmmkjrin2aprlyxsrnrvpmcmzy3g4wbid"
        );
        assert_eq!(
            response.private_key,
            Some(PrivateKey::Ed25519V3(
                "Pg3GEyssauPRW3jP6mHwKOxvl_fMsF0QsZC3DvQ8jZ9AxmfRvSP35m9l0vOYyOxkOqWM6ufjdYuM8Ae6cR2UdreG6".to_string()
            ))
        );

        let request = mock_state.take_requests().await.pop().unwrap();
        assert_eq!(request, "ADD_ONION NEW:BEST NumStreams=10 Port=8080,127.0.0.1:8080");
    }

    #[tokio_macros::test]
    async fn add_onion_discard_pk_ok() {
        let (mut tor, mock_state) = setup_test().await;

        mock_state
            .set_canned_response(canned_responses::ADD_ONION_DISCARDPK_OK)
            .await;

        let response = tor
            .add_onion_custom(
                KeyType::Rsa1024,
                KeyBlob::Rsa1024,
                vec![
                    AddOnionFlag::DiscardPK,
                    AddOnionFlag::Detach,
                    AddOnionFlag::BasicAuth,
                    AddOnionFlag::MaxStreamsCloseCircuit,
                    AddOnionFlag::NonAnonymous,
                ],
                PortMapping::new(8080, SocketAddr::from(([127u8, 0, 0, 1], 8081u16))),
                None,
            )
            .await
            .unwrap();

        assert_eq!(
            response.service_id,
            "qigbgbs4ue3ghbupsotgh73cmmkjrin2aprlyxsrnrvpmcmzy3g4wbid"
        );
        assert_eq!(response.private_key, None);

        let request = mock_state.take_requests().await.pop().unwrap();
        assert_eq!(
            request,
            "ADD_ONION RSA1024:RSA1024 Flags=DiscardPK,Detach,BasicAuth,MaxStreamsCloseCircuit,NonAnonymous \
             Port=8080,127.0.0.1:8081"
        );
    }

    #[tokio_macros::test]
    async fn add_onion_err() {
        let (mut tor, mock_state) = setup_test().await;

        mock_state.set_canned_response(canned_responses::ERR_552).await;

        let err = tor
            .add_onion_custom(KeyType::Ed25519V3, KeyBlob::Ed25519V3, vec![], 8080, None)
            .await
            .unwrap_err();

        unpack_enum!(TorClientError::TorCommandFailed(_s) = err);
    }

    #[tokio_macros::test]
    async fn del_onion_ok() {
        let (mut tor, mock_state) = setup_test().await;

        mock_state.set_canned_response(canned_responses::OK).await;

        tor.del_onion("some-fake-id").await.unwrap();

        let request = mock_state.take_requests().await.pop().unwrap();
        assert_eq!(request, "DEL_ONION some-fake-id");
    }

    #[tokio_macros::test]
    async fn del_onion_err() {
        let (mut tor, mock_state) = setup_test().await;

        mock_state.set_canned_response(canned_responses::ERR_552).await;

        tor.del_onion("some-fake-id").await.unwrap_err();

        let request = mock_state.take_requests().await.pop().unwrap();
        assert_eq!(request, "DEL_ONION some-fake-id");
    }
}
