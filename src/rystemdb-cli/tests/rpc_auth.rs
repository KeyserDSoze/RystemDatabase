//! RED/GREEN security contract: actual child processes, actual TCP and store.
use rystemdb_core::{decode as decode_doc, encode as encode_doc, Value};
use rystemdb_protocol::{crud, read_frame, write_frame, Frame, Operation};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const READ_TOKEN: [u8; 32] = [0x11; 32];
const WRITE_TOKEN: [u8; 32] = [0x22; 32];
static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("rystem-auth-{}-{nanos}-{serial}",std::process::id()));
        fs::create_dir(&dir).unwrap();
        Self(dir)
    }
    fn dir(&self) -> &Path { &self.0 }
}
impl Drop for Temp { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }

fn write_private(path: &Path, data: &str) {
    fs::write(path, data).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}
fn credentials(path: &Path) {
    let data = format!(
        "{} orders r\n{} orders w\n{} users r\n",
        "11".repeat(32), "22".repeat(32), "22".repeat(32)
    );
    write_private(path,&data);
}
struct Running { child: Child, address: SocketAddr }
impl Running {
    fn start(data: &Path, policy: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rystemdb"))
            .args(["rpc-serve","--listen","127.0.0.1:0","--data"])
            .arg(data).arg("--auth-file").arg(policy)
            .stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn().unwrap();
        let mut banner=String::new();
        let n=BufReader::new(child.stdout.take().unwrap()).read_line(&mut banner).unwrap();
        assert!(n>0 && banner.contains("LOOPBACK RPC listening on"),
            "authenticated server did not report actual bound port: {banner:?}");
        let address=banner.split_whitespace().last().unwrap().parse().unwrap();
        Self{child,address}
    }
    fn connect(&self)->TcpStream {
        let socket=TcpStream::connect(self.address).unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        socket.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
        socket
    }
}
impl Drop for Running { fn drop(&mut self) { let _=self.child.kill(); let _=self.child.wait(); } }

fn send(s:&mut TcpStream,op:Operation,id:u64,body:Vec<u8>)->Frame {
    write_frame(s,&Frame::request(op,id,body)).unwrap();
    let reply=read_frame(s).unwrap();
    assert_eq!(reply.operation,op);
    assert_eq!(reply.request_id,id);
    reply
}
fn hello(token: &[u8]) -> Vec<u8> {
    let mut p=vec![1,0];
    p.extend_from_slice(&(token.len() as u16).to_be_bytes());
    p.extend_from_slice(token);
    p
}
fn doc()->Value {
    Value::Object(BTreeMap::from([("private".into(),Value::String("not-public".into()))]))
}
fn put(collection:&str,id:&str)->Vec<u8> {
    let mut p=crud::encode_key(collection,id).unwrap();
    p.extend_from_slice(&encode_doc(&doc()).unwrap());
    p
}

#[test]
fn tdd_authentication_denies_missing_wrong_and_malformed_tokens_without_leaking_data() {
    let dir=Temp::new();
    let policy=dir.dir().join("acl.txt");
    credentials(&policy);
    let server=Running::start(&dir.dir().join("store"),&policy);
    for (n,payload) in [
        vec![1,0], hello(&[0x33;32]), hello(&[0x11;31]),
        vec![1,0,0,32,0x11], vec![2,0,0,32]
    ].into_iter().enumerate() {
        let mut conn=server.connect();
        let reply=send(&mut conn,Operation::Hello,(n+1) as u64,payload);
        assert_eq!(reply.status,6,"HELLO must return unauthorized without a valid token");
        assert!(reply.payload.is_empty(),"auth errors must not disclose information");
        assert!(read_frame(&mut conn).is_err(),"auth failure must close connection");
    }
    let mut valid=server.connect();
    assert_eq!(send(&mut valid,Operation::Hello,100,hello(&READ_TOKEN)).status,0);
    assert_eq!(send(&mut valid,Operation::Ping,101,vec![]).status,0);
}

#[test]
fn tdd_collection_acl_enforces_read_write_and_denies_unknown_collections() {
    let dir=Temp::new();
    let policy=dir.dir().join("acl.txt");
    credentials(&policy);
    let server=Running::start(&dir.dir().join("store"),&policy);
    let mut writer=server.connect();
    assert_eq!(send(&mut writer,Operation::Hello,1,hello(&WRITE_TOKEN)).status,0);
    assert_eq!(send(&mut writer,Operation::Put,2,put("orders","id1")).status,0);
    assert_eq!(send(&mut writer,Operation::Get,3,crud::encode_key("orders","id1").unwrap()).status,6);
    assert_eq!(send(&mut writer,Operation::Delete,4,crud::encode_key("users","u42").unwrap()).status,6);
    assert_eq!(send(&mut writer,Operation::Put,5,put("other","hidden")).status,6);
    let mut reader=server.connect();
    assert_eq!(send(&mut reader,Operation::Hello,6,hello(&READ_TOKEN)).status,0);
    let found=send(&mut reader,Operation::Get,7,crud::encode_key("orders","id1").unwrap());
    assert_eq!(found.status,0);
    assert_eq!(decode_doc(&found.payload).unwrap(),doc());
    assert_eq!(send(&mut reader,Operation::Put,8,put("orders","id1")).status,6);
    assert_eq!(send(&mut reader,Operation::Delete,9,crud::encode_key("orders","id1").unwrap()).status,6);
    assert_eq!(send(&mut reader,Operation::Get,10,crud::encode_key("other","hidden").unwrap()).status,6);
    assert_eq!(send(&mut writer,Operation::Delete,11,crud::encode_key("orders","id1").unwrap()).status,0);
    assert_eq!(send(&mut reader,Operation::Get,12,crud::encode_key("orders","id1").unwrap()).status,1);
}

#[test]
fn tdd_invalid_policy_must_fail_before_storage_directory_creation() {
    let dir=Temp::new();
    for (n,policy) in [
        "invalid\n".to_string(),
        format!("{} * rw\n","11".repeat(32)),
        format!("{} orders x\n","11".repeat(32)),
        format!("{} orders r\n{} orders w\n","11".repeat(32),"11".repeat(32)),
        format!("{} orders rw extra\n","11".repeat(32)),
        format!("{} orders rw\n","AA".repeat(32))
    ].iter().enumerate() {
        let acl=dir.dir().join(format!("invalid_acl_{n}.txt"));
        let data=dir.dir().join(format!("missing_store_{n}"));
        write_private(&acl,policy);
        let result=Command::new(env!("CARGO_BIN_EXE_rystemdb"))
            .args(["rpc-serve","--listen","127.0.0.1:0","--data"]).arg(&data)
            .arg("--auth-file").arg(&acl).output().unwrap();
        assert!(!result.status.success(),"invalid policy #{n} was accepted");
        assert!(!data.exists(),"invalid policy created storage #{n}");
        let stdout=String::from_utf8_lossy(&result.stdout);
        let stderr=String::from_utf8_lossy(&result.stderr);
        assert!(!stdout.contains("11".repeat(32).as_str()));
        assert!(!stderr.contains("11".repeat(32).as_str()));
    }
}

#[test]
fn tdd_auth_policy_requires_private_regular_file_on_unix() {
    let dir=Temp::new();
    let data=dir.dir().join("unopened");
    let acl=dir.dir().join("acl");
    credentials(&acl);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&acl,fs::Permissions::from_mode(0o644)).unwrap();
        let output=Command::new(env!("CARGO_BIN_EXE_rystemdb"))
            .args(["rpc-serve","--data"]).arg(&data)
            .arg("--auth-file").arg(&acl).output().unwrap();
        assert!(!output.status.success(),"world-readable credentials accepted");
        assert!(!data.exists());
        fs::set_permissions(&acl,fs::Permissions::from_mode(0o600)).unwrap();
        use std::os::unix::fs::symlink;
        let link=dir.dir().join("symlink_acl");
        symlink(&acl,&link).unwrap();
        let output=Command::new(env!("CARGO_BIN_EXE_rystemdb"))
            .args(["rpc-serve","--data"]).arg(&data)
            .arg("--auth-file").arg(&link).output().unwrap();
        assert!(!output.status.success(),"symlinked credentials accepted");
        assert!(!data.exists());
    }
    #[cfg(windows)]
    {
        // Windows ACL private-file security is a separate gate in parent #39.
        // Parsing and missing-file fail-closed tests still apply on Windows.
        let missing=dir.dir().join("missing");
        let output=Command::new(env!("CARGO_BIN_EXE_rystemdb"))
            .args(["rpc-serve","--data"]).arg(&data)
            .arg("--auth-file").arg(&missing).output().unwrap();
        assert!(!output.status.success());
        assert!(!data.exists());
    }
}

#[test]
fn tdd_nonloopback_listener_is_still_forbidden_even_with_valid_credentials() {
    let dir=Temp::new();
    let policy=dir.dir().join("acl");
    credentials(&policy);
    for addr in ["0.0.0.0:0","[::]:0","198.51.100.10:6543"] {
        let data=dir.dir().join(format!("forbidden_{}",addr.replace(':',"_")));
        let output=Command::new(env!("CARGO_BIN_EXE_rystemdb"))
            .args(["rpc-serve","--data"]).arg(&data)
            .args(["--listen",addr]).arg("--auth-file").arg(&policy)
            .output().unwrap();
        assert!(!output.status.success());
        assert!(!data.exists(),"external bind created database");
    }
}


// #72 RED: a valid RSWP HELLO header may declare a huge body while sending
// absolutely none. Before authenticating, a peer must not reserve a 16 MiB
// buffer or pin a scarce worker until the normal 3-second timeout. Require
// immediate connection teardown from a REAL Rust CLI TCP process.
#[test]
fn tdd_oversized_pre_auth_hello_header_closes_without_waiting_for_body() {
    let dir = Temp::new();
    let policy = dir.dir().join("acl.txt");
    credentials(&policy);
    let server = Running::start(&dir.dir().join("store"), &policy);

    for (pass, declared_len) in [37_u32, 16 * 1024 * 1024].into_iter().enumerate() {
        let mut socket = server.connect();
        socket.set_read_timeout(Some(Duration::from_millis(1200))).unwrap();
        // Only the 24-byte header is transmitted. Its magic, protocol
        // version, kind, operation, status and header length are all valid.
        let mut header = rystemdb_protocol::encode(
            &Frame::request(Operation::Hello, 700 + pass as u64, vec![])
        ).unwrap();
        header[20..24].copy_from_slice(&declared_len.to_be_bytes());
        assert_eq!(header.len(), rystemdb_protocol::HEADER_LEN);
        socket.write_all(&header).unwrap();
        let mut first = [0u8; 1];
        match socket.read(&mut first) {
            Ok(0) => {} // peer rejected the oversized unauthenticated header
            Err(err) if matches!(err.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted) => {}
            Ok(n) => panic!(
                "pre-auth oversized HELLO declaration {declared_len} produced {n} unexpected response byte(s)"
            ),
            Err(err) => panic!(
                "pre-auth HELLO declaration {declared_len} held a worker waiting for a missing body: {err:?}"
            ),
        }
    }

    // Refusing abusive headers must NOT break real authorized sessions.
    let mut valid = server.connect();
    assert_eq!(send(&mut valid, Operation::Hello, 801, hello(&READ_TOKEN)).status, 0);
    assert_eq!(send(&mut valid, Operation::Ping, 802, vec![]).status, 0);
}


/// #73 RED: per-syscall socket timeouts are insufficient for a peer that
/// trickles a header byte every <3s. The 3s budget must run from accept
/// through the entire unauthenticated handshake, not restart on each read.
#[test]
fn tdd_absolute_pre_auth_deadline_closes_trickling_hello_header() {
    let dir = Temp::new();
    let policy = dir.dir().join("acl.txt");
    credentials(&policy);
    let server = Running::start(&dir.dir().join("store"), &policy);
    let mut socket = server.connect();
    socket.set_read_timeout(Some(Duration::from_millis(800))).unwrap();

    let frame = rystemdb_protocol::encode(&Frame::request(
        Operation::Hello, 901, hello(&READ_TOKEN)
    )).unwrap();
    // Valid header/body, deliberately never completed. Two successful
    // partial writes occur 1.7s apart. Total time >3s; each individual
    // read would still be waiting under the old rolling socket timeout.
    socket.write_all(&frame[..1]).unwrap();
    std::thread::sleep(Duration::from_millis(1700));
    socket.write_all(&frame[1..2]).unwrap();
    std::thread::sleep(Duration::from_millis(1700));

    let mut first = [0u8; 1];
    match socket.read(&mut first) {
        Ok(0) => {}
        Err(err) if matches!(err.kind(),
            std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted) => {}
        Ok(n) => panic!("partial pre-auth HELLO unexpectedly returned {n} response bytes"),
        Err(err) => panic!(
            "drip-fed unauthenticated HELLO held a worker past 3s absolute deadline: {err:?}"
        ),
    }

    // An abusive connection must not prevent subsequent valid sessions.
    let mut valid = server.connect();
    assert_eq!(send(&mut valid, Operation::Hello, 902, hello(&READ_TOKEN)).status, 0);
    assert_eq!(send(&mut valid, Operation::Ping, 903, vec![]).status, 0);
}
