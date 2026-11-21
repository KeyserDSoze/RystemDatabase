//! Verify that synthetic filesystem errors are absent from standard builds.
#![cfg(not(feature = "fault-injection"))]

use rystemdb_core::Value;
use rystemdb_storage::DocumentStore;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn tdd_normal_build_child_ignores_io_fault_environment() {
    let Some(root) = std::env::var_os("RYSTEMDB_NORMAL_IO_ROOT") else { return; };
    let mut store = DocumentStore::open(PathBuf::from(root)).unwrap();
    store.put("normal-build", &Value::Integer(42)).unwrap();
}

#[test]
fn tdd_normal_build_has_no_io_injection_hooks() {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!(
        "rystemdb-no-io-fault-{}-{nanos}", std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tdd_normal_build_child_ignores_io_fault_environment", "--nocapture"])
        .env("RYSTEMDB_NORMAL_IO_ROOT", &root)
        .env("RYSTEMDB_TEST_IO_ERROR", "io_before_append")
        .output().unwrap();
    assert!(output.status.success(), "normal build unexpectedly affected by fault env: {}",
        String::from_utf8_lossy(&output.stderr));
    let store = DocumentStore::open(&root).unwrap();
    assert_eq!(store.get("normal-build").unwrap(), Some(Value::Integer(42)));
    drop(store);
    fs::remove_dir_all(&root).unwrap();
}
