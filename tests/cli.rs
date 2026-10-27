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
    assert!(output.stdout.ends_with(b"\n"));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["available"], false);
    assert_eq!(json["ok"], false);
    assert!(json["detail"].as_str().unwrap().contains("macOS 26"));
    let checks = json["checks"].as_array().unwrap();
    let check = |id: &str| checks.iter().find(|item| item["id"] == id).unwrap();
    assert_eq!(check("macos")["passed"], false);
    assert_eq!(check("macos")["required"], true);
    assert!(
        check("macos")["hint"]
            .as_str()
            .unwrap()
            .contains("macOS 26")
    );
    assert_eq!(check("xcode")["passed"], false);
    assert!(
        check("xcode")["hint"]
            .as_str()
            .unwrap()
            .contains("xcode-select")
    );
    assert_eq!(check("apple_intelligence")["passed"], false);
    assert!(
        check("apple_intelligence")["hint"]
            .as_str()
            .unwrap()
            .contains("Apple Intelligence")
    );
    assert_eq!(check("git")["passed"], true);
    assert_eq!(check("repository")["passed"], false);
    assert_eq!(check("repository")["required"], false);
    assert!(
        check("repository")["hint"]
            .as_str()
            .unwrap()
            .contains("--repo")
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("setup checks failed"));
    assert!(stderr.contains("macOS 26 or newer"));
    assert!(stderr.contains("Xcode 26 and Swift"));
    assert!(stderr.contains("Apple Intelligence"));

    let text = cli()
        .current_dir(directory.path())
        .arg("--doctor")
        .output()
        .unwrap();
    assert!(!text.status.success());
    let report = String::from_utf8_lossy(&text.stdout);
    for label in [
        "[fail] macOS 26 or newer",
        "[fail] Xcode 26 and Swift",
        "[fail] Apple Intelligence",
        "[pass] Git\n",
        "[fail] Git repository (optional)",
        "Fix:",
        "xcode-select",
    ] {
        assert!(report.contains(label), "{label}\n{report}");
    }
}

#[cfg(not(target_os = "macos"))]
#[test]
fn model_setup_errors_point_at_doctor() {
    let directory = tempfile::tempdir().unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--initial-branch=main"])
            .arg(directory.path())
            .status()
            .unwrap()
            .success()
    );
    let output = cli()
        .arg("--repo")
        .arg(directory.path())
        .arg("show my changes")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("githelper --doctor"), "{stderr}");
    assert!(stderr.contains("macOS 26"), "{stderr}");
    assert!(stderr.contains("Apple Intelligence"), "{stderr}");
}

#[test]
fn incompatible_modes_are_rejected() {
    for arguments in [
        vec!["--interactive", "--json", "show changes"],
        vec!["--interactive", "--doctor"],
        vec!["--interactive", "--draft-commit"],
        vec!["--draft-commit", "--doctor"],
        vec!["--draft-commit", "show changes"],
    ] {
        let output = cli().args(arguments).output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used with"));
    }
}

#[test]
fn interactive_requires_a_terminal_without_reading_piped_input() {
    let output = cli()
        .args(["--interactive", "commit my changes"])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires terminal input"));
}

#[test]
fn draft_mode_does_not_require_a_positional_request() {
    let directory = tempfile::tempdir().unwrap();
    let output = cli()
        .arg("--repo")
        .arg(directory.path())
        .arg("--draft-commit")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("provide a request"));
    assert!(output.stdout.is_empty());
}
