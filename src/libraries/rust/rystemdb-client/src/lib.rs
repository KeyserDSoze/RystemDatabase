//! Experimental Rystem Database **loopback-only synchronous** Rust SDK.
//!
//! Native RSWP v1 HELLO/PING/PUT/GET/DELETE over persistent TCP. No JSON,
//! and no dependency on the CLI process once connected.
//!
//! Security: optional local-only token HELLO; NO TLS. External IPs rejected before
//! TCP connection. This is a TDD proof-of-concept, not the future async SDK.
pub use rystemdb_protocol::{
    decode, encode, read_frame, write_frame, Frame, Kind, Operation, ProtocolError,
    HEADER_LEN, MAX_PAYLOAD_BYTES,
};
use rystemdb_core::{decode as decode_document, encode as encode_document, CodecError, Value};
use std::fmt;
use std::io;
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug)]
pub enum ClientError {
    Io(io::Error),
    Protocol(ProtocolError),
    Codec(CodecError),
    RemoteForbidden,
    InvalidKey,
    InvalidDocument,
    Rejected(u16),
    ResponseMismatch,
    RequestIdExhausted,
}
impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "local TCP I/O error: {err}"),
            Self::Protocol(err) => write!(f, "wire protocol error: {err}"),
            Self::Codec(err) => write!(f, "binary document error: {err}"),
            Self::RemoteForbidden => write!(f, "only localhost is supported without TLS and authentication"),
            Self::InvalidKey => write!(f, "invalid collection or document ID"),
            Self::InvalidDocument => write!(f, "PUT requires an object document"),
            Self::Rejected(status) => write!(f, "server returned RSWP status {status}"),
            Self::ResponseMismatch => write!(f, "server reply did not match the outstanding request"),
            Self::RequestIdExhausted => write!(f, "request ID counter exhausted"),
        }
    }
}
impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Protocol(err) => Some(err),
            Self::Codec(err) => Some(err),
            _ => None,
        }
    }
}
impl From<io::Error> for ClientError {
    fn from(err: io::Error) -> Self { Self::Io(err) }
}
impl From<ProtocolError> for ClientError {
    fn from(err: ProtocolError) -> Self { Self::Protocol(err) }
}
impl From<CodecError> for ClientError {
    fn from(err: CodecError) -> Self { Self::Codec(err) }
}

/// Single persistent TCP connection to an experimental *localhost only* Rystem
/// server. This is synchronous, not pooled, multi-connection or TLS capable.
pub struct LocalClient {
    stream: TcpStream,
    request_id: u64,
}

impl LocalClient {
    /// Connect only to numeric 127.0.0.1/::1, then negotiate RSWP 1.0.
    pub fn connect(addr: SocketAddr) -> Result<Self, ClientError> {
        Self::connect_inner(addr, None)
    }

    /// Authenticate a local RSWP 1.0 session with one 32-byte raw secret.
    /// The token is never placed on argv or logged. This is **not TLS**.
    pub fn connect_with_token(addr: SocketAddr, token: &[u8; 32]) -> Result<Self, ClientError> {
        Self::connect_inner(addr, Some(token))
    }

    fn connect_inner(addr: SocketAddr, token: Option<&[u8; 32]>) -> Result<Self, ClientError> {
        if !addr.ip().is_loopback() {
            return Err(ClientError::RemoteForbidden);
        }
        let stream = TcpStream::connect_timeout(&addr, TIMEOUT)?;
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;
        stream.set_nodelay(true)?;
        let mut client = Self { stream, request_id: 1 };
        let mut hello = vec![1, 0];
        if let Some(secret) = token {
            hello.extend_from_slice(&[0, 32]);
            hello.extend_from_slice(secret);
        }
        let frame = client.call(Operation::Hello, hello)?;
        if frame.status != 0 {
            return Err(ClientError::Rejected(frame.status));
        }
        if frame.payload != [1, 0] {
            return Err(ClientError::ResponseMismatch);
        }
        Ok(client)
    }

    /// Verify the open connection is still responsive; use the same socket.
    pub fn ping(&mut self) -> Result<(), ClientError> {
        let frame = self.call(Operation::Ping, Vec::new())?;
        if frame.status != 0 || !frame.payload.is_empty() {
            return Err(ClientError::Rejected(frame.status));
        }
        Ok(())
    }

    /// Persist one complete nested binary document; an error/timeout may have
    /// an *uncertain* write outcome. Do not blindly retry it.
    pub fn put(&mut self, collection: &str, id: &str, value: &Value) -> Result<(), ClientError> {
        if !matches!(value, Value::Object(_)) {
            return Err(ClientError::InvalidDocument);
        }
        let mut payload = encode_key(collection, id)?;
        payload.extend_from_slice(&encode_document(value)?);
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(ClientError::Protocol(ProtocolError::TooLarge));
        }
        let reply = self.call(Operation::Put, payload)?;
        if reply.status != 0 { return Err(ClientError::Rejected(reply.status)); }
        if !reply.payload.is_empty() { return Err(ClientError::ResponseMismatch); }
        Ok(())
    }

    pub fn get(&mut self, collection: &str, id: &str) -> Result<Option<Value>, ClientError> {
        let reply = self.call(Operation::Get, encode_key(collection, id)?)?;
        match reply.status {
            0 => Ok(Some(decode_document(&reply.payload)?)),
            1 if reply.payload.is_empty() => Ok(None),
            _ => Err(ClientError::Rejected(reply.status)),
        }
    }

    pub fn delete(&mut self, collection: &str, id: &str) -> Result<bool, ClientError> {
        let reply = self.call(Operation::Delete, encode_key(collection, id)?)?;
        if !reply.payload.is_empty() { return Err(ClientError::ResponseMismatch); }
        match reply.status {
            0 => Ok(true),
            1 => Ok(false),
            _ => Err(ClientError::Rejected(reply.status)),
        }
    }

    fn call(&mut self, op: Operation, payload: Vec<u8>) -> Result<Frame, ClientError> {
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(ClientError::Protocol(ProtocolError::TooLarge));
        }
        let id = self.request_id;
        self.request_id = id.checked_add(1).ok_or(ClientError::RequestIdExhausted)?;
        write_frame(&mut self.stream, &Frame::request(op, id, payload))?;
        let reply = read_frame(&mut self.stream)?;
        if reply.kind != Kind::Response || reply.request_id != id || reply.operation != op {
            return Err(ClientError::ResponseMismatch);
        }
        Ok(reply)
    }
}

/// Use the exact shared RSWP v1 key codec. No duplicate validation,
/// UTF-8 encoding or byte-length handling in the client.
fn encode_key(collection: &str, id: &str) -> Result<Vec<u8>, ClientError> {
    rystemdb_protocol::crud::encode_key(collection, id)
        .map_err(|_| ClientError::InvalidKey)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_octets_are_identical_to_server_contract() {
        assert_eq!(encode_key("users", "u42").unwrap(),
            b"\x00\x05users\x00\x03u42");
        for (collection, id) in [
            ("", "one"),
            ("bad/path", "one"),
            ("users", ""),
            ("users", "bad\nid"),
        ] {
            assert!(matches!(encode_key(collection, id), Err(ClientError::InvalidKey)));
        }
    }

    #[test]
    fn connection_is_explicitly_localhost_only() {
        for addr in ["0.0.0.0:8790", "198.51.100.10:8790", "[2001:db8::1]:8790"] {
            assert!(matches!(LocalClient::connect(addr.parse().unwrap()), Err(ClientError::RemoteForbidden)));
        }
    }
}
