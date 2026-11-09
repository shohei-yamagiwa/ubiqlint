use std::process::Command;

#[test]
fn prints_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_ubiqlint"))
        .arg("--version")
        .output()
        .expect("failed to run ubiqlint");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout is not UTF-8");
    assert_eq!(
        stdout.trim(),
        format!("ubiqlint {}", env!("CARGO_PKG_VERSION"))
    );
}
