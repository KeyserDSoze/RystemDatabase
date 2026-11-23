//! #82 RED: one synced RYSF frame for an optimistic multi-key batch.
//! Not a general SQL/serializable transaction engine.
use rystemdb_core::Value;
use rystemdb_storage::{BatchMutation, DocumentStore, StoreError};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
#[cfg(feature = "fault-injection")]
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static SERIAL: AtomicU64 = AtomicU64::new(0);
struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let n = SERIAL.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rystemdb-batch-{}-{stamp}-{n}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path { &self.0 }
}
impl Drop for TempDir {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}
fn log_len(root: &Path) -> u64 {
    fs::read_dir(root).unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".rdb"))
        .map(|e| e.metadata().unwrap().len())
        .sum()
}

#[test]
fn tdd_batch_commit_atomic_mixed_typed_values_and_snapshot_replay() {
    let root = TempDir::new();
    let mut db = DocumentStore::open(root.path()).unwrap();
    db.put("account/a", &Value::Integer(100)).unwrap();
    db.put("account/b", &Value::Integer(50)).unwrap();
    db.put("pending", &Value::Null).unwrap();
    let old = db.snapshot();
    let before = log_len(root.path());
    db.commit_batch(&old, &[
        BatchMutation::Put { id:"account/a", value:&Value::Integer(70) },
        BatchMutation::Put { id:"account/b", value:&Value::Integer(80) },
        BatchMutation::Delete { id:"pending" },
        BatchMutation::Put { id:"receipt", value:&Value::Array(vec![
            Value::Reference {collection:"account".into(),id:"a".into()},
            Value::Null, Value::Bytes(vec![0,255])
        ]) },
    ]).unwrap();
    assert!(log_len(root.path()) > before);
    assert_eq!(db.len(), 3);
    assert_eq!(db.get("account/a").unwrap(), Some(Value::Integer(70)));
    assert_eq!(db.get("account/b").unwrap(), Some(Value::Integer(80)));
    assert_eq!(db.get("pending").unwrap(), None);
    assert_eq!(old.get("account/a").unwrap(), Some(Value::Integer(100)));
    assert_eq!(old.get("pending").unwrap(), Some(Value::Null));
    assert_eq!(old.get("receipt").unwrap(), None);
    let current = db.snapshot();
    assert_eq!(current.get("account/a").unwrap(), Some(Value::Integer(70)));
    assert_eq!(current.get("account/b").unwrap(), Some(Value::Integer(80)));
    drop(db);
    let mut reopened = DocumentStore::open(root.path()).unwrap();
    assert_eq!(reopened.len(), 3);
    assert_eq!(reopened.get("account/a").unwrap(), Some(Value::Integer(70)));
    assert_eq!(reopened.get("account/b").unwrap(), Some(Value::Integer(80)));
    assert_eq!(reopened.get("pending").unwrap(), None);
    assert_eq!(old.get("account/a").unwrap(), Some(Value::Integer(100)));
    reopened.insert_if_unchanged(&current, "receipt-2", &Value::Bool(true)).unwrap();
    assert!(matches!(reopened.replace_if_unchanged(&old, "account/a", &Value::Null),
        Err(StoreError::Conflict)));
}

#[test]
fn tdd_batch_rejects_stale_or_invalid_group_without_any_append() {
    let root = TempDir::new();
    let mut db = DocumentStore::open(root.path()).unwrap();
    db.put("balance", &Value::Integer(10)).unwrap();
    let before = db.snapshot();
    db.put("unrelated", &Value::Integer(7)).unwrap();
    db.commit_batch(&before, &[
        BatchMutation::Put{id:"balance",value:&Value::Integer(8)},
        BatchMutation::Put{id:"new",value:&Value::Integer(2)},
    ]).unwrap();
    let stale_len = log_len(root.path());
    assert!(matches!(db.commit_batch(&before, &[
        BatchMutation::Put{id:"balance",value:&Value::Integer(9)},
        BatchMutation::Put{id:"not-committed",value:&Value::Bool(true)},
    ]),Err(StoreError::Conflict)));
    assert_eq!(log_len(root.path()),stale_len);
    assert_eq!(db.get("not-committed").unwrap(),None);
    assert!(matches!(db.commit_batch(&db.snapshot(), &[]),Err(StoreError::InvalidBatch)));
    assert!(matches!(db.commit_batch(&db.snapshot(), &[
        BatchMutation::Put{id:"dup",value:&Value::Null},
        BatchMutation::Delete{id:"dup"},
    ]),Err(StoreError::InvalidBatch)));
    assert!(matches!(db.commit_batch(&db.snapshot(), &[
        BatchMutation::Put{id:"invalid",value:&Value::Null},
        BatchMutation::Put{id:"",value:&Value::Integer(1)},
    ]),Err(StoreError::InvalidId)));
    assert!(matches!(db.commit_batch(&db.snapshot(), &[
        BatchMutation::Put{id:"x",value:&Value::Null},
        BatchMutation::Delete{id:"absent"},
    ]),Err(StoreError::Conflict)));
    assert_eq!(log_len(root.path()),stale_len);

    // An absent -> put -> delete ABA is detected by the tombstone ledger.
    let absent = db.snapshot();
    db.put("aba", &Value::Integer(1)).unwrap();
    assert!(db.delete("aba").unwrap());
    let aba_len = log_len(root.path());
    assert!(matches!(db.commit_batch(&absent, &[
        BatchMutation::Put{id:"balance",value:&Value::Integer(8)},
        BatchMutation::Put{id:"aba",value:&Value::Integer(2)},
    ]),Err(StoreError::Conflict)));
    assert_eq!(log_len(root.path()),aba_len);
}

#[test]
fn tdd_batch_replay_rollover_and_corrupt_complete_frame_fail_closed() {
    let root = TempDir::new();
    let mut db = DocumentStore::open_with_segment_limit(root.path(),120).unwrap();
    db.put("stable", &Value::String("x".repeat(90))).unwrap();
    let snap = db.snapshot();
    db.commit_batch(&snap, &[
        BatchMutation::Put{id:"a",value:&Value::Integer(1)},
        BatchMutation::Put{id:"b",value:&Value::Integer(2)},
    ]).unwrap();
    assert!(root.path().join("segment-00000002.rdb").exists());
    drop(db);
    let db = DocumentStore::open_with_segment_limit(root.path(),120).unwrap();
    assert_eq!(db.get("stable").unwrap(),Some(Value::String("x".repeat(90))));
    assert_eq!(db.get("a").unwrap(),Some(Value::Integer(1)));
    assert_eq!(db.get("b").unwrap(),Some(Value::Integer(2)));
    drop(db);

    let path = root.path().join("segment-00000002.rdb");
    let mut f=OpenOptions::new().read(true).write(true).open(path).unwrap();
    f.seek(SeekFrom::End(-1)).unwrap();
    let mut byte=[0u8;1]; f.read_exact(&mut byte).unwrap();
    f.seek(SeekFrom::End(-1)).unwrap();
    f.write_all(&[byte[0]^0x80]).unwrap(); f.sync_all().unwrap();
    assert!(matches!(DocumentStore::open(root.path()),Err(StoreError::Corrupt(_))),
        "complete checksummed batch damage must not replay as partial writes");
}

#[test]
fn tdd_batch_rejects_foreign_root_and_excessive_mutations() {
    let root=TempDir::new(); let other=TempDir::new();
    let foreign=DocumentStore::open(other.path()).unwrap().snapshot();
    let mut db=DocumentStore::open(root.path()).unwrap();
    let start=log_len(root.path());
    assert!(matches!(db.commit_batch(&foreign,&[
        BatchMutation::Put{id:"a",value:&Value::Null}
    ]),Err(StoreError::InvalidSnapshot)));
    let ids=(0..1025).map(|i|format!("key-{i}")).collect::<Vec<_>>();
    let operations=ids.iter().map(|id|BatchMutation::Put {
        id:id.as_str(),value:&Value::Null
    }).collect::<Vec<_>>();
    assert!(matches!(db.commit_batch(&db.snapshot(),&operations),
        Err(StoreError::InvalidBatch)));
    assert_eq!(log_len(root.path()),start);
}

// #82 real process faults are only active with the fault-injection feature.
#[test]
fn tdd_batch_fault_child() {
    let Some(root) = std::env::var_os("RYSTEMDB_BATCH_CHILD_ROOT") else {return};
    let mut db=DocumentStore::open(PathBuf::from(root)).unwrap();
    let snap=db.snapshot();
    db.commit_batch(&snap,&[
        BatchMutation::Put{id:"old",value:&Value::Integer(21)},
        BatchMutation::Put{id:"new",value:&Value::String("batch".repeat(20))},
    ]).unwrap();
}

#[cfg(feature="fault-injection")]
fn crash_batch(root: &Path,point: &str) {
    let o=Command::new(std::env::current_exe().unwrap())
        .args(["--exact","tdd_batch_fault_child","--nocapture"])
        .env("RYSTEMDB_BATCH_CHILD_ROOT",root)
        .env("RYSTEMDB_TEST_CRASH_POINT",point)
        .output().unwrap();
    assert_eq!(o.status.code(),Some(97),
        "child at {point} failed: status={:?} stdout={} stderr={}",
        o.status,String::from_utf8_lossy(&o.stdout),String::from_utf8_lossy(&o.stderr));
}

#[cfg(feature="fault-injection")]
#[test]
fn tdd_batch_torn_tail_and_synced_crash_replay_all_or_none() {
    for point in ["before_append","partial_frame","after_frame_before_sync","after_sync_before_publish"] {
        let root=TempDir::new();
        let mut db=DocumentStore::open(root.path()).unwrap();
        db.put("old",&Value::Integer(10)).unwrap(); drop(db);
        let before=log_len(root.path());
        crash_batch(root.path(),point);
        let repaired=DocumentStore::open(root.path()).unwrap();
        let a=repaired.get("old").unwrap(); let b=repaired.get("new").unwrap();
        match point {
            "before_append"|"partial_frame"=>{
                assert_eq!(a,Some(Value::Integer(10)),"{point}");
                assert_eq!(b,None,"{point}");
            }
            "after_sync_before_publish"=>{
                assert_eq!(a,Some(Value::Integer(21)),"{point}");
                assert_eq!(b,Some(Value::String("batch".repeat(20))),"{point}");
            }
            _=>{
                let all=a==Some(Value::Integer(21)) &&
                    b==Some(Value::String("batch".repeat(20)));
                let none=a==Some(Value::Integer(10)) && b.is_none();
                assert!(all||none,"unsynced frame partially replayed: {a:?}, {b:?}");
            }
        }
        if point=="partial_frame" {assert_eq!(log_len(root.path()),before);}
    }
}
