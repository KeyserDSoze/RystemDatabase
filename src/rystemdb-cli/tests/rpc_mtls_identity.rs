//! RED #61: real-process mTLS principal binding to verified URI SAN.
//! All identity keys are created afresh by OpenSSL in test-only tmp dirs.
use rystemdb_client::{ClientError, LocalClient};
#[path = "support/signed_duplicate_san.rs"]
mod signed_duplicate_san;
use rystemdb_core::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT: AtomicUsize = AtomicUsize::new(0);
const RW: [u8; 32] = [0x11; 32];
const OTHER_COLLECTION: [u8; 32] = [0x22; 32];

fn openssl(args: &[&str]) {
    let out = Command::new("openssl").args(args)
        .stdout(Stdio::null()).stderr(Stdio::null()).output()
        .expect("OpenSSL is required for ephemeral client certificate test fixtures");
    assert!(out.status.success(), "ephemeral OpenSSL command failed");
}
fn text_path(p: &Path) -> &str { p.to_str().expect("temporary test path is UTF-8") }
fn private(path: &Path) {
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    #[cfg(not(unix))]
    let _ = path;
}

struct Fixture { root: PathBuf }
impl Fixture {
    fn new() -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("rystem-mtls-identity-{}-{nanos}-{n}",std::process::id()));
        fs::create_dir(&root).unwrap();
        let server_fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tls");
        for filename in ["server-cert.pem", "server-key.pem", "root-ca.pem"] {
            fs::copy(server_fixture.join(filename), root.join(filename)).unwrap();
        }
        private(&root.join("server-key.pem"));
        let f = Self {root};
        fs::write(f.path("token.acl"), format!("{} orders rw\n{} inventory rw\n",
            "11".repeat(32), "22".repeat(32))).unwrap();
        fs::write(f.path("identity.acl"),
            "rystem-mtls-principal-acl-v1\nspiffe://rystemdb.local/client/writer orders rw\nspiffe://rystemdb.local/client/reader orders r\n").unwrap();
        private(&f.path("token.acl"));
        private(&f.path("identity.acl"));
        f.create_ca("client");
        f.create_ca("untrusted");
        f.create_identity("writer", "client",
            Some("URI:spiffe://rystemdb.local/client/writer"), "clientAuth");
        f.create_identity("reader", "client",
            Some("URI:spiffe://rystemdb.local/client/reader"), "clientAuth");
        f.create_identity("stranger", "client",
            Some("URI:spiffe://rystemdb.local/client/stranger"), "clientAuth");
        f.create_identity("badcase", "client",
            Some("URI:spiffe://rystemdb.local/client/Reader"), "clientAuth");
        f.create_identity("nosan", "client", None, "clientAuth");
        f.create_identity("multisan", "client",
            Some("URI:spiffe://rystemdb.local/client/writer,URI:spiffe://rystemdb.local/client/reader"),
            "clientAuth");
        f.create_identity("wrongeku", "client",
            Some("URI:spiffe://rystemdb.local/client/writer"), "serverAuth");
        f.create_identity("untrusted-writer", "untrusted",
            Some("URI:spiffe://rystemdb.local/client/writer"), "clientAuth");
        f.create_identity("query", "client",
            Some("URI:spiffe://rystemdb.local/client/writer?role=admin"), "clientAuth");
        f.create_identity("wrongprefix", "client",
            Some("URI:spiffe://other.local/client/writer"), "clientAuth");
        f.create_identity("percent", "client",
            Some("URI:spiffe://rystemdb.local/client/writer%2Fadmin"), "clientAuth");
        f.create_dated_identity("expired", "20100101000000Z", "20110101000000Z");
        f.create_dated_identity("future", "20350101000000Z", "20360101000000Z");
        const URI: &[u8] = b"spiffe://rystemdb.local/client/writer";
        f.create_raw_san_identity("raw-invalid-utf8", &Self::raw_names(&[
            Self::uri(URI), Self::uri(&[0xff])
        ]));
        f.create_raw_san_identity("raw-double", &Self::raw_names(&[
            Self::uri(URI), Self::uri(URI)
        ]));
        f.create_raw_san_identity("raw-uri-and-dns", &Self::raw_names(&[
            Self::uri(URI), {
                let mut dns=vec![0x82, 9];dns.extend_from_slice(b"localhost");dns
            }
        ]));
        f.create_raw_san_identity("raw-missing-uri", &Self::raw_names(&[
            { let mut dns=vec![0x82, 9]; dns.extend_from_slice(b"localhost");dns }
        ]));
        f.create_raw_san_identity("raw-truncated", &[0x30,0x03,0x86,0x04,0xff]);
        f.create_raw_san_identity("raw-constructed", &Self::raw_names(&[
            vec![0xa6,1,0x01]
        ]));
        f
    }
    fn path(&self, file: &str) -> PathBuf { self.root.join(file) }
    fn create_signed_duplicate_san_identity(&self, name: &str) -> signed_duplicate_san::Evidence {
        // Keep the same matching client identity key: only the duplicate
        // SAN extension, not a key mismatch, may cause TLS rejection.
        fs::copy(self.path("writer.key"), self.path(&format!("{name}.key"))).unwrap();
        private(&self.path(&format!("{name}.key")));
        signed_duplicate_san::create(
            &self.root, &self.path("writer.pem"), &self.path("client-ca.key"),
            &self.path(&format!("{name}.pem")),
        )
    }
    fn create_ca(&self, name: &str) {
        let cfg=self.path(&format!("{name}-ca.cnf"));
        fs::write(&cfg, format!(
            "[req]\ndistinguished_name=dn\nprompt=no\nx509_extensions=ca\n[dn]\nCN=TEST ONLY {name}\n[ca]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\n"
        )).unwrap();
        let key=self.path(&format!("{name}-ca.key"));
        let pem=self.path(&format!("{name}-ca.pem"));
        openssl(&["req","-x509","-newkey","rsa:2048","-nodes","-days","4",
            "-keyout",text_path(&key),"-out",text_path(&pem),"-config",text_path(&cfg)]);
        private(&key);
    }
    fn create_identity(&self, name: &str, ca: &str, san: Option<&str>, eku: &str) {
        let key=self.path(&format!("{name}.key"));
        let csr=self.path(&format!("{name}.csr"));
        let cert=self.path(&format!("{name}.pem"));
        let config=self.path(&format!("{name}.cnf"));
        let mut extensions=format!(
            "[identity]\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage={eku}\n");
        if let Some(san)=san { extensions.push_str(&format!("subjectAltName={san}\n")); }
        fs::write(&config, extensions).unwrap();
        openssl(&["req","-new","-newkey","rsa:2048","-nodes",
            "-keyout",text_path(&key),"-out",text_path(&csr),
            "-subj","/CN=DO-NOT-TRUST-CN"]);
        let ca_cert=self.path(&format!("{ca}-ca.pem"));
        let ca_key=self.path(&format!("{ca}-ca.key"));
        openssl(&["x509","-req","-in",text_path(&csr),
            "-CA",text_path(&ca_cert),"-CAkey",text_path(&ca_key),
            "-CAcreateserial","-out",text_path(&cert),"-days","2",
            "-extfile",text_path(&config),"-extensions","identity"]);
        private(&key);
    }
    fn create_raw_san_identity(&self, name: &str, raw_general_names: &[u8]) {
        // Real X.509 signed by the throwaway test CA. OpenSSL supports
        // `2.5.29.17=DER:...` to encode adversarial SAN extension contents
        // without checking any private identity key into the repository.
        let key=self.path(&format!("{name}.key"));
        let csr=self.path(&format!("{name}.csr"));
        let cert=self.path(&format!("{name}.pem"));
        let ext=self.path(&format!("{name}-raw.cnf"));
        let hex=raw_general_names.iter().map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>().join(":");
        fs::write(&ext, format!(
            "[identity]\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=clientAuth\n2.5.29.17=DER:{hex}\n"
        )).unwrap();
        openssl(&["req","-new","-newkey","rsa:2048","-nodes",
            "-keyout",text_path(&key),"-out",text_path(&csr),
            "-subj","/CN=writer"]);
        let ca=self.path("client-ca.pem");let ca_key=self.path("client-ca.key");
        openssl(&["x509","-req","-in",text_path(&csr),
            "-CA",text_path(&ca),"-CAkey",text_path(&ca_key),
            "-CAcreateserial","-out",text_path(&cert),"-days","2",
            "-extfile",text_path(&ext),"-extensions","identity"]);
        private(&key);
    }
    fn raw_names(names: &[Vec<u8>]) -> Vec<u8> {
        let combined=names.concat();
        assert!(combined.len()<128);
        let mut out=vec![0x30, combined.len() as u8];
        out.extend(combined);
        out
    }
    fn uri(bytes: &[u8]) -> Vec<u8> {
        assert!(bytes.len()<128);
        let mut out=vec![0x86, bytes.len() as u8];out.extend(bytes);out
    }
    fn create_dated_identity(&self, name: &str, start: &str, end: &str) {
        // OpenSSL "ca" supports fixed validity intervals for real negative
        // X.509 handshake tests. Every signing database is ephemeral and
        // never references production or committed key material.
        let key=self.path(&format!("{name}.key"));
        let csr=self.path(&format!("{name}.csr"));
        let cert=self.path(&format!("{name}.pem"));
        openssl(&["req","-new","-newkey","rsa:2048","-nodes",
            "-keyout",text_path(&key),"-out",text_path(&csr),
            "-subj","/CN=DO-NOT-TRUST-CN"]);
        let ext=self.path(&format!("{name}-ext.cnf"));
        fs::write(&ext,
            "[identity]\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=clientAuth\nsubjectAltName=URI:spiffe://rystemdb.local/client/writer\n").unwrap();
        let serial=self.path(&format!("{name}-serial.txt"));
        let db=self.path(&format!("{name}-index.txt"));
        fs::write(&serial, "01\n").unwrap();
        fs::write(&db, "").unwrap();
        let ca_conf=self.path(&format!("{name}-sign.cnf"));
        // OpenSSL configuration paths use forward slashes on all OSes,
        // including Windows, rather than backslash escape sequences.
        let root=text_path(&self.root).replace('\\', "/");
        fs::write(&ca_conf,format!(
            "[ca]\ndefault_ca=CA_default\n[CA_default]\ndatabase={root}/{name}-index.txt\nserial={root}/{name}-serial.txt\nnew_certs_dir={root}\ncertificate={root}/client-ca.pem\nprivate_key={root}/client-ca.key\ndefault_md=sha256\npolicy=policy_any\n[policy_any]\ncommonName=supplied\n"
        )).unwrap();
        openssl(&["ca","-batch","-config",text_path(&ca_conf),"-in",text_path(&csr),
            "-out",text_path(&cert),"-startdate",start,"-enddate",end,
            "-extfile",text_path(&ext),"-extensions","identity","-notext"]);
        private(&key);
    }
    fn start(&self) -> Running {
        let mut child=Command::new(env!("CARGO_BIN_EXE_rystemdb"))
            .args(["rpc-serve","--data"]).arg(self.path("store"))
            .args(["--listen","127.0.0.1:0","--auth-file"]).arg(self.path("token.acl"))
            .arg("--tls-cert").arg(self.path("server-cert.pem"))
            .arg("--tls-key").arg(self.path("server-key.pem"))
            .arg("--mtls-ca").arg(self.path("client-ca.pem"))
            .arg("--mtls-policy").arg(self.path("identity.acl"))
            .stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn().unwrap();
        let mut line=String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
        assert!(line.contains("LOOPBACK RPC listening on"),"mTLS CLI server failed to start");
        let addr=line.split_whitespace().last().unwrap().parse().unwrap();
        Running {child,addr}
    }
    fn connect(&self, addr:SocketAddr, identity:&str, token:&[u8;32])
        ->Result<LocalClient,ClientError> {
        LocalClient::connect_mtls(addr,"localhost",&self.path("root-ca.pem"),
            &self.path(&format!("{identity}.pem")),&self.path(&format!("{identity}.key")),token)
    }
}
impl Drop for Fixture {fn drop(&mut self){let _=fs::remove_dir_all(&self.root);}}
struct Running {child:Child,addr:SocketAddr}
impl Drop for Running {fn drop(&mut self){let _=self.child.kill();let _=self.child.wait();}}

fn typed_document()->Value {
    Value::Object(BTreeMap::from([
        ("bytes".into(),Value::Bytes(vec![0,128,255])),
        ("reference".into(),Value::Reference{collection:"users".into(),id:"u42".into()}),
        ("items".into(),Value::Array(vec![Value::Null, Value::Integer(i64::MIN)]))
    ]))
}

#[test]
fn tdd_verified_mtls_principal_token_intersection_allows_rw_and_prevents_escalation() {
    let f=Fixture::new(); let server=f.start();
    let mut writer=f.connect(server.addr,"writer",&RW).expect("verified writer must authenticate");
    writer.ping().unwrap();
    writer.put("orders","one",&typed_document()).unwrap();
    assert_eq!(writer.get("orders","one").unwrap(),Some(typed_document()));
    let mut reader=f.connect(server.addr,"reader",&RW).expect("read-only principal must authenticate");
    assert_eq!(reader.get("orders","one").unwrap(),Some(typed_document()));
    assert!(matches!(reader.put("orders","deny",&typed_document()),Err(ClientError::Rejected(6))));
    assert!(matches!(reader.delete("orders","one"),Err(ClientError::Rejected(6))));
    assert!(matches!(writer.get("inventory","hidden"),Err(ClientError::Rejected(6))));
    assert!(matches!(writer.get("users","u42"),Err(ClientError::Rejected(6))));
    assert_eq!(writer.get("orders","deny").unwrap(),None);
    assert!(writer.delete("orders","one").unwrap());
    assert_eq!(reader.get("orders","one").unwrap(),None);
}

#[test]
fn tdd_mtls_verified_identity_rejects_wrong_ca_eku_missing_multiple_ambiguous_san() {
    let f=Fixture::new(); let server=f.start();
    for identity in [
        "untrusted-writer", "wrongeku", "expired", "future",
        "nosan", "multisan", "badcase", "stranger",
        "query", "wrongprefix", "percent",
        "raw-invalid-utf8", "raw-double", "raw-uri-and-dns",
        "raw-missing-uri", "raw-truncated", "raw-constructed",
    ] {
        assert!(f.connect(server.addr, identity, &RW).is_err(),
            "untrusted or ambiguous client identity accepted: {identity}");
    }
    assert!(f.connect(server.addr,"writer",&[0u8;32]).is_err(),
        "a verified principal cannot authorize a wrong bearer token");
    assert!(f.connect(server.addr,"writer",&OTHER_COLLECTION).is_err(),
        "disjoint principal and bearer-token collection grants must fail HELLO");
    assert!(LocalClient::connect_tls(server.addr,"localhost",
        &f.path("root-ca.pem"),&RW).is_err(),
        "mTLS server accepted token without a client certificate");
}

#[test]
fn tdd_signed_duplicate_san_extension_is_rejected_by_real_mtls_server() {
    let f=Fixture::new();
    // This creates a REAL client CA-signed X.509 leaf with two separate
    // extension OIDs 2.5.29.17, not just two URIs in one SAN. An independent
    // DER inspection and RSA signature check must succeed before connecting.
    let evidence=f.create_signed_duplicate_san_identity("duplicate-signed");
    assert_eq!(evidence.san_extension_count, 2);
    assert!(evidence.signature_valid, "duplicate-SAN test leaf must be signed by ephemeral CA");

    let server=f.start();
    let mut valid=f.connect(server.addr,"writer",&RW)
        .expect("valid single-SAN writer should continue to authenticate");
    valid.put("orders","duplicate-probe",&typed_document()).unwrap();
    assert_eq!(valid.get("orders","duplicate-probe").unwrap(),Some(typed_document()));

    // Certificate signed by the trusted client CA with matching key and
    // canonical writer URI MUST NOT authorize HELLO if an extra SAN
    // extension is present. A local PEM/key configuration error is not a
    // substitute for testing server-side TLS certificate validation.
    let result=f.connect(server.addr,"duplicate-signed",&RW);
    assert!(!matches!(result, Ok(_) | Err(ClientError::TlsConfig)),
        "duplicate SAN OID reached RSWP, or client did not present the signed test certificate");
    assert!(valid.delete("orders","duplicate-probe").unwrap());
    assert_eq!(valid.get("orders","duplicate-probe").unwrap(),None);
}
