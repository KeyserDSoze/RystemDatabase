//! Experimental Rust client foundation.
//! RSWP v1 framing is tested; live local TCP methods are RED stubs for TDD.
pub use rystemdb_protocol::{
    decode, encode, read_frame, write_frame, Frame, Kind, Operation, ProtocolError,
    HEADER_LEN, MAX_PAYLOAD_BYTES,
};
use rystemdb_core::Value;
use std::net::SocketAddr;

#[derive(Debug)]
pub enum ClientError {
    Unavailable,
    RemoteForbidden,
}
impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ClientError {}

pub struct LocalClient;
impl LocalClient {
    pub fn connect(_addr: SocketAddr) -> Result<Self, ClientError> {
        Err(ClientError::Unavailable)
    }
    pub fn ping(&mut self) -> Result<(), ClientError> {
        Err(ClientError::Unavailable)
    }
    pub fn put(&mut self, _collection: &str, _id: &str, _value: &Value) -> Result<(), ClientError> {
        Err(ClientError::Unavailable)
    }
    pub fn get(&mut self, _collection: &str, _id: &str) -> Result<Option<Value>, ClientError> {
        Err(ClientError::Unavailable)
    }
    pub fn delete(&mut self, _collection: &str, _id: &str) -> Result<bool, ClientError> {
        Err(ClientError::Unavailable)
    }
}
