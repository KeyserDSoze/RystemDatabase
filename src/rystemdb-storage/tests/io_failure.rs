//! TDD: deterministic simulated I/O failures, not actual hardware faults.
//! Fault switches exist only with --features fault-injection.
#![cfg(feature = "fault-injection")]

use rystemdb_core::Value;
use rystemdb_storage::{DocumentStore, StoreError};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static SERIAL: AtomicU64 = AtomicU64::new(0);

struct Temporary(PathBuf);
impl Temporary {
    fn new() -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rystemdb-io-fault-{}-{nanos}-{serial}", std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path { &self.0 }
}
impl Drop for Temporary {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

fn baseline(root: &Path) {
    let mut store = DocumentStore::open(root).unwrap();
    store.put("baseline", &Value::String("B".repeat(85))).unwrap();
}

fn candidate() -> Value {
    Value::String("candidate-".repeat(7))
}

// Entry point run as a child process, not as a concurrent env-mutating test.
#[test]
fn tdd_io_failure_child() {
    let Some(root) = std::env::var_os("RYSTEMDB_IO_TEST_ROOT") else { return; };
    let point = std::env::var("RYSTEMDB_TEST_IO_ERROR").unwrap();
    let op = std::env::var("RYSTEMDB_IO_TEST_OP").unwrap();
    let limit = if point.starts_with("io_segment_") { 128 } else { 64 * 1024 * 1024 };
    let mut store = DocumentStore::open_with_segment_limit(PathBuf::from(root), limit).unwrap();
    let failed = if op == "delete" {
        store.delete("baseline").map(|_| ())
    } else {
        store.put("candidate", &candidate())
    };
    match failed {
        Err(StoreError::Io(error)) => {
            assert!(error.to_string().contains("injected"), "unexpected I/O error: {error}")
        }
        other => panic!("expected injected I/O error at {point}, got {other:?}"),
    }

    // Once uncertain, *all* mutations, including a no-op delete, fail closed.
    assert!(matches!(store.put("blocked", &Value::Integer(9)), Err(StoreError::Poisoned)));
    assert!(matches!(store.delete("baseline"), Err(StoreError::Poisoned)));
    assert!(matches!(store.delete("absent-id"), Err(StoreError::Poisoned)));
    assert_eq!(store.get("blocked").unwrap(), None);
}

fn run_failure(point: &str, op: &str) -> Temporary {
    let tmp = Temporary::new();
    baseline(tmp.path());
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tdd_io_failure_child", "--nocapture"])
        .env("RYSTEMDB_IO_TEST_ROOT", tmp.path())
        .env("RYSTEMDB_TEST_IO_ERROR", point)
        .env("RYSTEMDB_IO_TEST_OP", op)
        .output().unwrap();
    assert!(child.status.success(),
        "I/O fault child failed at {point}: status={:?}; stdout={}; stderr={}",
        child.status, String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr));
    tmp
}

fn assert_baseline(store: &DocumentStore) {
    assert_eq!(store.get("baseline").unwrap(), Some(Value::String("B".repeat(85))));
}

#[test]
fn tdd_failure_before_append_poisoned_and_reopens_cleanly() {
    let tmp = run_failure("io_before_append", "put");
    let mut store = DocumentStore::open(tmp.path()).unwrap();
    assert_baseline(&store);
    assert_eq!(store.get("candidate").unwrap(), None);
    store.put("recovered", &Value::Bool(true)).unwrap();
    drop(store);
    let store = DocumentStore::open(tmp.path()).unwrap();
    assert_eq!(store.get("recovered").unwrap(), Some(Value::Bool(true)));
}

#[test]
fn tdd_short_write_failure_truncates_only_incomplete_final_frame() {
    let tmp = Temporary::new();
    baseline(tmp.path());
    let segment = tmp.path().join("segment-00000001.rdb");
    let original = fs::metadata(&segment).unwrap().len();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tdd_io_failure_child", "--nocapture"])
        .env("RYSTEMDB_IO_TEST_ROOT", tmp.path())
        .env("RYSTEMDB_TEST_IO_ERROR", "io_short_write")
        .env("RYSTEMDB_IO_TEST_OP", "put")
        .output().unwrap();
    assert!(child.status.success(), "child: {}", String::from_utf8_lossy(&child.stderr));
    assert!(fs::metadata(&segment).unwrap().len() > original);
    let store = DocumentStore::open(tmp.path()).unwrap();
    assert_baseline(&store);
    assert_eq!(store.get("candidate").unwrap(), None);
    assert_eq!(fs::metadata(&segment).unwrap().len(), original);
}

#[test]
fn tdd_failed_sync_data_remains_uncertain_but_recoverable() {
    let tmp = run_failure("io_sync_data", "put");
    let mut store = DocumentStore::open(tmp.path()).unwrap();
    assert_baseline(&store);
    let found = store.get("candidate").unwrap();
    assert!(found.is_none() || found == Some(candidate()));
    store.put("post-replay", &Value::Integer(5)).unwrap();
}

#[test]
fn tdd_error_after_sync_must_replay_put_without_acknowledging_it() {
    let tmp = run_failure("io_after_sync", "put");
    let store = DocumentStore::open(tmp.path()).unwrap();
    assert_baseline(&store);
    assert_eq!(store.get("candidate").unwrap(), Some(candidate()));
}

#[test]
fn tdd_failed_sync_data_on_delete_does_not_corrupt_history() {
    let tmp = run_failure("io_sync_data", "delete");
    let store = DocumentStore::open(tmp.path()).unwrap();
    let found = store.get("baseline").unwrap();
    assert!(found.is_none() || found == Some(Value::String("B".repeat(85))));
}

#[test]
fn tdd_error_after_sync_replays_delete_tombstone() {
    let tmp = run_failure("io_after_sync", "delete");
    let store = DocumentStore::open(tmp.path()).unwrap();
    assert!(store.is_empty());
    assert_eq!(store.get("baseline").unwrap(), None);
}

#[test]
fn tdd_failed_segment_creation_keeps_previous_data_and_no_new_segment() {
    let tmp = run_failure("io_segment_create", "put");
    assert!(!tmp.path().join("segment-00000002.rdb").exists());
    let mut store = DocumentStore::open_with_segment_limit(tmp.path(), 128).unwrap();
    assert_baseline(&store);
    assert_eq!(store.get("candidate").unwrap(), None);
    store.put("candidate", &candidate()).unwrap();
}

#[test]
fn tdd_failed_new_segment_sync_reopens_empty_segment() {
    let tmp = run_failure("io_segment_sync", "put");
    let second = tmp.path().join("segment-00000002.rdb");
    assert_eq!(fs::metadata(&second).unwrap().len(), 0);
    let mut store = DocumentStore::open_with_segment_limit(tmp.path(), 128).unwrap();
    assert_baseline(&store);
    assert_eq!(store.get("candidate").unwrap(), None);
    store.put("candidate", &candidate()).unwrap();
}

#[test]
fn tdd_failed_directory_sync_keeps_previous_data_and_reopens() {
    let tmp = run_failure("io_segment_directory_sync", "put");
    assert!(tmp.path().join("segment-00000002.rdb").exists());
    let mut store = DocumentStore::open_with_segment_limit(tmp.path(), 128).unwrap();
    assert_baseline(&store);
    assert_eq!(store.get("candidate").unwrap(), None);
    store.put("candidate", &candidate()).unwrap();
}
