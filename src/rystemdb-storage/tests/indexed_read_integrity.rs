//! #97 RED: indexed reads must fail closed after truncation/corruption.
//! Reader optimization may skip fstat but must NEVER skip CRC or RDB decode.
use rystemdb_core::Value;
use rystemdb_storage::{BatchMutation, DocumentStore, StoreError};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Dir(PathBuf);
impl Dir {
    fn new() -> Self {
        let nanos=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let id=NEXT.fetch_add(1,Ordering::Relaxed);
        let path=std::env::temp_dir().join(format!(
            "rystemdb-indexed-read-{}-{nanos}-{id}",std::process::id(),
        ));
        fs::create_dir(&path).unwrap();Self(path)
    }
    fn path(&self)->&Path{&self.0}
}
impl Drop for Dir{fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}

fn typed()->Value {
    let mut m=BTreeMap::new();
    m.insert("nested".into(),Value::Array(vec![
        Value::Null,Value::Bytes(vec![0,255,42]),Value::Bool(false),
    ]));
    m.insert("ref".into(),Value::Reference{
        collection:"users".into(),id:"user/7".into(),
    });
    Value::Object(m)
}

#[test]
fn tdd_indexed_get_and_multi_get_retain_crc_typed_history_replay_and_null(){
    let root=Dir::new();
    let mut db=DocumentStore::open_with_segment_limit(root.path(),120).unwrap();
    db.put("k", &typed()).unwrap();
    let older=db.snapshot();
    db.put("k",&Value::Null).unwrap();
    db.put("other",&Value::Integer(44)).unwrap();
    let prior=db.snapshot();
    db.commit_batch(&prior,&[
        BatchMutation::Put{id:"k",value:&Value::Integer(9)},
        BatchMutation::Put{id:"batch/a",value:&typed()},
        BatchMutation::Put{id:"batch/b",value:&Value::Null},
    ]).unwrap();
    assert_eq!(db.get("k").unwrap(),Some(Value::Integer(9)));
    assert_eq!(db.get_many(&["k","batch/b","missing","batch/a","k"]).unwrap(),
        vec![Some(Value::Integer(9)),Some(Value::Null),None,
             Some(typed()),Some(Value::Integer(9))]);
    assert_eq!(prior.get("k").unwrap(),Some(Value::Null));
    assert_eq!(older.get("k").unwrap(),Some(typed()));
    assert_eq!(older.get_many(&["k","batch/a"]).unwrap(),
        vec![Some(typed()),None]);
    drop(db);
    let reopened=DocumentStore::open_with_segment_limit(root.path(),120).unwrap();
    assert_eq!(reopened.get_many(&["k","other","batch/a","batch/b"]).unwrap(),
        vec![Some(Value::Integer(9)),Some(Value::Integer(44)),
             Some(typed()),Some(Value::Null)]);
    assert_eq!(older.get("k").unwrap(),Some(typed()));
}

#[test]
fn tdd_truncated_standalone_header_is_corrupt_for_all_indexed_apis(){
    let root=Dir::new();
    let mut db=DocumentStore::open(root.path()).unwrap();
    db.put("short",&typed()).unwrap();
    let snapshot=db.snapshot();
    let segment=root.path().join("segment-00000001.rdb");
    OpenOptions::new().write(true).open(segment).unwrap().set_len(8).unwrap();
    assert!(matches!(db.get("short"),Err(StoreError::Corrupt(_))));
    assert!(matches!(snapshot.get("short"),Err(StoreError::Corrupt(_))));
    assert!(matches!(db.get_many(&["short","short"]),Err(StoreError::Corrupt(_))));
    assert!(matches!(snapshot.get_many(&["short"]),Err(StoreError::Corrupt(_))));
}

#[test]
fn tdd_truncated_batch_body_is_corrupt_not_missing_or_partial(){
    let root=Dir::new();
    let mut db=DocumentStore::open(root.path()).unwrap();
    let snapshot=db.snapshot();
    db.commit_batch(&snapshot,&[
        BatchMutation::Put{id:"a",value:&typed()},
        BatchMutation::Put{id:"b",value:&Value::Null},
    ]).unwrap();
    let segment=root.path().join("segment-00000001.rdb");
    let file=OpenOptions::new().write(true).open(segment).unwrap();
    let len=file.metadata().unwrap().len();assert!(len>32);
    file.set_len(len-7).unwrap();
    for id in ["a","b"] {
        assert!(matches!(db.get(id),Err(StoreError::Corrupt(_))));
        assert!(matches!(db.snapshot().get(id),Err(StoreError::Corrupt(_))));
    }
    assert!(matches!(db.get_many(&["a","b"]),Err(StoreError::Corrupt(_))));
}

#[test]
fn tdd_modified_batch_crc_fails_closed_even_for_one_requested_member(){
    let root=Dir::new();
    let mut db=DocumentStore::open(root.path()).unwrap();
    let snapshot=db.snapshot();
    db.commit_batch(&snapshot,&[
        BatchMutation::Put{id:"a",value:&Value::Null},
        BatchMutation::Put{id:"b",value:&typed()},
    ]).unwrap();
    let path=root.path().join("segment-00000001.rdb");
    let mut file=OpenOptions::new().read(true).write(true).open(path).unwrap();
    file.seek(SeekFrom::End(-1)).unwrap();
    let mut last=[0u8;1];file.read_exact(&mut last).unwrap();
    file.seek(SeekFrom::End(-1)).unwrap();
    file.write_all(&[last[0]^0x10]).unwrap();
    file.sync_all().unwrap();
    assert!(matches!(db.get("a"),Err(StoreError::Corrupt(_))));
    assert!(matches!(db.get_many(&["a","b"]),Err(StoreError::Corrupt(_))));
}

#[test]
fn tdd_missing_segment_after_close_is_error_not_cached_stale_result(){
    let root=Dir::new();
    let mut db=DocumentStore::open(root.path()).unwrap();
    db.put("a",&Value::Integer(1)).unwrap();
    let snapshot=db.snapshot();
    drop(db);
    fs::remove_file(root.path().join("segment-00000001.rdb")).unwrap();
    assert!(matches!(snapshot.get("a"),Err(StoreError::Io(_))));
    assert!(matches!(snapshot.get_many(&["a"]),Err(StoreError::Io(_))));
}
