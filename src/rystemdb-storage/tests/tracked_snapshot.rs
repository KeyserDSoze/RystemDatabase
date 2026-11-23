//! #84 RED: automatically record successful point reads for atomic batch OCC.
//! Not a general transaction/SQL serializability/phantom protection system.
use rystemdb_core::Value;
use rystemdb_storage::{BatchMutation, DocumentStore, StoreError};
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
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let n = SERIAL.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rystemdb-tracked-snapshot-{}-{stamp}-{n}", std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path { &self.0 }
}
impl Drop for TempDir {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}
fn log_bytes(root: &Path) -> u64 {
    fs::read_dir(root).unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".rdb"))
        .map(|entry| entry.metadata().unwrap().len())
        .sum()
}

#[test]
fn tdd_tracked_read_only_dependency_prevents_write_skew_and_zero_append() {
    let root = TempDir::new();
    let mut db = DocumentStore::open(root.path()).unwrap();
    db.put("oncall/alice", &Value::Bool(true)).unwrap();
    db.put("oncall/bob", &Value::Bool(true)).unwrap();

    let mut tx = db.tracked_snapshot();
    assert_eq!(tx.get("oncall/bob").unwrap(), Some(Value::Bool(true)));
    assert_eq!(tx.read_count(), 1);
    // The actual decision depends on Bob, but this batch does not WRITE Bob.
    db.put("oncall/bob", &Value::Bool(false)).unwrap();
    let before = log_bytes(root.path());
    assert!(matches!(db.commit_tracked_batch(&tx, &[
        BatchMutation::Put { id:"oncall/alice", value:&Value::Bool(false) },
        BatchMutation::Put { id:"audit/alice-off", value:&Value::Integer(1) },
    ]), Err(StoreError::Conflict)));
    assert_eq!(log_bytes(root.path()), before);
    assert_eq!(db.get("oncall/alice").unwrap(), Some(Value::Bool(true)));
    assert_eq!(db.get("audit/alice-off").unwrap(), None);
    assert_eq!(tx.get("oncall/bob").unwrap(), Some(Value::Bool(true)),
        "historical reads must never switch to newer committed versions");
    assert_eq!(tx.read_count(), 1, "reading the same key twice should not duplicate a dependency");

    let mut fresh = db.tracked_snapshot();
    assert_eq!(fresh.get("oncall/bob").unwrap(), Some(Value::Bool(false)));
    db.put("unrelated", &Value::Integer(99)).unwrap();
    db.commit_tracked_batch(&fresh, &[
        BatchMutation::Put { id:"oncall/alice", value:&Value::Bool(true) },
        BatchMutation::Put { id:"audit/safe", value:&Value::Bool(true) },
    ]).unwrap();
    assert_eq!(db.get("audit/safe").unwrap(), Some(Value::Bool(true)));
    drop(db);
    let reopened = DocumentStore::open(root.path()).unwrap();
    assert_eq!(reopened.get("audit/safe").unwrap(), Some(Value::Bool(true)));
}

#[test]
fn tdd_automatic_absent_read_detects_put_delete_aba_after_rollover_and_reopen() {
    let root = TempDir::new();
    let mut db = DocumentStore::open_with_segment_limit(root.path(), 120).unwrap();
    db.put("slot", &Value::Integer(7)).unwrap();
    let mut tx = db.tracked_snapshot();
    assert_eq!(tx.get("lock").unwrap(), None);
    assert_eq!(tx.read_count(), 1);

    db.put("lock", &Value::String("z".repeat(100))).unwrap();
    assert!(db.delete("lock").unwrap());
    db.put("other", &Value::String("y".repeat(100))).unwrap();
    assert!(root.path().join("segment-00000002.rdb").is_file());
    drop(db);

    let mut db = DocumentStore::open_with_segment_limit(root.path(), 120).unwrap();
    let before = log_bytes(root.path());
    assert!(matches!(db.commit_tracked_batch(&tx, &[
        BatchMutation::Put { id:"slot", value:&Value::Integer(8) },
        BatchMutation::Put { id:"confirmed", value:&Value::Bool(true) },
    ]), Err(StoreError::Conflict)));
    assert_eq!(log_bytes(root.path()), before);
    assert_eq!(db.get("confirmed").unwrap(), None);
    assert_eq!(tx.get("lock").unwrap(), None);

    let mut fresh = db.tracked_snapshot();
    assert_eq!(fresh.get("lock").unwrap(), None);
    db.commit_tracked_batch(&fresh, &[
        BatchMutation::Put { id:"slot", value:&Value::Integer(8) },
        BatchMutation::Put { id:"confirmed", value:&Value::Bool(true) },
    ]).unwrap();
    drop(db);
    let db = DocumentStore::open_with_segment_limit(root.path(), 120).unwrap();
    assert_eq!(db.get("slot").unwrap(), Some(Value::Integer(8)));
    assert_eq!(db.get("confirmed").unwrap(), Some(Value::Bool(true)));
}

#[test]
fn tdd_tracked_reads_preserve_nested_types_null_and_read_write_overlap() {
    let root = TempDir::new();
    let mut db = DocumentStore::open(root.path()).unwrap();
    let mut object = BTreeMap::new();
    object.insert("null".to_owned(), Value::Null);
    object.insert("bytes".to_owned(), Value::Bytes(vec![0, 127, 255]));
    object.insert("ref".to_owned(), Value::Reference {
        collection: "users".into(), id: "u-42".into(),
    });
    let nested = Value::Object(object);
    db.put("typed", &nested).unwrap();
    db.put("null", &Value::Null).unwrap();
    let mut tx = db.tracked_snapshot();
    assert_eq!(tx.get("typed").unwrap(), Some(nested.clone()));
    assert_eq!(tx.get("null").unwrap(), Some(Value::Null));
    assert_eq!(tx.get("absent").unwrap(), None);
    assert_eq!(tx.read_count(), 3);
    // Read and written keys may overlap; no spurious conflicts.
    db.commit_tracked_batch(&tx, &[
        BatchMutation::Put { id:"typed", value:&Value::Integer(7) },
        BatchMutation::Put { id:"added", value:&Value::Bool(true) },
    ]).unwrap();
    assert_eq!(tx.get("typed").unwrap(), Some(nested));
    assert_eq!(db.get("typed").unwrap(), Some(Value::Integer(7)));
    let before = log_bytes(root.path());
    assert!(matches!(db.commit_tracked_batch(&tx, &[
        BatchMutation::Put { id:"typed", value:&Value::Integer(7) },
        BatchMutation::Put { id:"after", value:&Value::Null },
    ]), Err(StoreError::Conflict)));
    assert_eq!(log_bytes(root.path()), before);
}

#[test]
fn tdd_tracked_reads_validate_ids_and_bound_distinct_keys_but_deduplicate() {
    let root = TempDir::new();
    let mut db = DocumentStore::open(root.path()).unwrap();
    let mut tx = db.tracked_snapshot();
    assert!(matches!(tx.get(""), Err(StoreError::InvalidId)));
    assert!(matches!(tx.get(&"x".repeat(1025)), Err(StoreError::InvalidId)));
    assert_eq!(tx.read_count(), 0);
    for n in 0..1024 {
        assert_eq!(tx.get(&format!("missing-{n}")).unwrap(), None);
    }
    assert_eq!(tx.read_count(), 1024);
    assert_eq!(tx.get("missing-0").unwrap(), None);
    assert_eq!(tx.read_count(), 1024);
    let before = log_bytes(root.path());
    assert!(matches!(tx.get("overflow"), Err(StoreError::InvalidBatch)));
    assert_eq!(tx.read_count(), 1024);
    assert_eq!(log_bytes(root.path()), before);
    db.commit_tracked_batch(&tx, &[
        BatchMutation::Put { id:"first", value:&Value::Integer(1) },
        BatchMutation::Put { id:"second", value:&Value::Integer(2) },
    ]).unwrap();
    assert_eq!(db.get("first").unwrap(), Some(Value::Integer(1)));
    assert_eq!(db.get("second").unwrap(), Some(Value::Integer(2)));
}

#[test]
fn tdd_failed_crc_read_does_not_record_successful_observation() {
    let root = TempDir::new();
    let mut db = DocumentStore::open(root.path()).unwrap();
    db.put("corrupt", &Value::String("original".into())).unwrap();
    let mut tx = db.tracked_snapshot();
    let path = root.path().join("segment-00000001.rdb");
    let mut file = OpenOptions::new().read(true).write(true).open(path).unwrap();
    file.seek(SeekFrom::End(-1)).unwrap();
    let mut byte = [0_u8; 1];
    file.read_exact(&mut byte).unwrap();
    file.seek(SeekFrom::End(-1)).unwrap();
    file.write_all(&[byte[0] ^ 0x80]).unwrap();
    file.sync_all().unwrap();
    drop(file);
    assert!(matches!(tx.get("corrupt"), Err(StoreError::Corrupt(_))));
    assert_eq!(tx.read_count(), 0);
}

#[test]
fn tdd_tracked_batch_rejects_foreign_database_snapshot_without_writing() {
    let root = TempDir::new();
    let other = TempDir::new();
    let foreign = DocumentStore::open(other.path()).unwrap().tracked_snapshot();
    let mut db = DocumentStore::open(root.path()).unwrap();
    let before = log_bytes(root.path());
    assert!(matches!(db.commit_tracked_batch(&foreign, &[
        BatchMutation::Put { id:"first", value:&Value::Null },
        BatchMutation::Put { id:"second", value:&Value::Bool(true) },
    ]), Err(StoreError::InvalidSnapshot)));
    assert_eq!(log_bytes(root.path()), before);
}
