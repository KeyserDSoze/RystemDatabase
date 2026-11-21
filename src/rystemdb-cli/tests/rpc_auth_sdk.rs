//! TDD real-process authenticated Rust client interoperability.
use rystemdb_client::{ClientError, LocalClient};
use rystemdb_core::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

struct AuthFixture { root: PathBuf, child: Child, address: SocketAddr }
impl AuthFixture {
    fn start() -> Self {
        let nanos=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root=std::env::temp_dir().join(format!("rystem-rust-auth-{}-{nanos}",std::process::id()));
        fs::create_dir(&root).unwrap();
        let policy=root.join("acl.txt");
        fs::write(&policy, format!("{} docs r\n{} docs w\n",
            "11".repeat(32),"22".repeat(32))).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&policy,fs::Permissions::from_mode(0o600)).unwrap();
        }
        let mut child=Command::new(env!("CARGO_BIN_EXE_rystemdb"))
            .args(["rpc-serve","--data"]).arg(root.join("store"))
            .args(["--listen","127.0.0.1:0","--auth-file"]).arg(&policy)
            .stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
        let mut banner=String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut banner).unwrap();
        assert!(banner.contains("LOOPBACK RPC listening on"),"missing server startup: {banner:?}");
        let address=banner.split_whitespace().last().unwrap().parse().unwrap();
        Self{root,child,address}
    }
}
impl Drop for AuthFixture {
    fn drop(&mut self) {
        let _=self.child.kill();let _=self.child.wait();let _=fs::remove_dir_all(&self.root);
    }
}
#[test]
fn tdd_rust_client_authenticated_hello_and_scoped_crud() {
    let fixture=AuthFixture::start();
    assert!(matches!(LocalClient::connect(fixture.address), Err(ClientError::Rejected(6))));
    assert!(matches!(LocalClient::connect_with_token(fixture.address,&[0;32]),
        Err(ClientError::Rejected(6))));
    let mut writer=LocalClient::connect_with_token(fixture.address,&[0x22;32]).unwrap();
    let mut reader=LocalClient::connect_with_token(fixture.address,&[0x11;32]).unwrap();
    writer.ping().unwrap();
    let doc=Value::Object(BTreeMap::from([("int".into(),Value::Integer(i64::MIN))]));
    writer.put("docs","private",&doc).unwrap();
    assert!(matches!(writer.get("docs","private"),Err(ClientError::Rejected(6))));
    assert_eq!(reader.get("docs","private").unwrap(),Some(doc.clone()));
    assert!(matches!(reader.put("docs","private",&doc),Err(ClientError::Rejected(6))));
    assert!(matches!(writer.put("other","private",&doc),Err(ClientError::Rejected(6))));
    assert!(writer.delete("docs","private").unwrap());
}
