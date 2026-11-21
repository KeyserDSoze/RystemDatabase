//! RED TDD skeleton. The persistent document segment store is not implemented yet.
//! Deliberately failing integration tests define the expected storage contract.
use rystemdb_core::{CodecError, Value};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum StoreError {
    Io(io::Error),
    Codec(CodecError),
    InvalidId,
    Corrupt(String),
    Unsupported,
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for StoreError {}

#[derive(Debug)]
pub struct DocumentStore {
    _root: PathBuf,
}

impl DocumentStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, StoreError> {
        std::fs::create_dir_all(&root).map_err(StoreError::Io)?;
        Ok(Self { _root: root.as_ref().to_path_buf() })
    }

    pub fn open_with_segment_limit(root: impl AsRef<Path>, _limit: u64) -> Result<Self, StoreError> {
        Self::open(root)
    }

    pub fn put(&mut self, _id: &str, _value: &Value) -> Result<(), StoreError> {
        Err(StoreError::Unsupported)
    }

    pub fn get(&self, _id: &str) -> Result<Option<Value>, StoreError> {
        Ok(None)
    }

    pub fn delete(&mut self, _id: &str) -> Result<bool, StoreError> {
        Ok(false)
    }

    pub fn len(&self) -> usize {
        0
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
