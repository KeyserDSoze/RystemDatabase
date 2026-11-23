//! #83 RED: explicit point-key read-set validation for one-frame atomic batches.
//! Guards stale *read-only dependencies* in addition to keys being written.
//! Does not claim automatic read tracking, serializability, or phantom protection.
use rystemdb_core::Value;
use rystemdb_storage::{BatchMutation, DocumentStore, StoreError};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "rystemdb-read-set-{}-{stamp}-{serial}", std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn path(&self) -> &Path { &self.0 }
}
impl Drop for TempDir {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}
fn log_bytes(path: &Path) -> u64 {
    fs::read_dir(path).unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".rdb"))
        .map(|entry| entry.metadata().unwrap().len())
        .sum()
}

#[test]
fn tdd_read_only_dependency_change_prevents_write_skew_without_append() {
    let root = TempDir::new();
    let mut db = DocumentStore::open(root.path()).unwrap();
    db.put("oncall/alice", &Value::Bool(true)).unwrap();
    db.put("oncall/bob", &Value::Bool(true)).unwrap();
    let observed = db.snapshot();
    assert_eq!(observed.get("oncall/bob").unwrap(), Some(Value::Bool(true)));

    // Bob goes off call independently; Alice was never modified.
    db.put("oncall/bob", &Value::Bool(false)).unwrap();
    let before = log_bytes(root.path());
    let result = db.commit_batch_checked(&observed, &["oncall/bob"], &[
        BatchMutation::Put { id:"oncall/alice", value:&Value::Bool(false) },
        BatchMutation::Put { id:"audit/alice-off", value:&Value::Integer(1) },
    ]);
    assert!(matches!(result, Err(StoreError::Conflict)),
        "stale observed on-call dependency must abort before any writes");
    assert_eq!(log_bytes(root.path()), before);
    assert_eq!(db.get("oncall/alice").unwrap(), Some(Value::Bool(true)));
    assert_eq!(db.get("oncall/bob").unwrap(), Some(Value::Bool(false)));
    assert_eq!(db.get("audit/alice-off").unwrap(), None);

    // Unchecked legacy write-key-only semantics are deliberately preserved.
    // This succeeds because Alice/audit did not change, showing why the
    // explicit read-set method matters for the application invariant.
    let fresh = db.snapshot();
    db.commit_batch_checked(&fresh, &["oncall/bob"], &[
        BatchMutation::Put { id:"oncall/alice", value:&Value::Bool(true) },
        BatchMutation::Put { id:"audit/confirmed", value:&Value::String("seen-new-bob".into()) },
    ]).unwrap();
    assert_eq!(db.get("audit/confirmed").unwrap(),
        Some(Value::String("seen-new-bob".into())));
    drop(db);
    let reopened = DocumentStore::open(root.path()).unwrap();
    assert_eq!(reopened.get("audit/confirmed").unwrap(),
        Some(Value::String("seen-new-bob".into())));
    assert_eq!(observed.get("oncall/bob").unwrap(), Some(Value::Bool(true)));
}

#[test]
fn tdd_missing_read_dependency_put_delete_aba_conflicts_after_reopen_rollover() {
    let root = TempDir::new();
    let mut db = DocumentStore::open_with_segment_limit(root.path(), 120).unwrap();
    db.put("slot", &Value::Integer(7)).unwrap();
    let old = db.snapshot();
    assert_eq!(old.get("reservation/lock").unwrap(), None);
    db.put("reservation/lock", &Value::String("x".repeat(100))).unwrap();
    assert!(db.delete("reservation/lock").unwrap());
    db.put("unrelated", &Value::String("y".repeat(100))).unwrap();
    assert!(root.path().join("segment-00000002.rdb").is_file());
    drop(db);

    let mut db = DocumentStore::open_with_segment_limit(root.path(), 120).unwrap();
    let before = log_bytes(root.path());
    assert!(matches!(db.commit_batch_checked(&old, &["reservation/lock"], &[
        BatchMutation::Put { id:"slot", value:&Value::Integer(8) },
        BatchMutation::Put { id:"confirmed", value:&Value::Bool(true) },
    ]), Err(StoreError::Conflict)),
        "a recreated and deleted read-only key is not never-seen absence");
    assert_eq!(log_bytes(root.path()), before);
    assert_eq!(db.get("slot").unwrap(), Some(Value::Integer(7)));
    assert_eq!(db.get("confirmed").unwrap(), None);

    // A fresh view knows the newest tombstone version and is allowed to
    // proceed. Writes to unrelated IDs do not create false conflicts.
    let fresh = db.snapshot();
    db.put("outside", &Value::Integer(5)).unwrap();
    db.commit_batch_checked(&fresh, &["reservation/lock"], &[
        BatchMutation::Put { id:"slot", value:&Value::Integer(8) },
        BatchMutation::Put { id:"confirmed", value:&Value::Bool(true) },
    ]).unwrap();
    assert_eq!(db.get("slot").unwrap(), Some(Value::Integer(8)));
    assert_eq!(db.get("confirmed").unwrap(), Some(Value::Bool(true)));
    assert_eq!(fresh.get("slot").unwrap(), Some(Value::Integer(7)));
    drop(db);
    let reopened = DocumentStore::open_with_segment_limit(root.path(), 120).unwrap();
    assert_eq!(reopened.get("slot").unwrap(), Some(Value::Integer(8)));
    assert_eq!(reopened.get("confirmed").unwrap(), Some(Value::Bool(true)));
}

#[test]
fn tdd_overlapping_read_and_write_ids_allow_stable_keys_reject_new_versions() {
    let root = TempDir::new();
    let mut db = DocumentStore::open(root.path()).unwrap();
    db.put("a", &Value::Integer(1)).unwrap();
    db.put("b", &Value::Integer(2)).unwrap();
    let old = db.snapshot();
    db.put("b", &Value::Integer(2)).unwrap(); // identical bytes, different frame version
    let before = log_bytes(root.path());
    assert!(matches!(db.commit_batch_checked(&old, &["a", "b"], &[
        BatchMutation::Put {id:"a",value:&Value::Integer(3)},
        BatchMutation::Put {id:"c",value:&Value::Bool(true)},
    ]), Err(StoreError::Conflict)));
    assert_eq!(log_bytes(root.path()), before);
    let fresh = db.snapshot();
    db.commit_batch_checked(&fresh, &["a", "b"], &[
        BatchMutation::Put {id:"a",value:&Value::Integer(3)},
        BatchMutation::Put {id:"c",value:&Value::Bool(true)},
    ]).unwrap(); // a is both in the read set and mutated: valid
    assert_eq!(db.get("a").unwrap(), Some(Value::Integer(3)));
    assert_eq!(db.get("c").unwrap(), Some(Value::Bool(true)));
    assert_eq!(old.get("a").unwrap(), Some(Value::Integer(1)));
}

#[test]
fn tdd_read_set_bound_duplicates_invalid_id_and_foreign_root_fail_without_writes() {
    let root = TempDir::new();
    let foreign = TempDir::new();
    let foreign_view = DocumentStore::open(foreign.path()).unwrap().snapshot();
    let mut db = DocumentStore::open(root.path()).unwrap();
    let same = db.snapshot();
    let baseline = log_bytes(root.path());
    let ops = [
        BatchMutation::Put {id:"first",value:&Value::Null},
        BatchMutation::Put {id:"second",value:&Value::Bool(true)},
    ];
    assert!(matches!(
        db.commit_batch_checked(&foreign_view, &["first"], &ops),
        Err(StoreError::InvalidSnapshot)
    ));
    assert!(matches!(
        db.commit_batch_checked(&same, &["", "first"], &ops),
        Err(StoreError::InvalidId)
    ));
    assert!(matches!(
        db.commit_batch_checked(&same, &["repeat", "repeat"], &ops),
        Err(StoreError::InvalidBatch)
    ));
    let too_many = (0..1025).map(|i| format!("read-{i}")).collect::<Vec<_>>();
    let refs = too_many.iter().map(String::as_str).collect::<Vec<_>>();
    assert!(matches!(
        db.commit_batch_checked(&same, &refs, &ops),
        Err(StoreError::InvalidBatch)
    ));
    assert_eq!(log_bytes(root.path()), baseline);
    assert!(db.is_empty());
}
