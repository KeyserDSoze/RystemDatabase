use std::process::Command;

#[test]
fn cli_returns_matching_version() {
    let out = Command::new(env!("CARGO_BIN_EXE_rystemdb"))
        .arg("--version").output().expect("spawn CLI");
    assert!(out.status.success());
    assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), format!("rystemdb {}", env!("CARGO_PKG_VERSION")));
}

#[test]
fn cli_help_distinguishes_http_health_from_loopback_binary_rpc() {
    let out = Command::new(env!("CARGO_BIN_EXE_rystemdb"))
        .arg("--help").output().expect("spawn CLI");
    assert!(out.status.success());
    let help = String::from_utf8(out.stdout).unwrap();
    assert!(help.contains("document put"));
    assert!(help.contains("document get"));
    assert!(help.contains("HTTP serve exposes health only"));
    assert!(help.contains("rpc-serve has local binary CRUD"));
    assert!(help.contains("--auth-file PRIVATE_PATH"));
    assert!(help.contains("No TLS"));
    assert!(help.contains("localhost-only"));
}
