use serde::{Deserialize, Serialize};
use std::{path::Path, process::Command};
use thiserror::Error;

/// Closed set of operations the model may select.
///
/// `CreateBranchAt` carries the recovered branch name and commit SHA. Other
/// operations keep their parameters on [`Step`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Status,
    Diff,
    StagedDiff,
    Log,
    ListBranches,
    CreateBranch,
    SwitchBranch,
    Stage,
    StageAll,
    Unstage,
    Commit,
    Stash,
    Push,
    Pull,
    /// Read-only `git reflog -n 20`.
    ShowReflog,
    /// `git branch recovered/<name> <sha>`, which does not move the current branch.
    CreateBranchAt {
        name: String,
        sha: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    Ready,
    NeedsClarification,
    Unsupported,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(from = "WireStep", into = "WireStep")]
pub struct Step {
    pub action: Action,
    pub value: String,
    pub paths: Vec<String>,
    /// Set only when a SHA was attached to an action that does not accept one.
    pub sha: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireStep {
    action: WireAction,
    value: String,
    paths: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    sha: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum WireAction {
    Status,
    Diff,
    StagedDiff,
    Log,
    ListBranches,
    CreateBranch,
    SwitchBranch,
    Stage,
    StageAll,
    Unstage,
    Commit,
    Stash,
    Push,
    Pull,
    ShowReflog,
    CreateBranchAt,
}

fn step(action: Action, value: String, paths: Vec<String>, sha: String) -> Step {
    Step {
        action,
        value,
        paths,
        sha,
    }
}

impl From<WireStep> for Step {
    fn from(wire: WireStep) -> Self {
        let WireStep {
            action,
            value,
            paths,
            sha,
        } = wire;
        match action {
            WireAction::CreateBranchAt => step(
                Action::CreateBranchAt { name: value, sha },
                String::new(),
                paths,
                String::new(),
            ),
            WireAction::Status => step(Action::Status, value, paths, sha),
            WireAction::Diff => step(Action::Diff, value, paths, sha),
            WireAction::StagedDiff => step(Action::StagedDiff, value, paths, sha),
            WireAction::Log => step(Action::Log, value, paths, sha),
            WireAction::ListBranches => step(Action::ListBranches, value, paths, sha),
            WireAction::CreateBranch => step(Action::CreateBranch, value, paths, sha),
            WireAction::SwitchBranch => step(Action::SwitchBranch, value, paths, sha),
            WireAction::Stage => step(Action::Stage, value, paths, sha),
            WireAction::StageAll => step(Action::StageAll, value, paths, sha),
            WireAction::Unstage => step(Action::Unstage, value, paths, sha),
            WireAction::Commit => step(Action::Commit, value, paths, sha),
            WireAction::Stash => step(Action::Stash, value, paths, sha),
            WireAction::Push => step(Action::Push, value, paths, sha),
            WireAction::Pull => step(Action::Pull, value, paths, sha),
            WireAction::ShowReflog => step(Action::ShowReflog, value, paths, sha),
        }
    }
}

impl From<Step> for WireStep {
    fn from(step: Step) -> Self {
        match step.action {
            Action::CreateBranchAt { name, sha } => WireStep {
                action: WireAction::CreateBranchAt,
                value: name,
                paths: step.paths,
                sha,
            },
            action => WireStep {
                action: WireAction::from(&action),
                value: step.value,
                paths: step.paths,
                sha: step.sha,
            },
        }
    }
}

impl From<&Action> for WireAction {
    fn from(action: &Action) -> Self {
        match action {
            Action::Status => WireAction::Status,
            Action::Diff => WireAction::Diff,
            Action::StagedDiff => WireAction::StagedDiff,
            Action::Log => WireAction::Log,
            Action::ListBranches => WireAction::ListBranches,
            Action::CreateBranch => WireAction::CreateBranch,
            Action::SwitchBranch => WireAction::SwitchBranch,
            Action::Stage => WireAction::Stage,
            Action::StageAll => WireAction::StageAll,
            Action::Unstage => WireAction::Unstage,
            Action::Commit => WireAction::Commit,
            Action::Stash => WireAction::Stash,
            Action::Push => WireAction::Push,
            Action::Pull => WireAction::Pull,
            Action::ShowReflog => WireAction::ShowReflog,
            Action::CreateBranchAt { .. } => WireAction::CreateBranchAt,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub outcome: Outcome,
    pub summary: String,
    pub clarification: String,
    pub steps: Vec<Step>,
}

#[derive(Debug, Serialize)]
pub struct Plan {
    pub outcome: Outcome,
    pub summary: String,
    pub clarification: Option<String>,
    pub commands: Vec<GitCommand>,
    pub changes_repository: bool,
}

#[derive(Debug, Serialize)]
pub struct GitCommand {
    pub program: &'static str,
    pub explanation: String,
    pub args: Vec<String>,
    /// Present in `--json` only when this step can discard work or rewrite history.
    #[serde(skip_serializing_if = "is_false")]
    pub destructive: bool,
    /// Warning text for a destructive step. Omitted for every current action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<&'static str>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Error)]
#[error("invalid model proposal: {0}")]
pub struct InvalidPlan(String);

fn invalid(message: &str) -> InvalidPlan {
    InvalidPlan(message.into())
}

fn text(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}

impl Proposal {
    pub fn validate(self, repository: &Path) -> Result<Plan, InvalidPlan> {
        if !text(&self.summary, 1_000) {
            return Err(invalid("missing or invalid summary"));
        }
        if self.outcome != Outcome::Ready {
            if !text(&self.clarification, 1_000) || !self.steps.is_empty() {
                return Err(invalid("clarification must have no commands"));
            }
            return Ok(Plan {
                outcome: self.outcome,
                summary: self.summary,
                clarification: Some(self.clarification),
                commands: vec![],
                changes_repository: false,
            });
        }
        if !self.clarification.is_empty() {
            return Err(invalid("ready plans must not request clarification"));
        }
        if self.steps.is_empty() || self.steps.len() > 6 {
            return Err(invalid("expected one to six steps"));
        }
        let mut commands = Vec::new();
        let mut changes_repository = false;
        for step in self.steps {
            use Action::*;
            let needs_paths = matches!(&step.action, Stage | Unstage);
            let needs_value = matches!(&step.action, CreateBranch | SwitchBranch | Commit);
            let read_only = matches!(
                &step.action,
                Status | Diff | StagedDiff | Log | ListBranches | ShowReflog
            );
            if !needs_paths && !step.paths.is_empty() {
                return Err(invalid("unexpected paths for this action"));
            }
            if !needs_value && !step.value.is_empty() {
                return Err(invalid("unexpected value for this action"));
            }
            if !step.sha.is_empty() {
                return Err(invalid("unexpected sha for this action"));
            }
            if needs_paths {
                if step.paths.is_empty() || step.paths.len() > 32 {
                    return Err(invalid("expected one to 32 explicit paths"));
                }
                for path in &step.paths {
                    if !text(path, 1_024)
                        || path.starts_with('/')
                        || path.split('/').any(|part| part == "..")
                    {
                        return Err(invalid("paths must stay inside the repository"));
                    }
                }
            }
            let destructive = step.action.destructive();
            let warning = step.action.warning();
            let explanation = step.action.explanation();
            debug_assert_eq!(destructive, warning.is_some());
            let args: Vec<String> = match step.action {
                Status => strings(&["status", "--short", "--branch"]),
                Diff => strings(&["diff", "--no-ext-diff", "--no-textconv", "--"]),
                StagedDiff => {
                    strings(&["diff", "--staged", "--no-ext-diff", "--no-textconv", "--"])
                }
                Log => strings(&["log", "--oneline", "--max-count=10", "--no-decorate"]),
                ListBranches => strings(&["branch", "--list"]),
                ShowReflog => strings(&["reflog", "-n", "20"]),
                CreateBranch => {
                    validate_branch(&step.value)?;
                    let mut args = strings(&["switch", "-c"]);
                    args.push(step.value);
                    args
                }
                SwitchBranch => {
                    validate_branch(&step.value)?;
                    let mut args = strings(&["switch"]);
                    args.push(step.value);
                    args
                }
                CreateBranchAt { name, sha } => {
                    validate_sha(&sha)?;
                    let branch = recovered_branch_name(&name, &sha)?;
                    if !commit_object_exists(repository, &sha)? {
                        return Ok(Plan {
                            outcome: Outcome::NeedsClarification,
                            summary: self.summary.clone(),
                            clarification: Some(format!(
                                "Commit {sha} does not resolve to an existing commit. Which commit SHA should be recovered? Use `git reflog -n 20` to find it."
                            )),
                            commands: vec![],
                            changes_repository: false,
                        });
                    }
                    vec!["branch".into(), branch, sha]
                }
                Stage => {
                    let mut args = strings(&["--literal-pathspecs", "add", "--"]);
                    args.extend(step.paths);
                    args
                }
                Unstage => {
                    let mut args = strings(&["--literal-pathspecs", "restore", "--staged", "--"]);
                    args.extend(step.paths);
                    args
                }
                StageAll => strings(&["add", "--all"]),
                Commit => {
                    if !text(&step.value, 1_000) {
                        return Err(invalid("commit requires an explicit message"));
                    }
                    vec!["commit".into(), "-m".into(), step.value]
                }
                Stash => strings(&["stash", "push"]),
                Push => strings(&["push"]),
                Pull => strings(&["pull", "--ff-only"]),
            };
            changes_repository |= !read_only;
            commands.push(GitCommand {
                program: "git",
                explanation: explanation.into(),
                args,
                destructive,
                warning,
            });
        }
        Ok(Plan {
            outcome: self.outcome,
            summary: self.summary,
            clarification: None,
            commands,
            changes_repository,
        })
    }
}

/// Loss classification for one action.
///
/// `Action::safety` is the only match that decides this. Undo, discard, merge,
/// and rebase should return `Destructive` with the warning a person reads
/// before running the command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Safety {
    Benign,
    /// Undo, discard, merge, and rebase plug in as this variant.
    #[expect(dead_code, reason = "no destructive action is implemented yet")]
    Destructive(&'static str),
}

impl Action {
    /// Whether this action can discard uncommitted work or rewrite history.
    ///
    /// Repository edits and ordinary remote sync are not destructive. Upcoming
    /// undo, discard, merge, and rebase actions should return true here.
    pub fn destructive(&self) -> bool {
        matches!(self.safety(), Safety::Destructive(_))
    }

    /// Warning copied onto a destructive step, in text output and `--json`.
    pub fn warning(&self) -> Option<&'static str> {
        match self.safety() {
            Safety::Benign => None,
            Safety::Destructive(warning) => {
                debug_assert!(
                    !warning.is_empty() && !warning.chars().any(char::is_control),
                    "destructive actions need a single-line warning"
                );
                Some(warning)
            }
        }
    }

    fn safety(&self) -> Safety {
        match self {
            // Read-only inspection, local edits, and ordinary remote sync do not
            // discard work. Recovering a commit creates a new branch and does
            // not move HEAD. Classify undo, discard, merge, and rebase as
            // `Safety::Destructive("...")` in this match.
            Self::Status
            | Self::Diff
            | Self::StagedDiff
            | Self::Log
            | Self::ListBranches
            | Self::ShowReflog
            | Self::CreateBranch
            | Self::CreateBranchAt { .. }
            | Self::SwitchBranch
            | Self::Stage
            | Self::StageAll
            | Self::Unstage
            | Self::Commit
            | Self::Stash
            | Self::Push
            | Self::Pull => Safety::Benign,
        }
    }

    fn explanation(&self) -> &'static str {
        match self {
            Self::Status => "Shows short repository status and the current branch.",
            Self::Diff => "Shows unstaged changes to tracked files.",
            Self::StagedDiff => "Shows changes currently staged for commit.",
            Self::Log => "Shows the ten most recent commits.",
            Self::ListBranches => "Lists local branches.",
            Self::ShowReflog => {
                "Shows the 20 most recent reflog entries. Find the lost commit or deleted branch in that list and note its SHA, then ask to recover that SHA."
            }
            Self::CreateBranchAt { .. } => {
                "Creates a branch at the recovered commit without moving the current branch."
            }
            Self::CreateBranch => "Creates the named branch and switches to it.",
            Self::SwitchBranch => "Switches to the named existing branch.",
            Self::Stage => {
                "Stages changes to the named literal paths, including additions and deletions."
            }
            Self::StageAll => "Stages all changes, including untracked files and deletions.",
            Self::Unstage => "Unstages the named literal paths while keeping working-tree changes.",
            Self::Commit => "Commits only staged changes using the supplied message.",
            Self::Stash => "Stashes tracked changes, leaving untracked files alone.",
            Self::Push => {
                "Pushes using Git defaults and the configured upstream; contacts a remote."
            }
            Self::Pull => {
                "Pulls from the configured upstream and updates by fast-forward only; contacts a remote."
            }
        }
    }
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_owned()).collect()
}

fn validate_branch(branch: &str) -> Result<(), InvalidPlan> {
    // A deliberately conservative subset of git-check-ref-format's branch syntax.
    if !text(branch, 200)
        || branch.starts_with(['-', '/', '.'])
        || branch.ends_with(['/', '.'])
        || branch == "HEAD"
        || branch.contains("..")
        || branch.contains("@{")
        || branch
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || "/-_.".contains(c)))
        || branch
            .split('/')
            .any(|part| part.is_empty() || part.starts_with('.') || part.ends_with(".lock"))
    {
        return Err(invalid("invalid or unsupported branch name"));
    }
    Ok(())
}

/// Accept 7–40 hexadecimal characters. Git object names are case-insensitive,
/// so mixed case is preserved and passed through to the printed command.
fn validate_sha(sha: &str) -> Result<(), InvalidPlan> {
    if !(7..=40).contains(&sha.len()) || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(invalid("commit SHA must be 7 to 40 hexadecimal characters"));
    }
    Ok(())
}

/// Force the printed branch under `recovered/` without moving the current branch.
fn recovered_branch_name(name: &str, sha: &str) -> Result<String, InvalidPlan> {
    let branch = if name.is_empty() {
        format!("recovered/{sha}")
    } else if name.starts_with("recovered/") {
        name.to_owned()
    } else {
        format!("recovered/{name}")
    };
    validate_branch(&branch)?;
    let suffix = branch
        .strip_prefix("recovered/")
        .filter(|suffix| !suffix.is_empty())
        .ok_or_else(|| invalid("invalid or unsupported branch name"))?;
    validate_branch(suffix)?;
    if branch.split('/').any(|part| part.starts_with('-')) {
        return Err(invalid("invalid or unsupported branch name"));
    }
    Ok(branch)
}

/// Read-only existence check. A non-zero status means the name is missing,
/// ambiguous, or not a commit. This never runs a generated plan.
fn commit_object_exists(repository: &Path, sha: &str) -> Result<bool, InvalidPlan> {
    let spec = format!("{sha}^{{commit}}");
    let output = Command::new("git")
        .args([
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "cat-file",
            "-e",
            spec.as_str(),
        ])
        .current_dir(repository)
        .output()
        .map_err(|_| invalid("could not verify the commit SHA"))?;
    Ok(output.status.success())
}

impl GitCommand {
    pub fn display(&self) -> String {
        std::iter::once(self.program.to_string())
            .chain(self.args.iter().map(|arg| shell_quote(arg)))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_./=-".contains(c))
    {
        value.into()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn proposal(action: Action, value: &str, paths: &[&str]) -> Proposal {
        Proposal {
            outcome: Outcome::Ready,
            summary: "Requested Git operation".into(),
            clarification: String::new(),
            steps: vec![Step {
                action,
                value: value.into(),
                paths: strings(paths),
                sha: String::new(),
            }],
        }
    }

    #[test]
    fn every_action_has_a_scope_specific_explanation_and_preserves_order() {
        use Action::*;
        let cases = [
            (Status, "", &[][..], "current branch", false),
            (Diff, "", &[][..], "unstaged", false),
            (StagedDiff, "", &[][..], "staged for commit", false),
            (Log, "", &[][..], "ten most recent", false),
            (ListBranches, "", &[][..], "local branches", false),
            (ShowReflog, "", &[][..], "note its SHA", false),
            (CreateBranch, "feature/test", &[][..], "and switches", true),
            (SwitchBranch, "main", &[][..], "existing branch", true),
            (Stage, "", &["README.md"][..], "literal paths", true),
            (StageAll, "", &[][..], "untracked files and deletions", true),
            (
                Unstage,
                "",
                &["README.md"][..],
                "keeping working-tree",
                true,
            ),
            (Commit, "update docs", &[][..], "only staged", true),
            (Stash, "", &[][..], "leaving untracked", true),
            (Push, "", &[][..], "Git defaults", true),
            (Pull, "", &[][..], "fast-forward only", true),
        ];
        for (action, value, paths, scope, mutates) in cases {
            let plan = proposal(action.clone(), value, paths)
                .validate(Path::new("."))
                .unwrap();
            assert!(plan.commands[0].explanation.contains(scope), "{action:?}");
            assert_eq!(plan.changes_repository, mutates);
            let json = serde_json::to_value(&plan).unwrap();
            assert_eq!(json["outcome"], "ready");
            assert_eq!(
                json["commands"][0]["explanation"],
                plan.commands[0].explanation
            );
            assert_eq!(json["commands"][0]["program"], "git");
        }
        let mut p = proposal(Stage, "", &["README.md"]);
        p.steps.push(Step {
            action: Commit,
            value: "update docs".into(),
            paths: vec![],
            sha: String::new(),
        });
        let plan = p.validate(Path::new(".")).unwrap();
        assert!(plan.commands[0].explanation.contains("literal paths"));
        assert!(plan.commands[1].explanation.contains("only staged"));
        assert_eq!(
            plan.commands[1].args,
            strings(&["commit", "-m", "update docs"])
        );
    }

    #[test]
    fn outcome_invariants_fail_closed() {
        for outcome in [Outcome::NeedsClarification, Outcome::Unsupported] {
            let mut p = proposal(Action::Status, "", &[]);
            p.outcome = outcome;
            assert!(p.validate(Path::new(".")).is_err());
            let mut p = proposal(Action::Status, "", &[]);
            p.outcome = outcome;
            p.steps.clear();
            p.clarification = "Please provide supported intent.".into();
            let plan = p.validate(Path::new(".")).unwrap();
            assert_eq!(plan.outcome, outcome);
            assert!(plan.commands.is_empty());
            assert!(!plan.changes_repository);
        }
        let mut p = proposal(Action::Status, "", &[]);
        p.clarification = "A question?".into();
        assert!(p.validate(Path::new(".")).is_err());
        let mut p = proposal(Action::Status, "", &[]);
        p.steps.clear();
        assert!(p.validate(Path::new(".")).is_err());
        assert!(
            serde_json::from_str::<Proposal>(r#"{"summary":"x","clarification":"","steps":[]}"#)
                .is_err()
        );
        assert!(serde_json::from_str::<Outcome>(r#""unknown""#).is_err());
        assert_eq!(
            serde_json::to_string(&Outcome::NeedsClarification).unwrap(),
            "\"needsClarification\""
        );
    }

    #[test]
    fn hostile_commit_message_is_a_single_quoted_argument() {
        let plan = proposal(Action::Commit, "fix 'login'; $(touch /tmp/pwned)", &[])
            .validate(Path::new("."))
            .unwrap();
        assert_eq!(
            plan.commands[0].display(),
            "git commit -m 'fix '\\''login'\\''; $(touch /tmp/pwned)'"
        );
        assert_eq!(plan.commands[0].args.len(), 3);
    }

    #[test]
    fn stage_treats_flags_and_pathspec_magic_as_literal_files() {
        let plan = proposal(Action::Stage, "", &["--all", ":(glob)*", "a b.txt"])
            .validate(Path::new("."))
            .unwrap();
        assert_eq!(
            plan.commands[0].args,
            strings(&[
                "--literal-pathspecs",
                "add",
                "--",
                "--all",
                ":(glob)*",
                "a b.txt"
            ])
        );
    }

    #[test]
    fn git_stages_only_the_named_literal_files() {
        use std::process::Command;
        let dir = tempfile::tempdir().unwrap();
        assert!(
            Command::new("git")
                .arg("init")
                .arg(dir.path())
                .output()
                .unwrap()
                .status
                .success()
        );
        for name in ["--all", ":(glob)*", "a b.txt", "leave-alone.txt"] {
            std::fs::write(dir.path().join(name), "test").unwrap();
        }
        let plan = proposal(Action::Stage, "", &["--all", ":(glob)*", "a b.txt"])
            .validate(Path::new("."))
            .unwrap();
        let command = &plan.commands[0];
        assert!(
            Command::new(command.program)
                .args(&command.args)
                .current_dir(dir.path())
                .output()
                .unwrap()
                .status
                .success()
        );
        let output = Command::new("git")
            .args(["diff", "--cached", "--name-only", "-z"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"--all\0:(glob)*\0a b.txt\0");
    }

    #[test]
    fn rejects_path_escape_and_control_characters() {
        for path in [
            "../secret",
            "src/../../secret",
            "/etc/passwd",
            "hello\nworld",
        ] {
            assert!(
                proposal(Action::Stage, "", &[path])
                    .validate(Path::new("."))
                    .is_err()
            );
        }
    }

    #[test]
    fn rejects_option_injection_and_invalid_refs() {
        for branch in [
            "--detach",
            "foo;whoami",
            "a..b",
            "a//b",
            "a/.b",
            "a.lock",
            "HEAD",
            "@{-1}",
        ] {
            assert!(
                proposal(Action::SwitchBranch, branch, &[])
                    .validate(Path::new("."))
                    .is_err()
            );
        }
        assert!(
            proposal(Action::CreateBranch, "fix/login-2", &[])
                .validate(Path::new("."))
                .is_ok()
        );
    }

    #[test]
    fn clarification_cannot_hide_commands() {
        let mut p = proposal(Action::Push, "", &[]);
        p.outcome = Outcome::NeedsClarification;
        p.clarification = "Which remote?".into();
        assert!(p.validate(Path::new(".")).is_err());
    }

    #[test]
    fn unsupported_operations_and_extra_arguments_fail_closed() {
        assert!(
            serde_json::from_str::<Step>(r#"{"action":"reset","value":"--hard","paths":[]}"#)
                .is_err()
        );
        assert!(
            proposal(Action::Status, "--porcelain", &[])
                .validate(Path::new("."))
                .is_err()
        );
        assert!(
            proposal(Action::Push, "", &["origin"])
                .validate(Path::new("."))
                .is_err()
        );
        assert!(
            proposal(Action::Commit, "", &[])
                .validate(Path::new("."))
                .is_err()
        );
    }

    #[test]
    fn pull_only_fast_forwards_and_mutations_are_marked() {
        let plan = proposal(Action::Pull, "", &[])
            .validate(Path::new("."))
            .unwrap();
        assert_eq!(plan.commands[0].display(), "git pull --ff-only");
        assert!(plan.changes_repository);
        assert!(
            !proposal(Action::Diff, "", &[])
                .validate(Path::new("."))
                .unwrap()
                .changes_repository
        );
    }

    macro_rules! each_action {
        ($($action:ident => $value:expr, $paths:expr;)*) => {
            fn each_action(
                mut visit: impl FnMut(Action, &'static str, &'static [&'static str]),
            ) {
                $(visit(Action::$action, $value, $paths);)*
            }

            fn every_variant_is_listed(action: &Action) {
                match action {
                    $(Action::$action => {} )*
                    Action::CreateBranchAt { .. } => {}
                }
            }
        };
    }

    each_action! {
        Status => "", &[];
        Diff => "", &[];
        StagedDiff => "", &[];
        Log => "", &[];
        ListBranches => "", &[];
        CreateBranch => "feature/test", &[];
        SwitchBranch => "main", &[];
        Stage => "", &["README.md"];
        Unstage => "", &["README.md"];
        StageAll => "", &[];
        Commit => "update docs", &[];
        Stash => "", &[];
        Push => "", &[];
        Pull => "", &[];
        ShowReflog => "", &[];
    }

    #[test]
    fn safety_metadata_is_paired_and_current_actions_are_benign() {
        each_action(|action, value, paths| {
            every_variant_is_listed(&action);
            assert_eq!(
                action.destructive(),
                action.warning().is_some(),
                "{action:?}"
            );
            assert!(!action.destructive(), "{action:?}");
            assert_eq!(action.warning(), None, "{action:?}");
            let destructive = action.destructive();
            let warning = action.warning();
            let command = &proposal(action.clone(), value, paths)
                .validate(Path::new("."))
                .unwrap()
                .commands[0];
            assert_eq!(command.destructive, destructive);
            assert_eq!(command.warning, warning);
            let json = serde_json::to_value(command).unwrap();
            assert!(json.get("destructive").is_none(), "{action:?}: {json}");
            assert!(json.get("warning").is_none(), "{action:?}: {json}");
        });
        let recover = Action::CreateBranchAt {
            name: "recovered/login".into(),
            sha: "abc1234".into(),
        };
        every_variant_is_listed(&recover);
        assert!(!recover.destructive());
        assert_eq!(recover.warning(), None);
    }

    #[test]
    fn destructive_command_serializes_flag_and_warning() {
        let command = GitCommand {
            program: "git",
            explanation: "Discards uncommitted changes in the working tree.".into(),
            args: strings(&["restore", "--worktree", "--", "."]),
            destructive: true,
            warning: Some("This permanently discards uncommitted work."),
        };
        assert_eq!(
            serde_json::to_value(&command).unwrap(),
            serde_json::json!({
                "program": "git",
                "explanation": "Discards uncommitted changes in the working tree.",
                "args": ["restore", "--worktree", "--", "."],
                "destructive": true,
                "warning": "This permanently discards uncommitted work."
            })
        );
        let benign = GitCommand {
            program: "git",
            explanation: "Shows short repository status and the current branch.".into(),
            args: strings(&["status", "--short", "--branch"]),
            destructive: false,
            warning: None,
        };
        let json = serde_json::to_value(&benign).unwrap();
        assert!(json.get("destructive").is_none());
        assert!(json.get("warning").is_none());
    }

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

    fn commit_repo() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let sha = {
            let hooks = dir.path().join("hooks");
            std::fs::create_dir(&hooks).unwrap();
            let run = |args: &[&str]| {
                let output = std::process::Command::new("git")
                    .args(args)
                    .current_dir(dir.path())
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{args:?}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                output
            };
            run(&["init", "--object-format=sha1", "--initial-branch=main"]);
            run(&["config", "user.email", "test@example.invalid"]);
            run(&["config", "user.name", "Githelper Test"]);
            run(&["config", "commit.gpgsign", "false"]);
            run(&["config", "core.hooksPath", hooks.to_str().unwrap()]);
            std::fs::write(dir.path().join("README.md"), "hello\n").unwrap();
            run(&["add", "--", "README.md"]);
            run(&["commit", "-m", "Initial"]);
            String::from_utf8(run(&["rev-parse", "HEAD"]).stdout).unwrap()
        };
        (dir, sha.trim().to_owned())
    }

    fn assert_recovery_command(plan: &Plan, branch: &str, sha: &str) {
        assert_eq!(plan.outcome, Outcome::Ready);
        assert!(plan.clarification.is_none());
        assert!(plan.changes_repository);
        assert_eq!(plan.commands.len(), 1);
        let command = &plan.commands[0];
        assert_eq!(
            command.args,
            vec!["branch".to_owned(), branch.to_owned(), sha.to_owned()]
        );
        assert_eq!(command.display(), format!("git branch {branch} {sha}"));
        assert!(!command.destructive);
        assert!(command.warning.is_none());
        assert!(command.explanation.contains("without moving"));
        for arg in &command.args {
            assert_ne!(arg, "reset");
            assert_ne!(arg, "checkout");
            assert_ne!(arg, "switch");
        }
        let json = serde_json::to_value(command).unwrap();
        assert!(json.get("destructive").is_none());
        assert!(json.get("warning").is_none());
    }

    #[test]
    fn show_reflog_is_read_only_and_explains_how_to_find_a_sha() {
        let plan = proposal(Action::ShowReflog, "", &[])
            .validate(Path::new("."))
            .unwrap();
        assert!(!plan.changes_repository);
        assert_eq!(plan.commands[0].args, strings(&["reflog", "-n", "20"]));
        assert_eq!(plan.commands[0].display(), "git reflog -n 20");
        assert!(plan.commands[0].explanation.contains("note its SHA"));
        assert!(!plan.commands[0].destructive);
        assert!(plan.commands[0].warning.is_none());
        assert!(
            serde_json::from_str::<Step>(r#"{"action":"showReflog","value":"","paths":[]}"#)
                .is_ok()
        );
        assert!(
            proposal(Action::ShowReflog, "HEAD", &[])
                .validate(Path::new("."))
                .is_err()
        );
        let mut with_sha = proposal(Action::ShowReflog, "", &[]);
        with_sha.steps[0].sha = "abc1234".into();
        assert!(with_sha.validate(Path::new(".")).is_err());
    }

    #[test]
    fn recovered_branch_rejects_bad_shas_and_branch_names() {
        let (dir, sha) = commit_repo();
        for bad in [
            sha[..6].to_owned(),
            format!("{sha}a"),
            "abc123g".to_owned(),
            "zzzzzzz".to_owned(),
            "abc123!".to_owned(),
            "abc1234\n".to_owned(),
        ] {
            assert!(
                recovery("login", &bad).validate(dir.path()).is_err(),
                "{bad}"
            );
        }
        for name in [
            "recovered/../main",
            "-lost",
            "recovered/-lost",
            "recovered/foo/-bar",
            "recovered/",
            "../main",
            "recovered/HEAD",
        ] {
            assert!(recovery(name, &sha).validate(dir.path()).is_err(), "{name}");
        }
        let with_paths: Proposal = serde_json::from_value(serde_json::json!({
            "outcome": "ready",
            "summary": "Recover the lost commit",
            "clarification": "",
            "steps": [{
                "action": "createBranchAt",
                "value": "login",
                "paths": ["README.md"],
                "sha": "abc1234"
            }]
        }))
        .unwrap();
        assert!(with_paths.validate(dir.path()).is_err());
        let mut status = proposal(Action::Status, "", &[]);
        status.steps[0].sha = "abc1234".into();
        assert!(status.validate(dir.path()).is_err());
    }

    #[test]
    fn create_branch_at_uses_recovered_prefix_and_checks_the_commit() {
        let (dir, sha) = commit_repo();
        let short = &sha[..7];
        assert_recovery_command(
            &recovery("login", short).validate(dir.path()).unwrap(),
            "recovered/login",
            short,
        );
        assert_recovery_command(
            &recovery("recovered/login", &sha)
                .validate(dir.path())
                .unwrap(),
            "recovered/login",
            &sha,
        );
        assert_recovery_command(
            &recovery("", short).validate(dir.path()).unwrap(),
            &format!("recovered/{short}"),
            short,
        );
        assert_recovery_command(
            &recovery("feature/lost", short)
                .validate(dir.path())
                .unwrap(),
            "recovered/feature/lost",
            short,
        );

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
        assert!(
            sha.chars().any(|c| matches!(c, 'a'..='f')),
            "fixture commit SHA should contain a hex letter so mixed case differs"
        );
        assert_ne!(mixed, sha);
        assert_recovery_command(
            &recovery("login", &mixed).validate(dir.path()).unwrap(),
            "recovered/login",
            &mixed,
        );

        let missing = recovery("login", "abc1234").validate(dir.path()).unwrap();
        assert_eq!(missing.outcome, Outcome::NeedsClarification);
        assert!(missing.commands.is_empty());
        assert!(!missing.changes_repository);
        let question = missing.clarification.unwrap();
        assert!(question.contains("abc1234"));
        assert!(question.contains("reflog"));
        assert!(!question.contains("reset"));

        let mixed_missing = recovery("login", "AbC1234").validate(dir.path()).unwrap();
        assert_eq!(mixed_missing.outcome, Outcome::NeedsClarification);
        assert!(mixed_missing.commands.is_empty());

        let tree = {
            let output = std::process::Command::new("git")
                .args(["rev-parse", "HEAD^{tree}"])
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(output.status.success());
            String::from_utf8(output.stdout).unwrap()
        };
        let tree = tree.trim();
        let not_commit = recovery("login", tree).validate(dir.path()).unwrap();
        assert_eq!(not_commit.outcome, Outcome::NeedsClarification);
        assert!(not_commit.commands.is_empty());

        let mut both = proposal(Action::ShowReflog, "", &[]);
        both.steps.extend(recovery("login", "abc1234").steps);
        let plan = both.validate(dir.path()).unwrap();
        assert_eq!(plan.outcome, Outcome::NeedsClarification);
        assert!(plan.commands.is_empty());
        assert!(!plan.changes_repository);
    }
}

#[cfg(test)]
#[path = "recovery_edge_tests.rs"]
mod recovery_edge_tests;
