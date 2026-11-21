//! Deliberate child-process exit tests for the single-document append-only contract.
//! The fault-injection feature is NEVER enabled in production builds.
use rystemdb_core::Value;
use rystemdb_storage::DocumentStore;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let serial = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir()
            .join(format!("rystem-crash-{}-{now}-{serial}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn path(&self) -> &Path { &self.0 }
}
impl Drop for TempDir {
    fn drop(&mut self) { fs::remove_dir_all(&self.0).ok(); }
}

fn baseline(root: &Path) {
    let mut store = DocumentStore::open(root).unwrap();
    store.put("baseline", &Value::String("B".repeat(85))).unwrap();
}

fn next_value() -> Value {
    Value::String("payload".repeat(10))
}

// The test binary is invoked as a separate OS process; feature hooks must exit
// after the selected file-system operation but before a regular Rust return.
#[test]
fn tdd_fault_child() {
    let Some(root) = std::env::var_os("RYSTEMDB_FAULT_CHILD_ROOT") else { return; };
    let root = PathBuf::from(root);
    let point = std::env::var("RYSTEMDB_TEST_CRASH_POINT").unwrap();
    let limit = if point == "after_rotate" { 128 } else { 64 * 1024 * 1024 };
    let mut store = DocumentStore::open_with_segment_limit(&root, limit).unwrap();
    if std::env::var("RYSTEMDB_FAULT_OP").unwrap() == "delete" {
        assert!(store.delete("baseline").unwrap());
    } else {
        store.put("new-document", &next_value()).unwrap();
    }
}

// A fresh child, using the same integration test executable, must be terminated
// at the hook. Plain sequential function calls do not constitute crash testing.
fn crash_child(root: &Path, point: &str, operation: &str) {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tdd_fault_child", "--nocapture"])
        .env("RYSTEMDB_FAULT_CHILD_ROOT", root)
        .env("RYSTEMDB_TEST_CRASH_POINT", point)
        .env("RYSTEMDB_FAULT_OP", operation)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(97),
        "child did not terminate at {point} ({operation}): status={:?}, stdout={}, stderr={}",
        output.status, String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr));
}

#[cfg(feature = "fault-injection")]
#[test]
fn tdd_crash_before_append_preserves_only_acknowledged_data() {
    let root = TempDir::new();
    baseline(root.path());
    crash_child(root.path(), "before_append", "put");
    let mut store = DocumentStore::open(root.path()).unwrap();
    assert_eq!(store.get("baseline").unwrap(), Some(Value::String("B".repeat(85))));
    assert_eq!(store.get("new-document").unwrap(), None);
    store.put("subsequent", &Value::Integer(1)).unwrap();
    drop(store);
    assert_eq!(DocumentStore::open(root.path()).unwrap().get("subsequent").unwrap(), Some(Value::Integer(1)));
}

#[cfg(feature = "fault-injection")]
#[test]
fn tdd_crash_mid_payload_truncates_only_last_incomplete_frame() {
    let root = TempDir::new();
    baseline(root.path());
    let segment = root.path().join("segment-00000001.rdb");
    let original_size = fs::metadata(&segment).unwrap().len();
    crash_child(root.path(), "partial_frame", "put");
    assert!(fs::metadata(&segment).unwrap().len() > original_size,
        "the fault must actually write a partial frame to disk");
    let store = DocumentStore::open(root.path()).unwrap();
    assert_eq!(store.get("baseline").unwrap(), Some(Value::String("B".repeat(85))));
    assert_eq!(store.get("new-document").unwrap(), None);
    assert_eq!(fs::metadata(&segment).unwrap().len(), original_size);
}

#[cfg(feature = "fault-injection")]
#[test]
fn tdd_crash_after_full_unsynced_frame_is_uncertain_but_never_corrupt() {
    let root = TempDir::new();
    baseline(root.path());
    crash_child(root.path(), "after_frame_before_sync", "put");
    let mut store = DocumentStore::open(root.path()).unwrap();
    assert_eq!(store.get("baseline").unwrap(), Some(Value::String("B".repeat(85))));
    let result = store.get("new-document").unwrap();
    assert!(result.is_none() || result == Some(next_value()),
        "an unacknowledged append may replay but never in a partial form");
    store.put("after-recovery", &Value::Bool(true)).unwrap();
}

#[cfg(feature = "fault-injection")]
#[test]
fn tdd_crash_after_sync_before_in_memory_publication_replays_put() {
    let root = TempDir::new();
    baseline(root.path());
    crash_child(root.path(), "after_sync_before_publish", "put");
    let store = DocumentStore::open(root.path()).unwrap();
    assert_eq!(store.get("baseline").unwrap(), Some(Value::String("B".repeat(85))));
    assert_eq!(store.get("new-document").unwrap(), Some(next_value()));
}

#[cfg(feature = "fault-injection")]
#[test]
fn tdd_crash_after_sync_before_in_memory_publication_replays_delete() {
    let root = TempDir::new();
    baseline(root.path());
    crash_child(root.path(), "after_sync_before_publish", "delete");
    let store = DocumentStore::open(root.path()).unwrap();
    assert_eq!(store.get("baseline").unwrap(), None);
    assert!(store.is_empty());
}

#[cfg(feature = "fault-injection")]
#[test]
fn tdd_crash_after_new_segment_directory_sync_keeps_old_records() {
    let root = TempDir::new();
    baseline(root.path());
    crash_child(root.path(), "after_rotate", "put");
    let active = root.path().join("segment-00000002.rdb");
    assert!(active.is_file(), "rotation should create the second segment before the crash");
    assert_eq!(fs::metadata(&active).unwrap().len(), 0,
        "the second segment must still be empty at this fault point");
    let mut store = DocumentStore::open_with_segment_limit(root.path(), 128).unwrap();
    assert_eq!(store.get("baseline").unwrap(), Some(Value::String("B".repeat(85))));
    assert_eq!(store.get("new-document").unwrap(), None);
    store.put("new-document", &next_value()).unwrap();
    drop(store);
    assert_eq!(DocumentStore::open(root.path()).unwrap().get("new-document").unwrap(), Some(next_value()));
}

#[cfg(not(feature = "fault-injection"))]
#[test]
fn tdd_production_build_ignores_test_crash_environment() {
    let root = TempDir::new();
    baseline(root.path());
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tdd_fault_child", "--nocapture"])
        .env("RYSTEMDB_FAULT_CHILD_ROOT", root.path())
        .env("RYSTEMDB_TEST_CRASH_POINT", "after_sync_before_publish")
        .env("RYSTEMDB_FAULT_OP", "put")
        .output().unwrap();
    assert!(output.status.success(),
        "default build must not terminate from fault hooks: {}",
        String::from_utf8_lossy(&output.stderr));
    assert_eq!(DocumentStore::open(root.path()).unwrap().get("new-document").unwrap(),
        Some(next_value()));
}
