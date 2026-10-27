use serde::{Deserialize, Serialize};
use thiserror::Error;

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
        let mut commands = Vec::new();
        let mut changes_repository = false;
        for step in self.steps {
            use Action::*;
            let needs_paths = matches!(step.action, Stage | Unstage);
            let needs_value = matches!(step.action, CreateBranch | SwitchBranch | Commit);
            if !needs_paths && !step.paths.is_empty() {
                return Err(invalid("unexpected paths for this action"));
            }
            if !needs_value && !step.value.is_empty() {
                return Err(invalid("unexpected value for this action"));
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
            // discard work. Classify undo, discard, merge, and rebase as
            // `Safety::Destructive("...")` in this match.
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
            | Self::Pull => Safety::Benign,
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
    }

    #[test]
    fn safety_metadata_is_paired_and_current_actions_are_benign() {
        each_action(|action, value, paths| {
            every_variant_is_listed(action);
            assert_eq!(
                action.destructive(),
                action.warning().is_some(),
                "{action:?}"
            );
            assert!(!action.destructive(), "{action:?}");
            assert_eq!(action.warning(), None, "{action:?}");
            let command = &proposal(action, value, paths).validate().unwrap().commands[0];
            assert_eq!(command.destructive, action.destructive());
            assert_eq!(command.warning, action.warning());
            let json = serde_json::to_value(command).unwrap();
            assert!(json.get("destructive").is_none(), "{action:?}: {json}");
            assert!(json.get("warning").is_none(), "{action:?}: {json}");
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
}
