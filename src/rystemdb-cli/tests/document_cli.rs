//! Contract tests invoke real CLI processes. They must fail RED before CLI implementation.
use rystemdb_core::Value;
use rystemdb_storage::DocumentStore;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rystemdb-cli-documents-{}-{nanos}-{serial}", std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path { &self.0 }
}
impl Drop for Temp {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

fn command(data: &Path, action: &str, collection: &str, id: &str) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rystemdb"));
    cmd.args(["document", action, "--data"])
        .arg(data)
        .args(["--collection", collection, "--id", id]);
    cmd
}

fn execute(mut cmd: Command, input: Option<&str>) -> Output {
    if let Some(input) = input {
        cmd.args(["--json", "-"]).stdin(Stdio::piped());
        let mut process = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        process.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
        process.wait_with_output().unwrap()
    } else {
        cmd.output().unwrap()
    }
}

fn ok(output: &Output) -> String {
    assert!(output.status.success(),
        "expected CLI success, got {:?}: {}",
        output.status, String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout.clone()).unwrap().trim().to_string()
}

#[test]
fn tdd_crud_roundtrips_across_independent_processes_and_persists_binary() {
    let tmp = Temp::new();
    let put = execute(command(tmp.path(), "put", "users", "u42"), Some(r#"{"name":"Ada","active":true}"#));
    ok(&put);

    let get = ok(&execute(command(tmp.path(), "get", "users", "u42"), None));
    assert_eq!(get, r#"{"active":true,"name":"Ada"}"#);

    ok(&execute(command(tmp.path(), "put", "users", "u42"),
        Some(r#"{"name":"Grace","age":34}"#)));
    let replaced = ok(&execute(command(tmp.path(), "get", "users", "u42"), None));
    assert_eq!(replaced, r#"{"age":34,"name":"Grace"}"#);

    let bytes = fs::read(tmp.path().join("segment-00000001.rdb")).unwrap();
    assert!(bytes.starts_with(b"RYSF"), "storage must use Rystem binary frames");
    assert!(!bytes.starts_with(b"{"));

    let deleted = ok(&execute(command(tmp.path(), "delete", "users", "u42"), None));
    assert!(deleted.contains(r#""deleted":true"#), "delete response: {deleted}");
    let missing = execute(command(tmp.path(), "get", "users", "u42"), None);
    assert!(!missing.status.success(), "deleted document was still readable");
}

#[test]
fn tdd_nested_unicode_null_absent_arrays_floats_and_signed_ints() {
    let tmp = Temp::new();
    let json = r#"{"z":null,"name":"Roma 🏛️","shipping":{"city":"Roma","zip":"00100"},"items":[{"sku":"A12","qty":2},false,null],"min":-9223372036854775808,"amount":12.5}"#;
    ok(&execute(command(tmp.path(), "put", "orders", "ord/1"), Some(json)));
    let get = ok(&execute(command(tmp.path(), "get", "orders", "ord/1"), None));
    assert_eq!(get,
        r#"{"amount":12.5,"items":[{"qty":2,"sku":"A12"},false,null],"min":-9223372036854775808,"name":"Roma 🏛️","shipping":{"city":"Roma","zip":"00100"},"z":null}"#);
    let store = DocumentStore::open(tmp.path()).unwrap();
    let Some(Value::Object(root)) = store.get("orders/ord/1").unwrap() else {
        panic!("CLI did not store native typed object");
    };
    assert!(root.contains_key("z"));
    assert!(!root.contains_key("unprovided"));
    assert_eq!(root.get("min"), Some(&Value::Integer(i64::MIN)));
}

#[test]
fn tdd_default_json_treats_special_looking_objects_as_plain_objects() {
    let tmp = Temp::new();
    let raw = r#"{"user":{"$rystem":{"type":"reference","version":1,"collection":"users","id":"u42"}}}"#;
    ok(&execute(command(tmp.path(), "put", "orders", "plain"), Some(raw)));
    let value = ok(&execute(command(tmp.path(), "get", "orders", "plain"), None));
    assert_eq!(value,
        r#"{"user":{"$rystem":{"collection":"users","id":"u42","type":"reference","version":1}}}"#);
    let store = DocumentStore::open(tmp.path()).unwrap();
    let Some(Value::Object(root)) = store.get("orders/plain").unwrap() else { panic!("expected map") };
    assert!(matches!(root.get("user"), Some(Value::Object(_))),
        "untyped input must never become a native relationship");
}

#[test]
fn tdd_typed_wire_json_roundtrips_reference_and_binary_across_embedded_api() {
    let tmp = Temp::new();
    let mut put = command(tmp.path(), "put", "orders", "typed");
    put.arg("--typed-json");
    let input = r#"{"customer":{"$rystem":{"version":1,"type":"reference","collection":"users","id":"u42"}},"payload":{"$rystem":{"version":1,"type":"bytes","hex":"00ff80"}}}"#;
    ok(&execute(put, Some(input)));
    let store = DocumentStore::open(tmp.path()).unwrap();
    let Some(Value::Object(root)) = store.get("orders/typed").unwrap() else { panic!("object") };
    assert_eq!(root.get("customer"), Some(&Value::Reference {
        collection: "users".to_string(), id: "u42".to_string()
    }));
    assert_eq!(root.get("payload"), Some(&Value::Bytes(vec![0, 255, 128])));
    drop(store);
    let output = ok(&execute(command(tmp.path(), "get", "orders", "typed"), None));
    assert_eq!(output,
        r#"{"customer":{"$rystem":{"collection":"users","id":"u42","type":"reference","version":1}},"payload":{"$rystem":{"hex":"00ff80","type":"bytes","version":1}}}"#);
}

#[test]
fn tdd_invalid_json_id_collection_and_typed_value_fail_without_writes() {
    let tmp = Temp::new();
    for (collection, id, input) in [
        ("users", "x", "{invalid"),
        ("users", "", r#"{"ok":1}"#),
        ("bad/slash", "id", r#"{"ok":1}"#),
        ("users", "x", r#"{"too_large":9223372036854775808}"#),
        ("users", "x", r#"{"bad_float":1e5000}"#),
    ] {
        let output = execute(command(tmp.path(), "put", collection, id), Some(input));
        assert!(!output.status.success(), "invalid data unexpectedly accepted");
    }
    let mut typed = command(tmp.path(), "put", "users", "bad-hex");
    typed.arg("--typed-json");
    let output = execute(typed, Some(r#"{"blob":{"$rystem":{"version":1,"type":"bytes","hex":"XX"}}}"#));
    assert!(!output.status.success(), "invalid typed bytes accepted");
    let segments = fs::read_dir(tmp.path()).unwrap().filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "rdb")).count();
    assert_eq!(segments, 0, "invalid input must fail before opening storage");
}

#[test]
fn tdd_cli_observes_embedded_writer_lock_and_releases_it_after_drop() {
    let tmp = Temp::new();
    let store = DocumentStore::open(tmp.path()).unwrap();
    let blocked = execute(command(tmp.path(), "put", "users", "u1"), Some(r#"{"a":1}"#));
    assert!(!blocked.status.success(), "competing CLI unexpectedly wrote while lock is held");
    drop(store);
    ok(&execute(command(tmp.path(), "put", "users", "u1"), Some(r#"{"a":1}"#)));
}

#[test]
fn tdd_usage_lists_document_operations_and_missing_fields_fail() {
    let help = ok(&Command::new(env!("CARGO_BIN_EXE_rystemdb")).arg("--help").output().unwrap());
    assert!(help.contains("document put"));
    assert!(help.contains("document get"));
    assert!(help.contains("document delete"));
    let tmp = Temp::new();
    let output = Command::new(env!("CARGO_BIN_EXE_rystemdb"))
        .args(["document", "put", "--data"])
        .arg(tmp.path())
        .output().unwrap();
    assert!(!output.status.success());
}
