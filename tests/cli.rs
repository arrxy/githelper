use std::process::Command;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_githelper"))
}

#[test]
fn help_explains_review_only_behavior() {
    let output = cli().arg("--help").output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Nothing is executed"));
}

#[test]
fn empty_request_is_actionable() {
    let output = cli().output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("provide a request"));
}

#[test]
fn missing_repository_reports_an_error() {
    let directory = tempfile::tempdir().unwrap();
    let output = cli()
        .arg("--repo")
        .arg(directory.path().join("missing"))
        .arg("show my changes")
        .output()
        .unwrap();
    assert!(!output.status.success());
}

#[cfg(not(target_os = "macos"))]
#[test]
fn doctor_reports_unsupported_platform_without_a_repository() {
    let directory = tempfile::tempdir().unwrap();
    let output = cli()
        .current_dir(directory.path())
        .args(["--doctor", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["available"], false);
    assert!(json["detail"].as_str().unwrap().contains("macOS 26"));
}
