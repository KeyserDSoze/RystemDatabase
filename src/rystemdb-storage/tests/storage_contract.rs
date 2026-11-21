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
fn tdd_put_get_reopen_preserves_nested_types_references_and_absent_fields() {
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
fn tdd_overwrite_and_delete_survive_restart_without_resurrecting_values() {
    let tmp = TempDir::new();
    {
        let mut store = DocumentStore::open(tmp.path()).unwrap();
        store.put("id", &Value::Integer(1)).unwrap();
        store.put("id", &Value::Integer(2)).unwrap();
        store.put("other", &Value::String("keep".into())).unwrap();
        assert_eq!(store.len(), 2);
        assert!(store.delete("id").unwrap());
        assert!(!store.delete("id").unwrap());
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
fn tdd_recovers_torn_final_frame_and_accepts_subsequent_appends() {
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
fn tdd_rejects_corrupted_crc_instead_of_silently_returning_data() {
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
fn tdd_rolls_segments_and_replays_all_records_in_order() {
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
fn tdd_rejects_truncation_in_older_segment_instead_of_repairing_history() {
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
fn tdd_rejects_invalid_ids_and_too_deep_values() {
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
fn tdd_binary_segments_do_not_store_json_document_strings() {
    let tmp = TempDir::new();
    let mut store = DocumentStore::open(tmp.path()).unwrap();
    store.put("id", &nested()).unwrap();
    let bytes = fs::read(tmp.path().join("segment-00000001.rdb")).unwrap();
    assert!(bytes.starts_with(b"RYSF"));
    assert!(!bytes.starts_with(b"{"));
}


#[test]
fn tdd_discards_only_an_incomplete_final_payload() {
    let tmp = TempDir::new();
    {
        let mut store = DocumentStore::open(tmp.path()).unwrap();
        store.put("valid", &Value::Integer(11)).unwrap();
    }
    let segment = tmp.path().join("segment-00000001.rdb");
    let before = fs::metadata(&segment).unwrap().len();
    let mut f = OpenOptions::new().append(true).open(&segment).unwrap();
    let mut torn_frame = Vec::from(b"RYSF\x01\x01\x00\x00".as_slice());
    torn_frame.extend_from_slice(&4u32.to_be_bytes()); // 4 id bytes
    torn_frame.extend_from_slice(&12u32.to_be_bytes()); // 12 payload bytes expected
    torn_frame.extend_from_slice(&0u32.to_be_bytes()); // missing checksum/body
    torn_frame.extend_from_slice(b"part");
    torn_frame.extend_from_slice(b"RDB"); // only 3 of 12 payload bytes
    f.write_all(&torn_frame).unwrap();
    f.sync_all().unwrap();
    drop(f);

    let mut store = DocumentStore::open(tmp.path()).unwrap();
    assert_eq!(store.get("valid").unwrap(), Some(Value::Integer(11)));
    assert_eq!(fs::metadata(&segment).unwrap().len(), before);
    store.put("new", &Value::Null).unwrap();
    drop(store);
    assert_eq!(DocumentStore::open(tmp.path()).unwrap().get("new").unwrap(), Some(Value::Null));
}

#[test]
fn tdd_detects_checksum_damage_even_after_initial_open() {
    let tmp = TempDir::new();
    let mut store = DocumentStore::open(tmp.path()).unwrap();
    store.put("checksum", &Value::Integer(400)).unwrap();
    let segment = tmp.path().join("segment-00000001.rdb");
    let mut f = OpenOptions::new().read(true).write(true).open(segment).unwrap();
    f.seek(SeekFrom::End(-1)).unwrap();
    f.write_all(&[0xff]).unwrap();
    f.sync_all().unwrap();
    drop(f);
    assert!(matches!(store.get("checksum"), Err(StoreError::Corrupt(_))));
}

#[test]
fn tdd_rejects_missing_segment_without_silent_data_loss() {
    let tmp = TempDir::new();
    {
        let mut store = DocumentStore::open_with_segment_limit(tmp.path(), 120).unwrap();
        for i in 0..5 {
            store.put(&format!("id:{i}"), &Value::String("z".repeat(70))).unwrap();
        }
    }
    fs::remove_file(tmp.path().join("segment-00000001.rdb")).unwrap();
    assert!(matches!(DocumentStore::open(tmp.path()), Err(StoreError::Corrupt(_))));
}

#[test]
fn tdd_rejects_invalid_segment_config() {
    let tmp = TempDir::new();
    assert!(matches!(DocumentStore::open_with_segment_limit(tmp.path(), 0), Err(StoreError::InvalidSegmentLimit)));
}


#[test]
fn tdd_exclusive_lock_rejects_competing_writer_and_releases_on_drop() {
    let tmp = TempDir::new();
    let first = DocumentStore::open(tmp.path()).unwrap();
    assert!(matches!(DocumentStore::open(tmp.path()), Err(StoreError::Locked)),
        "second writer must be rejected as Locked");
    drop(first);
    let reopened = DocumentStore::open(tmp.path());
    assert!(reopened.is_ok(), "reopen must work after normal close; actual result: {reopened:?}");
}

// Explicit child entrypoints only run when invoked by the parent test with an env variable.
#[test]
fn tdd_exclusive_lock_child_probe() {
    if let Some(path) = std::env::var_os("RYSTEMDB_LOCK_PROBE_DIR") {
        assert!(matches!(DocumentStore::open(PathBuf::from(path)), Err(StoreError::Locked)),
            "concurrent child process unexpectedly acquired writer lock");
    }
}

#[test]
fn tdd_exclusive_lock_rejects_another_process() {
    let tmp = TempDir::new();
    let _first = DocumentStore::open(tmp.path()).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tdd_exclusive_lock_child_probe", "--nocapture"])
        .env("RYSTEMDB_LOCK_PROBE_DIR", tmp.path())
        .output().unwrap();
    assert!(output.status.success(), "child probe must reject writer: {}",
        String::from_utf8_lossy(&output.stderr));
}

#[test]
fn tdd_lock_holder_child() {
    if let Some(path) = std::env::var_os("RYSTEMDB_LOCK_HOLDER_DIR") {
        let path = PathBuf::from(path);
        let _store = DocumentStore::open(&path).unwrap();
        fs::write(path.join("lock-holder-ready"), b"ready").unwrap();
        loop { std::thread::sleep(std::time::Duration::from_secs(1)); }
    }
}

#[test]
fn tdd_crashed_lock_holder_does_not_leave_stale_ownership() {
    use std::time::Duration;
    let tmp = TempDir::new();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tdd_lock_holder_child", "--nocapture"])
        .env("RYSTEMDB_LOCK_HOLDER_DIR", tmp.path())
        .spawn().unwrap();
    let ready = tmp.path().join("lock-holder-ready");
    let mut started = false;
    for _ in 0..100 {
        if ready.is_file() { started = true; break; }
        if child.try_wait().unwrap().is_some() { break; }
        std::thread::sleep(Duration::from_millis(20));
    }
    if !started {
        child.kill().ok();
        child.wait().ok();
        panic!("lock holder child failed to initialize");
    }
    let competing = DocumentStore::open(tmp.path());
    child.kill().ok(); // forced process termination: no Rust Drop in child
    child.wait().unwrap();
    assert!(matches!(competing, Err(StoreError::Locked)), "concurrent writer should not enter");
    assert!(DocumentStore::open(tmp.path()).is_ok(),
        "OS must release the lock automatically on process death");
}


#[test]
fn tdd_writer_lock_is_per_directory_and_file_is_not_deleted_on_exit() {
    let first_dir = TempDir::new();
    let second_dir = TempDir::new();
    let first = DocumentStore::open(first_dir.path()).unwrap();
    let other = DocumentStore::open(second_dir.path()).unwrap();
    let lock_path = first_dir.path().join("rystemdb.writer.lock");
    assert!(lock_path.is_file(), "the lock file must exist while held");
    drop(first);
    assert!(lock_path.is_file(), "unlock must not unlink the lock inode");
    drop(other);
    assert!(DocumentStore::open(first_dir.path()).is_ok(),
        "existing lock file must not block a new process without an OS lock");
}


// Child entrypoint: a separate process must acquire the writer lock immediately
// after the original holder exits, even when another process previously failed
// try_lock. The child verifies both lock ownership and replay of synced data.
#[test]
fn tdd_lock_recovery_child_probe() {
    if let Some(path) = std::env::var_os("RYSTEMDB_LOCK_RECOVERY_PROBE_DIR") {
        let pass: i64 = std::env::var("RYSTEMDB_LOCK_RECOVERY_PASS")
            .unwrap().parse().unwrap();
        let mut store = DocumentStore::open(PathBuf::from(path))
            .unwrap_or_else(|err| panic!("fresh process could not recover writer lock at pass {pass}: {err:?}"));
        assert_eq!(store.get("sentinel").unwrap(), Some(Value::Integer(pass)));
        store.put("child-confirmation", &Value::Integer(pass)).unwrap();
    }
}

// Regression for #19: repeated failed cross-process lock attempts must not
// poison ownership or storage replay. No sleep, retry, or ignored error masks
// a stale advisory lock. This complements the 50 same-process and 40 synced
// write/reopen cycles above/below and runs on Linux, macOS and Windows.
#[test]
fn tdd_lock_reopens_across_processes_after_competing_probe() {
    let tmp = TempDir::new();
    let test_binary = std::env::current_exe().unwrap();
    for pass in 0_i64..12 {
        let mut writer = DocumentStore::open(tmp.path())
            .unwrap_or_else(|err| panic!("original writer failed at pass {pass}: {err:?}"));
        writer.put("sentinel", &Value::Integer(pass)).unwrap();

        let competitor = std::process::Command::new(&test_binary)
            .args(["--exact", "tdd_exclusive_lock_child_probe", "--nocapture"])
            .env("RYSTEMDB_LOCK_PROBE_DIR", tmp.path())
            .output().unwrap();
        assert!(competitor.status.success(),
            "competing writer not rejected at pass {pass}: stdout={} stderr={}",
            String::from_utf8_lossy(&competitor.stdout),
            String::from_utf8_lossy(&competitor.stderr));

        drop(writer);
        let recovery = std::process::Command::new(&test_binary)
            .args(["--exact", "tdd_lock_recovery_child_probe", "--nocapture"])
            .env("RYSTEMDB_LOCK_RECOVERY_PROBE_DIR", tmp.path())
            .env("RYSTEMDB_LOCK_RECOVERY_PASS", pass.to_string())
            .output().unwrap();
        assert!(recovery.status.success(),
            "new process could not reopen after contention at pass {pass}: stdout={} stderr={}",
            String::from_utf8_lossy(&recovery.stdout),
            String::from_utf8_lossy(&recovery.stderr));

        let inspect = DocumentStore::open(tmp.path())
            .unwrap_or_else(|err| panic!("parent could not reopen at pass {pass}: {err:?}"));
        assert_eq!(inspect.get("child-confirmation").unwrap(), Some(Value::Integer(pass)));
    }
}

#[test]
fn tdd_repeated_competing_open_and_reopen_never_sticks() {
    let tmp = TempDir::new();
    for pass in 0..50 {
        let first = DocumentStore::open(tmp.path()).unwrap_or_else(|err| {
            panic!("writer unexpectedly locked at iteration {pass}: {err:?}");
        });
        assert!(matches!(DocumentStore::open(tmp.path()), Err(StoreError::Locked)),
            "second writer must be excluded at iteration {pass}");
        drop(first);
        let second = DocumentStore::open(tmp.path()).unwrap_or_else(|err| {
            panic!("lock was not released at iteration {pass}: {err:?}");
        });
        drop(second);
    }
}


#[test]
fn tdd_repeated_synced_writes_immediately_reopen_without_stale_lock() {
    let tmp = TempDir::new();
    for iteration in 0_i64..40 {
        let mut store = DocumentStore::open(tmp.path()).unwrap_or_else(|error| {
            panic!("open before synced write {iteration} failed: {error:?}");
        });
        store.put("latest", &Value::Integer(iteration)).unwrap();
        drop(store);
        let reopened = DocumentStore::open(tmp.path()).unwrap_or_else(|error| {
            panic!("immediate reopen after synced write {iteration} failed: {error:?}");
        });
        assert_eq!(reopened.get("latest").unwrap(), Some(Value::Integer(iteration)));
        drop(reopened);
    }
}
