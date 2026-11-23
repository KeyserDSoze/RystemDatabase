//! #86 RED: owned staged writes + read-your-own-writes over a tracked
//! historical view, one-frame atomic commit and no early disk changes.
//! This is intentionally not a multi-frame or serializable SQL transaction.
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
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let n = SERIAL.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rystemdb-staged-batch-{}-{stamp}-{n}", std::process::id()
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
    fs::read_dir(path).unwrap()
        .map(|item| item.unwrap())
        .filter(|item| item.file_name().to_string_lossy().ends_with(".rdb"))
        .map(|item| item.metadata().unwrap().len())
        .sum()
}

fn nested() -> Value {
    let mut doc = BTreeMap::new();
    doc.insert("null".to_owned(), Value::Null);
    doc.insert("bytes".to_owned(), Value::Bytes(vec![0, 127, 255]));
    doc.insert("ref".to_owned(), Value::Reference {
        collection: "users".into(), id: "u-42".into(),
    });
    Value::Object(doc)
}

#[test]
fn tdd_staged_put_delete_get_overlay_is_typed_and_never_written_before_commit() {
    let root = TempDir::new();
    let mut db = DocumentStore::open(root.path()).unwrap();
    db.put("old", &Value::Integer(1)).unwrap();
    let prior = db.snapshot();
    let baseline = log_bytes(root.path());

    let mut draft = db.staged_batch();
    assert_eq!(draft.get("old").unwrap(), Some(Value::Integer(1)));
    assert_eq!(draft.read_count(), 1);
    assert_eq!(draft.get("absent").unwrap(), None);
    assert_eq!(draft.read_count(), 2);

    draft.put("old", nested()).unwrap();
    assert_eq!(draft.get("old").unwrap(), Some(nested()));
    draft.put("new", Value::Null).unwrap();
    assert_eq!(draft.get("new").unwrap(), Some(Value::Null));
    draft.delete("new").unwrap();
    assert_eq!(draft.get("new").unwrap(), None);
    draft.put("new", Value::Bool(true)).unwrap();
    assert_eq!(draft.get("new").unwrap(), Some(Value::Bool(true)));
    assert_eq!(draft.staged_count(), 2, "last write for an ID must coalesce");
    assert_eq!(draft.read_count(), 2,
        "reading a staged value should not add an unrelated snapshot read");
    assert_eq!(db.get("old").unwrap(), Some(Value::Integer(1)));
    assert_eq!(db.get("new").unwrap(), None);
    assert_eq!(log_bytes(root.path()), baseline, "staging must not append or sync");

    db.commit_staged_batch(&draft).unwrap();
    assert!(log_bytes(root.path()) > baseline);
    assert_eq!(db.get("old").unwrap(), Some(nested()));
    assert_eq!(db.get("new").unwrap(), Some(Value::Bool(true)));
    assert_eq!(prior.get("old").unwrap(), Some(Value::Integer(1)));
    assert_eq!(draft.get("old").unwrap(), Some(nested()));
    drop(db);

    let reopened = DocumentStore::open(root.path()).unwrap();
    assert_eq!(reopened.get("old").unwrap(), Some(nested()));
    assert_eq!(reopened.get("new").unwrap(), Some(Value::Bool(true)));
}

#[test]
fn tdd_staged_read_only_dependency_conflict_rejects_entire_commit_before_append() {
    let root = TempDir::new();
    let mut db = DocumentStore::open(root.path()).unwrap();
    db.put("oncall/alice", &Value::Bool(true)).unwrap();
    db.put("oncall/bob", &Value::Bool(true)).unwrap();
    let mut draft = db.staged_batch();

    assert_eq!(draft.get("oncall/bob").unwrap(), Some(Value::Bool(true)));
    draft.put("oncall/alice", Value::Bool(false)).unwrap();
    draft.put("audit", Value::Integer(1)).unwrap();
    assert_eq!(draft.get("oncall/alice").unwrap(), Some(Value::Bool(false)));
    db.put("oncall/bob", &Value::Bool(false)).unwrap(); // read-only dependency changed
    let before_conflict = log_bytes(root.path());
    assert!(matches!(db.commit_staged_batch(&draft), Err(StoreError::Conflict)));
    assert_eq!(log_bytes(root.path()), before_conflict);
    assert_eq!(db.get("oncall/alice").unwrap(), Some(Value::Bool(true)));
    assert_eq!(db.get("audit").unwrap(), None);
    assert_eq!(draft.get("oncall/bob").unwrap(), Some(Value::Bool(true)));
    assert_eq!(draft.read_count(), 1);

    let mut fresh = db.staged_batch();
    assert_eq!(fresh.get("oncall/bob").unwrap(), Some(Value::Bool(false)));
    fresh.put("oncall/alice", Value::Bool(true)).unwrap();
    fresh.put("audit", Value::Integer(2)).unwrap();
    db.put("unobserved", &Value::Integer(500)).unwrap();
    db.commit_staged_batch(&fresh).unwrap();
    assert_eq!(db.get("audit").unwrap(), Some(Value::Integer(2)));
    assert_eq!(db.get("unobserved").unwrap(), Some(Value::Integer(500)));
}

#[test]
fn tdd_staged_absent_read_aba_survives_rollover_and_writer_reopen() {
    let root = TempDir::new();
    let mut db = DocumentStore::open_with_segment_limit(root.path(), 120).unwrap();
    db.put("slot", &Value::Integer(7)).unwrap();
    let mut draft = db.staged_batch();
    assert_eq!(draft.get("lock").unwrap(), None);
    draft.put("slot", Value::Integer(8)).unwrap();
    draft.put("receipt", Value::Bool(true)).unwrap();

    db.put("lock", &Value::String("x".repeat(100))).unwrap();
    assert!(db.delete("lock").unwrap());
    db.put("outside", &Value::String("y".repeat(100))).unwrap();
    assert!(root.path().join("segment-00000002.rdb").exists());
    drop(db);

    let mut reopened = DocumentStore::open_with_segment_limit(root.path(), 120).unwrap();
    let before_conflict = log_bytes(root.path());
    assert!(matches!(
        reopened.commit_staged_batch(&draft),
        Err(StoreError::Conflict)
    ));
    assert_eq!(log_bytes(root.path()), before_conflict);
    assert_eq!(reopened.get("slot").unwrap(), Some(Value::Integer(7)));
    assert_eq!(reopened.get("receipt").unwrap(), None);
    assert_eq!(draft.get("slot").unwrap(), Some(Value::Integer(8)));

    let mut current = reopened.staged_batch();
    assert_eq!(current.get("lock").unwrap(), None);
    current.put("slot", Value::Integer(9)).unwrap();
    current.put("receipt", Value::Bool(true)).unwrap();
    reopened.commit_staged_batch(&current).unwrap();
    drop(reopened);
    let reopened = DocumentStore::open_with_segment_limit(root.path(), 120).unwrap();
    assert_eq!(reopened.get("slot").unwrap(), Some(Value::Integer(9)));
    assert_eq!(reopened.get("receipt").unwrap(), Some(Value::Bool(true)));
}

#[test]
fn tdd_staged_last_write_wins_including_existing_key_delete() {
    let root = TempDir::new();
    let mut db = DocumentStore::open(root.path()).unwrap();
    db.put("key", &Value::Integer(1)).unwrap();
    let mut draft = db.staged_batch();
    draft.put("key", Value::Integer(2)).unwrap();
    draft.delete("key").unwrap();
    assert_eq!(draft.get("key").unwrap(), None);
    draft.put("other", Value::Null).unwrap();
    assert_eq!(draft.staged_count(), 2);
    assert_eq!(db.get("key").unwrap(), Some(Value::Integer(1)));
    db.commit_staged_batch(&draft).unwrap();
    assert_eq!(db.get("key").unwrap(), None);
    assert_eq!(db.get("other").unwrap(), Some(Value::Null));
    drop(db);
    let db = DocumentStore::open(root.path()).unwrap();
    assert_eq!(db.get("key").unwrap(), None);
    assert_eq!(db.get("other").unwrap(), Some(Value::Null));
}

#[test]
fn tdd_staged_capacity_invalid_ids_and_drop_do_not_append() {
    let root = TempDir::new();
    let mut db = DocumentStore::open(root.path()).unwrap();
    let start = log_bytes(root.path());
    {
        let mut draft = db.staged_batch();
        assert!(matches!(draft.put("", Value::Null), Err(StoreError::InvalidId)));
        assert!(matches!(draft.delete(&"a".repeat(1025)), Err(StoreError::InvalidId)));
        assert!(matches!(draft.get(""), Err(StoreError::InvalidId)));
        assert_eq!(draft.staged_count(), 0);
        assert_eq!(draft.read_count(), 0);
        draft.put("only", Value::Integer(1)).unwrap();
        assert!(matches!(db.commit_staged_batch(&draft), Err(StoreError::InvalidBatch)),
            "single staged mutation does not meet existing 2+ atomic batch rule");
    }
    assert_eq!(log_bytes(root.path()), start, "dropping uncommitted staging must be no-op");

    let mut draft = db.staged_batch();
    for i in 0..1024 {
        draft.put(&format!("key-{i}"), Value::Null).unwrap();
    }
    assert_eq!(draft.staged_count(), 1024);
    draft.put("key-0", Value::Bool(true)).unwrap();
    draft.delete("key-0").unwrap();
    draft.put("key-0", Value::Null).unwrap();
    assert_eq!(draft.staged_count(), 1024);
    assert!(matches!(draft.put("overflow", Value::Null), Err(StoreError::InvalidBatch)));
    assert!(matches!(draft.delete("also-overflow"), Err(StoreError::InvalidBatch)));
    assert_eq!(draft.staged_count(), 1024);
    assert_eq!(log_bytes(root.path()), start);
    drop(draft);
    assert!(db.is_empty());
    assert_eq!(log_bytes(root.path()), start);
}

#[test]
fn tdd_staged_failed_crc_read_and_foreign_root_are_safe() {
    let root = TempDir::new();
    let other = TempDir::new();
    let mut db = DocumentStore::open(root.path()).unwrap();
    let foreign = DocumentStore::open(other.path()).unwrap().staged_batch();
    let before = log_bytes(root.path());
    assert!(matches!(
        db.commit_staged_batch(&foreign),
        Err(StoreError::InvalidSnapshot)
    ));
    assert_eq!(log_bytes(root.path()), before);

    db.put("corrupt", &Value::String("original".into())).unwrap();
    let mut draft = db.staged_batch();
    let path = root.path().join("segment-00000001.rdb");
    let mut file = OpenOptions::new().read(true).write(true).open(path).unwrap();
    file.seek(SeekFrom::End(-1)).unwrap();
    let mut byte = [0_u8; 1];
    file.read_exact(&mut byte).unwrap();
    file.seek(SeekFrom::End(-1)).unwrap();
    file.write_all(&[byte[0] ^ 0x80]).unwrap();
    file.sync_all().unwrap();
    drop(file);
    assert!(matches!(draft.get("corrupt"), Err(StoreError::Corrupt(_))));
    assert_eq!(draft.read_count(), 0);
    draft.put("corrupt", Value::Bool(true)).unwrap();
    assert_eq!(draft.get("corrupt").unwrap(), Some(Value::Bool(true)),
        "a staged PUT should not read the corrupted historical record");
    assert_eq!(draft.read_count(), 0);
}
