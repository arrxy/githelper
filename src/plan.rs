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
    pub summary: String,
    pub clarification: String,
    pub steps: Vec<Step>,
}

#[derive(Debug, Serialize)]
pub struct Plan {
    pub summary: String,
    pub clarification: Option<String>,
    pub commands: Vec<GitCommand>,
    pub changes_repository: bool,
}

#[derive(Debug, Serialize)]
pub struct GitCommand {
    pub program: &'static str,
    pub args: Vec<String>,
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
        if !self.clarification.is_empty() {
            if !text(&self.clarification, 1_000) || !self.steps.is_empty() {
                return Err(invalid("clarification must have no commands"));
            }
            return Ok(Plan {
                summary: self.summary,
                clarification: Some(self.clarification),
                commands: vec![],
                changes_repository: false,
            });
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
            commands.push(GitCommand {
                program: "git",
                args,
            });
        }
        Ok(Plan {
            summary: self.summary,
            clarification: None,
            commands,
            changes_repository,
        })
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
}
