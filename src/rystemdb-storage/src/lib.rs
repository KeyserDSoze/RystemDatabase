//! Experimental single-process append-only document storage.
//!
//! A durable PUT/DELETE operation is a *single* checksummed frame that is synced
//! before the in-memory ID -> segment/offset index is updated. On restart the
//! index is rebuilt from the log; only an incomplete frame in the LAST segment
//! can be truncated. Complete corrupt frames and damage to older segments fail
//! closed. This is not yet a multi-process, multi-document ACID engine.
use rystemdb_core::{decode, encode, CodecError, Value};
use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"RYSF";
const VERSION: u8 = 1;
const PUT: u8 = 1;
const DELETE: u8 = 2;
// #82: One authenticated-by-CRC physical record for an entire mutation group.
// Older binaries fail closed on this new opcode; legacy PUT/DELETE unchanged.
const BATCH: u8 = 3;
const MAX_BATCH_OPERATIONS: usize = 1024;
const HEADER_SIZE: usize = 20;
const MAX_ID_BYTES: usize = 1024;
const MAX_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_SEGMENT_LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Debug)]
pub enum StoreError {
    Io(io::Error),
    Codec(CodecError),
    InvalidId,
    InvalidSegmentLimit,
    InvalidBatch,
    Locked,
    /// A key changed since the supplied committed snapshot, or it was absent
    /// at capture time (absent-key ABA is intentionally unsupported).
    Conflict,
    /// The snapshot belongs to a different storage root/path.
    InvalidSnapshot,
    Corrupt(String),
    Poisoned,
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Io(err) => write!(f, "storage I/O error: {err}"),
            StoreError::Codec(err) => write!(f, "document codec error: {err}"),
            StoreError::InvalidId => write!(f, "document ID must be 1..=1024 UTF-8 bytes"),
            StoreError::InvalidSegmentLimit => write!(f, "segment limit must be at least 64 bytes"),
            StoreError::InvalidBatch => write!(f, "batch must contain 2..=1024 unique, valid mutations"),
            StoreError::Locked => write!(f, "database directory is already open for writing"),
            StoreError::Conflict => write!(f, "document version conflict: snapshot is stale or key absent"),
            StoreError::InvalidSnapshot => write!(f, "snapshot belongs to a different database root"),
            StoreError::Corrupt(message) => write!(f, "corrupt segment: {message}"),
            StoreError::Poisoned => write!(f, "store write failed; close and reopen to recover"),
        }
    }
}
impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StoreError::Io(err) => Some(err),
            StoreError::Codec(err) => Some(err),
            _ => None,
        }
    }
}
impl From<io::Error> for StoreError {
    fn from(error: io::Error) -> Self { Self::Io(error) }
}
impl From<CodecError> for StoreError {
    fn from(error: CodecError) -> Self { Self::Codec(error) }
}

/// One operation in an experimental single-frame, multi-document commit.
/// All keys must be distinct, and DELETE requires an existing snapshot key.
/// A PUT may create or overwrite a key if its snapshot version is unchanged.
#[derive(Clone, Copy, Debug)]
pub enum BatchMutation<'a> {
    Put { id: &'a str, value: &'a Value },
    Delete { id: &'a str },
}

impl BatchMutation<'_> {
    fn id(&self) -> &str {
        match self {
            Self::Put { id, .. } | Self::Delete { id } => id,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Location {
    segment: u64,
    offset: u64,
}

/// Experimental immutable *read-only* view of the ID index at one point in
/// time. It owns no writer lock and copies only ID -> segment/offset metadata.
/// The underlying RYSF records remain on disk; each get verifies CRC+codec.
///
/// # Limits
/// The live index contains current values; a second last-committed-version
/// ledger also retains tombstone locations (#81), so snapshot capture clones
/// O(distinct IDs ever seen) metadata. No compaction is implemented, so
/// historical frames remain intact during normal operation.
/// This is NOT a transaction, MVCC manager or persisted snapshot handle:
/// external filesystem changes or future segment deletion can invalidate
/// old offsets. No cross-document atomic commit/write-conflict guarantees.
#[derive(Debug)]
pub struct DocumentSnapshot {
    root: PathBuf,
    entries: BTreeMap<String, Location>,
    // Latest acknowledged RYSF position for ALL known IDs, including
    // tombstones. Replayed from the log; enables absent-key ABA checks.
    versions: BTreeMap<String, Location>,
}

impl DocumentSnapshot {
    /// Read the document version visible when this view was captured.
    /// Absence, explicit RDB null and historical tombstones stay distinct.
    pub fn get(&self, id: &str) -> Result<Option<Value>, StoreError> {
        read_indexed(&self.root, &self.entries, id)
    }

    pub fn len(&self) -> usize { self.entries.len() }

    pub fn is_empty(&self) -> bool { self.entries.is_empty() }
}

#[derive(Debug)]
pub struct DocumentStore {
    root: PathBuf,
    active: File,
    active_id: u64,
    segment_limit: u64,
    entries: BTreeMap<String, Location>,
    // Never remove a tombstoned ID from this committed version ledger.
    // Distinct historical IDs consume metadata until a future GC scheme.
    versions: BTreeMap<String, Location>,
    poisoned: bool,
    // Last field drops last. The guard unlocks explicitly even on early
    // initialization errors; the OS is the fallback on abrupt process death.
    _writer_lock: WriterLockGuard,
}

impl DocumentStore {
    /// Open an experimental *single-writer* store, rebuilding its ID index.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, StoreError> {
        Self::open_with_segment_limit(root, DEFAULT_SEGMENT_LIMIT)
    }

    /// Same as open, with an optional small limit for bounded files or tests.
    pub fn open_with_segment_limit(root: impl AsRef<Path>, limit: u64) -> Result<Self, StoreError> {
        if limit < 64 {
            return Err(StoreError::InvalidSegmentLimit);
        }
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        // Acquire before scanning/creating segments. A persistent lock *file* is
        // intentional: the ownership is the OS advisory lock, not its existence.
        // std::fs::File::try_lock requires Rust 1.89+.
        let writer_lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false) // retain the inode: unlinking/truncating a lockfile is unsafe
            .open(root.join("rystemdb.writer.lock"))?;
        match writer_lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(StoreError::Locked),
            Err(TryLockError::Error(err)) => return Err(StoreError::Io(err)),
        }
        // #66: Every subsequent ? can return early (corrupt segments,
        // truncated historical frames, replay I/O). A plain File close was
        // observed leaving a transient Locked result on macOS after one
        // failed replay open. Guard the *acquired* handle immediately so it
        // explicitly unlocks even if DocumentStore is never constructed.
        let writer_lock = WriterLockGuard(writer_lock);
        let mut ids = Vec::new();
        for dir_entry in fs::read_dir(&root)? {
            let dir_entry = dir_entry?;
            let name = dir_entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("segment-") {
                let id = parse_segment_name(&name).ok_or_else(|| {
                    StoreError::Corrupt(format!("unrecognized segment filename: {name}"))
                })?;
                if !dir_entry.file_type()?.is_file() {
                    return Err(StoreError::Corrupt(format!("segment {id} is not a regular file")));
                }
                ids.push(id);
            }
        }
        ids.sort_unstable();
        for (index, &id) in ids.iter().enumerate() {
            if id != index as u64 + 1 {
                return Err(StoreError::Corrupt("missing or duplicated segment in sequence".into()));
            }
        }
        if ids.is_empty() {
            create_segment(&root, 1)?;
            ids.push(1);
        }

        let mut entries = BTreeMap::new();
        let mut versions = BTreeMap::new();
        let active_id = *ids.last().expect("at least one segment exists");
        for id in &ids {
            let path = segment_path(&root, *id);
            let mut file = OpenOptions::new().read(true).write(true).open(path)?;
            let length = file.metadata()?.len();
            let mut offset = 0_u64;
            while offset < length {
                match read_frame(&mut file, offset, length)? {
                    FrameRead::Incomplete if *id == active_id => {
                        file.set_len(offset)?;
                        file.sync_all()?;
                        break;
                    }
                    FrameRead::Incomplete => {
                        return Err(StoreError::Corrupt(format!(
                            "incomplete frame at segment {id} offset {offset} (not final segment)"
                        )));
                    }
                    FrameRead::Complete { id: document_id, value, length: frame_len } => {
                        let location = Location { segment: *id, offset };
                        if value.is_some() {
                            entries.insert(document_id.clone(), location);
                        } else {
                            entries.remove(&document_id);
                        }
                        // #81: Every valid committed frame, including DELETE,
                        // advances the last-known version.
                        versions.insert(document_id, location);
                        offset = offset.checked_add(frame_len).ok_or_else(|| {
                            StoreError::Corrupt("segment offset overflow".into())
                        })?;
                    }
                    FrameRead::Batch { changes, length: frame_len } => {
                        // #82: validate CRC and *all* members before replay
                        // publishes any of the batch. All keys share the
                        // same (segment,frame offset) version token.
                        let location = Location { segment: *id, offset };
                        for (document_id, value) in changes {
                            if value.is_some() {
                                entries.insert(document_id.clone(), location);
                            } else {
                                entries.remove(&document_id);
                            }
                            versions.insert(document_id, location);
                        }
                        offset = offset.checked_add(frame_len).ok_or_else(|| {
                            StoreError::Corrupt("segment offset overflow".into())
                        })?;
                    }
                }
            }
        }

        let active = OpenOptions::new()
            .read(true)
            .append(true)
            .open(segment_path(&root, active_id))?;
        Ok(Self {
            root, active, active_id, segment_limit: limit, entries, versions, poisoned: false,
            _writer_lock: writer_lock,
        })
    }

    /// PUT replaces any existing document under the same ID. Success means
    /// the complete frame has been synced before the method returned.
    pub fn put(&mut self, id: &str, value: &Value) -> Result<(), StoreError> {
        self.ensure_healthy()?;
        validate_id(id)?;
        let encoded = encode(value)?;
        let frame = frame(id, Some(&encoded))?;
        let location = self.append(&frame)?;
        self.entries.insert(id.to_owned(), location);
        self.versions.insert(id.to_owned(), location);
        Ok(())
    }

    /// Read and verify the current document version (CRC + typed RDB codec).
    pub fn get(&self, id: &str) -> Result<Option<Value>, StoreError> {
        read_indexed(&self.root, &self.entries, id)
    }

    /// Capture a read-only, committed ID→offset view. Later PUT/DELETE
    /// operations change only the writer's current index and append new
    /// records; this snapshot continues to address the original versions.
    ///
    /// Snapshot capture clones O(distinct IDs ever seen) metadata, NOT payloads.
    /// This is a limited local read-view primitive, not multi-document ACID.
    pub fn snapshot(&self) -> DocumentSnapshot {
        DocumentSnapshot {
            root: self.root.clone(),
            entries: self.entries.clone(),
            versions: self.versions.clone(),
        }
    }

    /// Insert an ABSENT document only if its last committed generation,
    /// including a previous DELETE tombstone, still matches a snapshot
    /// captured from this database root.
    ///
    /// The snapshot must have observed the ID absent; a snapshot of a
    /// present document is not an insert token, even after deletion.
    /// A concurrent PUT or PUT+DELETE changes the last-known RYSF offset,
    /// so both return Conflict without an append. Only the single writer
    /// can commit, and its check+append are serialized by &mut self.
    /// No global transactions or per-key version GC are implied.
    pub fn insert_if_unchanged(
        &mut self, snapshot: &DocumentSnapshot, id: &str, value: &Value,
    ) -> Result<(), StoreError> {
        self.ensure_healthy()?;
        validate_id(id)?;
        if self.root != snapshot.root {
            return Err(StoreError::InvalidSnapshot);
        }
        if snapshot.entries.contains_key(id)
            || self.entries.contains_key(id)
            || snapshot.versions.get(id) != self.versions.get(id)
        {
            return Err(StoreError::Conflict);
        }
        self.put(id, value)
    }

    /// Commit a bounded multi-document change set as ONE checksummed,
    /// synced RYSF frame, or commit none of it. Every key's last committed
    /// generation must match the supplied snapshot; missing-key DELETEs,
    /// duplicate IDs and malformed/overbudget batches fail before append.
    /// Unrelated keys may have changed. Values and frame bytes are completely
    /// serialized before the single append; index/version changes publish
    /// only after sync_data succeeds. A crash after an unsynced complete
    /// frame may replay the WHOLE batch or none, never a subset.
    ///
    /// This is *not* SQL/serializable transaction isolation, multi-writer
    /// concurrency, a multi-frame transaction, replication or MVCC GC.
    pub fn commit_batch(
        &mut self, snapshot: &DocumentSnapshot, mutations: &[BatchMutation<'_>],
    ) -> Result<(), StoreError> {
        use std::collections::BTreeSet;
        self.ensure_healthy()?;
        if self.root != snapshot.root {
            return Err(StoreError::InvalidSnapshot);
        }
        if !(2..=MAX_BATCH_OPERATIONS).contains(&mutations.len()) {
            return Err(StoreError::InvalidBatch);
        }
        let mut seen = BTreeSet::new();
        for mutation in mutations {
            let id = mutation.id();
            validate_id(id)?;
            if !seen.insert(id) {
                return Err(StoreError::InvalidBatch);
            }
            if self.versions.get(id) != snapshot.versions.get(id) {
                return Err(StoreError::Conflict);
            }
            if matches!(mutation, BatchMutation::Delete { .. })
                && (!snapshot.entries.contains_key(id) || !self.entries.contains_key(id))
            {
                return Err(StoreError::Conflict);
            }
        }

        // Build a single bounded frame and owned publication set before any
        // disk I/O; precondition failures leave both log and indexes untouched.
        let encoded = frame_batch(mutations)?;
        let changes = mutations.iter().map(|m| (
            m.id().to_owned(), matches!(m, BatchMutation::Put { .. }),
        )).collect::<Vec<_>>();
        let location = self.append(&encoded)?;
        for (id, is_put) in changes {
            if is_put {
                self.entries.insert(id.clone(), location);
            } else {
                self.entries.remove(&id);
            }
            self.versions.insert(id, location);
        }
        Ok(())
    }

    /// Replace an EXISTING committed document only if its version still
    /// matches a snapshot captured from this database root. This compares the
    /// actual append-only (segment,offset) version, not the document's bytes.
    /// A stale, deleted or snapshot-absent key returns Conflict **before**
    /// encoding or writing any data. A foreign root returns InvalidSnapshot.
    ///
    /// The check and append are both performed under the same exclusive
    /// &mut self writer. This is a single-key optimistic guard, NOT a
    /// multi-document transaction or MVCC GC. Absent-key CAS is a separate
    /// `insert_if_unchanged` operation backed by tombstone generations.
    pub fn replace_if_unchanged(
        &mut self, snapshot: &DocumentSnapshot, id: &str, value: &Value,
    ) -> Result<(), StoreError> {
        self.check_unchanged_existing(snapshot, id)?;
        self.put(id, value)
    }

    /// Append a durable tombstone only if this existing document is still
    /// at the exact version observed by the supplied committed snapshot.
    /// Conflicts are rejected before any disk I/O or index mutation.
    pub fn delete_if_unchanged(
        &mut self, snapshot: &DocumentSnapshot, id: &str,
    ) -> Result<(), StoreError> {
        self.check_unchanged_existing(snapshot, id)?;
        self.delete(id).map(|_| ())
    }

    /// Write an append-only tombstone; returns false without writing if absent.
    pub fn delete(&mut self, id: &str) -> Result<bool, StoreError> {
        self.ensure_healthy()?;
        validate_id(id)?;
        if !self.entries.contains_key(id) {
            return Ok(false);
        }
        let frame = frame(id, None)?;
        let location = self.append(&frame)?;
        self.entries.remove(id);
        self.versions.insert(id.to_owned(), location);
        Ok(true)
    }

    pub fn len(&self) -> usize { self.entries.len() }

    pub fn is_empty(&self) -> bool { self.entries.is_empty() }

    // These replacement/deletion preconditions require a key PRESENT at
    // snapshot time and the same root/path. Absent-key optimistic insertion
    // has its own stricter check against the committed tombstone ledger.
    //
    // &mut self prevents another cooperative writer from interleaving between
    // this test and the append; no reads, record allocations or disk writes
    // are performed until the precondition has succeeded.
    fn check_unchanged_existing(
        &self, snapshot: &DocumentSnapshot, id: &str,
    ) -> Result<(), StoreError> {
        self.ensure_healthy()?;
        validate_id(id)?;
        if self.root != snapshot.root {
            return Err(StoreError::InvalidSnapshot);
        }
        let Some(expected) = snapshot.entries.get(id) else {
            return Err(StoreError::Conflict);
        };
        if self.entries.get(id) != Some(expected) {
            return Err(StoreError::Conflict);
        }
        Ok(())
    }

    // Treat a failed fsync/write/segment creation as an uncertain operation:
    // no further mutation, including an absent-id deletion, can succeed.
    fn ensure_healthy(&self) -> Result<(), StoreError> {
        if self.poisoned { Err(StoreError::Poisoned) } else { Ok(()) }
    }

    fn append(&mut self, frame: &[u8]) -> Result<Location, StoreError> {
        if self.poisoned {
            return Err(StoreError::Poisoned);
        }
        let result = self.try_append(frame);
        if result.is_err() {
            // A failed write or fsync has an uncertain outcome. Prevent further
            // writes until the caller reopens the store and replays its log.
            self.poisoned = true;
        }
        result
    }

    fn try_append(&mut self, frame: &[u8]) -> Result<Location, StoreError> {
        // Fault hooks are compiled ONLY for explicitly feature-enabled tests.
        // Production/default builds do not read env vars or call exit().
        #[cfg(feature = "fault-injection")]
        fault_if("before_append");
        #[cfg(feature = "fault-injection")]
        inject_io_if("io_before_append")?;
        let offset = self.active.metadata()?.len();
        if offset > 0 && offset.saturating_add(frame.len() as u64) > self.segment_limit {
            let next = self.active_id.checked_add(1).ok_or_else(|| {
                StoreError::Corrupt("segment counter overflow".into())
            })?;
            if next > 99_999_999 {
                return Err(StoreError::Corrupt("segment counter exhausted".into()));
            }
            #[cfg(feature = "fault-injection")]
            inject_io_if("io_segment_create")?;
            let new_active = create_segment(&self.root, next)?;
            self.active = new_active;
            self.active_id = next;
            #[cfg(feature = "fault-injection")]
            fault_if("after_rotate");
        }
        let offset = self.active.metadata()?.len();
        #[cfg(feature = "fault-injection")]
        if test_fault_is("partial_frame") {
            // Persist an intentionally torn payload, then exit immediately
            // without destructors. Only test builds can take this path.
            self.active.write_all(&frame[..frame.len() / 2])?;
            self.active.sync_data()?;
            std::process::exit(97);
        }
        #[cfg(feature = "fault-injection")]
        if test_io_fault_is("io_short_write") {
            // Deterministic short-write simulation: persist a truncated record,
            // then return an I/O error without updating in-memory indexes.
            self.active.write_all(&frame[..frame.len() / 2])?;
            self.active.sync_data()?;
            inject_io_if("io_short_write")?;
        }
        self.active.write_all(frame)?;
        #[cfg(feature = "fault-injection")]
        fault_if("after_frame_before_sync");
        #[cfg(feature = "fault-injection")]
        inject_io_if("io_sync_data")?;
        self.active.sync_data()?;
        #[cfg(feature = "fault-injection")]
        fault_if("after_sync_before_publish");
        #[cfg(feature = "fault-injection")]
        inject_io_if("io_after_sync")?;
        Ok(Location { segment: self.active_id, offset })
    }
}

// Centralize the verified disk read shared by mutable current-index access
// and read-only historical snapshots. Every read reopens the original frame
// and revalidates ID, checksum and binary codec; snapshots never cache values.
fn read_indexed(
    root: &Path, entries: &BTreeMap<String, Location>, id: &str,
) -> Result<Option<Value>, StoreError> {
    let Some(location) = entries.get(id) else { return Ok(None); };
    let mut file = File::open(segment_path(root, location.segment))?;
    let size = file.metadata()?.len();
    match read_frame(&mut file, location.offset, size)? {
        FrameRead::Complete { id: stored_id, value: Some(value), .. } if stored_id == id => {
            Ok(Some(value))
        }
        FrameRead::Batch { changes, .. } => {
            changes.into_iter().find_map(|(key, value)| {
                if key == id { value } else { None }
            }).map(Some).ok_or_else(|| StoreError::Corrupt(format!(
                "indexed batch record {id} missing or deleted at segment {} offset {}",
                location.segment, location.offset
            )))
        }
        _ => Err(StoreError::Corrupt(format!(
            "indexed record {id} could not be verified at segment {} offset {}",
            location.segment, location.offset
        ))),
    }
}

#[cfg(feature = "fault-injection")]
fn test_fault_is(point: &str) -> bool {
    std::env::var("RYSTEMDB_TEST_CRASH_POINT").as_deref() == Ok(point)
}

#[cfg(feature = "fault-injection")]
fn fault_if(point: &str) {
    if test_fault_is(point) {
        // This deliberately bypasses unwinding and Drop. It models a killed
        // process, NOT sudden power loss or failed physical disk persistence.
        std::process::exit(97);
    }
}

// Synthetic I/O failures are compiled out of normal and release builds.
// They emulate error *returns*, not physical power-loss effects.
#[cfg(feature = "fault-injection")]
fn test_io_fault_is(point: &str) -> bool {
    std::env::var("RYSTEMDB_TEST_IO_ERROR").as_deref() == Ok(point)
}

#[cfg(feature = "fault-injection")]
fn inject_io_if(point: &str) -> Result<(), StoreError> {
    if test_io_fault_is(point) {
        return Err(StoreError::Io(io::Error::other(format!(
            "injected I/O failure at {point}"
        ))));
    }
    Ok(())
}

// #66: The lock's lifetime begins *before* replay, and must not depend
// on successfully constructing DocumentStore. This guard's Drop runs after
// both a normal store close and any early-Err from the open/replay path.
// Keep the lock-file inode (do not remove it) and drop the guard last so
// other open segment handles are already closed before releasing ownership.
#[derive(Debug)]
struct WriterLockGuard(File);

impl Drop for WriterLockGuard {
    fn drop(&mut self) {
        if let Err(err) = self.0.unlock() {
            // Never panic in Drop. File close is the fallback; log no
            // sensitive data and do not pretend that unlock succeeded.
            eprintln!("Rystem Database: error unlocking writer lock: {err}");
        }
    }
}

fn validate_id(id: &str) -> Result<(), StoreError> {
    if id.is_empty() || id.len() > MAX_ID_BYTES {
        Err(StoreError::InvalidId)
    } else {
        Ok(())
    }
}

fn segment_path(root: &Path, segment_id: u64) -> PathBuf {
    root.join(format!("segment-{segment_id:08}.rdb"))
}

fn parse_segment_name(name: &str) -> Option<u64> {
    let number = name.strip_prefix("segment-")?.strip_suffix(".rdb")?;
    if number.len() != 8 || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let id: u64 = number.parse().ok()?;
    (id > 0).then_some(id)
}

fn create_segment(root: &Path, id: u64) -> Result<File, StoreError> {
    let file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .append(true)
        .open(segment_path(root, id))?;
    #[cfg(feature = "fault-injection")]
    inject_io_if("io_segment_sync")?;
    file.sync_all()?;
    #[cfg(feature = "fault-injection")]
    inject_io_if("io_segment_directory_sync")?;
    sync_directory(root)?;
    Ok(file)
}

#[cfg(unix)]
fn sync_directory(root: &Path) -> Result<(), StoreError> {
    File::open(root)?.sync_all()?;
    Ok(())
}
#[cfg(not(unix))]
fn sync_directory(_root: &Path) -> Result<(), StoreError> {
    // Portable Rust 1.85 has no directory fsync on Windows. The required
    // Windows power-loss guarantees remain an explicit future acceptance gate.
    Ok(())
}

fn frame(id: &str, encoded: Option<&[u8]>) -> Result<Vec<u8>, StoreError> {
    validate_id(id)?;
    let data = encoded.unwrap_or(&[]);
    if data.len() > MAX_DOCUMENT_BYTES {
        return Err(StoreError::Codec(CodecError::TooLarge));
    }
    let id_len = u32::try_from(id.len()).map_err(|_| StoreError::InvalidId)?;
    let value_len = u32::try_from(data.len()).map_err(|_| StoreError::Codec(CodecError::TooLarge))?;
    let mut output = Vec::with_capacity(HEADER_SIZE + id.len() + data.len());
    output.extend_from_slice(MAGIC);
    output.push(VERSION);
    output.push(if encoded.is_some() { PUT } else { DELETE });
    output.extend_from_slice(&[0, 0]);
    output.extend_from_slice(&id_len.to_be_bytes());
    output.extend_from_slice(&value_len.to_be_bytes());
    output.extend_from_slice(&[0; 4]); // checksum at bytes 16..20
    output.extend_from_slice(id.as_bytes());
    output.extend_from_slice(data);
    let checksum = crc32(&[&output[4..16], id.as_bytes(), data]);
    output[16..20].copy_from_slice(&checksum.to_be_bytes());
    Ok(output)
}

// #82: id_len=0 reserved in the outer RYSF header and the entire batch body
// protected by the same CRC as legacy PUT/DELETE. Body format:
// u16 BE count (2..=1024); each: u8 op, u16 BE id_len, UTF-8 id,
// u32 BE RDB value_len, RDB bytes (empty for DELETE). No trailing bytes.
// Aggregate body <=64 MiB, unique document IDs and fully decoded values.
fn frame_batch(mutations: &[BatchMutation<'_>]) -> Result<Vec<u8>, StoreError> {
    if !(2..=MAX_BATCH_OPERATIONS).contains(&mutations.len()) {
        return Err(StoreError::InvalidBatch);
    }
    let mut body = Vec::new();
    body.extend_from_slice(&(mutations.len() as u16).to_be_bytes());
    for mutation in mutations {
        let id = mutation.id();
        validate_id(id)?;
        let data = match mutation {
            BatchMutation::Put { value, .. } => Some(encode(value)?),
            BatchMutation::Delete { .. } => None,
        };
        let payload = data.as_deref().unwrap_or(&[]);
        let added = 1 + 2 + id.len() + 4 + payload.len();
        if body.len().checked_add(added).is_none_or(|n| n > MAX_DOCUMENT_BYTES) {
            return Err(StoreError::Codec(CodecError::TooLarge));
        }
        body.push(if data.is_some() { PUT } else { DELETE });
        body.extend_from_slice(&(id.len() as u16).to_be_bytes());
        body.extend_from_slice(id.as_bytes());
        body.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        body.extend_from_slice(payload);
    }
    let mut output = Vec::with_capacity(HEADER_SIZE + body.len());
    output.extend_from_slice(MAGIC);
    output.push(VERSION);
    output.push(BATCH);
    output.extend_from_slice(&[0, 0]);
    output.extend_from_slice(&0_u32.to_be_bytes()); // no outer ID
    output.extend_from_slice(&(body.len() as u32).to_be_bytes());
    output.extend_from_slice(&[0; 4]);
    output.extend_from_slice(&body);
    let checksum = crc32(&[&output[4..16], &body]);
    output[16..20].copy_from_slice(&checksum.to_be_bytes());
    Ok(output)
}

fn decode_batch(bytes: &[u8], offset: u64) -> Result<Vec<(String, Option<Value>)>, StoreError> {
    use std::collections::BTreeSet;
    fn take<'a>(bytes: &'a [u8], cursor: &mut usize, count: usize)
        -> Result<&'a [u8], StoreError>
    {
        let end = cursor.checked_add(count)
            .ok_or_else(|| StoreError::Corrupt("batch offset overflow".into()))?;
        let result = bytes.get(*cursor..end)
            .ok_or_else(|| StoreError::Corrupt("truncated batch member".into()))?;
        *cursor = end;
        Ok(result)
    }

    let mut cursor = 0;
    let count = u16::from_be_bytes(take(bytes, &mut cursor, 2)?
        .try_into().expect("2-byte batch count")) as usize;
    if !(2..=MAX_BATCH_OPERATIONS).contains(&count) || count > (bytes.len()-2)/8 {
        return Err(StoreError::Corrupt(format!("invalid batch member count at {offset}")));
    }
    let mut changes = Vec::with_capacity(count);
    let mut seen = BTreeSet::new();
    for _ in 0..count {
        let op = take(bytes, &mut cursor, 1)?[0];
        let id_len = u16::from_be_bytes(take(bytes, &mut cursor, 2)?
            .try_into().expect("2-byte batch ID length")) as usize;
        if !(1..=MAX_ID_BYTES).contains(&id_len) {
            return Err(StoreError::Corrupt(format!("invalid batch ID length at {offset}")));
        }
        let id_bytes = take(bytes, &mut cursor, id_len)?;
        let id = std::str::from_utf8(id_bytes)
            .map_err(|_| StoreError::Corrupt(format!("invalid batch UTF-8 ID at {offset}")))?;
        if validate_id(id).is_err() || !seen.insert(id.to_owned()) {
            return Err(StoreError::Corrupt(format!("invalid or duplicate batch ID at {offset}")));
        }
        let value_len = u32::from_be_bytes(take(bytes, &mut cursor, 4)?
            .try_into().expect("4-byte batch value length")) as usize;
        if value_len > MAX_DOCUMENT_BYTES
            || !((op == PUT && value_len >= 5) || (op == DELETE && value_len == 0))
        {
            return Err(StoreError::Corrupt(format!("invalid batch operation/length at {offset}")));
        }
        let data = take(bytes, &mut cursor, value_len)?;
        let value = if op == PUT {
            Some(decode(data).map_err(|err| {
                StoreError::Corrupt(format!("invalid batch RDB document at {offset}: {err}"))
            })?)
        } else {
            None
        };
        changes.push((id.to_owned(), value));
    }
    if cursor != bytes.len() {
        return Err(StoreError::Corrupt(format!("trailing batch bytes at {offset}")));
    }
    Ok(changes)
}

/// A valid batch is decoded/CRC-verified as a unit before any member can
/// become visible. Readers of one indexed ID reverify that entire frame.
enum FrameRead {
    Complete { id: String, value: Option<Value>, length: u64 },
    Batch { changes: Vec<(String, Option<Value>)>, length: u64 },
    Incomplete,
}

fn read_frame(file: &mut File, offset: u64, end: u64) -> Result<FrameRead, StoreError> {
    let remaining = end.checked_sub(offset)
        .ok_or_else(|| StoreError::Corrupt("frame offset exceeds segment size".into()))?;
    if remaining < HEADER_SIZE as u64 {
        return Ok(FrameRead::Incomplete);
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut header = [0u8; HEADER_SIZE];
    file.read_exact(&mut header)?;
    if &header[..4] != MAGIC {
        return Err(StoreError::Corrupt(format!("invalid segment magic at {offset}")));
    }
    if header[4] != VERSION {
        return Err(StoreError::Corrupt(format!("unsupported segment version at {offset}")));
    }
    let op = header[5];
    if (op != PUT && op != DELETE && op != BATCH) || header[6..8] != [0, 0] {
        return Err(StoreError::Corrupt(format!("invalid operation or flags at {offset}")));
    }
    let id_len = u32::from_be_bytes(header[8..12].try_into().expect("fixed header")) as usize;
    let value_len = u32::from_be_bytes(header[12..16].try_into().expect("fixed header")) as usize;
    if value_len > MAX_DOCUMENT_BYTES
        || if op == BATCH {
            id_len != 0 || value_len < 18
        } else {
            !(1..=MAX_ID_BYTES).contains(&id_len)
                || (op == DELETE && value_len != 0)
                || (op == PUT && value_len < 5)
        }
    {
        return Err(StoreError::Corrupt(format!("invalid frame lengths at {offset}")));
    }
    let length = (HEADER_SIZE as u64)
        .checked_add(id_len as u64)
        .and_then(|x| x.checked_add(value_len as u64))
        .ok_or_else(|| StoreError::Corrupt("frame length overflow".into()))?;
    if length > remaining {
        return Ok(FrameRead::Incomplete);
    }
    let mut id_bytes = vec![0_u8; id_len];
    let mut value_bytes = vec![0_u8; value_len];
    file.read_exact(&mut id_bytes)?;
    file.read_exact(&mut value_bytes)?;
    let expected_crc = u32::from_be_bytes(header[16..20].try_into().expect("fixed header"));
    let actual_crc = crc32(&[&header[4..16], &id_bytes, &value_bytes]);
    if expected_crc != actual_crc {
        return Err(StoreError::Corrupt(format!("checksum mismatch at {offset}")));
    }
    if op == BATCH {
        return Ok(FrameRead::Batch {
            changes: decode_batch(&value_bytes, offset)?,
            length,
        });
    }
    let id = String::from_utf8(id_bytes)
        .map_err(|_| StoreError::Corrupt(format!("invalid UTF-8 document ID at {offset}")))?;
    let value = if op == PUT {
        Some(decode(&value_bytes).map_err(|err| {
            StoreError::Corrupt(format!("invalid document at {offset}: {err}"))
        })?)
    } else {
        None
    };
    Ok(FrameRead::Complete { id, value, length })
}

// CRC-32/ISO-HDLC (IEEE 802.3), including version, op, flags, lengths,
// document ID and document payload; excludes magic and checksum field.
const CRC32_TABLE: [u32; 256] = crc32_table();
const fn crc32_table() -> [u32; 256] {
    let mut table = [0_u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut value = i as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 != 0 { (value >> 1) ^ 0xedb8_8320 } else { value >> 1 };
            bit += 1;
        }
        table[i] = value;
        i += 1;
    }
    table
}
fn crc32(parts: &[&[u8]]) -> u32 {
    let mut checksum = !0_u32;
    for part in parts {
        for &byte in *part {
            let index = ((checksum ^ u32::from(byte)) & 0xff) as usize;
            checksum = (checksum >> 8) ^ CRC32_TABLE[index];
        }
    }
    !checksum
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_standard_vector() {
        assert_eq!(crc32(&[b"123456789"]), 0xcbf4_3926);
    }

    #[test]
    fn validates_segment_filename() {
        assert_eq!(parse_segment_name("segment-00000001.rdb"), Some(1));
        assert_eq!(parse_segment_name("segment-99999999.rdb"), Some(99_999_999));
        assert_eq!(parse_segment_name("segment-00000000.rdb"), None);
        assert_eq!(parse_segment_name("segment-000001.rdb"), None);
        assert_eq!(parse_segment_name("segment-00000001.rdb.bak"), None);
    }
}
