//! #81 TDD: ABA-safe conditional insertion of keys missing in an immutable
//! committed snapshot, including durable tombstone generation after replay.
use rystemdb_core::Value;
use rystemdb_storage::{DocumentStore, StoreError};
use std::fs;
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
            "rystemdb-conditional-insert-{}-{stamp}-{n}", std::process::id()
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

#[test]
fn tdd_insert_from_absent_snapshot_allows_once_and_rejects_existing_value() {
    let dir = TempDir::new();
    let mut db = DocumentStore::open(dir.path()).unwrap();
    let absent = db.snapshot();
    db.put("other", &Value::Bool(true)).unwrap();
    db.insert_if_unchanged(&absent, "key", &Value::Integer(41)).unwrap();
    assert_eq!(db.get("key").unwrap(), Some(Value::Integer(41)));
    assert_eq!(absent.get("key").unwrap(), None);

    let accepted_len = log_bytes(dir.path());
    assert!(matches!(
        db.insert_if_unchanged(&absent, "key", &Value::Integer(42)),
        Err(StoreError::Conflict)
    ));
    assert_eq!(log_bytes(dir.path()), accepted_len,
        "conflicting create must append no bytes");
    assert_eq!(db.get("key").unwrap(), Some(Value::Integer(41)));

    let present = db.snapshot();
    assert!(db.delete("key").unwrap());
    let deleted_len = log_bytes(dir.path());
    assert!(matches!(
        db.insert_if_unchanged(&present, "key", &Value::Integer(43)),
        Err(StoreError::Conflict)
    ));
    assert_eq!(log_bytes(dir.path()), deleted_len,
        "a snapshot of PRESENT data must not become an insert token");
}

#[test]
fn tdd_absent_put_delete_aba_is_detected_without_appending() {
    let dir = TempDir::new();
    let mut db = DocumentStore::open(dir.path()).unwrap();
    let originally_absent = db.snapshot();

    db.put("key", &Value::String("transient".into())).unwrap();
    assert!(db.delete("key").unwrap());
    assert_eq!(db.get("key").unwrap(), None);
    let before_conflict = log_bytes(dir.path());
    assert!(matches!(
        db.insert_if_unchanged(&originally_absent, "key", &Value::Integer(1)),
        Err(StoreError::Conflict)
    ), "absent -> PUT -> DELETE must invalidate initial absent snapshot");
    assert_eq!(log_bytes(dir.path()), before_conflict);

    let after_first_tombstone = db.snapshot();
    db.insert_if_unchanged(&after_first_tombstone, "key", &Value::Integer(2)).unwrap();
    assert!(db.delete("key").unwrap());
    let after_second_tombstone = log_bytes(dir.path());
    assert!(matches!(
        db.insert_if_unchanged(&after_first_tombstone, "key", &Value::Integer(3)),
        Err(StoreError::Conflict)
    ), "a later tombstone must have its own last-committed version");
    assert_eq!(log_bytes(dir.path()), after_second_tombstone);
    assert_eq!(after_first_tombstone.get("key").unwrap(), None);
    assert_eq!(db.get("key").unwrap(), None);
}

#[test]
fn tdd_tombstone_version_replays_across_rollover_and_writer_reopen() {
    let dir = TempDir::new();
    let mut db = DocumentStore::open_with_segment_limit(dir.path(), 120).unwrap();
    let before_any_write = db.snapshot();
    db.put("key", &Value::String("x".repeat(85))).unwrap();
    assert!(db.delete("key").unwrap());
    let after_tombstone = db.snapshot();

    for i in 0..9 {
        db.put(&format!("unrelated-{i}"), &Value::String("a".repeat(85))).unwrap();
    }
    assert!(dir.path().join("segment-00000002.rdb").is_file());
    drop(db);

    let mut reopened = DocumentStore::open_with_segment_limit(dir.path(), 120).unwrap();
    let before_conflict = log_bytes(dir.path());
    assert!(matches!(
        reopened.insert_if_unchanged(&before_any_write, "key", &Value::Integer(1)),
        Err(StoreError::Conflict)
    ), "replay must preserve tombstone version even if live index has no key");
    assert_eq!(log_bytes(dir.path()), before_conflict);
    reopened.insert_if_unchanged(&after_tombstone, "key", &Value::Integer(2)).unwrap();
    assert_eq!(reopened.get("key").unwrap(), Some(Value::Integer(2)));
    assert_eq!(after_tombstone.get("key").unwrap(), None);
    let after_insert_len = log_bytes(dir.path());
    assert!(matches!(
        reopened.insert_if_unchanged(&after_tombstone, "key", &Value::Integer(3)),
        Err(StoreError::Conflict)
    ));
    assert_eq!(log_bytes(dir.path()), after_insert_len);
}

#[test]
fn tdd_conditional_insert_checks_foreign_root_and_invalid_id_before_append() {
    let left = TempDir::new();
    let right = TempDir::new();
    let from_other = DocumentStore::open(left.path()).unwrap().snapshot();
    let mut db = DocumentStore::open(right.path()).unwrap();
    let before = log_bytes(right.path());
    assert!(matches!(
        db.insert_if_unchanged(&from_other, "key", &Value::Null),
        Err(StoreError::InvalidSnapshot)
    ));
    assert_eq!(log_bytes(right.path()), before);

    let from_this = db.snapshot();
    assert!(matches!(
        db.insert_if_unchanged(&from_this, "", &Value::Null),
        Err(StoreError::InvalidId)
    ));
    assert_eq!(log_bytes(right.path()), before);
}
