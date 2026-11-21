//! Rystem Database Rust client foundation.
//! This initial commit exposes typed wire framing, not an authenticated remote connection.
pub use rystemdb_protocol::{
    decode, encode, read_frame, write_frame, Frame, Kind, Operation, ProtocolError,
    HEADER_LEN, MAX_PAYLOAD_BYTES,
};
