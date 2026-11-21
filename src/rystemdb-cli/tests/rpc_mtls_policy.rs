//! RED first for #57: canonical mTLS principal ACL configuration must fail
//! closed until actual verified client-certificate identity is implemented.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT: AtomicUsize = AtomicUsize::new(0);
const INVALID_POLICY: &str = "invalid private mTLS principal policy";
const NOT_ENABLED: &str = "mTLS client certificate authentication is not yet enabled";
const VALID: &str =
    "rystem-mtls-principal-acl-v1\nspiffe://rystemdb.local/client/reader orders r\nspiffe://rystemdb.local/client/writer orders rw\n";

struct Tmp(PathBuf);
impl Tmp {
    fn new() -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("rystem-mtls-policy-{}-{nanos}-{id}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path { &self.0 }
}
impl Drop for Tmp { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }

fn private_file(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn assert_rejected(tmp: &Tmp, path: &Path, marker: &str, case: &str) {
    let data = tmp.path().join(format!("not-created-{case}"));
    let output = Command::new(env!("CARGO_BIN_EXE_rystemdb"))
        .args(["rpc-serve", "--data"]).arg(&data)
        .args(["--listen", "127.0.0.1:0", "--mtls-policy"]).arg(path)
        .output().unwrap();
    assert!(!output.status.success(), "mTLS policy unexpectedly enabled a server: {case}");
    assert!(!data.exists(), "mTLS policy unexpectedly created a database: {case}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(marker), "incorrect mTLS rejection at {case}: {stderr}");
    assert!(!stderr.contains("spiffe://"), "principal identity leaked into diagnostics");
}

#[test]
fn tdd_mtls_policy_validated_but_never_starts_without_verified_certificate_identity() {
    let tmp = Tmp::new();
    let policy = tmp.path().join("policy");
    private_file(&policy, VALID.as_bytes());
    assert_rejected(&tmp, &policy, NOT_ENABLED, "valid");

    let invalid = [
        ("version", "rystem-mtls-principal-acl-v2\nspiffe://rystemdb.local/client/reader orders r\n"),
        ("empty", "rystem-mtls-principal-acl-v1\n"),
        ("duplicate", "rystem-mtls-principal-acl-v1\nspiffe://rystemdb.local/client/reader orders r\nspiffe://rystemdb.local/client/reader orders w\n"),
        ("wildcard", "rystem-mtls-principal-acl-v1\nspiffe://rystemdb.local/client/* orders rw\n"),
        ("uri-query", "rystem-mtls-principal-acl-v1\nspiffe://rystemdb.local/client/reader?x=1 orders r\n"),
        ("noncanonical", "rystem-mtls-principal-acl-v1\nSPIFFE://rystemdb.local/client/reader orders r\n"),
        ("extra", "rystem-mtls-principal-acl-v1\nspiffe://rystemdb.local/client/reader orders rw admin\n"),
        ("collection", "rystem-mtls-principal-acl-v1\nspiffe://rystemdb.local/client/reader * r\n"),
        ("rights", "rystem-mtls-principal-acl-v1\nspiffe://rystemdb.local/client/reader orders read\n"),
    ];
    for (case, body) in invalid {
        private_file(&policy, body.as_bytes());
        assert_rejected(&tmp, &policy, INVALID_POLICY, case);
    }
}

#[test]
fn tdd_mtls_policy_private_file_bounds_and_symlink_are_fail_closed() {
    let tmp = Tmp::new();
    let missing = tmp.path().join("missing");
    assert_rejected(&tmp, &missing, INVALID_POLICY, "missing");

    let path = tmp.path().join("policy");
    private_file(&path, &[0xff, 0xfe, 0xfd]);
    assert_rejected(&tmp, &path, INVALID_POLICY, "not-utf8");
    private_file(&path, &vec![b'x'; 65 * 1024]);
    assert_rejected(&tmp, &path, INVALID_POLICY, "oversized");
    let mut too_many = String::from("rystem-mtls-principal-acl-v1\n");
    for i in 0..129 { too_many.push_str(&format!("spiffe://rystemdb.local/client/user{i} orders r\n")); }
    private_file(&path, too_many.as_bytes());
    assert_rejected(&tmp, &path, INVALID_POLICY, "too-many");

    #[cfg(unix)]
    {
        use std::os::unix::fs::{symlink, PermissionsExt};
        private_file(&path, VALID.as_bytes());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert_rejected(&tmp, &path, INVALID_POLICY, "public-policy");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let link = tmp.path().join("linked");
        symlink(&path, &link).unwrap();
        assert_rejected(&tmp, &link, INVALID_POLICY, "symlink");
    }
}
