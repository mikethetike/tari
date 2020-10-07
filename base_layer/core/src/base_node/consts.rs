// Copyright 2020. The Tari Project
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

use std::time::Duration;

/// The allocated waiting time for a request waiting for service responses from remote base nodes.
pub const BASE_NODE_SERVICE_REQUEST_TIMEOUT: Duration = Duration::from_secs(600);
/// The minimum allocated waiting time for a request waiting for service responses from remote base nodes.
pub const BASE_NODE_SERVICE_REQUEST_MIN_TIMEOUT: Duration = Duration::from_secs(10);
/// The allocated waiting time for a fetch blocks request waiting for service responses from remote base nodes.
pub const FETCH_BLOCKS_SERVICE_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// The minimum allocated waiting time for a fetch blocks request waiting for service responses from remote base nodes.
pub const FETCH_BLOCKS_SERVICE_REQUEST_MIN_TIMEOUT: Duration = Duration::from_secs(10);
/// The fraction of responses that need to be received for a corresponding service request to be finalize.
pub const BASE_NODE_SERVICE_DESIRED_RESPONSE_FRACTION: f32 = 0.6;
