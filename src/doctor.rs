//! Environment checks for `githelper --doctor`.
//!
//! Probes collect facts. [`assess`] turns those facts into pass/fail checks, so
//! the wording and the exit decision can be tested without Xcode or Apple
//! Intelligence. The repository check is optional: doctor is a preflight and
//! must stay successful outside a Git work tree when the machine itself is ready.

use crate::model::AppleModel;
use crate::setup_text::{self, ToolchainIssue};
use serde::Serialize;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MacosFacts {
    Version(String),
    NotMacos { os: String },
    Unreadable { detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XcodeFacts {
    Ready { xcode_line: String, sdk: String },
    Problem(ToolchainIssue),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelFacts {
    Available { detail: String },
    Unavailable { detail: String },
    BridgeFailed { detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitFacts {
    Ready { version: String },
    Missing { detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoFacts {
    Ready { root: String },
    Failed { detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    pub macos: MacosFacts,
    pub xcode: XcodeFacts,
    pub model: ModelFacts,
    pub git: GitFacts,
    pub repository: RepoFacts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    pub id: &'static str,
    pub label: &'static str,
    pub passed: bool,
    pub required: bool,
    pub message: String,
    pub hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    /// True when every required check passed. The repository check is not required.
    pub ok: bool,
    /// Apple's on-device model specifically. Preserved for `--doctor --json` callers.
    pub available: bool,
    /// Model status line. Preserved for `--doctor --json` callers.
    pub detail: String,
    pub checks: Vec<Check>,
}

impl Report {
    pub fn failure_summary(&self) -> String {
        let failed: Vec<&str> = self
            .checks
            .iter()
            .filter(|check| check.required && !check.passed)
            .map(|check| check.label)
            .collect();
        if failed.is_empty() {
            "setup checks failed".to_string()
        } else {
            format!("setup checks failed: {}", failed.join("; "))
        }
    }
}

pub fn diagnose(repo: &Path) -> Report {
    assess(&probe(repo))
}

pub fn assess(facts: &Facts) -> Report {
    let checks = vec![
        macos_check(&facts.macos),
        xcode_check(&facts.xcode),
        model_check(&facts.model),
        git_check(&facts.git),
        repo_check(&facts.repository),
    ];
    let available = matches!(facts.model, ModelFacts::Available { .. });
    let detail = match &facts.model {
        ModelFacts::Available { detail }
        | ModelFacts::Unavailable { detail }
        | ModelFacts::BridgeFailed { detail } => detail.clone(),
    };
    let ok = checks.iter().all(|check| !check.required || check.passed);
    Report {
        ok,
        available,
        detail,
        checks,
    }
}

pub fn render_text(report: &Report) -> String {
    let mut text = String::new();
    for check in &report.checks {
        let state = if check.passed { "pass" } else { "fail" };
        let optional = if check.required { "" } else { " (optional)" };
        text.push_str(&format!("[{state}] {}{optional}\n", check.label));
        text.push_str(&format!("       {}\n", check.message));
        if !check.passed && !check.hint.is_empty() {
            text.push_str(&format!("       Fix: {}\n", check.hint));
        }
    }
    let required_failed = report
        .checks
        .iter()
        .filter(|check| check.required && !check.passed)
        .count();
    match required_failed {
        0 => text.push_str("All required checks passed.\n"),
        1 => text.push_str("1 required check failed.\n"),
        count => text.push_str(&format!("{count} required checks failed.\n")),
    }
    text
}

fn macos_check(facts: &MacosFacts) -> Check {
    match facts {
        MacosFacts::Version(version) => match setup_text::major_version(version) {
            Some(major) if major >= 26 => {
                pass("macos", "macOS 26 or newer", format!("macOS {version}"))
            }
            Some(_) => fail(
                "macos",
                "macOS 26 or newer",
                format!("Found macOS {version}."),
                setup_text::MACOS_FIX,
            ),
            None => fail(
                "macos",
                "macOS 26 or newer",
                format!("Could not parse the macOS version {version:?}."),
                setup_text::MACOS_FIX,
            ),
        },
        MacosFacts::NotMacos { os } => fail(
            "macos",
            "macOS 26 or newer",
            format!("This system is {os}, not macOS."),
            setup_text::MACOS_FIX,
        ),
        MacosFacts::Unreadable { detail } => fail(
            "macos",
            "macOS 26 or newer",
            format!("Could not read the macOS version ({detail})."),
            setup_text::MACOS_FIX,
        ),
    }
}

fn xcode_check(facts: &XcodeFacts) -> Check {
    match facts {
        XcodeFacts::Ready { xcode_line, sdk } => pass(
            "xcode",
            "Xcode 26 and Swift",
            format!("{xcode_line}, macOS SDK {sdk}"),
        ),
        XcodeFacts::Problem(issue) => {
            fail("xcode", "Xcode 26 and Swift", issue.message(), issue.hint())
        }
    }
}

fn model_check(facts: &ModelFacts) -> Check {
    match facts {
        ModelFacts::Available { detail } => {
            pass("apple_intelligence", "Apple Intelligence", detail.clone())
        }
        ModelFacts::Unavailable { detail } => fail(
            "apple_intelligence",
            "Apple Intelligence",
            detail.clone(),
            setup_text::APPLE_INTELLIGENCE_FIX,
        ),
        ModelFacts::BridgeFailed { detail } => Check {
            id: "apple_intelligence",
            label: "Apple Intelligence",
            passed: false,
            required: true,
            message: detail.clone(),
            hint: format!(
                "{} {}",
                setup_text::XCODE_FIX,
                setup_text::APPLE_INTELLIGENCE_FIX
            ),
        },
    }
}

fn git_check(facts: &GitFacts) -> Check {
    match facts {
        GitFacts::Ready { version } => pass("git", "Git", version.clone()),
        GitFacts::Missing { detail } => fail(
            "git",
            "Git",
            format!("Git is not available ({detail})."),
            setup_text::GIT_FIX,
        ),
    }
}

fn repo_check(facts: &RepoFacts) -> Check {
    match facts {
        RepoFacts::Ready { root } => Check {
            id: "repository",
            label: "Git repository",
            passed: true,
            required: false,
            message: root.clone(),
            hint: String::new(),
        },
        RepoFacts::Failed { detail } => Check {
            id: "repository",
            label: "Git repository",
            passed: false,
            required: false,
            message: detail.clone(),
            hint: setup_text::REPO_FIX.to_string(),
        },
    }
}

fn pass(id: &'static str, label: &'static str, message: String) -> Check {
    Check {
        id,
        label,
        passed: true,
        required: true,
        message,
        hint: String::new(),
    }
}

fn fail(id: &'static str, label: &'static str, message: String, hint: &str) -> Check {
    Check {
        id,
        label,
        passed: false,
        required: true,
        message,
        hint: hint.to_string(),
    }
}

pub fn probe(repo: &Path) -> Facts {
    let macos = probe_macos();
    let xcode = probe_xcode();
    let model = probe_model();
    let git = probe_git();
    let git_ready = matches!(git, GitFacts::Ready { .. });
    let repository = probe_repository(repo, git_ready);
    Facts {
        macos,
        xcode,
        model,
        git,
        repository,
    }
}

fn probe_macos() -> MacosFacts {
    if std::env::consts::OS != "macos" {
        return MacosFacts::NotMacos {
            os: std::env::consts::OS.to_string(),
        };
    }
    match capture("sw_vers", &["-productVersion"]) {
        Ok(version) => {
            let version = version.lines().next().unwrap_or("").trim().to_string();
            if version.is_empty() {
                MacosFacts::Unreadable {
                    detail: "sw_vers returned an empty version".into(),
                }
            } else {
                MacosFacts::Version(version)
            }
        }
        Err(detail) => MacosFacts::Unreadable { detail },
    }
}

fn probe_xcode() -> XcodeFacts {
    if let Err(detail) = capture("xcrun", &["--find", "swiftc"]) {
        return XcodeFacts::Problem(issue_from_command(&detail));
    }
    let xcodebuild = capture("xcodebuild", &["-version"]);
    let sdk = capture("xcrun", &["--sdk", "macosx", "--show-sdk-version"]);
    match (xcodebuild, sdk) {
        (Ok(xcode), Ok(sdk_text)) => match setup_text::validate_toolchain(&xcode, &sdk_text) {
            Ok(toolchain) => XcodeFacts::Ready {
                xcode_line: toolchain.xcode_line,
                sdk: toolchain.sdk,
            },
            Err(issue) => XcodeFacts::Problem(issue),
        },
        (Err(detail), _) | (_, Err(detail)) => XcodeFacts::Problem(issue_from_command(&detail)),
    }
}

fn issue_from_command(detail: &str) -> ToolchainIssue {
    if setup_text::mentions_license(detail) {
        ToolchainIssue::License
    } else {
        ToolchainIssue::Missing {
            detail: detail.to_string(),
        }
    }
}

fn probe_model() -> ModelFacts {
    match AppleModel.status() {
        Ok(status) if status.available => ModelFacts::Available {
            detail: status.detail,
        },
        Ok(status) => ModelFacts::Unavailable {
            detail: status.detail,
        },
        Err(error) => ModelFacts::BridgeFailed {
            detail: format!("Could not query Apple's on-device model: {error}"),
        },
    }
}

fn probe_git() -> GitFacts {
    match capture("git", &["--version"]) {
        Ok(version) => GitFacts::Ready { version },
        Err(detail) => GitFacts::Missing { detail },
    }
}

fn probe_repository(path: &Path, git_ready: bool) -> RepoFacts {
    if !git_ready {
        return RepoFacts::Failed {
            detail: "Git is not installed, so the repository was not checked.".into(),
        };
    }
    if !path.exists() {
        return RepoFacts::Failed {
            detail: format!("Repository path does not exist: {}.", path.display()),
        };
    }
    match git_output(path, &["rev-parse", "--is-inside-work-tree"]) {
        Ok(value) if value == "true" => match git_output(path, &["rev-parse", "--show-toplevel"]) {
            Ok(root) => RepoFacts::Ready { root },
            Err(detail) => RepoFacts::Failed {
                detail: format!("{}: {detail}", path.display()),
            },
        },
        Ok(_) => RepoFacts::Failed {
            detail: format!("{} is not a Git working tree.", path.display()),
        },
        Err(detail) => {
            let lower = detail.to_ascii_lowercase();
            let message = if lower.contains("not a git repository") {
                format!("{} is not a Git repository.", path.display())
            } else {
                format!("{}: {detail}", path.display())
            };
            RepoFacts::Failed { detail: message }
        }
    }
}

fn capture(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("{program}: {error}"))?;
    command_text(program, &output)
}

fn git_output(directory: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(["--no-optional-locks", "-c", "core.fsmonitor=false", "-C"])
        .arg(directory)
        .args(args)
        .output()
        .map_err(|error| format!("git: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() {
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            Err(format!("git exited with {}", output.status))
        } else {
            Err(stderr)
        }
    }
}

fn command_text(program: &str, output: &std::process::Output) -> Result<String, String> {
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.success() {
        Ok(text.trim().to_string())
    } else if text.trim().is_empty() {
        Err(format!("{program} exited with {}", output.status))
    } else {
        Err(format!(
            "{program} exited with {}: {}",
            output.status,
            text.trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready() -> Facts {
        Facts {
            macos: MacosFacts::Version("26.0".into()),
            xcode: XcodeFacts::Ready {
                xcode_line: "Xcode 26.0".into(),
                sdk: "26.0".into(),
            },
            model: ModelFacts::Available {
                detail: "Apple's on-device model is ready. Inference stays on this Mac.".into(),
            },
            git: GitFacts::Ready {
                version: "git version 2.50.1".into(),
            },
            repository: RepoFacts::Ready {
                root: "/Users/me/code/your-repository".into(),
            },
        }
    }

    #[test]
    fn ready_machine_passes_every_check() {
        let report = assess(&ready());
        assert!(report.ok);
        assert!(report.available);
        assert!(report.detail.contains("on-device model is ready"));
        assert!(report.checks.iter().all(|check| check.passed));
        assert!(report.checks.iter().all(|check| check.hint.is_empty()));
        assert_eq!(
            report
                .checks
                .iter()
                .map(|check| check.id)
                .collect::<Vec<_>>(),
            vec!["macos", "xcode", "apple_intelligence", "git", "repository"]
        );
        assert!(!report.checks[4].required);
        let text = render_text(&report);
        assert_eq!(
            text,
            "\
[pass] macOS 26 or newer
       macOS 26.0
[pass] Xcode 26 and Swift
       Xcode 26.0, macOS SDK 26.0
[pass] Apple Intelligence
       Apple's on-device model is ready. Inference stays on this Mac.
[pass] Git
       git version 2.50.1
[pass] Git repository (optional)
       /Users/me/code/your-repository
All required checks passed.
"
        );
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["ok"], true);
        assert_eq!(json["available"], true);
        assert!(json["checks"].as_array().unwrap().len() == 5);
    }

    #[test]
    fn each_required_failure_has_a_fix_and_fails_the_report() {
        let mut facts = ready();
        facts.macos = MacosFacts::Version("15.6".into());
        facts.xcode = XcodeFacts::Problem(ToolchainIssue::TooOld {
            found: "Xcode 16.4".into(),
        });
        facts.model = ModelFacts::Unavailable {
            detail: setup_text::MODEL_UNSUPPORTED_PLATFORM.into(),
        };
        facts.git = GitFacts::Missing {
            detail: "git: No such file or directory (os error 2)".into(),
        };
        facts.repository = RepoFacts::Failed {
            detail: "/tmp/work is not a Git repository.".into(),
        };
        let report = assess(&facts);
        assert!(!report.ok);
        assert!(!report.available);
        assert!(report.detail.contains("macOS 26"));
        assert_eq!(
            report.failure_summary(),
            "setup checks failed: macOS 26 or newer; Xcode 26 and Swift; Apple Intelligence; Git"
        );
        let text = render_text(&report);
        assert!(text.contains("[fail] macOS 26 or newer"));
        assert!(text.contains("[fail] Xcode 26 and Swift"));
        assert!(text.contains("[fail] Apple Intelligence"));
        assert!(text.contains("[fail] Git"));
        assert!(text.contains("[fail] Git repository (optional)"));
        assert!(text.contains("Fix: "));
        assert!(text.contains(setup_text::MACOS_FIX));
        assert!(text.contains(setup_text::XCODE_FIX));
        assert!(text.contains("xcode-select"));
        assert!(text.contains(setup_text::APPLE_INTELLIGENCE_FIX));
        assert!(text.contains(setup_text::GIT_FIX));
        assert!(text.contains("--repo"));
        assert!(text.contains("4 required checks failed."));
        assert!(!report.checks[4].required);
    }

    #[test]
    fn missing_repository_does_not_fail_a_ready_machine() {
        let mut facts = ready();
        facts.repository = RepoFacts::Failed {
            detail: ". is not a Git repository.".into(),
        };
        let report = assess(&facts);
        assert!(report.ok);
        assert!(report.available);
        assert!(!report.checks[4].passed);
        assert!(!report.checks[4].required);
        assert!(render_text(&report).contains("All required checks passed."));
    }

    #[test]
    fn unsupported_platform_matches_the_build_requirements() {
        let facts = Facts {
            macos: MacosFacts::NotMacos { os: "linux".into() },
            xcode: XcodeFacts::Problem(ToolchainIssue::Missing {
                detail: "xcrun: No such file or directory (os error 2)".into(),
            }),
            model: ModelFacts::Unavailable {
                detail: setup_text::MODEL_UNSUPPORTED_PLATFORM.into(),
            },
            git: GitFacts::Ready {
                version: "git version 2.43.0".into(),
            },
            repository: RepoFacts::Failed {
                detail: "/tmp/not-a-repo is not a Git repository.".into(),
            },
        };
        let report = assess(&facts);
        assert!(!report.ok);
        assert!(!report.available);
        assert!(report.detail.contains("macOS 26"));
        assert!(report.checks[0].message.contains("linux"));
        assert!(report.checks[1].hint.contains("xcode-select"));
        assert!(
            report.checks[1]
                .message
                .contains("Swift toolchain is missing")
        );
        assert!(report.checks[3].passed);
    }

    #[test]
    fn bridge_failure_points_at_xcode_and_apple_intelligence() {
        let mut facts = ready();
        facts.model = ModelFacts::BridgeFailed {
            detail: "Could not start the Apple model bridge (No such file or directory).".into(),
        };
        let report = assess(&facts);
        assert!(!report.ok);
        assert!(!report.available);
        let hint = &report.checks[2].hint;
        assert!(hint.contains("xcode-select"));
        assert!(hint.contains("Apple Intelligence"));
        assert_eq!(
            report.failure_summary(),
            "setup checks failed: Apple Intelligence"
        );
        assert!(render_text(&report).contains("1 required check failed."));
    }

    #[test]
    fn macos_boundary_is_26() {
        let mut facts = ready();
        facts.macos = MacosFacts::Version("26".into());
        assert!(assess(&facts).checks[0].passed);
        facts.macos = MacosFacts::Version("26.0.1".into());
        assert!(assess(&facts).checks[0].passed);
        facts.macos = MacosFacts::Version("25.9".into());
        let report = assess(&facts);
        assert!(!report.checks[0].passed);
        assert!(!report.ok);
        assert!(report.checks[0].hint.contains("macOS 26"));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn probe_on_this_platform_separates_facts_from_the_report() {
        let directory = tempfile::tempdir().unwrap();
        let facts = probe(directory.path());
        assert!(matches!(facts.macos, MacosFacts::NotMacos { .. }));
        assert!(matches!(facts.xcode, XcodeFacts::Problem(_)));
        assert!(matches!(facts.model, ModelFacts::Unavailable { .. }));
        let report = assess(&facts);
        assert_eq!(report.checks.len(), 5);
        assert!(!report.ok);
        assert!(!report.available);
        assert_eq!(
            report.ok,
            report
                .checks
                .iter()
                .all(|check| !check.required || check.passed)
        );
    }
}
