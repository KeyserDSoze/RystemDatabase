//! Provisional RSWP v1 CRUD key body encoder/decoder (TDD RED stub).
//! Network frame envelope is separately defined in crate root.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrudError {
    InvalidCollection,
    InvalidId,
    Truncated,
    TooLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrudKey<'a> {
    pub collection: &'a str,
    pub id: &'a str,
    pub document: &'a [u8],
}

pub fn encode_key(_collection: &str, _id: &str) -> Result<Vec<u8>, CrudError> {
    Err(CrudError::InvalidId)
}

pub fn decode_key(_bytes: &[u8]) -> Result<CrudKey<'_>, CrudError> {
    Err(CrudError::Truncated)
}
