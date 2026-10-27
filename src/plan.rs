use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    process::{Command, Output},
};
use thiserror::Error;

const DISCARD_WARNING: &str = "Uncommitted changes to this file are lost permanently.";
const PUBLISHED_UNDO_WARNING: &str = "This commit is already on the upstream. Rewriting it needs a force push, which githelper does not support.";

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
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
    UndoLastCommit,
    Discard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    Ready,
    NeedsClarification,
    Unsupported,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub action: Action,
    pub value: String,
    pub paths: Vec<String>,
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
    /// Warning text for a destructive step. Omitted when the step is benign.
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
    pub fn validate(self) -> Result<Plan, InvalidPlan> {
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
        if self
            .steps
            .iter()
            .filter(|step| matches!(step.action, Action::UndoLastCommit))
            .count()
            > 1
        {
            return Err(invalid("only the latest commit can be undone"));
        }
        let mut commands = Vec::new();
        let mut changes_repository = false;
        for step in self.steps {
            use Action::*;
            let needs_paths = matches!(step.action, Stage | Unstage | Discard);
            let needs_value = matches!(step.action, CreateBranch | SwitchBranch | Commit);
            if !needs_paths && !step.paths.is_empty() {
                return Err(invalid("unexpected paths for this action"));
            }
            if !needs_value && !step.value.is_empty() {
                return Err(invalid("unexpected value for this action"));
            }
            if matches!(step.action, Discard) && step.paths.is_empty() {
                return Ok(state_plan(
                    Outcome::NeedsClarification,
                    self.summary.clone(),
                    "Name the file whose changes should be discarded.",
                ));
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
                if matches!(step.action, Discard)
                    && step
                        .paths
                        .iter()
                        .any(|path| discards_entire_repository(path))
                {
                    return Ok(state_plan(
                        Outcome::Unsupported,
                        self.summary.clone(),
                        "Discarding every change is not supported.",
                    ));
                }
            }
            let args: Vec<String> = match step.action {
                Status => strings(&["status", "--short", "--branch"]),
                Diff => strings(&["diff", "--no-ext-diff", "--no-textconv", "--"]),
                StagedDiff => {
                    strings(&["diff", "--staged", "--no-ext-diff", "--no-textconv", "--"])
                }
                Log => strings(&["log", "--oneline", "--max-count=10", "--no-decorate"]),
                ListBranches => strings(&["branch", "--list"]),
                CreateBranch | SwitchBranch => {
                    validate_branch(&step.value)?;
                    let mut args = strings(&["switch"]);
                    if matches!(step.action, CreateBranch) {
                        args.push("-c".into());
                    }
                    args.push(step.value);
                    args
                }
                Stage | Unstage => {
                    let mut args = strings(&["--literal-pathspecs"]);
                    if matches!(step.action, Stage) {
                        args.push("add".into());
                    } else {
                        args.extend(strings(&["restore", "--staged"]));
                    }
                    args.push("--".into());
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
                UndoLastCommit => strings(&["reset", "--soft", "HEAD~1"]),
                Discard => {
                    let mut args = strings(&["--literal-pathspecs", "restore", "--"]);
                    args.extend(step.paths);
                    args
                }
            };
            changes_repository |=
                !matches!(step.action, Status | Diff | StagedDiff | Log | ListBranches);
            let destructive = step.action.destructive();
            let warning = step.action.warning();
            debug_assert_eq!(destructive, warning.is_some());
            commands.push(GitCommand {
                program: "git",
                explanation: step.action.explanation().into(),
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
/// `Action::safety` is the static classification. Discard, merge, and rebase
/// return `Destructive` with the warning a person reads before running the
/// command. A soft undo stays benign here; `validate_repository` adds the
/// force-push warning when that commit is already on the upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Safety {
    Benign,
    /// Discard, merge, and rebase plug in as this variant.
    Destructive(&'static str),
}

impl Action {
    /// Whether this action can discard uncommitted work or rewrite history.
    ///
    /// Repository edits and ordinary remote sync are not destructive. Discard,
    /// merge, and rebase return true here. A soft undo of an unpublished commit
    /// does not; a published undo is marked by `validate_repository`.
    pub fn destructive(self) -> bool {
        matches!(self.safety(), Safety::Destructive(_))
    }

    /// Warning copied onto a destructive step, in text output and `--json`.
    pub fn warning(self) -> Option<&'static str> {
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

    fn safety(self) -> Safety {
        match self {
            // Read-only inspection, local edits, and ordinary remote sync do not
            // discard work. Classify discard, merge, and rebase as
            // `Safety::Destructive("...")` in this match. Soft undo is benign
            // until repository state shows the commit is already published.
            Self::Status
            | Self::Diff
            | Self::StagedDiff
            | Self::Log
            | Self::ListBranches
            | Self::CreateBranch
            | Self::SwitchBranch
            | Self::Stage
            | Self::StageAll
            | Self::Unstage
            | Self::Commit
            | Self::Stash
            | Self::Push
            | Self::Pull
            | Self::UndoLastCommit => Safety::Benign,
            Self::Discard => Safety::Destructive(DISCARD_WARNING),
        }
    }

    fn explanation(self) -> &'static str {
        match self {
            Self::Status => "Shows short repository status and the current branch.",
            Self::Diff => "Shows unstaged changes to tracked files.",
            Self::StagedDiff => "Shows changes currently staged for commit.",
            Self::Log => "Shows the ten most recent commits.",
            Self::ListBranches => "Lists local branches.",
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
            Self::UndoLastCommit => {
                "Removes the last commit; nothing is lost, and the changes stay staged."
            }
            Self::Discard => {
                "Discards uncommitted changes to the named paths by restoring the index version."
            }
        }
    }
}

fn state_plan(outcome: Outcome, summary: String, clarification: impl Into<String>) -> Plan {
    Plan {
        outcome,
        summary,
        clarification: Some(clarification.into()),
        commands: vec![],
        changes_repository: false,
    }
}

/// `.` and `./.` name the whole work tree. Discarding that is unsupported.
fn discards_entire_repository(path: &str) -> bool {
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    matches!(parts.next(), Some(".")) && parts.all(|part| part == ".")
}

/// Apply checks that depend on the current repository.
///
/// A missing parent commit, or a path with no worktree changes for `git restore`,
/// becomes a clarification and no commands. A last commit already contained in
/// the upstream keeps the soft reset and gains a force-push warning. This only
/// reads Git state. It never runs the planned commands.
pub fn validate_repository(plan: Plan, repo: &Path) -> Result<Plan, Box<dyn std::error::Error>> {
    if plan.outcome != Outcome::Ready {
        return Ok(plan);
    }
    let Plan {
        summary,
        mut commands,
        changes_repository,
        ..
    } = plan;
    let mut published = Vec::new();
    for (index, command) in commands.iter().enumerate() {
        if is_soft_undo(command) {
            if !has_parent_commit(repo)? {
                return Ok(state_plan(
                    Outcome::NeedsClarification,
                    summary,
                    "There is no parent commit to undo.",
                ));
            }
            if head_is_on_upstream(repo)? {
                published.push(index);
            }
        } else if let Some(paths) = discard_paths(command) {
            for path in paths {
                if !worktree_has_changes(repo, path)? {
                    return Ok(state_plan(
                        Outcome::NeedsClarification,
                        summary,
                        format!("{path} has no changes to discard."),
                    ));
                }
            }
        }
    }
    for index in published {
        commands[index].destructive = true;
        commands[index].warning = Some(PUBLISHED_UNDO_WARNING);
    }
    Ok(Plan {
        outcome: Outcome::Ready,
        summary,
        clarification: None,
        commands,
        changes_repository,
    })
}

fn is_soft_undo(command: &GitCommand) -> bool {
    command.args.len() == 3
        && command.args[0] == "reset"
        && command.args[1] == "--soft"
        && command.args[2] == "HEAD~1"
}

fn discard_paths(command: &GitCommand) -> Option<&[String]> {
    let args = &command.args;
    if args.len() >= 3
        && args[0] == "--literal-pathspecs"
        && args[1] == "restore"
        && args[2] == "--"
    {
        Some(&args[3..])
    } else {
        None
    }
}

fn git(repo: &Path, args: &[&str]) -> Result<Output, Box<dyn std::error::Error>> {
    Command::new("git")
        .args(["--no-optional-locks", "-c", "core.fsmonitor=false"])
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|error| format!("could not run git: {error}").into())
}

fn git_failure(repo: &Path, output: &Output) -> Box<dyn std::error::Error> {
    format!(
        "Git could not inspect {}: {}",
        repo.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    )
    .into()
}

fn has_parent_commit(repo: &Path) -> Result<bool, Box<dyn std::error::Error>> {
    let output = git(repo, &["rev-parse", "--verify", "--quiet", "HEAD~1"])?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(git_failure(repo, &output)),
    }
}

fn head_is_on_upstream(repo: &Path) -> Result<bool, Box<dyn std::error::Error>> {
    let upstream = git(repo, &["rev-parse", "--verify", "--quiet", "@{upstream}"])?;
    match upstream.status.code() {
        Some(0) => {}
        Some(1) => return Ok(false),
        _ => return Err(git_failure(repo, &upstream)),
    }
    let ancestor = git(
        repo,
        &["merge-base", "--is-ancestor", "HEAD", "@{upstream}"],
    )?;
    match ancestor.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(git_failure(repo, &ancestor)),
    }
}

fn worktree_has_changes(repo: &Path, path: &str) -> Result<bool, Box<dyn std::error::Error>> {
    let output = git(
        repo,
        &[
            "--literal-pathspecs",
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--quiet",
            "--",
            path,
        ],
    )?;
    match output.status.code() {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Err(git_failure(repo, &output)),
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

    fn proposal(action: Action, value: &str, paths: &[&str]) -> Proposal {
        Proposal {
            outcome: Outcome::Ready,
            summary: "Requested Git operation".into(),
            clarification: String::new(),
            steps: vec![Step {
                action,
                value: value.into(),
                paths: strings(paths),
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
            (UndoLastCommit, "", &[][..], "nothing is lost", true),
            (Discard, "", &["README.md"][..], "named paths", true),
        ];
        for (action, value, paths, scope, mutates) in cases {
            let plan = proposal(action, value, paths).validate().unwrap();
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
        });
        let plan = p.validate().unwrap();
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
            assert!(p.validate().is_err());
            let mut p = proposal(Action::Status, "", &[]);
            p.outcome = outcome;
            p.steps.clear();
            p.clarification = "Please provide supported intent.".into();
            let plan = p.validate().unwrap();
            assert_eq!(plan.outcome, outcome);
            assert!(plan.commands.is_empty());
            assert!(!plan.changes_repository);
        }
        let mut p = proposal(Action::Status, "", &[]);
        p.clarification = "A question?".into();
        assert!(p.validate().is_err());
        let mut p = proposal(Action::Status, "", &[]);
        p.steps.clear();
        assert!(p.validate().is_err());
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
            .validate()
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
            .validate()
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
            .validate()
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
            assert!(proposal(Action::Stage, "", &[path]).validate().is_err());
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
                    .validate()
                    .is_err()
            );
        }
        assert!(
            proposal(Action::CreateBranch, "fix/login-2", &[])
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn clarification_cannot_hide_commands() {
        let mut p = proposal(Action::Push, "", &[]);
        p.outcome = Outcome::NeedsClarification;
        p.clarification = "Which remote?".into();
        assert!(p.validate().is_err());
    }

    #[test]
    fn unsupported_operations_and_extra_arguments_fail_closed() {
        assert!(
            serde_json::from_str::<Step>(r#"{"action":"reset","value":"--hard","paths":[]}"#)
                .is_err()
        );
        assert!(
            proposal(Action::Status, "--porcelain", &[])
                .validate()
                .is_err()
        );
        assert!(proposal(Action::Push, "", &["origin"]).validate().is_err());
        assert!(proposal(Action::Commit, "", &[]).validate().is_err());
    }

    #[test]
    fn pull_only_fast_forwards_and_mutations_are_marked() {
        let plan = proposal(Action::Pull, "", &[]).validate().unwrap();
        assert_eq!(plan.commands[0].display(), "git pull --ff-only");
        assert!(plan.changes_repository);
        assert!(
            !proposal(Action::Diff, "", &[])
                .validate()
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

            fn every_variant_is_listed(action: Action) {
                match action {
                    $(Action::$action => {} )*
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
        UndoLastCommit => "", &[];
        Discard => "", &["README.md"];
    }

    #[test]
    fn safety_metadata_is_paired_for_every_action() {
        each_action(|action, value, paths| {
            every_variant_is_listed(action);
            assert_eq!(
                action.destructive(),
                action.warning().is_some(),
                "{action:?}"
            );
            let command = &proposal(action, value, paths).validate().unwrap().commands[0];
            assert_eq!(command.destructive, action.destructive());
            assert_eq!(command.warning, action.warning());
            let json = serde_json::to_value(command).unwrap();
            if action.destructive() {
                assert_eq!(json["destructive"], true, "{action:?}");
                assert_eq!(json["warning"], action.warning().unwrap(), "{action:?}");
            } else {
                assert_eq!(action.warning(), None, "{action:?}");
                assert!(json.get("destructive").is_none(), "{action:?}: {json}");
                assert!(json.get("warning").is_none(), "{action:?}: {json}");
            }
        });
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

    fn git_ok(dir: &std::path::Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn commit_repo(commits: usize) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git_ok(dir.path(), &["init", "--initial-branch=main"]);
        git_ok(dir.path(), &["config", "user.name", "Test"]);
        git_ok(dir.path(), &["config", "user.email", "test@example.com"]);
        for index in 0..commits {
            std::fs::write(dir.path().join("file.txt"), format!("version {index}\n")).unwrap();
            git_ok(dir.path(), &["add", "--", "file.txt"]);
            git_ok(dir.path(), &["commit", "-m", &format!("commit {index}")]);
        }
        dir
    }

    fn head(dir: &std::path::Path) -> String {
        let output = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    /// Keep the bare remote alive for the caller; dropping it deletes the upstream.
    fn publish(dir: &std::path::Path) -> tempfile::TempDir {
        let origin = tempfile::tempdir().unwrap();
        git_ok(origin.path(), &["init", "--bare", "--initial-branch=main"]);
        git_ok(
            dir,
            &["remote", "add", "origin", origin.path().to_str().unwrap()],
        );
        git_ok(dir, &["push", "-u", "origin", "HEAD"]);
        origin
    }

    #[test]
    fn undo_is_soft_head_tilde_one_and_rejects_hard_or_longer_history() {
        let plan = proposal(Action::UndoLastCommit, "", &[])
            .validate()
            .unwrap();
        assert_eq!(
            plan.commands[0].args,
            strings(&["reset", "--soft", "HEAD~1"])
        );
        assert_eq!(plan.commands[0].display(), "git reset --soft 'HEAD~1'");
        assert!(plan.changes_repository);
        assert!(!plan.commands[0].destructive);
        assert_eq!(plan.commands[0].warning, None);
        let explanation = &plan.commands[0].explanation;
        assert!(!explanation.contains('\n'));
        assert!(explanation.contains("nothing is lost"));
        assert!(explanation.contains("changes stay staged"));
        let json = serde_json::to_value(&plan.commands[0]).unwrap();
        assert!(json.get("destructive").is_none());
        assert!(json.get("warning").is_none());
        assert!(!plan.commands[0].args.iter().any(|arg| arg == "--hard"));

        for value in ["--hard", "--mixed", "HEAD~2", "HEAD~3", "2", "HEAD~1"] {
            assert!(
                proposal(Action::UndoLastCommit, value, &[])
                    .validate()
                    .is_err(),
                "{value}"
            );
        }
        assert!(
            proposal(Action::UndoLastCommit, "", &["README.md"])
                .validate()
                .is_err()
        );
        assert!(
            serde_json::from_str::<Step>(r#"{"action":"reset","value":"--hard","paths":[]}"#)
                .is_err()
        );
        assert!(
            serde_json::from_str::<Step>(r#"{"action":"revert","value":"","paths":["README.md"]}"#)
                .is_err()
        );
        let mut repeated = proposal(Action::UndoLastCommit, "", &[]);
        repeated.steps.push(Step {
            action: Action::UndoLastCommit,
            value: String::new(),
            paths: vec![],
        });
        assert!(repeated.validate().is_err());
    }

    #[test]
    fn undo_root_commit_clarifies_and_published_commit_warns() {
        let empty = tempfile::tempdir().unwrap();
        git_ok(empty.path(), &["init", "--initial-branch=main"]);
        let plan = validate_repository(
            proposal(Action::UndoLastCommit, "", &[])
                .validate()
                .unwrap(),
            empty.path(),
        )
        .unwrap();
        assert_eq!(plan.outcome, Outcome::NeedsClarification);
        assert!(plan.commands.is_empty());
        assert!(!plan.changes_repository);
        assert!(plan.clarification.unwrap().contains("no parent"));

        let root = commit_repo(1);
        let before = head(root.path());
        let plan = validate_repository(
            proposal(Action::UndoLastCommit, "", &[])
                .validate()
                .unwrap(),
            root.path(),
        )
        .unwrap();
        assert_eq!(plan.outcome, Outcome::NeedsClarification);
        assert!(plan.commands.is_empty());
        assert!(plan.clarification.unwrap().contains("no parent"));
        assert_eq!(head(root.path()), before);
        let _published_root = publish(root.path());
        let plan = validate_repository(
            proposal(Action::UndoLastCommit, "", &[])
                .validate()
                .unwrap(),
            root.path(),
        )
        .unwrap();
        assert_eq!(plan.outcome, Outcome::NeedsClarification);
        assert!(plan.commands.is_empty());

        let local = commit_repo(2);
        let before = head(local.path());
        let plan = validate_repository(
            proposal(Action::UndoLastCommit, "", &[])
                .validate()
                .unwrap(),
            local.path(),
        )
        .unwrap();
        assert_eq!(plan.outcome, Outcome::Ready);
        assert_eq!(
            plan.commands[0].args,
            strings(&["reset", "--soft", "HEAD~1"])
        );
        assert!(!plan.commands[0].destructive);
        assert_eq!(plan.commands[0].warning, None);
        assert_eq!(head(local.path()), before);
        let json = serde_json::to_value(&plan.commands[0]).unwrap();
        assert!(json.get("destructive").is_none());
        assert!(json.get("warning").is_none());

        let published = commit_repo(2);
        let _origin = publish(published.path());
        let before = head(published.path());
        let plan = validate_repository(
            proposal(Action::UndoLastCommit, "", &[])
                .validate()
                .unwrap(),
            published.path(),
        )
        .unwrap();
        assert_eq!(plan.outcome, Outcome::Ready);
        assert_eq!(
            plan.commands[0].args,
            strings(&["reset", "--soft", "HEAD~1"])
        );
        assert!(plan.commands[0].destructive);
        assert_eq!(plan.commands[0].warning, Some(PUBLISHED_UNDO_WARNING));
        assert!(PUBLISHED_UNDO_WARNING.contains("force push"));
        assert!(
            !plan.commands[0]
                .args
                .iter()
                .any(|arg| arg.contains("force"))
        );
        assert_eq!(head(published.path()), before);
        let json = serde_json::to_value(&plan).unwrap();
        assert_eq!(json["commands"][0]["destructive"], true);
        assert_eq!(json["commands"][0]["warning"], PUBLISHED_UNDO_WARNING);

        std::fs::write(published.path().join("file.txt"), "local only\n").unwrap();
        git_ok(published.path(), &["add", "--", "file.txt"]);
        git_ok(published.path(), &["commit", "-m", "not pushed"]);
        let plan = validate_repository(
            proposal(Action::UndoLastCommit, "", &[])
                .validate()
                .unwrap(),
            published.path(),
        )
        .unwrap();
        assert!(!plan.commands[0].destructive);
        assert_eq!(plan.commands[0].warning, None);
    }

    #[test]
    fn discard_validates_paths_and_marks_permanent_loss() {
        let plan = proposal(Action::Discard, "", &[]).validate().unwrap();
        assert_eq!(plan.outcome, Outcome::NeedsClarification);
        assert!(plan.commands.is_empty());
        assert!(!plan.changes_repository);
        assert!(plan.clarification.unwrap().to_lowercase().contains("file"));

        for path in [".", "./", "./."] {
            let plan = proposal(Action::Discard, "", &[path]).validate().unwrap();
            assert_eq!(plan.outcome, Outcome::Unsupported, "{path}");
            assert!(plan.commands.is_empty(), "{path}");
            assert!(!plan.changes_repository, "{path}");
        }
        let nested = proposal(Action::Discard, "", &["./README.md"])
            .validate()
            .unwrap();
        assert_eq!(nested.outcome, Outcome::Ready);
        assert_eq!(
            nested.commands[0].args,
            strings(&["--literal-pathspecs", "restore", "--", "./README.md"])
        );

        for path in [
            "../secret",
            "src/../../secret",
            "/etc/passwd",
            "hello\nworld",
        ] {
            assert!(
                proposal(Action::Discard, "", &[path]).validate().is_err(),
                "{path}"
            );
        }
        assert!(
            proposal(Action::Discard, "--hard", &["README.md"])
                .validate()
                .is_err()
        );

        let plan = proposal(Action::Discard, "", &["README.md", "a b.txt"])
            .validate()
            .unwrap();
        assert_eq!(
            plan.commands[0].args,
            strings(&[
                "--literal-pathspecs",
                "restore",
                "--",
                "README.md",
                "a b.txt"
            ])
        );
        assert_eq!(
            plan.commands[0].display(),
            "git --literal-pathspecs restore -- README.md 'a b.txt'"
        );
        assert!(plan.commands[0].destructive);
        assert_eq!(plan.commands[0].warning, Some(DISCARD_WARNING));
        assert!(DISCARD_WARNING.contains("lost permanently"));
        let json = serde_json::to_value(&plan).unwrap();
        assert_eq!(json["commands"][0]["destructive"], true);
        assert_eq!(json["commands"][0]["warning"], DISCARD_WARNING);
        assert_eq!(json["outcome"], "ready");
    }

    #[test]
    fn discard_clarifies_when_a_real_file_has_no_changes_and_restores_only_that_file() {
        let dir = commit_repo(1);
        let untouched = validate_repository(
            proposal(Action::Discard, "", &["file.txt"])
                .validate()
                .unwrap(),
            dir.path(),
        )
        .unwrap();
        assert_eq!(untouched.outcome, Outcome::NeedsClarification);
        assert!(untouched.commands.is_empty());
        let question = untouched.clarification.unwrap();
        assert!(question.contains("file.txt"));
        assert!(question.contains("no changes"));

        std::fs::write(dir.path().join("file.txt"), "staged only\n").unwrap();
        git_ok(dir.path(), &["add", "--", "file.txt"]);
        let staged_only = validate_repository(
            proposal(Action::Discard, "", &["file.txt"])
                .validate()
                .unwrap(),
            dir.path(),
        )
        .unwrap();
        assert_eq!(staged_only.outcome, Outcome::NeedsClarification);

        std::fs::write(dir.path().join("file.txt"), "dirty worktree\n").unwrap();
        std::fs::write(dir.path().join("other.txt"), "leave me\n").unwrap();
        git_ok(dir.path(), &["add", "--", "other.txt"]);
        git_ok(dir.path(), &["commit", "-m", "other"]);
        std::fs::write(dir.path().join("other.txt"), "still dirty\n").unwrap();
        std::fs::write(dir.path().join("--all"), "option-like\n").unwrap();
        git_ok(dir.path(), &["add", "--", "--all"]);
        git_ok(dir.path(), &["commit", "-m", "option-like name"]);
        std::fs::write(dir.path().join("--all"), "option-like dirty\n").unwrap();

        let before = std::fs::read_to_string(dir.path().join("file.txt")).unwrap();
        let plan = validate_repository(
            proposal(Action::Discard, "", &["file.txt"])
                .validate()
                .unwrap(),
            dir.path(),
        )
        .unwrap();
        assert_eq!(plan.outcome, Outcome::Ready);
        assert_eq!(
            plan.commands[0].args,
            strings(&["--literal-pathspecs", "restore", "--", "file.txt"])
        );
        assert!(plan.commands[0].destructive);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("file.txt")).unwrap(),
            before
        );

        let missing = validate_repository(
            proposal(Action::Discard, "", &["missing.txt"])
                .validate()
                .unwrap(),
            dir.path(),
        )
        .unwrap();
        assert_eq!(missing.outcome, Outcome::NeedsClarification);
        assert!(missing.clarification.unwrap().contains("missing.txt"));
        let partial = validate_repository(
            proposal(Action::Discard, "", &["file.txt", "missing.txt"])
                .validate()
                .unwrap(),
            dir.path(),
        )
        .unwrap();
        assert_eq!(partial.outcome, Outcome::NeedsClarification);
        assert!(partial.commands.is_empty());
        assert!(partial.clarification.unwrap().contains("missing.txt"));

        let literal = validate_repository(
            proposal(Action::Discard, "", &["--all"])
                .validate()
                .unwrap(),
            dir.path(),
        )
        .unwrap();
        let command = &literal.commands[0];
        assert!(
            std::process::Command::new(command.program)
                .args(&command.args)
                .current_dir(dir.path())
                .output()
                .unwrap()
                .status
                .success()
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("--all")).unwrap(),
            "option-like\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("file.txt")).unwrap(),
            "dirty worktree\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("other.txt")).unwrap(),
            "still dirty\n"
        );
    }
}
