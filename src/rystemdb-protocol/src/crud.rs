//! Bounded, borrowed RSWP v1 binary CRUD key body codec.
//!
//! Prefix: u16 BE collection byte length, ASCII collection, u16 BE ID byte
//! length, UTF-8 ID, then *opaque* typed RDB bytes for PUT. GET and DELETE
//! callers must require an empty document suffix. This module does NOT parse
//! document values or authenticate callers.
use crate::MAX_PAYLOAD_BYTES;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrudError {
    InvalidCollection,
    InvalidId,
    Truncated,
    TooLarge,
}

impl fmt::Display for CrudError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for CrudError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrudKey<'a> {
    pub collection: &'a str,
    pub id: &'a str,
    /// Unparsed binary bytes after collection+ID; only valid for a PUT request.
    pub document: &'a [u8],
}

fn collection_is_valid(s: &str) -> bool {
    (1..=128).contains(&s.len())
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

fn id_is_valid(collection_len: usize, id: &str) -> bool {
    !id.is_empty()
        && collection_len + 1 + id.len() <= 1024
        && !id.chars().any(char::is_control)
}

/// Produces one exact binary collection/ID prefix without an intermediate JSON
/// object. Append a complete typed RDB v1 document only for PUT requests.
pub fn encode_key(collection: &str, id: &str) -> Result<Vec<u8>, CrudError> {
    if !collection_is_valid(collection) {
        return Err(CrudError::InvalidCollection);
    }
    if !id_is_valid(collection.len(), id) {
        return Err(CrudError::InvalidId);
    }
    let coll_len = u16::try_from(collection.len()).map_err(|_| CrudError::InvalidCollection)?;
    let id_len = u16::try_from(id.len()).map_err(|_| CrudError::InvalidId)?;
    let mut out = Vec::with_capacity(4 + collection.len() + id.len());
    out.extend_from_slice(&coll_len.to_be_bytes());
    out.extend_from_slice(collection.as_bytes());
    out.extend_from_slice(&id_len.to_be_bytes());
    out.extend_from_slice(id.as_bytes());
    Ok(out)
}

/// Parse one bounded CRUD key prefix from an existing RSWP payload. No copies
/// or allocations occur; document suffix is returned as a byte slice.
/// Invalid UTF-8, collection names, and lengths are rejected before use.
pub fn decode_key(bytes: &[u8]) -> Result<CrudKey<'_>, CrudError> {
    if bytes.len() > MAX_PAYLOAD_BYTES {
        return Err(CrudError::TooLarge);
    }
    let coll_length = u16::from_be_bytes(bytes.get(..2).ok_or(CrudError::Truncated)?
        .try_into().map_err(|_| CrudError::Truncated)?) as usize;
    if !(1..=128).contains(&coll_length) {
        return Err(CrudError::InvalidCollection);
    }
    let collection_bytes = bytes.get(2..2 + coll_length).ok_or(CrudError::Truncated)?;
    let collection = std::str::from_utf8(collection_bytes).map_err(|_| CrudError::InvalidCollection)?;
    if !collection_is_valid(collection) {
        return Err(CrudError::InvalidCollection);
    }
    let id_start = 2 + coll_length;
    let id_length = u16::from_be_bytes(bytes.get(id_start..id_start + 2).ok_or(CrudError::Truncated)?
        .try_into().map_err(|_| CrudError::Truncated)?) as usize;
    if id_length == 0 || coll_length + 1 + id_length > 1024 {
        return Err(CrudError::InvalidId);
    }
    let begin = id_start + 2;
    let id_bytes = bytes.get(begin..begin + id_length).ok_or(CrudError::Truncated)?;
    let id = std::str::from_utf8(id_bytes).map_err(|_| CrudError::InvalidId)?;
    if !id_is_valid(coll_length, id) {
        return Err(CrudError::InvalidId);
    }
    Ok(CrudKey {
        collection,
        id,
        document: &bytes[begin + id_length..],
    })
}
