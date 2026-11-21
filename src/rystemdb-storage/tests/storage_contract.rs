use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rystemdb_core::Value;
use rystemdb_storage::{DocumentStore, StoreError};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let counter = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rystemdb-storage-test-{}-{now}-{counter}", std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path { &self.0 }
}
impl Drop for TempDir {
    fn drop(&mut self) { fs::remove_dir_all(&self.0).ok(); }
}

fn nested() -> Value {
    let mut address = BTreeMap::new();
    address.insert("city".to_string(), Value::String("Roma 🏛️".into()));
    address.insert("null".to_string(), Value::Null);
    let mut root = BTreeMap::new();
    root.insert("shipping".to_string(), Value::Object(address));
    root.insert("items".to_string(), Value::Array(vec![
        Value::Object(BTreeMap::new()),
        Value::Reference { collection: "users".into(), id: "user/42".into() },
        Value::Array(vec![Value::Integer(-123), Value::Bool(true)]),
    ]));
    root.insert("bytes".to_string(), Value::Bytes(vec![0, 1, 255]));
    Value::Object(root)
}

#[test]
fn tdd_red_put_get_reopen_preserves_nested_types_references_and_absent_fields() {
    let tmp = TempDir::new();
    let document = nested();
    {
        let mut store = DocumentStore::open(tmp.path()).unwrap();
        assert!(store.is_empty());
        store.put("orders/order:1", &document).unwrap();
        assert_eq!(store.len(), 1);
        assert_eq!(store.get("orders/order:1").unwrap(), Some(document.clone()));
        assert_eq!(store.get("does-not-exist").unwrap(), None);
    }
    let store = DocumentStore::open(tmp.path()).unwrap();
    assert_eq!(store.get("orders/order:1").unwrap(), Some(document));
    assert_eq!(store.len(), 1);
}

#[test]
fn tdd_red_overwrite_and_delete_survive_restart_without_resurrecting_values() {
    let tmp = TempDir::new();
    {
        let mut store = DocumentStore::open(tmp.path()).unwrap();
        store.put("id", &Value::Integer(1)).unwrap();
        store.put("id", &Value::Integer(2)).unwrap();
        store.put("other", &Value::String("keep".into())).unwrap();
        assert_eq!(store.len(), 2);
        assert_eq!(store.delete("id").unwrap(), true);
        assert_eq!(store.delete("id").unwrap(), false);
        assert_eq!(store.get("id").unwrap(), None);
    }
    let mut store = DocumentStore::open(tmp.path()).unwrap();
    assert_eq!(store.len(), 1);
    assert_eq!(store.get("other").unwrap(), Some(Value::String("keep".into())));
    assert_eq!(store.get("id").unwrap(), None);
    store.put("id", &Value::Integer(3)).unwrap();
    assert_eq!(store.get("id").unwrap(), Some(Value::Integer(3)));
}

#[test]
fn tdd_red_recovers_torn_final_frame_and_accepts_subsequent_appends() {
    let tmp = TempDir::new();
    {
        let mut store = DocumentStore::open(tmp.path()).unwrap();
        store.put("safe", &Value::Bool(true)).unwrap();
    }
    let segment = tmp.path().join("segment-00000001.rdb");
    let committed_length = fs::metadata(&segment).unwrap().len();
    {
        let mut f = OpenOptions::new().append(true).open(&segment).unwrap();
        f.write_all(b"RYSF\x01\x01").unwrap(); // interrupted frame header
        f.sync_all().unwrap();
    }
    {
        let mut store = DocumentStore::open(tmp.path()).unwrap();
        assert_eq!(store.get("safe").unwrap(), Some(Value::Bool(true)));
        assert_eq!(fs::metadata(&segment).unwrap().len(), committed_length);
        store.put("after-recovery", &Value::Integer(9)).unwrap();
    }
    let store = DocumentStore::open(tmp.path()).unwrap();
    assert_eq!(store.get("after-recovery").unwrap(), Some(Value::Integer(9)));
}

#[test]
fn tdd_red_rejects_corrupted_crc_instead_of_silently_returning_data() {
    let tmp = TempDir::new();
    {
        let mut store = DocumentStore::open(tmp.path()).unwrap();
        store.put("stable", &Value::String("original".into())).unwrap();
    }
    let segment = tmp.path().join("segment-00000001.rdb");
    let mut f = OpenOptions::new().read(true).write(true).open(segment).unwrap();
    f.seek(SeekFrom::End(-1)).unwrap();
    let mut last = [0u8; 1];
    f.read_exact(&mut last).unwrap();
    f.seek(SeekFrom::End(-1)).unwrap();
    f.write_all(&[last[0] ^ 0x80]).unwrap();
    f.sync_all().unwrap();
    drop(f);
    assert!(matches!(DocumentStore::open(tmp.path()), Err(StoreError::Corrupt(_))));
}

#[test]
fn tdd_red_rolls_segments_and_replays_all_records_in_order() {
    let tmp = TempDir::new();
    {
        let mut store = DocumentStore::open_with_segment_limit(tmp.path(), 120).unwrap();
        for i in 0..12 {
            store.put(&format!("id:{i}"), &Value::String(format!("test_{i}_{}", "x".repeat(70)))).unwrap();
        }
        assert_eq!(store.len(), 12);
    }
    let segments = fs::read_dir(tmp.path()).unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".rdb"))
        .count();
    assert!(segments >= 2, "expected rollover, found {segments} segment(s)");
    let store = DocumentStore::open(tmp.path()).unwrap();
    for i in 0..12 {
        assert_eq!(store.get(&format!("id:{i}")).unwrap(),
            Some(Value::String(format!("test_{i}_{}", "x".repeat(70)))));
    }
}

#[test]
fn tdd_red_rejects_truncation_in_older_segment_instead_of_repairing_history() {
    let tmp = TempDir::new();
    {
        let mut store = DocumentStore::open_with_segment_limit(tmp.path(), 120).unwrap();
        for i in 0..6 {
            store.put(&format!("id:{i}"), &Value::String("x".repeat(70))).unwrap();
        }
    }
    let first = tmp.path().join("segment-00000001.rdb");
    let file = OpenOptions::new().write(true).open(first).unwrap();
    let len = file.metadata().unwrap().len();
    file.set_len(len - 1).unwrap();
    file.sync_all().unwrap();
    assert!(matches!(DocumentStore::open(tmp.path()), Err(StoreError::Corrupt(_))));
}

#[test]
fn tdd_red_rejects_invalid_ids_and_too_deep_values() {
    let tmp = TempDir::new();
    let mut store = DocumentStore::open(tmp.path()).unwrap();
    assert!(matches!(store.put("", &Value::Null), Err(StoreError::InvalidId)));
    assert!(matches!(store.put(&"x".repeat(1025), &Value::Null), Err(StoreError::InvalidId)));
    let mut deep = Value::Null;
    for _ in 0..66 { deep = Value::Array(vec![deep]); }
    assert!(store.put("too-deep", &deep).is_err());
    assert_eq!(store.len(), 0);
}

#[test]
fn tdd_red_binary_segments_do_not_store_json_document_strings() {
    let tmp = TempDir::new();
    let mut store = DocumentStore::open(tmp.path()).unwrap();
    store.put("id", &nested()).unwrap();
    let bytes = fs::read(tmp.path().join("segment-00000001.rdb")).unwrap();
    assert!(bytes.starts_with(b"RYSF"));
    assert!(!bytes.starts_with(b"{"));
}
