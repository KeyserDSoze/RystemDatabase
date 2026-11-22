//! #58 RED/GREEN: an explicit TLS1.3 mTLS client-certificate gate.
//! This stage deliberately authorizes NO RSWP traffic under --mtls-policy.
use rystemdb_client::LocalClient;
use std::fs;
use std::io::{BufRead, BufReader};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

const READ_WRITE_TOKEN: [u8; 32] = [0x11; 32];

struct Fixture { root: PathBuf, public_certs: PathBuf }
impl Fixture {
    fn new() -> Self {
        let time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("rystem-mtls-gate-{}-{time}-{id}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let public_certs = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tls");
        fs::copy(public_certs.join("server-cert.pem"), root.join("server-cert.pem")).unwrap();
        fs::copy(public_certs.join("server-key.pem"), root.join("server-key.pem")).unwrap();
        Self { root, public_certs }
    }
    fn file(&self, path: &str) -> PathBuf { self.root.join(path) }
    fn write_private(&self, name: &str, contents: &str) {
        let p = self.file(name);
        fs::write(&p, contents).unwrap();
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(p, fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
    fn auth(&self) {
        self.write_private("auth", &format!("{} orders rw\n", "11".repeat(32)));
        self.write_private("mtls-policy",
            "rystem-mtls-principal-acl-v1\nspiffe://rystemdb.local/client/writer orders rw\n");
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(self.file("server-key.pem"),
                fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
    fn start(&self) -> Running {
        self.auth();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rystemdb"))
            .args(["rpc-serve", "--data"]).arg(self.file("store"))
            .args(["--listen", "127.0.0.1:0", "--auth-file"]).arg(self.file("auth"))
            .arg("--tls-cert").arg(self.file("server-cert.pem"))
            .arg("--tls-key").arg(self.file("server-key.pem"))
            .arg("--mtls-policy").arg(self.file("mtls-policy"))
            .arg("--mtls-ca").arg(self.public_certs.join("root-ca.pem"))
            .stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn().unwrap();
        let mut banner = String::new();
        let n = BufReader::new(child.stdout.take().unwrap()).read_line(&mut banner).unwrap();
        assert!(n > 0 && banner.contains("LOOPBACK RPC listening on"),
            "configured mTLS gate did not start a loopback listener");
        let address = banner.split_whitespace().last().unwrap().parse().unwrap();
        Running { child, address }
    }
}
impl Drop for Fixture { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.root); } }
struct Running { child: Child, address: SocketAddr }
impl Drop for Running { fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); } }

#[test]
fn tdd_mtls_gate_rejects_anonymous_tls_before_rswp_and_does_not_write_documents() {
    let f = Fixture::new();
    let server = f.start();
    let ca = f.public_certs.join("root-ca.pem");
    // This verified-server client does not present an X.509 client certificate.
    // It MUST fail its TLS handshake; it must not be allowed through on a
    // bearer token or silently downgrade to the existing TLS-only mode.
    for _ in 0..4 {
        assert!(LocalClient::connect_tls(
            server.address, "localhost", &ca, &READ_WRITE_TOKEN
        ).is_err(), "mTLS gate accepted a client without a certificate");
    }
    let mut plaintext = TcpStream::connect(server.address).unwrap();
    plaintext.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    use std::io::Write;
    plaintext.write_all(b"RSWP").unwrap();
    assert!(rystemdb_protocol::read_frame(&mut plaintext).is_err(),
        "mTLS gate accepted plaintext frames");
    assert!(!f.file("store").join("segment-00000001.rdb").exists()
        || fs::metadata(f.file("store").join("segment-00000001.rdb")).unwrap().len() == 0,
        "denied handshakes caused document mutations");
}

#[test]
fn tdd_mtls_trust_root_and_flags_fail_closed_before_storage_creation() {
    let f = Fixture::new();
    f.auth();
    let public_ca = f.public_certs.join("root-ca.pem");
    let cases = [
        // A trust anchor and principal policy must always travel together
        // with TLS certificate/key AND an explicit bearer-token policy.
        (vec!["--mtls-ca".to_owned(), public_ca.display().to_string()], "ca-alone"),
        (vec!["--mtls-policy".to_owned(), f.file("mtls-policy").display().to_string(),
             "--mtls-ca".to_owned(), f.file("missing").display().to_string()], "missing-ca"),
        (vec!["--mtls-policy".to_owned(), f.file("mtls-policy").display().to_string(),
             "--mtls-ca".to_owned(), f.file("server-key.pem").display().to_string()], "not-ca"),
    ];
    for (flags, name) in cases {
        let data = f.file(&format!("must-not-create-{name}"));
        let out = Command::new(env!("CARGO_BIN_EXE_rystemdb"))
            .args(["rpc-serve", "--data"]).arg(&data)
            .args(flags).output().unwrap();
        assert!(!out.status.success(), "unsafe mTLS startup accepted: {name}");
        assert!(!data.exists(), "unsafe mTLS startup created storage: {name}");
    }
}

#[test]
fn tdd_mtls_client_ca_is_parsed_before_store_and_rejects_unsafe_pem() {
    let f = Fixture::new();
    f.auth();
    let invalid_pem = f.file("invalid-ca.pem");
    fs::write(&invalid_pem, b"this is not a certificate").unwrap();
    let oversized = f.file("oversized-ca.pem");
    fs::write(&oversized, vec![b'x'; 65 * 1024]).unwrap();
    for (case, ca) in [
        ("missing", f.file("missing-root.pem")),
        ("non-certificate", invalid_pem),
        ("private-key-not-ca", f.file("server-key.pem")),
        ("oversized", oversized),
    ] {
        let data = f.file(&format!("reject-ca-{case}"));
        let out = Command::new(env!("CARGO_BIN_EXE_rystemdb"))
            .args(["rpc-serve", "--data"]).arg(&data)
            .args(["--listen", "127.0.0.1:0", "--auth-file"]).arg(f.file("auth"))
            .arg("--tls-cert").arg(f.file("server-cert.pem"))
            .arg("--tls-key").arg(f.file("server-key.pem"))
            .arg("--mtls-policy").arg(f.file("mtls-policy"))
            .arg("--mtls-ca").arg(ca)
            .output().unwrap();
        assert!(!out.status.success(), "malformed client trust root accepted: {case}");
        assert!(!data.exists(), "invalid CA caused storage creation: {case}");
        assert!(!String::from_utf8_lossy(&out.stderr).contains("BEGIN PRIVATE KEY"),
            "secret key contents leaked to stderr");
    }
    #[cfg(unix)] {
        use std::os::unix::fs::symlink;
        let data = f.file("reject-ca-symlink");
        let link = f.file("linked-ca.pem");
        symlink(f.public_certs.join("root-ca.pem"), &link).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rystemdb"))
            .args(["rpc-serve", "--data"]).arg(&data)
            .args(["--auth-file"]).arg(f.file("auth"))
            .arg("--tls-cert").arg(f.file("server-cert.pem"))
            .arg("--tls-key").arg(f.file("server-key.pem"))
            .arg("--mtls-policy").arg(f.file("mtls-policy"))
            .arg("--mtls-ca").arg(link)
            .output().unwrap();
        assert!(!out.status.success(), "symlinked trust root accepted");
        assert!(!data.exists());
    }
}
