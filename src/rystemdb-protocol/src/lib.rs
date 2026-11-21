//! Wire protocol RED skeleton. A codec is not a network server.
use std::fmt;
use std::io::{Read, Write};

pub const HEADER_LEN: usize = 24;
pub const MAX_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind { Request = 1, Response = 2 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Operation { Hello = 1, Ping = 2, Put = 16, Get = 17, Delete = 18 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolError {
    Incomplete, InvalidMagic, UnsupportedVersion, InvalidKind,
    InvalidOperation, InvalidStatus, InvalidFlags, InvalidHeaderLength,
    TooLarge, TrailingBytes, Io,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{self:?}") }
}
impl std::error::Error for ProtocolError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub kind: Kind,
    pub operation: Operation,
    pub request_id: u64,
    pub status: u16,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn request(operation: Operation, request_id: u64, payload: Vec<u8>) -> Self {
        Self { kind: Kind::Request, operation, request_id, status: 0, payload }
    }
}

pub fn encode(_frame: &Frame) -> Result<Vec<u8>, ProtocolError> {
    Err(ProtocolError::InvalidOperation)
}
pub fn decode(_bytes: &[u8]) -> Result<Frame, ProtocolError> {
    Err(ProtocolError::InvalidMagic)
}
pub fn read_frame<R: Read>(_reader: &mut R) -> Result<Frame, ProtocolError> {
    Err(ProtocolError::Incomplete)
}
pub fn write_frame<W: Write>(_writer: &mut W, _frame: &Frame) -> Result<(), ProtocolError> {
    Err(ProtocolError::Io)
}
