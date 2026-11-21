//! Real TCP interoperability between rystemdb-client Rust SDK and rpc-serve.
use rystemdb_client::{ClientError, LocalClient};
use rystemdb_core::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

struct Fixture { root: PathBuf, child: Child, addr: SocketAddr }
impl Fixture {
    fn start() -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("rystem-sdk-{}-{nanos}", std::process::id()));
        fs::create_dir(&root).unwrap();
        // Let the child own the ephemeral port selection to avoid TOCTOU races
        // with other test processes on heavily parallel macOS/Windows runners.
        let mut child = Command::new(env!("CARGO_BIN_EXE_rystemdb"))
            .args(["rpc-serve", "--data"]).arg(&root)
            .args(["--listen", "127.0.0.1:0"])
            .stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn().unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut banner = String::new();
        let n = BufReader::new(stdout).read_line(&mut banner).unwrap();
        assert!(n > 0 && banner.contains("LOOPBACK RPC listening on"),
            "server did not report bound port: {banner:?}");
        let addr: SocketAddr = banner.split_whitespace().last()
            .expect("bound address").parse().unwrap();
        assert!(addr.ip().is_loopback());
        Self { root, child, addr }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn tdd_rust_sdk_end_to_end_typed_crud_with_same_connection() {
    let fixture = Fixture::start();
    let mut client = LocalClient::connect(fixture.addr).unwrap();
    client.ping().unwrap();
    let mut nested = BTreeMap::new();
    nested.insert("ref".into(), Value::Reference {
        collection: "users".into(), id: "42".into()
    });
    nested.insert("bytes".into(), Value::Bytes(vec![0,255,128]));
    nested.insert("items".into(), Value::Array(vec![Value::Integer(-12),Value::Null]));
    let doc = Value::Object(nested);
    client.put("orders", "ord:1", &doc).unwrap();
    assert_eq!(client.get("orders", "ord:1").unwrap(), Some(doc));
    assert_eq!(client.get("orders", "missing").unwrap(), None);
    assert_eq!(client.delete("orders", "ord:1").unwrap(), true);
    assert_eq!(client.delete("orders", "ord:1").unwrap(), false);
    assert_eq!(client.get("orders", "ord:1").unwrap(), None);
}

#[test]
fn tdd_rust_sdk_refuses_nonloopback_until_tls_is_implemented() {
    let address: SocketAddr = "192.0.2.10:8790".parse().unwrap();
    assert!(matches!(LocalClient::connect(address), Err(ClientError::RemoteForbidden)));
}
