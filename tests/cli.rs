use std::process::Command;

fn cli() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_githelper"));
    // A developer shell may export the test hook. Default tests must not see it,
    // and feature tests opt in per invocation.
    command.env_remove("GITHELPER_FAKE_MODEL");
    command
}

fn binary_contains(needle: &[u8]) -> bool {
    let bytes = std::fs::read(env!("CARGO_BIN_EXE_githelper")).unwrap();
    bytes.windows(needle.len()).any(|window| window == needle)
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

#[cfg(not(feature = "fake-model"))]
#[test]
fn default_build_does_not_contain_the_fake_model() {
    assert!(
        !binary_contains(b"GITHELPER_FAKE_MODEL"),
        "default build contains the fake model hook"
    );
}

#[cfg(feature = "fake-model")]
#[test]
fn feature_build_contains_the_fake_model_hook() {
    assert!(
        binary_contains(b"GITHELPER_FAKE_MODEL"),
        "fake-model build is missing the fake model hook"
    );
}

/// Repository plus a `git` on `PATH` that records every invocation.
///
/// Inspect commands from context collection are allowed. Anything else means
/// githelper executed a planned command.
#[cfg(any(feature = "fake-model", not(target_os = "macos")))]
mod plan_path {
    use super::cli;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    struct Workspace {
        _root: tempfile::TempDir,
        root: PathBuf,
        repo: PathBuf,
        git_log: PathBuf,
        fingerprint: String,
    }

    impl Workspace {
        fn new() -> Self {
            let root_dir = tempfile::tempdir().unwrap();
            let root = root_dir.path().to_path_buf();
            let repo = root.join("repo");
            std::fs::create_dir(&repo).unwrap();
            let git = real_git();
            let run_git = |args: &[&str]| {
                let output = Command::new(&git)
                    .args(args)
                    .current_dir(&repo)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{args:?}\n{}",
                    String::from_utf8_lossy(&output.stderr)
                );
            };
            run_git(&["init", "--initial-branch=main"]);
            run_git(&["config", "user.name", "Githelper Test"]);
            run_git(&["config", "user.email", "githelper@example.invalid"]);
            std::fs::write(repo.join("README.md"), "hello\n").unwrap();
            std::fs::write(repo.join("notes with spaces.txt"), "notes\n").unwrap();
            run_git(&["add", "--", "README.md", "notes with spaces.txt"]);
            run_git(&["commit", "-m", "initial"]);
            std::fs::write(repo.join("README.md"), "changed\n").unwrap();
            std::fs::write(repo.join("untracked.txt"), "secret\n").unwrap();

            let bin = root.join("bin");
            std::fs::create_dir(&bin).unwrap();
            let git_log = root.join("git.log");
            let script = format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\nexec {} \"$@\"\n",
                shell_single(&git_log),
                shell_single(&git)
            );
            let wrapper = bin.join("git");
            std::fs::write(&wrapper, script).unwrap();
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();

            let mut workspace = Self {
                _root: root_dir,
                root,
                repo,
                git_log,
                fingerprint: String::new(),
            };
            workspace.fingerprint = workspace.capture();
            workspace
        }

        fn capture(&self) -> String {
            let git = |args: &[&str]| {
                let output = Command::new("git")
                    .args(args)
                    .current_dir(&self.repo)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{args:?}\n{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                String::from_utf8(output.stdout).unwrap()
            };
            format!(
                "HEAD {head}\nSTATUS {status}\nDIFF {diff}\nCACHED {cached}\nBRANCH {branch}\nREADME {readme}\nNOTES {notes}\nUNTRACKED {untracked}\n",
                head = git(&["rev-parse", "HEAD"]),
                status = git(&["status", "--porcelain=v1", "--untracked-files=all"]),
                diff = git(&["diff"]),
                cached = git(&["diff", "--cached"]),
                branch = git(&["branch", "--list"]),
                readme = std::fs::read_to_string(self.repo.join("README.md")).unwrap(),
                notes = std::fs::read_to_string(self.repo.join("notes with spaces.txt")).unwrap(),
                untracked = std::fs::read_to_string(self.repo.join("untracked.txt")).unwrap(),
            )
        }

        fn run(&self, fake: &str, args: &[&str]) -> Output {
            let _ = std::fs::remove_file(&self.git_log);
            let path = format!(
                "{}:{}",
                self.root.join("bin").display(),
                std::env::var("PATH").unwrap_or_default()
            );
            cli()
                .current_dir(&self.repo)
                .env("GITHELPER_FAKE_MODEL", fake)
                .env("PATH", &path)
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_PAGER", "cat")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .arg("--repo")
                .arg(&self.repo)
                .args(args)
                .output()
                .unwrap()
        }

        fn assert_unchanged(&self) {
            assert_eq!(
                self.capture(),
                self.fingerprint,
                "githelper changed the repository"
            );
        }

        fn assert_only_inspected(&self) {
            let log = std::fs::read_to_string(&self.git_log).unwrap_or_default();
            assert!(
                !log.trim().is_empty(),
                "inspection git was not observed; the PATH wrapper did not run"
            );
            for line in log.lines() {
                assert!(
                    is_inspect(line),
                    "githelper executed a command outside repository inspection:\n{line}\n{log}"
                );
            }
        }
    }

    fn real_git() -> PathBuf {
        let output = Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
    }

    fn shell_single(path: &Path) -> String {
        format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
    }

    fn is_inspect(line: &str) -> bool {
        let mut parts = line.split_whitespace();
        let mut subcommand = None;
        while let Some(part) = parts.next() {
            if part == "-c" || part == "-C" || part == "--git-dir" {
                let _ = parts.next();
                continue;
            }
            if part.starts_with('-') {
                continue;
            }
            subcommand = Some(part);
            break;
        }
        match subcommand {
            Some("rev-parse") => line.contains("--show-toplevel"),
            Some("for-each-ref") => line.contains("refs/heads/"),
            Some("status") => {
                line.contains("--porcelain=v1") && line.contains("--untracked-files=no")
            }
            Some("branch") => line.contains("--show-current"),
            _ => false,
        }
    }

    fn model_plan(
        summary: &str,
        outcome: &str,
        clarification: &str,
        steps: serde_json::Value,
    ) -> String {
        serde_json::json!({
            "kind": "plan",
            "plan": {
                "outcome": outcome,
                "summary": summary,
                "clarification": clarification,
                "steps": steps,
            }
        })
        .to_string()
    }

    #[cfg(not(feature = "fake-model"))]
    #[test]
    fn default_build_ignores_fake_model_env() {
        let workspace = Workspace::new();
        let fixture = model_plan(
            "FAKE_MODEL_SENTINEL_SHOULD_NOT_APPEAR",
            "ready",
            "",
            serde_json::json!([{
                "action": "commit",
                "value": "this must not be committed",
                "paths": []
            }]),
        );
        let output = workspace.run(&fixture, &["--json", "commit everything"]);
        assert!(!output.status.success());
        assert!(
            output.stdout.is_empty(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("macOS 26"), "{stderr}");
        assert!(
            !stderr.contains("FAKE_MODEL_SENTINEL_SHOULD_NOT_APPEAR"),
            "{stderr}"
        );
        workspace.assert_only_inspected();
        workspace.assert_unchanged();
    }

    #[cfg(feature = "fake-model")]
    fn ready(action: &str, value: &str, paths: &[&str]) -> String {
        model_plan(
            "Requested Git operation",
            "ready",
            "",
            serde_json::json!([{ "action": action, "value": value, "paths": paths }]),
        )
    }

    #[cfg(feature = "fake-model")]
    fn displayed(args: &[&str]) -> String {
        let mut parts = vec!["git".to_string()];
        for arg in args {
            if !arg.is_empty()
                && arg
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_./=-".contains(c))
            {
                parts.push((*arg).to_string());
            } else {
                parts.push(format!("'{}'", arg.replace('\'', "'\\''")));
            }
        }
        parts.join(" ")
    }

    #[cfg(feature = "fake-model")]
    struct Expected {
        action: &'static str,
        value: &'static str,
        paths: &'static [&'static str],
        args: &'static [&'static str],
        explanation: &'static str,
        mutates: bool,
        /// `None` for an action `Plan::validate` classifies as benign, so
        /// `--json` omits `destructive` and `warning`. A destructive action
        /// sets this to the warning text validation attaches.
        warning: Option<&'static str>,
    }

    #[cfg(feature = "fake-model")]
    #[test]
    fn supported_operations_print_validated_plans_and_never_run_them() {
        let workspace = Workspace::new();
        let root = String::from_utf8(
            Command::new("git")
                .args(["rev-parse", "--show-toplevel"])
                .current_dir(&workspace.repo)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        let root = root.trim();
        let cases = [
            Expected {
                action: "status",
                value: "",
                paths: &[],
                args: &["status", "--short", "--branch"],
                explanation: "Shows short repository status and the current branch.",
                mutates: false,
                warning: None,
            },
            Expected {
                action: "diff",
                value: "",
                paths: &[],
                args: &["diff", "--no-ext-diff", "--no-textconv", "--"],
                explanation: "Shows unstaged changes to tracked files.",
                mutates: false,
                warning: None,
            },
            Expected {
                action: "stagedDiff",
                value: "",
                paths: &[],
                args: &["diff", "--staged", "--no-ext-diff", "--no-textconv", "--"],
                explanation: "Shows changes currently staged for commit.",
                mutates: false,
                warning: None,
            },
            Expected {
                action: "log",
                value: "",
                paths: &[],
                args: &["log", "--oneline", "--max-count=10", "--no-decorate"],
                explanation: "Shows the ten most recent commits.",
                mutates: false,
                warning: None,
            },
            Expected {
                action: "listBranches",
                value: "",
                paths: &[],
                args: &["branch", "--list"],
                explanation: "Lists local branches.",
                mutates: false,
                warning: None,
            },
            Expected {
                action: "createBranch",
                value: "feature/ratio-fix",
                paths: &[],
                args: &["switch", "-c", "feature/ratio-fix"],
                explanation: "Creates the named branch and switches to it.",
                mutates: true,
                warning: None,
            },
            Expected {
                action: "switchBranch",
                value: "main",
                paths: &[],
                args: &["switch", "main"],
                explanation: "Switches to the named existing branch.",
                mutates: true,
                warning: None,
            },
            Expected {
                action: "stage",
                value: "",
                paths: &["notes with spaces.txt"],
                args: &["--literal-pathspecs", "add", "--", "notes with spaces.txt"],
                explanation: "Stages changes to the named literal paths, including additions and deletions.",
                mutates: true,
                warning: None,
            },
            Expected {
                action: "stageAll",
                value: "",
                paths: &[],
                args: &["add", "--all"],
                explanation: "Stages all changes, including untracked files and deletions.",
                mutates: true,
                warning: None,
            },
            Expected {
                action: "unstage",
                value: "",
                paths: &["README.md"],
                args: &[
                    "--literal-pathspecs",
                    "restore",
                    "--staged",
                    "--",
                    "README.md",
                ],
                explanation: "Unstages the named literal paths while keeping working-tree changes.",
                mutates: true,
                warning: None,
            },
            Expected {
                action: "commit",
                value: "update docs",
                paths: &[],
                args: &["commit", "-m", "update docs"],
                explanation: "Commits only staged changes using the supplied message.",
                mutates: true,
                warning: None,
            },
            Expected {
                action: "commit",
                value: "fix 'login'; touch pwned",
                paths: &[],
                args: &["commit", "-m", "fix 'login'; touch pwned"],
                explanation: "Commits only staged changes using the supplied message.",
                mutates: true,
                warning: None,
            },
            Expected {
                action: "stash",
                value: "",
                paths: &[],
                args: &["stash", "push"],
                explanation: "Stashes tracked changes, leaving untracked files alone.",
                mutates: true,
                warning: None,
            },
            Expected {
                action: "push",
                value: "",
                paths: &[],
                args: &["push"],
                explanation: "Pushes using Git defaults and the configured upstream; contacts a remote.",
                mutates: true,
                warning: None,
            },
            Expected {
                action: "pull",
                value: "",
                paths: &[],
                args: &["pull", "--ff-only"],
                explanation: "Pulls from the configured upstream and updates by fast-forward only; contacts a remote.",
                mutates: true,
                warning: None,
            },
        ];

        for case in cases {
            let fixture = ready(case.action, case.value, case.paths);
            let request = format!("perform the canned plan {}", case.paths.join(" "));
            let text = workspace.run(&fixture, &[&request]);
            assert!(
                text.status.success(),
                "{:?} stderr {}",
                case.action,
                String::from_utf8_lossy(&text.stderr)
            );
            assert!(
                text.stderr.is_empty(),
                "{}",
                String::from_utf8_lossy(&text.stderr)
            );
            workspace.assert_only_inspected();
            workspace.assert_unchanged();
            let text = String::from_utf8(text.stdout).unwrap();
            let display = displayed(case.args);
            let mut expected = format!(
                "Requested Git operation\n\n{explanation}\n",
                explanation = case.explanation
            );
            if let Some(warning) = case.warning {
                expected.push_str(&format!("  Warning: {warning}\n"));
            }
            expected.push_str(&format!("  {display}\n"));
            expected.push_str(&format!(
                "\nReview these commands before running them in {root}.\n"
            ));
            if case.mutates {
                expected.push_str("This plan changes repository state or contacts a remote.\n");
            }
            assert_eq!(text, expected, "{}", case.action);
            assert_eq!(case.warning.is_some(), text.contains("Warning:"));

            let json_output = workspace.run(&fixture, &["--json", &request]);
            assert!(
                json_output.status.success(),
                "{}",
                String::from_utf8_lossy(&json_output.stderr)
            );
            assert!(json_output.stderr.is_empty());
            workspace.assert_only_inspected();
            workspace.assert_unchanged();
            let json: serde_json::Value = serde_json::from_slice(&json_output.stdout).unwrap();
            assert_eq!(json["outcome"], "ready", "{}", case.action);
            assert_eq!(json["summary"], "Requested Git operation");
            assert!(json["clarification"].is_null());
            assert_eq!(json["changes_repository"], case.mutates, "{}", case.action);
            let commands = json["commands"].as_array().unwrap();
            assert_eq!(commands.len(), 1);
            assert_eq!(commands[0]["program"], "git");
            assert_eq!(commands[0]["explanation"], case.explanation);
            assert_eq!(commands[0]["args"], serde_json::json!(case.args));
            match case.warning {
                None => {
                    assert!(commands[0].get("destructive").is_none(), "{}", case.action);
                    assert!(commands[0].get("warning").is_none(), "{}", case.action);
                }
                Some(warning) => {
                    assert_eq!(commands[0]["destructive"], true, "{}", case.action);
                    assert_eq!(commands[0]["warning"], warning, "{}", case.action);
                }
            }
        }
    }

    #[cfg(feature = "fake-model")]
    #[test]
    fn multistep_plan_keeps_order_in_text_and_json() {
        let workspace = Workspace::new();
        let fixture = model_plan(
            "Stage and commit",
            "ready",
            "",
            serde_json::json!([
                {"action": "stage", "value": "", "paths": ["README.md"]},
                {"action": "commit", "value": "Update documentation", "paths": []}
            ]),
        );
        let output = workspace.run(&fixture, &["--json", "stage README.md and commit"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        workspace.assert_only_inspected();
        workspace.assert_unchanged();
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            json["commands"][0]["args"],
            serde_json::json!(["--literal-pathspecs", "add", "--", "README.md"])
        );
        assert_eq!(
            json["commands"][1]["args"],
            serde_json::json!(["commit", "-m", "Update documentation"])
        );
        assert_eq!(json["changes_repository"], true);
        assert!(json["commands"][0].get("destructive").is_none());
        assert!(json["commands"][1].get("warning").is_none());
        let text = workspace.run(&fixture, &["stage README.md and commit"]);
        assert!(
            text.status.success(),
            "{}",
            String::from_utf8_lossy(&text.stderr)
        );
        let text = String::from_utf8(text.stdout).unwrap();
        let stage = text
            .find("Stages changes to the named literal paths")
            .unwrap();
        let commit = text.find("Commits only staged changes").unwrap();
        assert!(stage < commit, "{text}");
        assert!(text.contains("git --literal-pathspecs add -- README.md"));
        assert!(text.contains("git commit -m 'Update documentation'"));
        assert!(!text.contains("Warning:"));
        workspace.assert_only_inspected();
        workspace.assert_unchanged();
    }

    #[cfg(feature = "fake-model")]
    #[test]
    fn unrequested_discard_path_prints_only_clarification() {
        let workspace = Workspace::new();
        let fixture = model_plan(
            "Restore requested changes",
            "ready",
            "",
            serde_json::json!([{
                "action": "discard",
                "value": "",
                "paths": ["README.md", "src/main.rs"]
            }]),
        );
        let request = "revert my changes to README.md";
        let text = workspace.run(&fixture, &[request]);
        assert!(text.status.success());
        assert_eq!(
            String::from_utf8(text.stdout).unwrap(),
            "Clarification needed: Which exact file paths should be changed? Name each file explicitly.\n"
        );
        let json_output = workspace.run(&fixture, &["--json", request]);
        assert!(json_output.status.success());
        let json: serde_json::Value = serde_json::from_slice(&json_output.stdout).unwrap();
        assert_eq!(json["outcome"], "needsClarification");
        assert_eq!(json["commands"], serde_json::json!([]));
        assert_eq!(json["changes_repository"], false);
        workspace.assert_only_inspected();
        workspace.assert_unchanged();
    }

    #[cfg(feature = "fake-model")]
    #[test]
    fn clarification_and_unsupported_print_no_commands() {
        let workspace = Workspace::new();
        let clarification = model_plan(
            "Need a commit message",
            "needsClarification",
            "What commit message should I use?",
            serde_json::json!([]),
        );
        let text = workspace.run(&clarification, &["commit my changes"]);
        assert!(
            text.status.success(),
            "{}",
            String::from_utf8_lossy(&text.stderr)
        );
        assert!(text.stderr.is_empty());
        assert_eq!(
            String::from_utf8(text.stdout).unwrap(),
            "Clarification needed: What commit message should I use?\n"
        );
        workspace.assert_only_inspected();
        workspace.assert_unchanged();

        let json_output = workspace.run(&clarification, &["--json", "commit my changes"]);
        assert!(
            json_output.status.success(),
            "{}",
            String::from_utf8_lossy(&json_output.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&json_output.stdout).unwrap();
        assert_eq!(json["outcome"], "needsClarification");
        assert_eq!(json["clarification"], "What commit message should I use?");
        assert_eq!(json["commands"], serde_json::json!([]));
        assert_eq!(json["changes_repository"], false);
        workspace.assert_only_inspected();
        workspace.assert_unchanged();

        let unsupported = model_plan(
            "Force push is outside the supported operations",
            "unsupported",
            "Force push is not supported.",
            serde_json::json!([]),
        );
        let text = workspace.run(&unsupported, &["force push my branch"]);
        assert!(
            text.status.success(),
            "{}",
            String::from_utf8_lossy(&text.stderr)
        );
        assert_eq!(
            String::from_utf8(text.stdout).unwrap(),
            "Unsupported request: Force push is not supported.\n"
        );
        workspace.assert_only_inspected();
        workspace.assert_unchanged();
        let json_output = workspace.run(&unsupported, &["--json", "force push my branch"]);
        assert!(
            json_output.status.success(),
            "{}",
            String::from_utf8_lossy(&json_output.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&json_output.stdout).unwrap();
        assert_eq!(json["outcome"], "unsupported");
        assert_eq!(json["commands"], serde_json::json!([]));
        assert_eq!(json["changes_repository"], false);
        workspace.assert_only_inspected();
        workspace.assert_unchanged();
    }

    #[cfg(feature = "fake-model")]
    #[test]
    fn invalid_and_forbidden_model_actions_are_rejected() {
        let workspace = Workspace::new();
        let cases = [
            (
                model_plan(
                    "Reset the worktree",
                    "ready",
                    "",
                    serde_json::json!([{
                        "action": "reset",
                        "value": "--hard",
                        "paths": []
                    }]),
                ),
                "reset",
            ),
            (
                model_plan(
                    "Force push",
                    "ready",
                    "",
                    serde_json::json!([{
                        "action": "push",
                        "value": "--force",
                        "paths": []
                    }]),
                ),
                "invalid model proposal",
            ),
            (
                model_plan(
                    "Force push",
                    "ready",
                    "",
                    serde_json::json!([{
                        "action": "forcePush",
                        "value": "",
                        "paths": []
                    }]),
                ),
                "forcePush",
            ),
            (
                model_plan(
                    "Forge a destructive flag",
                    "ready",
                    "",
                    serde_json::json!([{
                        "action": "push",
                        "value": "",
                        "paths": [],
                        "destructive": true,
                        "warning": "This rewrites history."
                    }]),
                ),
                "destructive",
            ),
            (
                r#"{"kind":"plan","plan":{"outcome":"ready","summary":"Run reset","clarification":"","steps":[],"commands":[{"program":"git","args":["reset","--hard"]}]}}"#.to_string(),
                "commands",
            ),
            (
                model_plan(
                    "Escape the repository",
                    "ready",
                    "",
                    serde_json::json!([{
                        "action": "stage",
                        "value": "",
                        "paths": ["../secret"]
                    }]),
                ),
                "invalid model proposal",
            ),
            (
                model_plan(
                    "Hide a push inside a question",
                    "needsClarification",
                    "Which remote?",
                    serde_json::json!([{"action": "push", "value": "", "paths": []}]),
                ),
                "invalid model proposal",
            ),
        ];
        for (fixture, marker) in cases {
            let output = workspace.run(&fixture, &["--json", "run forbidden operation"]);
            assert!(
                !output.status.success(),
                "{marker} unexpectedly succeeded: {}",
                String::from_utf8_lossy(&output.stdout)
            );
            assert!(
                output.stdout.is_empty(),
                "{marker} printed {}",
                String::from_utf8_lossy(&output.stdout)
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains(marker), "{marker} not in {stderr}");
            assert!(!stderr.contains("git reset"), "{stderr}");
            workspace.assert_only_inspected();
            workspace.assert_unchanged();
        }
    }

    #[cfg(feature = "fake-model")]
    #[test]
    fn fixture_file_matches_inline_json_and_a_blank_hook_fails() {
        let workspace = Workspace::new();
        let inline = ready("status", "", &[]);
        let path = workspace.root.join("fixture.json");
        std::fs::write(&path, &inline).unwrap();
        let from_file = workspace.run(path.to_str().unwrap(), &["--json", "show status"]);
        assert!(
            from_file.status.success(),
            "{}",
            String::from_utf8_lossy(&from_file.stderr)
        );
        workspace.assert_only_inspected();
        let from_inline = workspace.run(&inline, &["--json", "show status"]);
        assert!(
            from_inline.status.success(),
            "{}",
            String::from_utf8_lossy(&from_inline.stderr)
        );
        assert_eq!(from_file.stdout, from_inline.stdout);
        workspace.assert_only_inspected();
        workspace.assert_unchanged();

        let blank = workspace.run("   ", &["show status"]);
        assert!(!blank.status.success());
        assert!(blank.stdout.is_empty());
        assert!(String::from_utf8_lossy(&blank.stderr).contains("empty"));
        workspace.assert_only_inspected();
        workspace.assert_unchanged();

        let missing = workspace.run(
            workspace.root.join("missing.json").to_str().unwrap(),
            &["show status"],
        );
        assert!(!missing.status.success());
        assert!(missing.stdout.is_empty());
        assert!(String::from_utf8_lossy(&missing.stderr).contains("could not read"));
        workspace.assert_unchanged();
    }

    #[cfg(feature = "fake-model")]
    #[test]
    fn control_characters_in_the_request_are_still_rejected() {
        let workspace = Workspace::new();
        let output = workspace.run(&ready("status", "", &[]), &["show\nstatus"]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("control characters"));
        assert!(!workspace.git_log.exists());
        workspace.assert_unchanged();
    }
}
