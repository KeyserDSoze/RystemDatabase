use std::process::Command;

#[test]
fn cli_returns_matching_version() {
    let out = Command::new(env!("CARGO_BIN_EXE_rystemdb"))
        .arg("--version").output().expect("spawn CLI");
    assert!(out.status.success());
    assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), format!("rystemdb {}", env!("CARGO_PKG_VERSION")));
}

#[test]
fn cli_help_discloses_foundation_limitations() {
    let out = Command::new(env!("CARGO_BIN_EXE_rystemdb"))
        .arg("--help").output().expect("spawn CLI");
    assert!(out.status.success());
    assert!(String::from_utf8(out.stdout).unwrap().contains("No data is stored"));
}
