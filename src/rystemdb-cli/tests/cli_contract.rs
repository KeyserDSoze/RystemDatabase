use std::process::Command;

#[test]
fn cli_returns_matching_version() {
    let out = Command::new(env!("CARGO_BIN_EXE_rystemdb"))
        .arg("--version").output().expect("spawn CLI");
    assert!(out.status.success());
    assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), format!("rystemdb {}", env!("CARGO_PKG_VERSION")));
}

#[test]
fn cli_help_describes_binary_document_cli_and_health_only_server() {
    let out = Command::new(env!("CARGO_BIN_EXE_rystemdb"))
        .arg("--help").output().expect("spawn CLI");
    assert!(out.status.success());
    let help = String::from_utf8(out.stdout).unwrap();
    assert!(help.contains("document put"));
    assert!(help.contains("document get"));
    assert!(help.contains("network server exposes only health endpoints"));
}
