//! RED #58: actual rustls mutual-TLS client identity and intersected bearer ACL.
//! These fixtures are PUBLIC TEST-ONLY keys. No real credential or remote listener.
use rystemdb_client::{ClientError, LocalClient};
use rystemdb_core::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const TOKEN: [u8; 32] = [0x11; 32];

struct Fixture { root: PathBuf, certs: PathBuf }
impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("rystem-mtls-{}-{nonce}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let certs = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mtls");
        let existing = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tls");
        fs::copy(existing.join("server-cert.pem"), root.join("server-cert.pem")).unwrap();
        fs::copy(existing.join("server-key.pem"), root.join("server-key.pem")).unwrap();
        fs::copy(existing.join("root-ca.pem"), root.join("server-root.pem")).unwrap();
        Self { root, certs }
    }
    fn path(&self, name: &str) -> PathBuf { self.root.join(name) }
    fn cert(&self, name: &str) -> PathBuf { self.certs.join(name) }
    fn private(&self, name: &str, data: &str) {
        let path = self.path(name);
        fs::write(&path, data).unwrap();
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
    fn server(&self) -> Running {
        self.private("token-acl", &format!("{} orders rw\n", "11".repeat(32)));
        self.private("principal-acl",
            "rystem-mtls-principal-acl-v1\nspiffe://rystemdb.local/client/reader orders r\nspiffe://rystemdb.local/client/writer orders rw\n");
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(self.path("server-key.pem"), fs::Permissions::from_mode(0o600)).unwrap();
        }
        let mut child = Command::new(env!("CARGO_BIN_EXE_rystemdb"))
            .args(["rpc-serve", "--data"]).arg(self.path("store"))
            .args(["--listen", "127.0.0.1:0", "--auth-file"]).arg(self.path("token-acl"))
            .arg("--tls-cert").arg(self.path("server-cert.pem"))
            .arg("--tls-key").arg(self.path("server-key.pem"))
            .arg("--mtls-policy").arg(self.path("principal-acl"))
            .arg("--mtls-ca").arg(self.cert("client-root.pem"))
            .stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn().unwrap();
        let mut banner = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut banner).unwrap();
        assert!(banner.contains("LOOPBACK RPC listening on"), "mTLS server failed startup");
        let addr = banner.split_whitespace().last().unwrap().parse().unwrap();
        Running { child, addr }
    }
    fn connect(&self, addr: SocketAddr, certificate: &str, token: &[u8; 32])
        -> Result<LocalClient, ClientError> {
        LocalClient::connect_mtls(addr, "localhost", &self.path("server-root.pem"),
            &self.cert(certificate), &self.cert("client-key.pem"), token)
    }
}
impl Drop for Fixture { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.root); } }
struct Running { child: Child, addr: SocketAddr }
impl Drop for Running { fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); } }

fn doc() -> Value {
    Value::Object(BTreeMap::from([
        ("ref".into(), Value::Reference { collection: "users".into(), id: "u42".into() }),
        ("n".into(), Value::Integer(i64::MIN)),
        ("bytes".into(), Value::Bytes(vec![0, 255, 128])),
    ]))
}

#[test]
fn tdd_mtls_verified_uri_san_and_intersected_token_rights() {
    let f = Fixture::new();
    let server = f.server();
    let mut writer = f.connect(server.addr, "writer-cert.pem", &TOKEN)
        .expect("verified client principal with matching token must connect");
    writer.ping().unwrap();
    writer.put("orders", "one", &doc()).unwrap();
    let mut reader = f.connect(server.addr, "reader-cert.pem", &TOKEN)
        .expect("read-only client principal must be accepted");
    assert_eq!(reader.get("orders", "one").unwrap(), Some(doc()));
    // Same RW token cannot escalate the certificate principal's R-only grant.
    assert!(matches!(reader.put("orders", "denied", &doc()), Err(ClientError::Rejected(6))));
    assert!(matches!(reader.delete("orders", "one"), Err(ClientError::Rejected(6))));
    assert_eq!(writer.get("orders", "denied").unwrap(), None);
    assert!(writer.delete("orders", "one").unwrap());
}

#[test]
fn tdd_mtls_rejects_unverified_or_ambiguous_client_id_before_any_operation() {
    let f = Fixture::new();
    let server = f.server();
    assert!(LocalClient::connect_tls(server.addr, "localhost",
        &f.path("server-root.pem"), &TOKEN).is_err(), "missing client cert accepted");
    for invalid in ["wrong-ca-cert.pem", "wrong-eku-cert.pem",
                    "expired-cert.pem", "future-cert.pem", "guest-cert.pem",
                    "no-san-cert.pem", "multi-san-cert.pem"] {
        assert!(f.connect(server.addr, invalid, &TOKEN).is_err(),
            "insecure/ambiguous mTLS client certificate accepted: {invalid}");
    }
    assert!(matches!(f.connect(server.addr, "writer-cert.pem", &[0; 32]),
        Err(ClientError::Rejected(6))), "wrong token must be rejected even with a valid cert");
    assert!(f.connect("192.0.2.1:8790".parse().unwrap(), "writer-cert.pem", &TOKEN).is_err(),
        "mTLS must never allow remote addresses");
}

#[test]
fn tdd_mtls_flag_dependencies_fail_before_storage_creation() {
    let f = Fixture::new();
    f.private("token-acl", &format!("{} orders rw\n", "11".repeat(32)));
    f.private("principal-acl",
        "rystem-mtls-principal-acl-v1\nspiffe://rystemdb.local/client/writer orders rw\n");
    let data = f.path("must-not-create");
    let options = [
        vec!["--mtls-policy", "principal-acl"],
        vec!["--mtls-ca", "client-root.pem"],
    ];
    for (i, flags) in options.iter().enumerate() {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_rystemdb"));
        cmd.arg("rpc-serve").arg("--data").arg(&data);
        if i == 0 { cmd.arg("--mtls-policy").arg(f.path(flags[1])); }
        else { cmd.arg("--mtls-ca").arg(f.cert(flags[1])); }
        let output = cmd.output().unwrap();
        assert!(!output.status.success(), "incomplete mTLS config started server");
        assert!(!data.exists(), "bad mTLS config created store");
    }
}
