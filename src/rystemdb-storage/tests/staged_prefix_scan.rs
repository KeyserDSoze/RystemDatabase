//! #87 RED: prefix-scan read-your-own-writes and predicate conflict checks.
//! Prefix protection is NOT general SQL serializability or arbitrary range query.
use rystemdb_core::Value;
use rystemdb_storage::{DocumentStore, StoreError};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static SERIAL: AtomicU64 = AtomicU64::new(0);
struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let id = SERIAL.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rystemdb-prefix-{}-{time}-{id}", std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path { &self.0 }
}
impl Drop for TempDir {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}
fn log_bytes(path: &Path) -> u64 {
    fs::read_dir(path).unwrap().map(|e| e.unwrap())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".rdb"))
        .map(|e| e.metadata().unwrap().len()).sum()
}
fn nested() -> Value {
    let mut fields = BTreeMap::new();
    fields.insert("null".to_owned(), Value::Null);
    fields.insert("raw".to_owned(), Value::Bytes(vec![0,255]));
    Value::Object(fields)
}

#[test]
fn tdd_prefix_scan_merges_overlay_and_sorted_historical_typed_values() {
    let root = TempDir::new();
    let mut db = DocumentStore::open(root.path()).unwrap();
    db.put("users/alice", &nested()).unwrap();
    db.put("users/bob", &Value::Integer(4)).unwrap();
    db.put("users/delete", &Value::Null).unwrap();
    db.put("system/other", &Value::Bool(true)).unwrap();
    let before = log_bytes(root.path());
    let mut tx = db.staged_batch();

    tx.put("users/charlie", Value::Bool(true)).unwrap();
    tx.put("users/alice", Value::Integer(7)).unwrap();
    tx.delete("users/delete").unwrap();
    assert_eq!(tx.scan_prefix("users/").unwrap(), vec![
        ("users/alice".into(), Value::Integer(7)),
        ("users/bob".into(), Value::Integer(4)),
        ("users/charlie".into(), Value::Bool(true)),
    ]);
    assert_eq!(tx.watched_prefix_count(), 1);
    assert_eq!(tx.scan_prefix("users/").unwrap().len(), 3);
    assert_eq!(tx.watched_prefix_count(), 1, "same predicate is idempotent");
    assert_eq!(tx.read_count(), 0,
        "prefix predicates must be validated as predicates, not individual point gets");
    assert_eq!(log_bytes(root.path()), before, "scan and staging must not append");
    assert_eq!(db.get("users/alice").unwrap(), Some(nested()));
    assert_eq!(db.get("users/charlie").unwrap(), None);

    db.put("system/other", &Value::Bool(false)).unwrap();
    db.commit_staged_batch(&tx).unwrap();
    assert_eq!(db.get("users/alice").unwrap(), Some(Value::Integer(7)));
    assert_eq!(db.get("users/charlie").unwrap(), Some(Value::Bool(true)));
    assert_eq!(db.get("users/delete").unwrap(), None);
    drop(db);
    let replay = DocumentStore::open(root.path()).unwrap();
    assert_eq!(replay.get("users/alice").unwrap(), Some(Value::Integer(7)));
    assert_eq!(replay.get("users/charlie").unwrap(), Some(Value::Bool(true)));
}

#[test]
fn tdd_empty_prefix_scan_guards_new_phantom_insert_before_commit() {
    let root = TempDir::new();
    let mut db = DocumentStore::open(root.path()).unwrap();
    let mut tx = db.staged_batch();
    assert!(tx.scan_prefix("orders/").unwrap().is_empty());
    assert_eq!(tx.watched_prefix_count(), 1);
    tx.put("audit/attempt", Value::Bool(true)).unwrap();
    tx.put("balance", Value::Integer(1)).unwrap();
    db.put("orders/new", &Value::Integer(9)).unwrap();
    let before = log_bytes(root.path());
    assert!(matches!(db.commit_staged_batch(&tx), Err(StoreError::Conflict)));
    assert_eq!(log_bytes(root.path()), before,
        "phantom insertion must reject the entire batch before append");
    assert_eq!(db.get("audit/attempt").unwrap(), None);
    assert_eq!(db.get("balance").unwrap(), None);
    assert_eq!(tx.scan_prefix("orders/").unwrap(), vec![],
        "old snapshot sees historical empty range");
}

#[test]
fn tdd_prefix_changed_value_and_delete_conflict_even_after_reopen_rollover() {
    let root = TempDir::new();
    let mut db = DocumentStore::open_with_segment_limit(root.path(), 120).unwrap();
    db.put("users/a", &Value::Integer(2)).unwrap();
    db.put("users/b", &Value::Integer(3)).unwrap();
    let mut tx = db.staged_batch();
    assert_eq!(tx.scan_prefix("users/").unwrap().len(), 2);
    tx.put("outside/one", Value::Integer(1)).unwrap();
    tx.put("outside/two", Value::Integer(2)).unwrap();
    db.put("users/a", &Value::Integer(2)).unwrap(); // same bytes, new version
    assert!(db.delete("users/b").unwrap());
    db.put("other", &Value::String("x".repeat(100))).unwrap();
    assert!(root.path().join("segment-00000002.rdb").exists());
    drop(db);
    let mut db = DocumentStore::open_with_segment_limit(root.path(), 120).unwrap();
    let before = log_bytes(root.path());
    assert!(matches!(db.commit_staged_batch(&tx), Err(StoreError::Conflict)));
    assert_eq!(log_bytes(root.path()), before);
    assert_eq!(db.get("outside/one").unwrap(), None);
    assert_eq!(tx.scan_prefix("users/").unwrap(), vec![
        ("users/a".into(), Value::Integer(2)),
        ("users/b".into(), Value::Integer(3)),
    ]);
}

#[test]
fn tdd_prefix_aba_tombstone_invalidates_historical_empty_range() {
    let root = TempDir::new();
    let mut db = DocumentStore::open_with_segment_limit(root.path(), 120).unwrap();
    let mut tx = db.staged_batch();
    assert_eq!(tx.scan_prefix("locks/").unwrap(), vec![]);
    tx.put("audit/1", Value::Bool(true)).unwrap();
    tx.put("audit/2", Value::Bool(true)).unwrap();
    db.put("locks/ephemeral", &Value::String("x".repeat(100))).unwrap();
    assert!(db.delete("locks/ephemeral").unwrap());
    db.put("not-locked", &Value::String("y".repeat(100))).unwrap();
    drop(db);
    let mut reopened = DocumentStore::open_with_segment_limit(root.path(), 120).unwrap();
    let baseline = log_bytes(root.path());
    assert!(matches!(reopened.commit_staged_batch(&tx), Err(StoreError::Conflict)),
        "tombstone ledger prevents phantom ABA with empty final live range");
    assert_eq!(log_bytes(root.path()), baseline);
    let mut fresh = reopened.staged_batch();
    assert!(fresh.scan_prefix("locks/").unwrap().is_empty());
    fresh.put("audit/1", Value::Bool(true)).unwrap();
    fresh.put("audit/2", Value::Bool(true)).unwrap();
    reopened.commit_staged_batch(&fresh).unwrap();
    assert_eq!(reopened.get("audit/1").unwrap(), Some(Value::Bool(true)));
}

#[test]
fn tdd_prefix_errors_do_not_watch_and_enforce_distinct_watch_bound() {
    let root = TempDir::new();
    let mut db = DocumentStore::open(root.path()).unwrap();
    let baseline = log_bytes(root.path());
    let mut tx = db.staged_batch();
    assert!(matches!(tx.scan_prefix(""), Err(StoreError::InvalidId)));
    assert!(matches!(tx.scan_prefix(&"a".repeat(1025)), Err(StoreError::InvalidId)));
    assert_eq!(tx.watched_prefix_count(), 0);
    for i in 0..64 {
        assert!(tx.scan_prefix(&format!("p-{i}/")).unwrap().is_empty());
    }
    assert_eq!(tx.watched_prefix_count(), 64);
    assert!(tx.scan_prefix("p-0/").unwrap().is_empty());
    assert_eq!(tx.watched_prefix_count(), 64);
    assert!(matches!(tx.scan_prefix("p-overflow/"), Err(StoreError::InvalidBatch)));
    assert_eq!(tx.watched_prefix_count(), 64);
    assert_eq!(log_bytes(root.path()), baseline);
    tx.put("audit/a", Value::Bool(true)).unwrap();
    tx.put("audit/b", Value::Bool(true)).unwrap();
    db.commit_staged_batch(&tx).unwrap();
}

#[test]
fn tdd_prefix_scan_over_1024_candidate_keys_fails_before_tracking() {
    let root = TempDir::new();
    let mut db = DocumentStore::open(root.path()).unwrap();
    for i in 0..1025 {
        db.put(&format!("large/{i:04}"), &Value::Null).unwrap();
    }
    let mut tx = db.staged_batch();
    let baseline = log_bytes(root.path());
    assert!(matches!(tx.scan_prefix("large/"), Err(StoreError::InvalidBatch)));
    assert_eq!(tx.watched_prefix_count(), 0);
    assert_eq!(log_bytes(root.path()), baseline);
    assert_eq!(tx.scan_prefix("another/").unwrap().len(), 0);
    assert_eq!(tx.watched_prefix_count(), 1);
}

#[test]
fn tdd_corrupt_snapshot_read_does_not_publish_prefix_watch() {
    let root = TempDir::new();
    let mut db = DocumentStore::open(root.path()).unwrap();
    db.put("docs/a", &Value::Integer(10)).unwrap();
    let mut tx = db.staged_batch();
    let path = root.path().join("segment-00000001.rdb");
    let mut file = OpenOptions::new().read(true).write(true).open(path).unwrap();
    file.seek(SeekFrom::End(-1)).unwrap();
    let mut byte = [0_u8;1];
    file.read_exact(&mut byte).unwrap();
    file.seek(SeekFrom::End(-1)).unwrap();
    file.write_all(&[byte[0] ^ 0x80]).unwrap();
    file.sync_all().unwrap();
    drop(file);
    assert!(matches!(tx.scan_prefix("docs/"), Err(StoreError::Corrupt(_))));
    assert_eq!(tx.watched_prefix_count(), 0);
    assert_eq!(tx.read_count(), 0);
}
