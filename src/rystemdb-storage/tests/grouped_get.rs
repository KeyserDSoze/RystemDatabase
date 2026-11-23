//! #91 RED: CRC-verified, bounded multi-get with one read per shared frame.
//! No persistent descriptor cache; result order, types and historical views
//! must retain the existing point-get correctness contract.
use rystemdb_core::Value;
use rystemdb_storage::{BatchMutation, DocumentStore, StoreError};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("rystemdb-multi-get-{}-{nanos}-{id}",std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn root(&self) -> &Path { &self.0 }
}
impl Drop for Temp {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}
fn typed() -> Value {
    let mut root = BTreeMap::new();
    root.insert("nested".into(), Value::Array(vec![Value::Null,Value::Bytes(vec![0,255])]));
    root.insert("reference".into(), Value::Reference {
        collection:"users".into(),id:"user-42".into(),
    });
    Value::Object(root)
}

#[test]
fn tdd_grouped_get_mixed_legacy_batch_missing_duplicate_and_null_order() {
    let root=Temp::new();
    let mut db=DocumentStore::open(root.root()).unwrap();
    db.put("legacy", &Value::Null).unwrap();
    let old=db.snapshot();
    db.commit_batch(&old,&[
        BatchMutation::Put{id:"batched/a",value:&typed()},
        BatchMutation::Put{id:"batched/b",value:&Value::Integer(19)},
        BatchMutation::Put{id:"batched/c",value:&Value::Bool(false)},
    ]).unwrap();
    let expected=vec![
        Some(Value::Integer(19)),
        None,
        Some(Value::Null),
        Some(typed()),
        Some(Value::Integer(19)),
        Some(Value::Bool(false)),
        None,
    ];
    let ids=["batched/b","missing","legacy","batched/a","batched/b","batched/c","missing"];
    assert_eq!(db.get_many(&ids).unwrap(),expected);
    assert_eq!(db.snapshot().get_many(&ids).unwrap(),expected);
    assert_eq!(old.get_many(&["legacy","batched/a"]).unwrap(),
        vec![Some(Value::Null),None]);
}

#[test]
fn tdd_grouped_get_handles_rollover_historical_versions_and_restart() {
    let root=Temp::new();
    let mut db=DocumentStore::open_with_segment_limit(root.root(),120).unwrap();
    db.put("old", &Value::Integer(7)).unwrap();
    db.put("other", &Value::String("x".repeat(90))).unwrap();
    let history=db.snapshot();
    let current=db.snapshot();
    db.commit_batch(&current,&[
        BatchMutation::Put{id:"old",value:&Value::Integer(8)},
        BatchMutation::Put{id:"batch/null",value:&Value::Null},
    ]).unwrap();
    assert!(db.delete("other").unwrap());
    assert!(root.root().join("segment-00000002.rdb").is_file());
    let expected=vec![Some(Value::Integer(8)),None,Some(Value::Null),Some(Value::Integer(8))];
    assert_eq!(db.get_many(&["old","other","batch/null","old"]).unwrap(),expected);
    assert_eq!(history.get_many(&["old","other","batch/null"]).unwrap(),vec![
        Some(Value::Integer(7)), Some(Value::String("x".repeat(90))), None,
    ]);
    drop(db);
    let reopened=DocumentStore::open_with_segment_limit(root.root(),120).unwrap();
    assert_eq!(reopened.get_many(&["old","other","batch/null","old"]).unwrap(),expected);
    assert_eq!(history.get_many(&["old","other","batch/null"]).unwrap(),vec![
        Some(Value::Integer(7)), Some(Value::String("x".repeat(90))), None,
    ]);
}

#[test]
fn tdd_grouped_get_crc_corruption_is_error_not_partial_success() {
    let root=Temp::new();
    let mut db=DocumentStore::open(root.root()).unwrap();
    db.put("safe",&Value::Integer(1)).unwrap();
    let snapshot=db.snapshot();
    db.commit_batch(&snapshot,&[
        BatchMutation::Put{id:"bad/one",value:&Value::Integer(2)},
        BatchMutation::Put{id:"bad/two",value:&Value::Integer(3)},
    ]).unwrap();
    let path=root.root().join("segment-00000001.rdb");
    let mut file=OpenOptions::new().read(true).write(true).open(path).unwrap();
    file.seek(SeekFrom::End(-1)).unwrap();
    let mut b=[0u8;1];file.read_exact(&mut b).unwrap();
    file.seek(SeekFrom::End(-1)).unwrap();
    file.write_all(&[b[0]^0x40]).unwrap();file.sync_all().unwrap();
    drop(file);
    assert!(matches!(db.get_many(&["safe","bad/one","bad/two"]),Err(StoreError::Corrupt(_))));
    assert!(matches!(db.snapshot().get_many(&["bad/two"]),Err(StoreError::Corrupt(_))),
        "read must verify full shared BATCH CRC even when requesting one member");
    assert_eq!(snapshot.get_many(&["safe"]).unwrap(),vec![Some(Value::Integer(1))]);
}

#[test]
fn tdd_grouped_get_missing_segment_fails_closed_and_no_handle_is_cached() {
    let root=Temp::new();
    let mut db=DocumentStore::open(root.root()).unwrap();
    db.put("a",&Value::Integer(1)).unwrap();
    let old=db.snapshot();
    db.commit_batch(&old,&[
        BatchMutation::Put{id:"b",value:&Value::Bool(true)},
        BatchMutation::Put{id:"c",value:&Value::Bool(false)},
    ]).unwrap();
    let view=db.snapshot();
    assert_eq!(view.get_many(&["b","c"]).unwrap(),vec![
        Some(Value::Bool(true)),Some(Value::Bool(false)),
    ]);
    drop(db);
    fs::remove_file(root.root().join("segment-00000001.rdb")).unwrap();
    assert!(matches!(view.get_many(&["b","c"]),Err(StoreError::Io(_))),
        "unlinked segment must not be served from a cached open descriptor");
}

#[test]
fn tdd_grouped_get_empty_duplicate_and_input_bounds_without_partial_reads() {
    let root=Temp::new();
    let mut db=DocumentStore::open(root.root()).unwrap();
    db.put("x",&Value::Bool(true)).unwrap();
    assert_eq!(db.get_many(&[]).unwrap(),Vec::<Option<Value>>::new());
    assert_eq!(db.get_many(&["x","x","missing"]).unwrap(),vec![
        Some(Value::Bool(true)),Some(Value::Bool(true)),None,
    ]);
    assert!(matches!(db.get_many(&["x",""]),Err(StoreError::InvalidId)));
    assert!(matches!(db.get_many(&[&"z".repeat(1025)]),Err(StoreError::InvalidId)));
    let too_many=vec!["x";1025];
    assert!(matches!(db.get_many(&too_many),Err(StoreError::InvalidReadBatch)));
    let view=db.snapshot();
    assert!(matches!(view.get_many(&too_many),Err(StoreError::InvalidReadBatch)));
    assert!(matches!(view.get_many(&["x",""]),Err(StoreError::InvalidId)));
    assert_eq!(view.get_many(&["missing","x"]).unwrap(),vec![
        None,Some(Value::Bool(true)),
    ]);
}
