use super::{Outcome, Proposal, recovered_branch_name, validate_sha};
use std::{os::unix::fs::PermissionsExt, path::Path, process::Command};

fn recovery(name: &str, sha: &str) -> Proposal {
    serde_json::from_value(serde_json::json!({
        "outcome": "ready",
        "summary": "Recover the lost commit",
        "clarification": "",
        "steps": [{
            "action": "createBranchAt",
            "value": name,
            "paths": [],
            "sha": sha
        }]
    }))
    .unwrap()
}

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn git_text(dir: &Path, args: &[&str]) -> String {
    String::from_utf8(git(dir, args).stdout)
        .unwrap()
        .trim()
        .to_owned()
}

/// `git cat-file -e <sha>^{commit}` — the same read-only check the validator uses.
fn cat_file_commit(dir: &Path, sha: &str) -> bool {
    Command::new("git")
        .args([
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "cat-file",
            "-e",
            &format!("{sha}^{{commit}}"),
        ])
        .current_dir(dir)
        .output()
        .unwrap()
        .status
        .success()
}

struct Repo {
    dir: tempfile::TempDir,
    head: String,
    branch: String,
    refs: String,
}

impl Repo {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let hooks = dir.path().join("empty-hooks");
        std::fs::create_dir(&hooks).unwrap();
        git(
            dir.path(),
            &["init", "--object-format=sha1", "--initial-branch=main"],
        );
        git(
            dir.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        git(dir.path(), &["config", "user.name", "Githelper Test"]);
        git(dir.path(), &["config", "commit.gpgsign", "false"]);
        git(dir.path(), &["config", "tag.gpgsign", "false"]);
        git(
            dir.path(),
            &["config", "core.hooksPath", hooks.to_str().unwrap()],
        );
        std::fs::write(dir.path().join("README.md"), "hello\n").unwrap();
        git(dir.path(), &["add", "--", "README.md"]);
        git(dir.path(), &["commit", "-m", "Initial"]);
        Self {
            head: git_text(dir.path(), &["rev-parse", "HEAD"]),
            branch: git_text(dir.path(), &["branch", "--show-current"]),
            refs: git_text(dir.path(), &["for-each-ref", "refs/heads"]),
            dir,
        }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn arm_execution_trap(&self) {
        let hooks = self.path().join("execution-trap");
        std::fs::create_dir(&hooks).unwrap();
        let marker = self.path().join("EXECUTED");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$0 $*\" >> {}\n",
            marker.display()
        );
        for name in [
            "reference-transaction",
            "update",
            "post-checkout",
            "post-merge",
            "post-commit",
            "pre-commit",
            "pre-rebase",
            "post-rewrite",
            "post-reset",
            "fsmonitor-watchman",
        ] {
            let path = hooks.join(name);
            std::fs::write(&path, &script).unwrap();
            let mut permissions = std::fs::metadata(&path).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&path, permissions).unwrap();
        }
        git(
            self.path(),
            &["config", "core.hooksPath", hooks.to_str().unwrap()],
        );
    }

    fn assert_untouched(&self) {
        assert_eq!(git_text(self.path(), &["rev-parse", "HEAD"]), self.head);
        assert_eq!(
            git_text(self.path(), &["branch", "--show-current"]),
            self.branch
        );
        assert_eq!(
            git_text(self.path(), &["for-each-ref", "refs/heads"]),
            self.refs
        );
        assert_eq!(self.branch, "main");
        let marker = self.path().join("EXECUTED");
        assert!(
            !marker.exists(),
            "validation executed git:\n{}",
            std::fs::read_to_string(&marker).unwrap_or_default()
        );
    }
}

fn assert_format_rejected(sha: &str) {
    let error = validate_sha(sha).expect_err(sha);
    assert!(
        error.to_string().contains("7 to 40 hexadecimal"),
        "{sha}: {error}"
    );
    let missing = Path::new("/no/such/githelper-recovery-repo");
    let error = recovery("login", sha).validate(missing).expect_err(sha);
    assert!(
        error.to_string().contains("7 to 40 hexadecimal"),
        "{sha}: {error} (git must not be required to reject a bad SHA)"
    );
    assert!(
        !error.to_string().contains("could not verify"),
        "{sha}: {error}"
    );
}

fn assert_recovery_is_only_a_branch(plan: &super::Plan, branch: &str, sha: &str) {
    assert_eq!(plan.outcome, Outcome::Ready);
    assert!(plan.clarification.is_none());
    assert_eq!(plan.commands.len(), 1);
    let command = &plan.commands[0];
    assert_eq!(command.program, "git");
    assert_eq!(
        command.args,
        vec!["branch".to_owned(), branch.to_owned(), sha.to_owned()]
    );
    assert_eq!(command.display(), format!("git branch {branch} {sha}"));
    assert!(!command.destructive);
    assert!(command.warning.is_none());
    assert!(command.explanation.contains("without moving"));
    for arg in &command.args {
        for forbidden in [
            "reset", "checkout", "switch", "rebase", "clean", "restore", "--hard", "-D", "--force",
        ] {
            assert_ne!(arg, forbidden, "recovery command contained {forbidden}");
        }
    }
    let json = serde_json::to_value(command).unwrap();
    assert!(json.get("destructive").is_none());
    assert!(json.get("warning").is_none());
    assert!(!json.to_string().contains("\"reset\""));
}

fn assert_clarification(plan: &super::Plan, sha: &str) {
    assert_eq!(plan.outcome, Outcome::NeedsClarification);
    assert!(plan.commands.is_empty());
    assert!(!plan.changes_repository);
    let expected = format!(
        "Commit {sha} does not resolve to an existing commit. Which commit SHA should be recovered? Use `git reflog -n 20` to find it."
    );
    assert_eq!(plan.clarification.as_deref(), Some(expected.as_str()));
    assert!(!expected.contains("reset"));
}

#[test]
fn sha_length_and_alphabet_boundaries() {
    for sha in ["", "a", "abcd", "aaaaaa", &"a".repeat(41), &"A".repeat(41)] {
        assert_format_rejected(sha);
    }
    for sha in [
        "aaaaaaa",
        &"a".repeat(8),
        &"a".repeat(39),
        &"a".repeat(40),
        "AAAAAAA",
        &"A".repeat(40),
        "aAbBcCd",
        "0123456789abcdefABCDEF",
    ] {
        assert!(validate_sha(sha).is_ok(), "{sha}");
    }
    for sha in [
        "abc123g",
        "zzzzzzz",
        "abc123!",
        "abc1234\n",
        " abc1234",
        "abc1234 ",
        "abc1234\t",
        "0xabc123",
        "abcdefG",
        "ggggggg",
        &format!("{}g", "a".repeat(39)),
        &format!("g{}", "a".repeat(39)),
        "abc12é",
    ] {
        assert_format_rejected(sha);
    }
}

#[test]
fn six_is_rejected_and_seven_and_forty_are_accepted_for_a_real_commit() {
    let repo = Repo::new();
    let sha = repo.head.clone();
    assert_eq!(sha.len(), 40);
    assert!(
        cat_file_commit(repo.path(), &sha[..6]),
        "fixture prefix should exist"
    );
    assert_format_rejected(&sha[..6]);

    let forty_one = format!("{sha}a");
    assert_eq!(forty_one.len(), 41);
    assert!(!cat_file_commit(repo.path(), &forty_one));
    assert_format_rejected(&forty_one);
    repo.arm_execution_trap();
    for bad in [sha[..6].to_owned(), forty_one] {
        let error = recovery("login", &bad).validate(repo.path()).unwrap_err();
        assert!(
            error.to_string().contains("7 to 40 hexadecimal"),
            "{bad}: {error}"
        );
    }

    for length in [7_usize, 8, 39, 40] {
        let prefix = &sha[..length];
        assert!(cat_file_commit(repo.path(), prefix), "{prefix}");
        let plan = recovery("login", prefix).validate(repo.path()).unwrap();
        assert_recovery_is_only_a_branch(&plan, "recovered/login", prefix);
    }
    repo.assert_untouched();
}

#[test]
fn mixed_case_sha_is_preserved_and_selects_the_same_commit() {
    let repo = Repo::new();
    let sha = repo.head.clone();
    let mixed: String = sha
        .chars()
        .enumerate()
        .map(|(index, character)| {
            if index % 2 == 0 {
                character.to_ascii_uppercase()
            } else {
                character.to_ascii_lowercase()
            }
        })
        .collect();
    assert_ne!(mixed, sha);
    assert!(validate_sha(&mixed).is_ok());
    assert!(cat_file_commit(repo.path(), &mixed));
    assert_eq!(
        git_text(repo.path(), &["rev-parse", &format!("{mixed}^{{commit}}")]),
        sha
    );
    repo.arm_execution_trap();

    let plan = recovery("login", &mixed).validate(repo.path()).unwrap();
    assert_recovery_is_only_a_branch(&plan, "recovered/login", &mixed);

    let unnamed = recovery("", &mixed).validate(repo.path()).unwrap();
    assert_recovery_is_only_a_branch(&unnamed, &format!("recovered/{mixed}"), &mixed);

    let missing = "AbC1234";
    assert!(!cat_file_commit(repo.path(), missing));
    let plan = recovery("login", missing).validate(repo.path()).unwrap();
    assert_clarification(&plan, missing);
    assert!(plan.clarification.as_deref().unwrap().contains("AbC1234"));
    repo.assert_untouched();
}

#[test]
fn nonexistent_and_non_commit_objects_clarify_instead_of_planning() {
    let repo = Repo::new();
    let sha = repo.head.clone();
    let mut missing = sha.clone();
    missing.replace_range(0..1, if sha.starts_with('a') { "b" } else { "a" });
    assert_eq!(missing.len(), 40);
    assert_ne!(missing, sha);
    assert!(!cat_file_commit(repo.path(), &missing));
    assert!(!cat_file_commit(repo.path(), "abc1234"));
    let tree = git_text(repo.path(), &["rev-parse", "HEAD^{tree}"]);
    let blob = git_text(repo.path(), &["rev-parse", "HEAD:README.md"]);
    assert!(!cat_file_commit(repo.path(), &tree));
    assert!(!cat_file_commit(repo.path(), &blob));
    git(
        repo.path(),
        &["tag", "-a", "v-commit", "-m", "commit tag", &sha],
    );
    git(
        repo.path(),
        &["tag", "-a", "v-tree", "-m", "tree tag", &tree],
    );
    let tag = git_text(repo.path(), &["rev-parse", "v-commit"]);
    let tag_tree = git_text(repo.path(), &["rev-parse", "v-tree"]);
    assert_ne!(tag, sha);
    assert!(cat_file_commit(repo.path(), &tag));
    assert!(!cat_file_commit(repo.path(), &tag_tree));
    repo.arm_execution_trap();

    assert_clarification(
        &recovery("login", &missing).validate(repo.path()).unwrap(),
        &missing,
    );
    assert_clarification(
        &recovery("login", "abc1234").validate(repo.path()).unwrap(),
        "abc1234",
    );
    assert_clarification(
        &recovery("login", &tree).validate(repo.path()).unwrap(),
        &tree,
    );
    assert_clarification(
        &recovery("login", &blob).validate(repo.path()).unwrap(),
        &blob,
    );
    let peeled = recovery("from-tag", &tag).validate(repo.path()).unwrap();
    assert_recovery_is_only_a_branch(&peeled, "recovered/from-tag", &tag);
    assert_clarification(
        &recovery("from-tree-tag", &tag_tree)
            .validate(repo.path())
            .unwrap(),
        &tag_tree,
    );

    let mut with_reflog = recovery("login", "abc1234");
    with_reflog.steps.insert(
        0,
        serde_json::from_value(serde_json::json!({
            "action": "showReflog",
            "value": "",
            "paths": []
        }))
        .unwrap(),
    );
    let plan = with_reflog.validate(repo.path()).unwrap();
    assert_clarification(&plan, "abc1234");
    repo.assert_untouched();
}

#[test]
fn invalid_branch_names_are_rejected_and_valid_ones_stay_under_recovered() {
    let repo = Repo::new();
    let sha = repo.head.clone();
    repo.arm_execution_trap();
    let rejected = [
        "recovered/../main",
        "../main",
        "foo/../../main",
        "recovered/foo/../../main",
        "foo/../main",
        "./main",
        "recovered/./main",
        "/main",
        "recovered//main",
        "foo//bar",
        "-lost",
        "--force",
        "--hard",
        "-D",
        "-",
        "recovered/-lost",
        "recovered/--force",
        "recovered/foo/-bar",
        "foo/--delete",
        "recovered/",
        "recovered/HEAD",
        "HEAD",
        "recovered/foo.lock",
        "foo.lock",
        "recovered/.hidden",
        ".hidden",
        "recovered/foo.",
        "foo.",
        "recovered/foo..bar",
        "a..b",
        "recovered/@{1}",
        "@{-1}",
        "recovered/foo~1",
        "foo~1",
        "recovered/foo bar",
        "foo bar",
        "recovered/foo*",
        "*",
        "recovered/foo?bar",
        "recovered/foo^2",
        "recovered/foo:bar",
        "recovered/foo[bar]",
        "recovered/foo\\bar",
        "recovered/foo/.git",
        "recovered/foo\nbar",
        "recovered/@",
    ];
    for name in rejected {
        assert!(
            recovered_branch_name(name, &sha).is_err(),
            "branch helper accepted {name}"
        );
        let error = recovery(name, &sha)
            .validate(Path::new("/no/such/githelper-recovery-repo"))
            .expect_err(name);
        assert!(
            error
                .to_string()
                .contains("invalid or unsupported branch name"),
            "{name}: {error}"
        );
        assert!(
            !error.to_string().contains("could not verify"),
            "{name}: validation executed git"
        );
        assert!(
            recovery(name, &sha).validate(repo.path()).is_err(),
            "{name}"
        );
    }

    let accepted = [
        ("login", "recovered/login".to_string()),
        ("recovered/login", "recovered/login".to_string()),
        ("feature/lost", "recovered/feature/lost".to_string()),
        ("fix/login-2", "recovered/fix/login-2".to_string()),
        ("Fix_1.2", "recovered/Fix_1.2".to_string()),
        ("", format!("recovered/{sha}")),
        ("refs/heads/main", "recovered/refs/heads/main".to_string()),
        ("reset", "recovered/reset".to_string()),
    ];
    for (name, branch) in accepted {
        assert_eq!(recovered_branch_name(name, &sha).unwrap(), branch);
        let plan = recovery(name, &sha).validate(repo.path()).unwrap();
        assert_recovery_is_only_a_branch(&plan, &branch, &sha);
        assert!(branch.starts_with("recovered/"));
        assert_ne!(branch, "main");
        assert_ne!(branch, "HEAD");
        let full = format!("refs/heads/{branch}");
        assert!(
            Command::new("git")
                .args(["check-ref-format", &full])
                .output()
                .unwrap()
                .status
                .success(),
            "{full}"
        );
    }
    repo.assert_untouched();
}

#[test]
fn no_sha_does_not_create_a_branch() {
    let repo = Repo::new();
    repo.arm_execution_trap();
    let reflog = serde_json::from_value::<Proposal>(serde_json::json!({
        "outcome": "ready",
        "summary": "Recover the lost commit without moving the current branch.",
        "clarification": "",
        "steps": [{"action": "showReflog", "value": "", "paths": []}]
    }))
    .unwrap()
    .validate(repo.path())
    .unwrap();
    assert_eq!(reflog.outcome, Outcome::Ready);
    assert!(reflog.clarification.is_none());
    assert!(!reflog.changes_repository);
    assert_eq!(reflog.commands.len(), 1);
    assert_eq!(
        reflog.commands[0].args,
        super::strings(&["reflog", "-n", "20"])
    );
    assert_eq!(reflog.commands[0].display(), "git reflog -n 20");
    assert!(reflog.commands[0].explanation.contains("note its SHA"));
    assert!(!reflog.commands[0].destructive);
    assert!(reflog.commands[0].warning.is_none());
    for arg in &reflog.commands[0].args {
        assert_ne!(arg, "reset");
        assert_ne!(arg, "branch");
        assert_ne!(arg, "switch");
        assert_ne!(arg, "checkout");
    }

    for sha in ["", "   "] {
        assert_format_rejected(sha);
    }
    let omitted = serde_json::from_value::<Proposal>(serde_json::json!({
        "outcome": "ready",
        "summary": "Recover the lost commit",
        "clarification": "",
        "steps": [{"action": "createBranchAt", "value": "login", "paths": []}]
    }))
    .unwrap();
    assert!(omitted.steps[0].sha.is_empty());
    let error = omitted.validate(repo.path()).unwrap_err();
    assert!(error.to_string().contains("7 to 40 hexadecimal"), "{error}");

    let asked = serde_json::from_value::<Proposal>(serde_json::json!({
        "outcome": "needsClarification",
        "summary": "Which commit should be recovered?",
        "clarification": "Which commit SHA should be recovered?",
        "steps": []
    }))
    .unwrap()
    .validate(repo.path())
    .unwrap();
    assert_eq!(asked.outcome, Outcome::NeedsClarification);
    assert!(asked.commands.is_empty());
    assert!(!asked.changes_repository);
    repo.assert_untouched();
}

#[test]
fn recovery_never_emits_reset_or_runs_the_plan() {
    let repo = Repo::new();
    let sha = repo.head.clone();
    repo.arm_execution_trap();
    for (name, sha) in [
        ("login", sha.as_str()),
        ("recovered/login", sha.as_str()),
        ("", sha.as_str()),
        ("reset", sha.as_str()),
    ] {
        let plan = recovery(name, sha).validate(repo.path()).unwrap();
        assert!(!plan.commands.is_empty());
        assert!(plan.changes_repository);
        for command in &plan.commands {
            assert_eq!(command.args[0], "branch");
            assert!(!command.destructive);
            assert!(command.warning.is_none());
            assert!(command.args.iter().all(|arg| arg != "reset"));
            assert!(command.display().starts_with("git branch "));
            assert!(!command.display().contains("checkout"));
            assert!(!command.display().contains("switch"));
        }
    }

    for action in [
        "reset", "checkout", "rebase", "clean", "restore", "switch", "branch",
    ] {
        let json = format!(r#"{{"action":"{action}","value":"--hard","paths":[]}}"#);
        assert!(
            serde_json::from_str::<super::Step>(&json).is_err(),
            "{action}"
        );
    }

    let closed = serde_json::from_value::<Proposal>(serde_json::json!({
        "outcome": "unsupported",
        "summary": "Reset is not a recovery plan",
        "clarification": "Recovering a commit does not reset the current branch.",
        "steps": []
    }))
    .unwrap()
    .validate(repo.path())
    .unwrap();
    assert_eq!(closed.outcome, Outcome::Unsupported);
    assert!(closed.commands.is_empty());
    assert!(!closed.changes_repository);
    repo.assert_untouched();
}
