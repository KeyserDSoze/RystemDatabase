//! TDD contract: real OS subprocess and actual loopback TCP, never a mock.
use rystemdb_core::{decode as decode_document, encode as encode_document, Value};
use rystemdb_protocol::{read_frame, write_frame, Frame, Kind, Operation, ProtocolError};
use std::collections::BTreeMap;
use std::fs;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Tmp(PathBuf);
impl Tmp {
    fn new() -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("rystem-rpc-{}-{nanos}-{serial}", std::process::id()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn path(&self) -> &Path { &self.0 }
}
impl Drop for Tmp {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

struct Server { child: Child, address: SocketAddr }
impl Server {
    fn start(data: &Path) -> Self {
        // A temporary port is allocated by the OS and immediately released.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let mut child = Command::new(env!("CARGO_BIN_EXE_rystemdb"))
            .args(["rpc-serve", "--data"]).arg(data)
            .args(["--listen", &address.to_string()])
            .stdout(Stdio::null()).stderr(Stdio::null())
            .spawn().unwrap();
        for _ in 0..35 {
            if TcpStream::connect_timeout(&address, Duration::from_millis(50)).is_ok() {
                return Self { child, address };
            }
            if let Ok(Some(status)) = child.try_wait() {
                panic!("rpc-serve stopped prematurely: {status}");
            }
            thread::sleep(Duration::from_millis(35));
        }
        let _ = child.kill(); let _ = child.wait();
        panic!("rpc-serve never listened on loopback");
    }
    fn connect(&self) -> TcpStream {
        let stream = TcpStream::connect(self.address).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        stream.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
        stream
    }
}
impl Drop for Server {
    fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); }
}

fn send(stream: &mut TcpStream, op: Operation, id: u64, body: Vec<u8>) -> Frame {
    write_frame(stream, &Frame::request(op, id, body)).unwrap();
    let reply = read_frame(stream).expect("server response must be complete");
    assert_eq!(reply.kind, Kind::Response);
    assert_eq!(reply.operation, op);
    assert_eq!(reply.request_id, id);
    reply
}

fn handshake(s: &mut TcpStream) {
    let hello = send(s, Operation::Hello, 1, vec![1,0]);
    assert_eq!(hello.status, 0);
    assert_eq!(hello.payload, [1,0]);
}

fn key(collection: &str, id: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&(collection.len() as u16).to_be_bytes());
    buf.extend_from_slice(collection.as_bytes());
    buf.extend_from_slice(&(id.len() as u16).to_be_bytes());
    buf.extend_from_slice(id.as_bytes());
    buf
}

fn document() -> Value {
    let mut root = BTreeMap::new();
    root.insert("name".into(), Value::String("Roma 🏛".into()));
    root.insert("count".into(), Value::Integer(-42));
    root.insert("items".into(), Value::Array(vec![Value::Null, Value::Bool(true)]));
    root.insert("customer".into(), Value::Reference {
        collection: "users".into(), id: "u42".into()
    });
    root.insert("blob".into(), Value::Bytes(vec![0,255,128]));
    Value::Object(root)
}

#[test]
fn tdd_hello_ping_keepalive_correlates_binary_frames() {
    let db = Tmp::new();
    let server = Server::start(db.path());
    let mut s = server.connect();
    handshake(&mut s);
    for id in [2_u64,3,42,u64::MAX] {
        let response = send(&mut s, Operation::Ping, id, Vec::new());
        assert_eq!(response.status, 0);
        assert!(response.payload.is_empty());
    }
}

#[test]
fn tdd_roundtrip_nested_binary_documents_put_get_delete_and_restart() {
    let db = Tmp::new();
    {
        let server = Server::start(db.path());
        let mut conn = server.connect();
        handshake(&mut conn);
        let mut body = key("orders", "order_1");
        body.extend_from_slice(&encode_document(&document()).unwrap());
        assert_eq!(send(&mut conn, Operation::Put, 2, body).status, 0);
        let get = send(&mut conn, Operation::Get, 3, key("orders", "order_1"));
        assert_eq!(get.status, 0);
        assert_eq!(decode_document(&get.payload).unwrap(), document());
        // A second CLI process must not obtain an independent writer lock.
        let other = Command::new(env!("CARGO_BIN_EXE_rystemdb"))
            .args(["document", "delete", "--data"]).arg(db.path())
            .args(["--collection", "orders", "--id", "order_1"])
            .output().unwrap();
        assert!(!other.status.success());
    }
    let server = Server::start(db.path());
    let mut conn = server.connect();
    handshake(&mut conn);
    let get = send(&mut conn, Operation::Get, 4, key("orders", "order_1"));
    assert_eq!(decode_document(&get.payload).unwrap(), document());
    assert_eq!(send(&mut conn, Operation::Delete, 5, key("orders", "order_1")).status, 0);
    assert_eq!(send(&mut conn, Operation::Get, 6, key("orders", "order_1")).status, 1);
    assert_eq!(send(&mut conn, Operation::Delete, 7, key("orders", "order_1")).status, 1);
}

#[test]
fn tdd_handshake_required_and_malformed_body_rejected() {
    let db = Tmp::new();
    let server = Server::start(db.path());
    let mut unauth = server.connect();
    let reply = send(&mut unauth, Operation::Get, 9, key("users","u1"));
    assert_eq!(reply.status, 2, "get cannot run before HELLO");
    let mut conn = server.connect();
    handshake(&mut conn);
    for (id, op, body) in [
        (4, Operation::Get, vec![0, 5, b'u']),
        (5, Operation::Delete, key("users", "")),
        (6, Operation::Put, key("users", "u1")),
        (7, Operation::Get, {
            let mut v = key("users", "u1"); v.push(1); v
        }),
        (8, Operation::Put, {
            let mut v = key("users", "u1"); v.extend_from_slice(b"{not json}"); v
        }),
    ] {
        let resp = send(&mut conn, op, id, body);
        assert_eq!(resp.status, 2, "malformed request {id} must be rejected");
    }
    assert_eq!(send(&mut conn, Operation::Get, 10, key("users", "u1")).status, 1);
}

#[test]
fn tdd_wrong_hello_and_protocol_version_fail_closed() {
    let db = Tmp::new();
    let server = Server::start(db.path());
    let mut s = server.connect();
    let response = send(&mut s, Operation::Hello, 1, vec![2,0]);
    assert_ne!(response.status, 0, "wrong handshake version accepted");
    let mut fresh = server.connect();
    let mut bad = rystemdb_protocol::encode(&Frame::request(Operation::Hello, 2, vec![1,0])).unwrap();
    bad[4] = 2; // unsupported major version
    use std::io::Write;
    fresh.write_all(&bad).unwrap();
    assert!(matches!(read_frame(&mut fresh), Err(ProtocolError::Incomplete) | Err(ProtocolError::Io)),
        "invalid wire major must close/reject the connection");
    let mut valid = server.connect();
    handshake(&mut valid);
}

#[test]
fn tdd_multiple_clients_do_not_race_the_single_writer() {
    let db = Tmp::new();
    let server = Server::start(db.path());
    thread::scope(|scope| {
        let address = server.address;
        let mut handles = Vec::new();
        for n in 0..10_u64 {
            handles.push(scope.spawn(move || {
                let mut stream = TcpStream::connect(address).unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                handshake(&mut stream);
                let mut body = key("items", &format!("id_{n}"));
                body.extend_from_slice(&encode_document(&document()).unwrap());
                assert_eq!(send(&mut stream, Operation::Put, 10+n, body).status, 0);
                let get = send(&mut stream, Operation::Get, 40+n, key("items", &format!("id_{n}")));
                assert_eq!(get.status, 0);
                assert_eq!(decode_document(&get.payload).unwrap(), document());
            }));
        }
        for handle in handles { handle.join().unwrap(); }
    });
    let stored = rystemdb_storage::DocumentStore::open(db.path());
    assert!(stored.is_err(), "server must keep single writer ownership until exit");
    drop(server);
    // Restart replay is independently verified above.
}

#[test]
fn tdd_rpc_rejects_nonloopback_bind_before_creating_data() {
    let tmp = Tmp::new();
    for listen in ["0.0.0.0:0", "[::]:0", "192.0.2.10:12345"] {
        let path = tmp.path().join(format!("reject_{}", listen.replace(':', "_")));
        let output = Command::new(env!("CARGO_BIN_EXE_rystemdb"))
            .args(["rpc-serve", "--data"]).arg(&path)
            .args(["--listen",listen])
            .output().unwrap();
        assert!(!output.status.success(), "external bind unexpectedly permitted: {listen}");
        assert!(!path.exists(), "invalid bind must not create database files: {listen}");
    }
}
