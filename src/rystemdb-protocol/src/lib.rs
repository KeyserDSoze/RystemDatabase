//! Rystem Wire Protocol v1 (RSWP): bounded, versioned binary RPC framing.
//!
//! This crate is a *codec*, not a server or a security transport. Do NOT bind
//! an unauthenticated public endpoint to this format. TLS, auth and topology
//! are protocol-server/client features tracked separately.
//!
//! On the wire (all multi-byte integers BE):
//! magic[4]="RSWP", major=1, minor=0, kind[1], op[1],
//! status[2], header_len[2]=24, request_id[8], payload_len[4], payload[N].
pub mod crud;
use std::fmt;
use std::io::{self, Read, Write};

pub const HEADER_LEN: usize = 24;
pub const MAX_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;
const MAGIC: &[u8; 4] = b"RSWP";
const MAJOR: u8 = 1;
const MINOR: u8 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Request = 1,
    Response = 2,
}

impl TryFrom<u8> for Kind {
    type Error = ProtocolError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Request),
            2 => Ok(Self::Response),
            _ => Err(ProtocolError::InvalidKind),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Operation {
    Hello = 1,
    Ping = 2,
    Put = 16,
    Get = 17,
    Delete = 18,
}

impl TryFrom<u8> for Operation {
    type Error = ProtocolError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Hello),
            2 => Ok(Self::Ping),
            16 => Ok(Self::Put),
            17 => Ok(Self::Get),
            18 => Ok(Self::Delete),
            _ => Err(ProtocolError::InvalidOperation),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolError {
    Incomplete,
    InvalidMagic,
    UnsupportedVersion,
    InvalidKind,
    InvalidOperation,
    InvalidStatus,
    InvalidFlags,
    InvalidHeaderLength,
    TooLarge,
    TrailingBytes,
    Io,
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
    /// 0 for request/success; 1=not found, 2=invalid request, 3=busy,
    /// 4=internal, 5=unsupported version, 6=unauthorized, 7=redirect.
    pub status: u16,
    /// Opaque binary payload. Versioned CRUD and Hello schemas are specified
    /// in a separate protocol section; this layer never parses JSON.
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn request(operation: Operation, request_id: u64, payload: Vec<u8>) -> Self {
        Self { kind: Kind::Request, operation, request_id, status: 0, payload }
    }

    pub fn response(operation: Operation, request_id: u64, status: u16, payload: Vec<u8>) -> Self {
        Self { kind: Kind::Response, operation, request_id, status, payload }
    }
}

fn validate_status(kind: Kind, status: u16) -> Result<(), ProtocolError> {
    if (kind == Kind::Request && status != 0) || status > 7 {
        Err(ProtocolError::InvalidStatus)
    } else {
        Ok(())
    }
}

fn header_info(bytes: &[u8]) -> Result<(Kind, Operation, u16, u64, usize), ProtocolError> {
    if bytes.len() < HEADER_LEN {
        return Err(ProtocolError::Incomplete);
    }
    if &bytes[..4] != MAGIC { return Err(ProtocolError::InvalidMagic); }
    if bytes[4] != MAJOR || bytes[5] != MINOR {
        return Err(ProtocolError::UnsupportedVersion);
    }
    let kind = Kind::try_from(bytes[6])?;
    let operation = Operation::try_from(bytes[7])?;
    let status = u16::from_be_bytes([bytes[8], bytes[9]]);
    validate_status(kind, status)?;
    let header_len = u16::from_be_bytes([bytes[10], bytes[11]]);
    if header_len != HEADER_LEN as u16 {
        return Err(ProtocolError::InvalidHeaderLength);
    }
    let request_id = u64::from_be_bytes(bytes[12..20].try_into().expect("fixed 24-byte header"));
    let len = u32::from_be_bytes(bytes[20..24].try_into().expect("fixed 24-byte header")) as usize;
    if len > MAX_PAYLOAD_BYTES {
        return Err(ProtocolError::TooLarge);
    }
    Ok((kind, operation, status, request_id, len))
}

/// Encode one complete RSWP frame. No compression or encryption is performed.
pub fn encode(frame: &Frame) -> Result<Vec<u8>, ProtocolError> {
    validate_status(frame.kind, frame.status)?;
    let len = frame.payload.len();
    if len > MAX_PAYLOAD_BYTES {
        return Err(ProtocolError::TooLarge);
    }
    let mut out = Vec::with_capacity(HEADER_LEN + len);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&[MAJOR, MINOR, frame.kind as u8, frame.operation as u8]);
    out.extend_from_slice(&frame.status.to_be_bytes());
    out.extend_from_slice(&(HEADER_LEN as u16).to_be_bytes());
    out.extend_from_slice(&frame.request_id.to_be_bytes());
    out.extend_from_slice(&(len as u32).to_be_bytes());
    out.extend_from_slice(&frame.payload);
    Ok(out)
}

/// Decode exactly one complete frame; reject trailing bytes.
pub fn decode(bytes: &[u8]) -> Result<Frame, ProtocolError> {
    let (kind, operation, status, request_id, len) = header_info(bytes)?;
    let total = HEADER_LEN + len; // Max size is bounded by MAX_PAYLOAD_BYTES.
    if bytes.len() < total {
        return Err(ProtocolError::Incomplete);
    }
    if bytes.len() > total {
        return Err(ProtocolError::TrailingBytes);
    }
    Ok(Frame { kind, operation, status, request_id, payload: bytes[HEADER_LEN..].to_vec() })
}

/// Read one frame from a stream. The global 16 MiB body size is checked
/// BEFORE allocation, and concatenated frames can follow this one.
pub fn read_frame<R: Read>(reader: &mut R) -> Result<Frame, ProtocolError> {
    read_frame_with_limit(reader, MAX_PAYLOAD_BYTES)
}

/// #72: A caller may set a tighter framing budget (e.g. the tiny RSWP HELLO
/// before authentication) without weakening or duplicating ANY protocol
/// header validation. The declared body length is rejected from the 24-byte
/// header BEFORE allocating memory or attempting to read a single body byte.
/// A later authenticated frame may still use the standard 16 MiB limit.
pub fn read_frame_with_limit<R: Read>(
    reader: &mut R, max_payload: usize,
) -> Result<Frame, ProtocolError> {
    let mut header = [0_u8; HEADER_LEN];
    match reader.read_exact(&mut header) {
        Ok(()) => {}
        Err(err) => return Err(map_io(err)),
    }
    let (kind, operation, status, request_id, len) = header_info(&header)?;
    if len > max_payload {
        return Err(ProtocolError::TooLarge);
    }
    let mut payload = vec![0_u8; len];
    reader.read_exact(&mut payload).map_err(map_io)?;
    Ok(Frame { kind, operation, status, request_id, payload })
}

/// Send one RSWP frame, without an implicit fsync or flush call.
pub fn write_frame<W: Write>(writer: &mut W, frame: &Frame) -> Result<(), ProtocolError> {
    let bytes = encode(frame)?;
    writer.write_all(&bytes).map_err(map_io)
}

fn map_io(err: io::Error) -> ProtocolError {
    if err.kind() == io::ErrorKind::UnexpectedEof {
        ProtocolError::Incomplete
    } else {
        ProtocolError::Io
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_rejects_unknown_status_even_on_response() {
        let frame = Frame::response(Operation::Get, 1, 255, vec![]);
        assert_eq!(encode(&frame), Err(ProtocolError::InvalidStatus));
    }

    #[test]
    fn streaming_rejects_large_declared_length_before_body_alloc() {
        let mut head = encode(&Frame::request(Operation::Put, 1, Vec::new())).unwrap();
        head[20..24].copy_from_slice(&((MAX_PAYLOAD_BYTES + 1) as u32).to_be_bytes());
        assert_eq!(read_frame(&mut head.as_slice()), Err(ProtocolError::TooLarge));
    }
}
