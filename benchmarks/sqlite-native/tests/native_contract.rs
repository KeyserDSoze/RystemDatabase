//! #94 RED: a NATIVE Rust SQLite WAL/FULL driver, not a Python timing loop.
//! Fixture must match actual Rust-generated typed RDB v1 bytes and seed trace.
use rystemdb_bench_sqlite::{load_fixture, NativeSqlite, run_fixture, validate_report};
use rystemdb_core::{decode, encode, Value};
use rystemdb_storage::performance::{seeded_indexes, synthetic_document};
use serde_json::{json, Value as JsonValue};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Dir(PathBuf);
impl Dir {
    fn new() -> Self {
        let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let p = std::env::temp_dir().join(format!(
            "rystem-native-sqlite-test-{}-{t}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn path(&self) -> &Path { &self.0 }
}
impl Drop for Dir { fn drop(&mut self){let _=fs::remove_dir_all(&self.0);} }

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|n|format!("{n:02x}")).collect()
}
fn fixture_lines(seed:u64,records:usize,operations:usize,payload:usize)->Vec<JsonValue>{
    let mut rows=vec![json!({"schema":1,"kind":"fixture_run","seed":seed,
        "records":records,"operations":operations,"payload_bytes":payload,
        "engine":"rystemdb-storage","value_encoding":"rdb_v1_frame_hex"})];
    for i in 0..records {
        rows.push(json!({"schema":1,"kind":"initial",
            "id":format!("bench/{i:08}"),
            "value_hex":hex(&encode(&synthetic_document(seed,i,payload)).unwrap())}));
    }
    let order=seeded_indexes(seed^0xcafe_d00d,operations,records);
    for (i,index) in order.iter().enumerate(){
        rows.push(json!({"schema":1,"kind":"read","sequence":i,
            "id":format!("bench/{index:08}")}));
    }
    for (i,index) in order.iter().enumerate(){
        rows.push(json!({"schema":1,"kind":"update","sequence":i,
            "id":format!("bench/{index:08}"),
            "value_hex":hex(&encode(&synthetic_document(seed^0xdeb6_5d53,records+i,payload)).unwrap())}));
    }
    // Do not name this variable `null`: json! parses that literal as
    // JSON null rather than interpolating the Rust string expression.
    let null_rdb_hex=hex(&encode(&Value::Null).unwrap());
    for i in 0..operations {
        rows.push(json!({"schema":1,"kind":"batch","sequence":i,
            "id_a":format!("batch/{i:08}/a"),
            "value_a_hex":hex(&encode(&Value::Integer(i as i64)).unwrap()),
            "id_b":format!("batch/{i:08}/b"),"value_b_hex":null_rdb_hex}));
    }
    rows
}
fn save(root:&Path, rows:&[JsonValue])->PathBuf {
    let f=root.join("fixture.ndjson");
    fs::write(&f,rows.iter().map(JsonValue::to_string).collect::<Vec<_>>().join("\n")+"\n").unwrap();
    f
}
#[test]
fn tdd_load_exact_typed_nested_rdb_and_trace() {
    let root=Dir::new();let path=save(root.path(),&fixture_lines(42,3,5,128));
    let f=load_fixture(&path).unwrap();
    assert_eq!(f.records,3);assert_eq!(f.operations,5);assert_eq!(f.seed,42);
    assert_eq!(f.initial.len(),3);assert_eq!(f.updates.len(),5);assert_eq!(f.batches.len(),5);
    for (i,record) in f.initial.iter().enumerate() {
        assert_eq!(record.id,format!("bench/{i:08}"));
        assert_eq!(decode(&record.bytes).unwrap(),record.typed);
        assert_eq!(record.typed,synthetic_document(42,i,128));
    }
    for (i,index) in seeded_indexes(42^0xcafe_d00d,5,3).iter().enumerate() {
        assert_eq!(f.read_ids[i],format!("bench/{index:08}"));
        assert_eq!(f.updates[i].id,f.read_ids[i]);
    }
}
#[test]
fn tdd_reject_tampered_fixture_even_if_rdb_encoding_valid() {
    let root=Dir::new();
    let mut rows=fixture_lines(42,3,5,64);
    rows[1]["value_hex"]=json!(hex(&encode(&Value::Null).unwrap()));
    assert!(load_fixture(&save(root.path(),&rows)).is_err(),
        "valid RDB bytes must still match the actual seed-generated value");
    let mut rows=fixture_lines(42,3,5,64);
    rows[4]["id"]=json!("bench/00000000"); // first read may not be index 0
    assert!(load_fixture(&save(root.path(),&rows)).is_err(),
        "must match exact seeded ordering");
    let mut rows=fixture_lines(42,3,5,64);
    rows[0]["unsupported"]="surprise".into();
    assert!(load_fixture(&save(root.path(),&rows)).is_err());
}
#[test]
fn tdd_native_sqlite_wal_full_typed_get_and_atomic_failure_rollback() {
    let root=Dir::new();let mut db=NativeSqlite::open(&root.path().join("native.db")).unwrap();
    assert_eq!(db.journal_mode(),"wal");
    assert_eq!(db.synchronous(),2);
    let value=synthetic_document(10,1,32);
    let bytes=encode(&value).unwrap();
    assert_eq!(db.put("docs/a",&bytes).unwrap(),1);
    assert_eq!(db.get_typed("docs/a").unwrap(),Some((bytes.clone(),value)));
    assert_eq!(db.get_typed("docs/absent").unwrap(),None);
    assert_eq!(db.update("docs/a",&encode(&Value::Null).unwrap()).unwrap(),1);
    assert_eq!(db.get_typed("docs/a").unwrap().unwrap().1,Value::Null);
    assert!(db.put_pair("docs/b",&bytes,"docs/a",&bytes).is_err(),
       "second duplicate must ROLLBACK first insert, not leak partial commit");
    assert_eq!(db.get_typed("docs/b").unwrap(),None);
    assert_eq!(db.delete("docs/a").unwrap(),1);
    assert_eq!(db.get_typed("docs/a").unwrap(),None);
    assert_eq!(db.put_pair("docs/c",&bytes,"docs/d",&encode(&Value::Null).unwrap()).unwrap(),2);
    drop(db);
    let db=NativeSqlite::open(&root.path().join("native.db")).unwrap();
    assert_eq!(db.get_typed("docs/c").unwrap().unwrap().0,bytes);
    assert_eq!(db.get_typed("docs/d").unwrap().unwrap().1,Value::Null);
}
#[test]
fn tdd_corrupt_typed_value_fails_closed_and_no_raw_blob_success() {
    let root=Dir::new();let file=root.path().join("native.db");
    let mut db=NativeSqlite::open(&file).unwrap();
    db.put("bad",&encode(&Value::Integer(9)).unwrap()).unwrap();
    db.overwrite_raw_for_corruption_test("bad",&[0x12,0x34]).unwrap();
    assert!(db.get_typed("bad").is_err(),"malformed RDB must be rejected");
}
#[test]
fn tdd_native_runner_emits_real_typed_metric_count_and_parity_flags() {
    let root=Dir::new();
    let fixture=load_fixture(&save(root.path(),&fixture_lines(42,3,5,64))).unwrap();
    let report=run_fixture(&fixture,&root.path().join("bench.db")).unwrap();
    validate_report(&report,&fixture).unwrap();
    let run=&report[0];
    assert_eq!(run["engine"],"sqlite");
    assert_eq!(run["driver"],"native_rust_rusqlite");
    assert_eq!(run["sqlite_journal_mode"],"wal");
    assert_eq!(run["sqlite_synchronous"],"FULL");
    assert_eq!(run["payload_semantics"],"rdb_v1_typed_decode_no_rysf_outer_crc");
    for row in report.iter().filter(|x|x["kind"]=="metric"){
        assert!(row["p50_ns"].as_u64().unwrap()<=row["p95_ns"].as_u64().unwrap());
        assert!(row["p95_ns"].as_u64().unwrap()<=row["p99_ns"].as_u64().unwrap());
        assert!(row["p99_ns"].as_u64().unwrap()<=row["max_ns"].as_u64().unwrap());
    }
    let metrics=report.iter().filter(|x|x["kind"]=="metric").collect::<Vec<_>>();
    assert_eq!(metrics.len(),7);
    let get=metrics.iter().find(|x|x["name"]=="get_hit_typed_rdb_warm").unwrap();
    assert_eq!(get["count"],5);
    assert!(report.last().unwrap()["disk_bytes"].as_u64().unwrap()>0);
}
