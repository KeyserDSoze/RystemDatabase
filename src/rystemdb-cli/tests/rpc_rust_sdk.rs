//! Real TCP interoperability between rystemdb-client Rust SDK and rpc-serve.
use rystemdb_client::{ClientError, LocalClient};
use rystemdb_core::Value;
use std::collections::BTreeMap;
use std::fs;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::thread;

struct Fixture { root: PathBuf, child: Child, addr: SocketAddr }
impl Fixture {
    fn start() -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("rystem-sdk-{}-{nanos}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let sock = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = sock.local_addr().unwrap();
        drop(sock);
        let mut child = Command::new(env!("CARGO_BIN_EXE_rystemdb"))
            .args(["rpc-serve", "--data"]).arg(&root)
            .args(["--listen", &addr.to_string()])
            .stdout(Stdio::null()).stderr(Stdio::null())
            .spawn().unwrap();
        for _ in 0..40 {
            if TcpStream::connect_timeout(&addr, Duration::from_millis(40)).is_ok() {
                return Self { root, child, addr };
            }
            if child.try_wait().unwrap().is_some() { break; }
            thread::sleep(Duration::from_millis(30));
        }
        let _ = child.kill(); let _ = child.wait();
        panic!("real rpc-serve failed to start");
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
