//! #80 RED: per-key optimistic conditional mutation based on a committed
//! historical snapshot. No multi-document transactions or absent-key ABA.
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
        let root = std::env::temp_dir().join(format!(
            "rystemdb-optimistic-{}-{stamp}-{n}", std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn path(&self) -> &Path { &self.0 }
}
impl Drop for TempDir {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

fn segment_len(root: &Path, segment: u64) -> u64 {
    fs::metadata(root.join(format!("segment-{segment:08}.rdb")))
        .unwrap().len()
}

#[test]
fn tdd_existing_key_conditional_replace_detects_stale_snapshot_without_appending() {
    let dir = TempDir::new();
    let mut store = DocumentStore::open(dir.path()).unwrap();
    store.put("orders/1", &Value::Integer(7)).unwrap();
    let original = store.snapshot();

    // A write to another key does not invalidate this key's old version.
    store.put("orders/unrelated", &Value::Bool(true)).unwrap();
    store.replace_if_unchanged(&original, "orders/1", &Value::Integer(7)).unwrap();
    // Even identical payloads generate a newer committed RYSF version.
    let current_len = segment_len(dir.path(), 1);
    assert!(matches!(
        store.replace_if_unchanged(&original, "orders/1", &Value::Integer(99)),
        Err(StoreError::Conflict)
    ));
    assert!(matches!(
        store.delete_if_unchanged(&original, "orders/1"),
        Err(StoreError::Conflict)
    ));
    assert_eq!(segment_len(dir.path(), 1), current_len,
        "rejected stale writes must not append a single byte");
    assert_eq!(store.get("orders/1").unwrap(), Some(Value::Integer(7)));
    assert_eq!(original.get("orders/1").unwrap(), Some(Value::Integer(7)));

    let fresh = store.snapshot();
    store.replace_if_unchanged(&fresh, "orders/1", &Value::Integer(99)).unwrap();
    assert_eq!(store.get("orders/1").unwrap(), Some(Value::Integer(99)));
    assert_eq!(fresh.get("orders/1").unwrap(), Some(Value::Integer(7)));
    drop(store);
    let reopened = DocumentStore::open(dir.path()).unwrap();
    assert_eq!(reopened.get("orders/1").unwrap(), Some(Value::Integer(99)));
}

#[test]
fn tdd_conditional_delete_prevents_stale_and_aba_mutations() {
    let dir = TempDir::new();
    let mut store = DocumentStore::open(dir.path()).unwrap();
    store.put("orders/1", &Value::String("first".into())).unwrap();
    let before = store.snapshot();
    assert!(store.delete_if_unchanged(&before, "orders/1").is_ok());
    assert_eq!(store.get("orders/1").unwrap(), None);

    let after_delete_len = segment_len(dir.path(), 1);
    assert!(matches!(
        store.delete_if_unchanged(&before, "orders/1"),
        Err(StoreError::Conflict)
    ));
    assert_eq!(segment_len(dir.path(), 1), after_delete_len);

    // Delete followed by a new PUT must never match a stale old offset.
    store.put("orders/1", &Value::String("second".into())).unwrap();
    let after_reinsert_len = segment_len(dir.path(), 1);
    assert!(matches!(
        store.replace_if_unchanged(&before, "orders/1", &Value::Null),
        Err(StoreError::Conflict)
    ));
    assert_eq!(segment_len(dir.path(), 1), after_reinsert_len);
    assert_eq!(before.get("orders/1").unwrap(), Some(Value::String("first".into())));
    assert_eq!(store.get("orders/1").unwrap(), Some(Value::String("second".into())));

    let fresh = store.snapshot();
    store.delete_if_unchanged(&fresh, "orders/1").unwrap();
    assert_eq!(store.get("orders/1").unwrap(), None);

    // A snapshot of an ABSENT key has no tombstone/version token. Do not
    // accidentally claim absent-key ABA detection or conditional insertion.
    let absent = store.snapshot();
    let absent_len = segment_len(dir.path(), 1);
    assert!(matches!(
        store.replace_if_unchanged(&absent, "orders/1", &Value::Integer(1)),
        Err(StoreError::Conflict)
    ));
    assert!(matches!(
        store.delete_if_unchanged(&absent, "orders/1"),
        Err(StoreError::Conflict)
    ));
    assert_eq!(segment_len(dir.path(), 1), absent_len);
    assert!(matches!(
        store.replace_if_unchanged(&absent, "", &Value::Integer(1)),
        Err(StoreError::InvalidId)
    ));
}

#[test]
fn tdd_conditional_write_rejects_foreign_snapshot_even_at_matching_offset() {
    let left = TempDir::new();
    let right = TempDir::new();
    let mut a = DocumentStore::open(left.path()).unwrap();
    let mut b = DocumentStore::open(right.path()).unwrap();
    // Both first records normally have the same segment ID + offset (1,0).
    a.put("same", &Value::Integer(10)).unwrap();
    b.put("same", &Value::Integer(20)).unwrap();
    let wrong_db_view = a.snapshot();
    let before_len = segment_len(right.path(), 1);
    assert!(matches!(
        b.replace_if_unchanged(&wrong_db_view, "same", &Value::Integer(99)),
        Err(StoreError::InvalidSnapshot)
    ));
    assert!(matches!(
        b.delete_if_unchanged(&wrong_db_view, "same"),
        Err(StoreError::InvalidSnapshot)
    ));
    assert_eq!(segment_len(right.path(), 1), before_len);
    assert_eq!(b.get("same").unwrap(), Some(Value::Integer(20)));
}

#[test]
fn tdd_matching_version_allows_conditional_write_after_reopen_and_rollover() {
    let dir = TempDir::new();
    let old = Value::String("a".repeat(75));
    let mut store = DocumentStore::open_with_segment_limit(dir.path(), 120).unwrap();
    store.put("orders/1", &old).unwrap();
    let before = store.snapshot();
    for i in 0..8 {
        store.put(&format!("other/{i}"), &Value::Integer(i)).unwrap();
    }
    assert!(dir.path().join("segment-00000002.rdb").exists(),
        "test must exercise actual on-disk segment rollover");
    drop(store);

    let mut reopened = DocumentStore::open_with_segment_limit(dir.path(), 120).unwrap();
    reopened.replace_if_unchanged(&before, "orders/1", &Value::Integer(42)).unwrap();
    assert_eq!(before.get("orders/1").unwrap(), Some(old));
    assert_eq!(reopened.get("orders/1").unwrap(), Some(Value::Integer(42)));
    let stale_len = segment_len(dir.path(), 9);
    assert!(matches!(
        reopened.replace_if_unchanged(&before, "orders/1", &Value::Integer(43)),
        Err(StoreError::Conflict)
    ));
    assert_eq!(segment_len(dir.path(), 9), stale_len);
}
