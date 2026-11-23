//! #79 TDD: experimental immutable committed read views over append-only RYSF.
//! A snapshot owns only stable index metadata, never cached document payloads.
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
        let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let id = SERIAL.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rystemdb-snapshot-{}-{n}-{id}", std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path { &self.0 }
}
impl Drop for TempDir {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

fn nested() -> Value {
    Value::Object(BTreeMap::from([
        ("items".into(), Value::Array(vec![
            Value::Null,
            Value::Reference { collection:"users".into(), id:"u42".into() },
            Value::Bytes(vec![0, 255, 128]),
        ])),
        ("label".into(), Value::String("Roma 🏛".into())),
    ]))
}

#[test]
fn tdd_immutable_snapshots_preserve_distinct_committed_read_views() {
    let dir = TempDir::new();
    let mut store = DocumentStore::open(dir.path()).unwrap();
    let empty = store.snapshot();
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);

    store.put("orders/1", &nested()).unwrap();
    store.put("users/u42", &Value::Null).unwrap();
    let before = store.snapshot();
    assert_eq!(before.len(), 2);

    store.put("orders/1", &Value::Integer(8)).unwrap();
    assert!(store.delete("users/u42").unwrap());
    store.put("orders/new", &Value::Bool(true)).unwrap();
    let during = store.snapshot();

    store.put("orders/1", &Value::Integer(9)).unwrap();
    assert!(store.delete("orders/new").unwrap());
    let latest = store.snapshot();

    assert_eq!(empty.get("orders/1").unwrap(), None);
    assert_eq!(empty.get("users/u42").unwrap(), None);
    assert_eq!(before.get("orders/1").unwrap(), Some(nested()));
    assert_eq!(before.get("users/u42").unwrap(), Some(Value::Null));
    assert_eq!(before.get("orders/new").unwrap(), None);
    assert_eq!(before.len(), 2);
    assert_eq!(during.get("orders/1").unwrap(), Some(Value::Integer(8)));
    assert_eq!(during.get("users/u42").unwrap(), None);
    assert_eq!(during.get("orders/new").unwrap(), Some(Value::Bool(true)));
    assert_eq!(during.len(), 2);
    assert_eq!(latest.get("orders/1").unwrap(), Some(Value::Integer(9)));
    assert_eq!(latest.get("orders/new").unwrap(), None);
    assert_eq!(latest.len(), 1);
    assert_eq!(store.get("orders/1").unwrap(), Some(Value::Integer(9)));
}

#[test]
fn tdd_snapshot_retains_old_versions_during_rollover_and_after_writer_reopen() {
    let dir = TempDir::new();
    let old = Value::String("old-".to_owned() + &"x".repeat(84));
    let mut store = DocumentStore::open_with_segment_limit(dir.path(), 120).unwrap();
    store.put("orders/1", &old).unwrap();
    store.put("orders/2", &Value::Integer(2)).unwrap();
    let view = store.snapshot();
    store.put("orders/1", &Value::String("updated".into())).unwrap();
    assert!(store.delete("orders/2").unwrap());
    store.put("orders/3", &Value::Integer(3)).unwrap();
    drop(store);

    // Snapshot can open its historical immutable offsets without owning the
    // store's OS writer lock. A new writer can replay and append independently.
    let mut reopened = DocumentStore::open_with_segment_limit(dir.path(), 120).unwrap();
    reopened.put("orders/1", &Value::Integer(42)).unwrap();
    assert_eq!(view.get("orders/1").unwrap(), Some(old));
    assert_eq!(view.get("orders/2").unwrap(), Some(Value::Integer(2)));
    assert_eq!(view.get("orders/3").unwrap(), None);
    assert_eq!(view.len(), 2);
    assert_eq!(reopened.get("orders/1").unwrap(), Some(Value::Integer(42)));

    // Thread-safe read views can coexist with a writer appending new frames.
    let reader = std::thread::spawn(move || {
        for _ in 0..12 {
            assert_eq!(view.get("orders/2").unwrap(), Some(Value::Integer(2)));
        }
    });
    for i in 0..12 {
        reopened.put("orders/1", &Value::Integer(i)).unwrap();
    }
    reader.join().unwrap();
}

#[test]
fn tdd_snapshot_get_rechecks_checksum_instead_of_caching_unverified_payload() {
    let dir = TempDir::new();
    let mut store = DocumentStore::open(dir.path()).unwrap();
    store.put("orders/1", &Value::String("original".into())).unwrap();
    let view = store.snapshot();
    store.put("orders/1", &Value::String("new".into())).unwrap();

    // Corrupt the *old*, no-longer-current committed record on disk.
    // Header (20) + ID ("orders/1":8) + RDB header (5) +
    // string tag (1) + u32 length (4) = first byte of "original".
    let first_body_byte = 20 + "orders/1".len() + 5 + 1 + 4;
    let path = dir.path().join("segment-00000001.rdb");
    let mut file = OpenOptions::new().read(true).write(true).open(&path).unwrap();
    file.seek(SeekFrom::Start(first_body_byte as u64)).unwrap();
    let mut byte = [0u8; 1];
    file.read_exact(&mut byte).unwrap();
    file.seek(SeekFrom::Start(first_body_byte as u64)).unwrap();
    file.write_all(&[byte[0] ^ 0x40]).unwrap();
    file.sync_all().unwrap();
    drop(file);

    assert!(matches!(view.get("orders/1"), Err(StoreError::Corrupt(_))),
        "a snapshot must verify the original RYSF checksum on every read");
    assert_eq!(store.get("orders/1").unwrap(), Some(Value::String("new".into())));
}
