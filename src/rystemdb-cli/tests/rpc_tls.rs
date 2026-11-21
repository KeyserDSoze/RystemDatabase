//! TDD: real TLS 1.3 transport, actual server process and verified certificate.
use rystemdb_client::{ClientError, LocalClient};
use rystemdb_core::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static NEXT: AtomicUsize = AtomicUsize::new(0);
const READ_WRITE_TOKEN: [u8; 32] = [0x11; 32];

struct Fixture {
    root: PathBuf,
    policy: PathBuf,
    cert: PathBuf,
    key: PathBuf,
    ca: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let n = NEXT.fetch_add(1,Ordering::Relaxed);
        let root=std::env::temp_dir().join(format!("rystem-tls-{}-{t}-{n}",std::process::id()));
        fs::create_dir(&root).unwrap();
        let original = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tls");
        let policy=root.join("acl.txt");
        let cert=root.join("cert.pem");
        let key=root.join("key.pem");
        let ca=root.join("ca.pem");
        fs::write(&policy,format!("{} orders rw\n","11".repeat(32))).unwrap();
        fs::copy(original.join("server-cert.pem"),&cert).unwrap();
        fs::copy(original.join("server-key.pem"),&key).unwrap();
        fs::copy(original.join("root-ca.pem"),&ca).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [&key,&policy] {
                fs::set_permissions(path,fs::Permissions::from_mode(0o600)).unwrap();
            }
        }
        Self{root,policy,cert,key,ca}
    }
    fn start(&self) -> Running {
        let mut child=Command::new(env!("CARGO_BIN_EXE_rystemdb"))
            .args(["rpc-serve","--data"]).arg(self.root.join("store"))
            .args(["--listen","127.0.0.1:0","--auth-file"]).arg(&self.policy)
            .arg("--tls-cert").arg(&self.cert)
            .arg("--tls-key").arg(&self.key)
            .stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn().unwrap();
        let mut line=String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
        assert!(line.contains("LOOPBACK RPC listening on"),"TLS server failed startup: {line:?}");
        let address=line.split_whitespace().last().unwrap().parse().unwrap();
        Running {child,address}
    }
}
impl Drop for Fixture {
    fn drop(&mut self){ let _=fs::remove_dir_all(&self.root); }
}
struct Running {child:Child,address:SocketAddr}
impl Drop for Running {
    fn drop(&mut self){let _=self.child.kill();let _=self.child.wait();}
}
fn put_doc()->Value {
    Value::Object(BTreeMap::from([
        ("reference".into(),Value::Reference { collection:"users".into(),id:"u42".into() }),
        ("bytes".into(),Value::Bytes(vec![0,255,128])),
        ("items".into(),Value::Array(vec![Value::Integer(i64::MIN),Value::Null]))
    ]))
}

#[test]
fn tdd_verified_tls13_local_rust_client_binary_crud_and_token_acl() {
    let f=Fixture::new();
    let server=f.start();
    let mut client=LocalClient::connect_tls(server.address,"localhost",&f.ca,&READ_WRITE_TOKEN)
        .expect("verified TLS 1.3 local client must connect");
    client.ping().unwrap();
    client.put("orders","🦊",&put_doc()).unwrap();
    assert_eq!(client.get("orders","🦊").unwrap(),Some(put_doc()));
    assert_eq!(client.get("orders","absent").unwrap(),None);
    assert!(client.delete("orders","🦊").unwrap());
    assert_eq!(client.get("orders","🦊").unwrap(),None);
}

#[test]
fn tdd_tls_rejects_wrong_trust_anchor_and_hostname_without_fallback() {
    let f=Fixture::new();
    let server=f.start();
    // Named endpoint verification must not allow a client to claim an
    // arbitrary name despite the root CA being trusted.
    assert!(LocalClient::connect_tls(server.address,"not-localhost.test",&f.ca,&READ_WRITE_TOKEN).is_err());
    // A valid name with the wrong root must not verify.
    assert!(LocalClient::connect_tls(server.address,"localhost",&f.cert,&READ_WRITE_TOKEN).is_err());
    // Plaintext RSWP sent to TLS-only port cannot be reinterpreted as RSWP.
    let mut socket=TcpStream::connect(server.address).unwrap();
    socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let hello=rystemdb_protocol::encode(&rystemdb_protocol::Frame::request(
        rystemdb_protocol::Operation::Hello,1,vec![1,0,0,32]
            .into_iter().chain(READ_WRITE_TOKEN).collect())).unwrap();
    socket.write_all(&hello).unwrap();
    assert!(rystemdb_protocol::read_frame(&mut socket).is_err(),
        "TLS endpoint accepted a plaintext RSWP frame");
    // Bad raw bearer secret must be rejected within TLS too.
    assert!(matches!(
        LocalClient::connect_tls(server.address,"localhost",&f.ca,&[0;32]),
        Err(ClientError::Rejected(6))
    ));
}

#[test]
fn tdd_denied_tls_hello_stays_a_complete_status_six_before_teardown() {
    let f = Fixture::new();
    let server = f.start();
    // Repeated fresh TLS sessions catch response-vs-FIN teardown races.
    // Never reinterpret EOF or a transport timeout as an RSWP denial.
    for attempt in 0..16 {
        let rejected = LocalClient::connect_tls(
            server.address, "localhost", &f.ca, &[0; 32]
        );
        assert!(matches!(rejected, Err(ClientError::Rejected(6))),
            "denial at attempt {attempt} must be RSWP status 6, never EOF");
    }
    let mut allowed = LocalClient::connect_tls(
        server.address, "localhost", &f.ca, &READ_WRITE_TOKEN
    ).expect("denied connections must not starve authorized TLS clients");
    allowed.ping().unwrap();
}

#[test]
fn tdd_tls_flags_and_bad_key_fail_before_storage_is_created() {
    let f=Fixture::new();
    let cases: Vec<Vec<String>>=vec![
        vec!["--tls-cert".into(),f.cert.display().to_string()],
        vec!["--tls-key".into(),f.key.display().to_string()],
        vec!["--tls-cert".into(),f.cert.display().to_string(),"--tls-key".into(),f.key.display().to_string()],
        vec!["--auth-file".into(),f.policy.display().to_string(),
             "--tls-cert".into(),f.cert.display().to_string(),"--tls-key".into(),f.root.join("missing").display().to_string()],
        vec!["--auth-file".into(),f.policy.display().to_string(),
             "--tls-cert".into(),f.ca.display().to_string(),"--tls-key".into(),f.key.display().to_string()]
    ];
    for (n,flags) in cases.iter().enumerate() {
        let data=f.root.join(format!("must-not-create-{n}"));
        let output=Command::new(env!("CARGO_BIN_EXE_rystemdb"))
            .args(["rpc-serve","--data"]).arg(&data).args(flags)
            .output().unwrap();
        assert!(!output.status.success(),"insecure/broken TLS configuration accepted: #{n}");
        assert!(!data.exists(),"bad TLS config created database before reject: #{n}");
        assert!(!String::from_utf8_lossy(&output.stdout).contains("BEGIN EC PRIVATE KEY"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("BEGIN EC PRIVATE KEY"));
    }
}

#[test]
fn tdd_tls_configuration_never_enables_nonloopback_or_wildcard_binding() {
    let f=Fixture::new();
    for (n,listen) in ["0.0.0.0:0","[::]:0","198.51.100.10:8989"].iter().enumerate() {
        let data=f.root.join(format!("external-must-not-create-{n}"));
        let out=Command::new(env!("CARGO_BIN_EXE_rystemdb"))
            .args(["rpc-serve","--data"]).arg(&data)
            .args(["--listen",listen,"--auth-file"]).arg(&f.policy)
            .arg("--tls-cert").arg(&f.cert).arg("--tls-key").arg(&f.key)
            .output().unwrap();
        assert!(!out.status.success());
        assert!(!data.exists(),"TLS mode unexpectedly enabled external listener");
    }
}

#[test]
fn tdd_tls_key_file_requires_private_unlinked_permissions_on_unix() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{symlink,PermissionsExt};
        let f=Fixture::new();
        let data=f.root.join("must-not-open-with-unsafe-key");
        fs::set_permissions(&f.key,fs::Permissions::from_mode(0o644)).unwrap();
        let r=Command::new(env!("CARGO_BIN_EXE_rystemdb"))
            .args(["rpc-serve","--data"]).arg(&data)
            .args(["--auth-file"]).arg(&f.policy)
            .arg("--tls-cert").arg(&f.cert).arg("--tls-key").arg(&f.key)
            .output().unwrap();
        assert!(!r.status.success(),"loose Unix key permissions allowed TLS startup");
        assert!(!data.exists(),"bad TLS key created storage");
        fs::set_permissions(&f.key,fs::Permissions::from_mode(0o600)).unwrap();
        let link=f.root.join("link-key.pem");
        symlink(&f.key,&link).unwrap();
        let r=Command::new(env!("CARGO_BIN_EXE_rystemdb"))
            .args(["rpc-serve","--data"]).arg(&data)
            .args(["--auth-file"]).arg(&f.policy)
            .arg("--tls-cert").arg(&f.cert).arg("--tls-key").arg(&link)
            .output().unwrap();
        assert!(!r.status.success(),"symlinked key was accepted");
        assert!(!data.exists());
    }
}
