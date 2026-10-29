use crate::model::ClarificationTurn;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::Path,
    process::{Command, Output},
};
use thiserror::Error;

const DISCARD_WARNING: &str = "Uncommitted changes to this file are lost permanently.";
const PUBLISHED_UNDO_WARNING: &str = "This commit is already on the upstream. Rewriting it needs a force push, which githelper does not support.";
pub const ISSUE_REQUEST: &str =
    "Want this supported? Open an issue at github.com/arrxy/githelper/issues";

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
    AddRemote,
    ListRemotes,
    SetRemoteUrl,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(from = "WireStep", into = "WireStep")]
pub struct Step {
    pub action: Action,
    pub value: String,
    pub paths: Vec<String>,
    /// Set only when a SHA was attached to an action that does not accept one.
    pub sha: String,
    pub url: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireStep {
    action: WireAction,
    value: String,
    paths: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    sha: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    url: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum WireAction {
    Status,
    Diff,
    StagedDiff,
    Log,
    ListBranches,
    AddRemote,
    ListRemotes,
    SetRemoteUrl,
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
    UndoLastCommit,
    Discard,
}

fn step(action: Action, value: String, paths: Vec<String>, sha: String, url: String) -> Step {
    Step {
        action,
        value,
        paths,
        sha,
        url,
    }
}

impl From<WireStep> for Step {
    fn from(wire: WireStep) -> Self {
        let WireStep {
            action,
            value,
            paths,
            sha,
            url,
        } = wire;
        match action {
            WireAction::CreateBranchAt => step(
                Action::CreateBranchAt { name: value, sha },
                String::new(),
                paths,
                String::new(),
                url,
            ),
            WireAction::Status => step(Action::Status, value, paths, sha, url),
            WireAction::Diff => step(Action::Diff, value, paths, sha, url),
            WireAction::StagedDiff => step(Action::StagedDiff, value, paths, sha, url),
            WireAction::Log => step(Action::Log, value, paths, sha, url),
            WireAction::ListBranches => step(Action::ListBranches, value, paths, sha, url),
            WireAction::AddRemote => step(Action::AddRemote, value, paths, sha, url),
            WireAction::ListRemotes => step(Action::ListRemotes, value, paths, sha, url),
            WireAction::SetRemoteUrl => step(Action::SetRemoteUrl, value, paths, sha, url),
            WireAction::CreateBranch => step(Action::CreateBranch, value, paths, sha, url),
            WireAction::SwitchBranch => step(Action::SwitchBranch, value, paths, sha, url),
            WireAction::Stage => step(Action::Stage, value, paths, sha, url),
            WireAction::StageAll => step(Action::StageAll, value, paths, sha, url),
            WireAction::Unstage => step(Action::Unstage, value, paths, sha, url),
            WireAction::Commit => step(Action::Commit, value, paths, sha, url),
            WireAction::Stash => step(Action::Stash, value, paths, sha, url),
            WireAction::Push => step(Action::Push, value, paths, sha, url),
            WireAction::Pull => step(Action::Pull, value, paths, sha, url),
            WireAction::ShowReflog => step(Action::ShowReflog, value, paths, sha, url),
            WireAction::UndoLastCommit => step(Action::UndoLastCommit, value, paths, sha, url),
            WireAction::Discard => step(Action::Discard, value, paths, sha, url),
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
                url: step.url,
            },
            action => WireStep {
                action: WireAction::from(&action),
                value: step.value,
                paths: step.paths,
                sha: step.sha,
                url: step.url,
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
            Action::AddRemote => WireAction::AddRemote,
            Action::ListRemotes => WireAction::ListRemotes,
            Action::SetRemoteUrl => WireAction::SetRemoteUrl,
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
            Action::UndoLastCommit => WireAction::UndoLastCommit,
            Action::Discard => WireAction::Discard,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(rename = "manualCommand", skip_serializing_if = "Option::is_none")]
    pub manual_command: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feedback: Option<&'static str>,
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

fn path_mentioned(text: &str, path: &str) -> bool {
    fn path_character(character: char) -> bool {
        character.is_alphanumeric() || matches!(character, '_' | '-' | '.' | '/' | '\\')
    }

    text.match_indices(path).any(|(start, _)| {
        let before = text[..start].chars().next_back();
        let rest = &text[start + path.len()..];
        // Sentence punctuation right after a path ("discard README.md.") is a
        // boundary, not part of the path.
        let rest = rest.trim_start_matches(['.', ',', '!', '?']);
        let after = rest.chars().next();
        before.is_none_or(|character| !path_character(character))
            && after.is_none_or(|character| character.is_whitespace() || !path_character(character))
    })
}

impl Proposal {
    /// Like [`Self::validate`], but when validation fails and
    /// `GITHELPER_DUMP_PROPOSAL` is set, writes the raw proposal JSON to that
    /// path for eval/debug inspection. Normal CLI output is unchanged.
    pub fn validate_dumping_on_failure(
        self,
        repository: &Path,
        request: &str,
        history: &[ClarificationTurn],
    ) -> Result<Plan, InvalidPlan> {
        let dump_path = std::env::var_os("GITHELPER_DUMP_PROPOSAL")
            .filter(|path| !path.is_empty())
            .map(std::path::PathBuf::from);
        self.validate_writing_dump_on_failure(repository, request, history, dump_path.as_deref())
    }

    fn validate_writing_dump_on_failure(
        self,
        repository: &Path,
        request: &str,
        history: &[ClarificationTurn],
        dump_path: Option<&Path>,
    ) -> Result<Plan, InvalidPlan> {
        // Serialize before consume so a failed proposal can still be dumped.
        let dump = dump_path.and_then(|path| {
            serde_json::to_vec_pretty(&self)
                .ok()
                .map(|bytes| (path.to_path_buf(), bytes))
        });
        match self.validate_for_request(repository, request, history) {
            Ok(plan) => Ok(plan),
            Err(err) => {
                if let Some((path, bytes)) = dump {
                    let _ = std::fs::write(path, bytes);
                }
                Err(err)
            }
        }
    }

    /// A named file may only come from text the user actually typed. Check the
    /// normalized steps because the model sometimes puts a stage path in `value`.
    pub fn validate_for_request(
        self,
        repository: &Path,
        request: &str,
        history: &[ClarificationTurn],
    ) -> Result<Plan, InvalidPlan> {
        let steps: Vec<_> = self
            .steps
            .iter()
            .cloned()
            .map(normalize_step_fields)
            .collect();
        let plan = self.validate(repository)?;
        if plan.outcome != Outcome::Ready {
            return Ok(plan);
        }
        let unmentioned = steps.iter().any(|step| {
            matches!(
                step.action,
                Action::Discard | Action::Stage | Action::Unstage
            ) && step.paths.iter().any(|path| {
                !path_mentioned(request, path)
                    && !history
                        .iter()
                        .any(|turn| path_mentioned(&turn.answer, path))
            })
        });
        if unmentioned {
            return Ok(state_plan(
                Outcome::NeedsClarification,
                plan.summary,
                "Which exact file paths should be changed? Name each file explicitly.",
            ));
        }
        Ok(plan)
    }

    pub fn validate(mut self, repository: &Path) -> Result<Plan, InvalidPlan> {
        if !text(&self.summary, 1_000) {
            return Err(invalid("missing or invalid summary"));
        }
        if self.outcome != Outcome::Ready {
            // Unsupported refusals sometimes still include leftover steps from the
            // on-device model. Drop them. NeedsClarification with steps still fails
            // closed so a clarification cannot hide commands.
            if self.outcome == Outcome::Unsupported {
                self.steps.clear();
                if self.clarification.trim().is_empty() {
                    self.clarification = self.summary.clone();
                }
            }
            if !text(&self.clarification, 1_000) || !self.steps.is_empty() {
                return Err(invalid("clarification must have no commands"));
            }
            return Ok(state_plan(self.outcome, self.summary, self.clarification));
        }
        if !self.clarification.is_empty() {
            return Err(invalid("ready outcome cannot include clarification"));
        }
        self.steps = self.steps.into_iter().map(normalize_step_fields).collect();
        if self.steps.is_empty() || self.steps.len() > 6 {
            return Err(invalid("expected one to six steps"));
        }
        if self
            .steps
            .iter()
            .filter(|step| matches!(&step.action, Action::UndoLastCommit))
            .count()
            > 1
        {
            return Err(invalid("only the latest commit can be undone"));
        }
        let mut commands = Vec::new();
        let mut changes_repository = false;
        let mut remote_names: Option<HashSet<String>> = None;
        for step in self.steps {
            use Action::*;
            let needs_paths = matches!(&step.action, Stage | Unstage | Discard);
            let needs_value = matches!(
                &step.action,
                CreateBranch | SwitchBranch | Commit | AddRemote | SetRemoteUrl
            );
            let needs_url = matches!(&step.action, AddRemote | SetRemoteUrl);
            let read_only = matches!(
                &step.action,
                Status | Diff | StagedDiff | Log | ListBranches | ListRemotes | ShowReflog
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
            if !needs_url && !step.url.is_empty() {
                return Err(invalid("unexpected url for this action"));
            }
            if matches!(&step.action, AddRemote) && !step.value.trim().is_empty() {
                validate_remote_name(&step.value)?;
                let names = match &mut remote_names {
                    Some(names) => names,
                    None => remote_names.insert(remote_names_in(repository)?),
                };
                if names.contains(&step.value) {
                    return Ok(state_plan(
                        Outcome::NeedsClarification,
                        self.summary.clone(),
                        format!(
                            "Remote {} already exists. Use set-url to change its URL.",
                            step.value
                        ),
                    ));
                }
            }
            if needs_url && (step.value.trim().is_empty() || step.url.trim().is_empty()) {
                return Ok(state_plan(
                    Outcome::NeedsClarification,
                    self.summary.clone(),
                    "What remote name and URL should I use?",
                ));
            }
            if matches!(&step.action, Discard) && step.paths.is_empty() {
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
                if matches!(&step.action, Discard)
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
                ListRemotes => strings(&["remote", "-v"]),
                AddRemote | SetRemoteUrl => {
                    validate_remote_name(&step.value)?;
                    validate_remote_url(&step.url)?;
                    if matches!(step.action, AddRemote) {
                        let names = match &mut remote_names {
                            Some(names) => names,
                            None => remote_names.insert(remote_names_in(repository)?),
                        };
                        if !names.insert(step.value.clone()) {
                            return Ok(state_plan(
                                Outcome::NeedsClarification,
                                self.summary.clone(),
                                format!(
                                    "Remote {} already exists. Use set-url to change its URL.",
                                    step.value
                                ),
                            ));
                        }
                        vec!["remote".into(), "add".into(), step.value, step.url]
                    } else {
                        vec!["remote".into(), "set-url".into(), step.value, step.url]
                    }
                }
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
                            reason: None,
                            manual_command: None,
                            suggestions: vec![],
                            feedback: None,
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
                        // Models sometimes emit commit without a message instead of
                        // needsClarification; treat that as a clarification outcome.
                        return Ok(state_plan(
                            Outcome::NeedsClarification,
                            self.summary.clone(),
                            "What commit message should be used?",
                        ));
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
            reason: None,
            manual_command: None,
            suggestions: vec![],
            feedback: None,
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
    /// Repository edits and ordinary remote sync are not destructive. Recovery
    /// does not move HEAD and is benign. Discard, merge, and rebase return true
    /// here. A soft undo of an unpublished commit does not; a published undo is
    /// marked by `validate_repository`.
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
            // not move HEAD. Classify discard, merge, and rebase as
            // `Safety::Destructive("...")` in this match. Soft undo is benign
            // until repository state shows the commit is already published.
            Self::Status
            | Self::Diff
            | Self::StagedDiff
            | Self::Log
            | Self::ListBranches
            | Self::AddRemote
            | Self::ListRemotes
            | Self::SetRemoteUrl
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
            | Self::Pull
            | Self::UndoLastCommit => Safety::Benign,
            Self::Discard => Safety::Destructive(DISCARD_WARNING),
        }
    }

    fn explanation(&self) -> &'static str {
        match self {
            Self::Status => "Shows short repository status and the current branch.",
            Self::Diff => "Shows unstaged changes to tracked files.",
            Self::StagedDiff => "Shows changes currently staged for commit.",
            Self::Log => "Shows the ten most recent commits.",
            Self::ListBranches => "Lists local branches.",
            Self::AddRemote => "Adds the named remote with the supplied URL.",
            Self::ListRemotes => "Lists configured remotes and their fetch and push URLs.",
            Self::SetRemoteUrl => "Changes the named remote's URL.",
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
    let clarification = clarification.into();
    let unsupported = outcome == Outcome::Unsupported;
    let lower_reason = clarification.to_ascii_lowercase();
    let remove_remote = lower_reason.contains("remote") && lower_reason.contains("remov");
    let suggestion = if remove_remote {
        "Use set-url if you need to change a remote's URL."
    } else if lower_reason.contains("force push") {
        "Use a regular push when history does not need rewriting."
    } else if lower_reason.contains("commit") && lower_reason.contains("undo") {
        "Only the latest commit can be undone."
    } else {
        "Open an issue to request this operation."
    };
    Plan {
        outcome,
        summary,
        clarification: Some(clarification.clone()),
        commands: vec![],
        changes_repository: false,
        reason: unsupported.then_some(clarification),
        // Text-only hint for unsupported remove; never a runnable plan command.
        manual_command: (unsupported && remove_remote).then(|| "git remote remove <name>".into()),
        suggestions: if unsupported {
            vec![suggestion.into()]
        } else {
            vec![]
        },
        feedback: unsupported.then_some(ISSUE_REQUEST),
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
        reason: None,
        manual_command: None,
        suggestions: vec![],
        feedback: None,
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

/// Move mis-filed model output into the fields each action accepts.
/// Newer on-device builds often put stage/discard filenames in `value`.
/// Newer on-device builds often put stage/unstage/discard filenames in `value`
/// instead of `paths`. Move that filename into `paths` when `paths` is empty.
/// For stage/unstage/discard, also drop a stray `value` when `paths` is already set.
fn normalize_step_fields(mut step: Step) -> Step {
    use Action::*;
    match &step.action {
        Stage | Unstage => {
            if step.paths.is_empty() && !step.value.trim().is_empty() {
                step.paths.push(std::mem::take(&mut step.value));
            } else if !step.value.is_empty() {
                step.value.clear();
            }
        }
        Discard => {
            if step.paths.is_empty() && !step.value.trim().is_empty() {
                step.paths.push(std::mem::take(&mut step.value));
            } else if !step.paths.is_empty() {
                step.value.clear();
            }
        }
        _ => {}
    }
    if matches!(&step.action, Stage | Unstage | Discard) {
        for path in &mut step.paths {
            *path = strip_wrapping_quotes(path);
        }
    }
    step
}

fn strip_wrapping_quotes(path: &str) -> String {
    let trimmed = path.trim();
    let pairs = [
        ('"', '"'),
        ('\'', '\''),
        ('`', '`'),
        ('\u{201c}', '\u{201d}'),
        ('"', '\u{201d}'),
        ('\u{201c}', '"'),
    ];
    for (open, close) in pairs {
        if trimmed.len() >= 2 && trimmed.starts_with(open) && trimmed.ends_with(close) {
            return trimmed[open.len_utf8()..trimmed.len() - close.len_utf8()].to_owned();
        }
    }
    trimmed.to_owned()
}

fn validate_branch(branch: &str) -> Result<(), InvalidPlan> {
    validate_ref_name(branch, "branch")
}

fn validate_remote_name(name: &str) -> Result<(), InvalidPlan> {
    validate_ref_name(name, "remote")
}

fn validate_ref_name(branch: &str, kind: &str) -> Result<(), InvalidPlan> {
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
        return Err(InvalidPlan(format!("invalid or unsupported {kind} name")));
    }
    Ok(())
}

fn validate_remote_url(url: &str) -> Result<(), InvalidPlan> {
    if !text(url, 2_048) || url.chars().any(char::is_whitespace) {
        return Err(invalid("invalid or unsupported remote URL"));
    }
    let valid = if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("ssh://"))
    {
        let host = rest.split(['/', '?', '#']).next().unwrap_or("");
        !host.is_empty() && !host.starts_with('-') && !host.contains('\\')
    } else if let Some(rest) = url.strip_prefix("git@") {
        rest.split_once(':').is_some_and(|(host, path)| {
            !host.is_empty()
                && !host.starts_with('-')
                && !host.contains(['/', '\\', '@'])
                && !path.is_empty()
        })
    } else {
        false
    };
    if !valid {
        return Err(invalid("invalid or unsupported remote URL"));
    }
    Ok(())
}

/// Recover explicit remote requests from model proposals without trusting the
/// model to copy a user-supplied URL. Names and URLs always come from `request`.
pub fn rewrite_remote_request(request: &str, mut proposal: Proposal) -> Proposal {
    let words: Vec<&str> = request
        .split_whitespace()
        .map(|word| word.trim_matches(['"', '\'', '`', '(', ')', ',', ';', '.', '!', '?']))
        .collect();
    let lower: Vec<String> = words.iter().map(|word| word.to_ascii_lowercase()).collect();
    let word_is =
        |index: usize, expected: &str| lower.get(index).is_some_and(|word| word == expected);

    if lower
        .windows(2)
        .any(|pair| matches!(pair[0].as_str(), "remove" | "delete") && pair[1] == "remote")
    {
        proposal.outcome = Outcome::Unsupported;
        proposal.summary = "Remove remote".into();
        proposal.clarification = "Removing remotes is not supported.".into();
        proposal.steps.clear();
        return proposal;
    }

    let has_other_remote_intent = lower.iter().any(|word| {
        matches!(
            word.as_str(),
            "add" | "set" | "set-url" | "change" | "remove" | "delete"
        )
    });
    if !has_other_remote_intent
        && (lower
            .windows(2)
            .any(|pair| matches!(pair[0].as_str(), "list" | "show") && pair[1] == "remotes")
            || lower.windows(3).any(|pair| {
                matches!(pair[0].as_str(), "list" | "show")
                    && pair[1] == "my"
                    && pair[2] == "remotes"
            }))
    {
        proposal.outcome = Outcome::Ready;
        proposal.summary = "List remotes".into();
        proposal.clarification.clear();
        proposal.steps = vec![step(
            Action::ListRemotes,
            String::new(),
            Vec::new(),
            String::new(),
            String::new(),
        )];
        return proposal;
    }

    let set_name_index = (0..words.len()).find_map(|index| {
        if word_is(index, "set-url") {
            Some(index + 1)
        } else if (word_is(index, "set") || word_is(index, "change"))
            && word_is(index + 1, "remote")
        {
            Some(index + 2)
        } else {
            None
        }
    });
    if let Some(name_index) = set_name_index {
        if let (Some(name), Some(url)) = (
            remote_name_at(&words, name_index),
            remote_url_after(&words, name_index + 1),
        ) {
            proposal.outcome = Outcome::Ready;
            proposal.summary = format!("Change remote {name} URL");
            proposal.clarification.clear();
            proposal.steps = vec![step(
                Action::SetRemoteUrl,
                name.to_owned(),
                Vec::new(),
                String::new(),
                url.to_owned(),
            )];
        }
        return proposal;
    }

    let add_remote_index = (0..words.len()).find_map(|index| {
        if !word_is(index, "add") {
            return None;
        }
        let mut next = index + 1;
        if word_is(next, "a") {
            next += 1;
        }
        if word_is(next, "new") {
            next += 1;
        }
        word_is(next, "remote").then_some(next)
    });
    if let Some(remote_index) = add_remote_index {
        let mut name_index = remote_index + 1;
        if word_is(name_index, "named") {
            name_index += 1;
        }
        if let (Some(name), Some(url)) = (
            remote_name_at(&words, name_index),
            remote_url_after(&words, name_index + 1),
        ) {
            proposal.outcome = Outcome::Ready;
            proposal.summary = format!("Add remote {name}");
            proposal.clarification.clear();
            proposal.steps = vec![step(
                Action::AddRemote,
                name.to_owned(),
                Vec::new(),
                String::new(),
                url.to_owned(),
            )];
        } else if remote_url_after(&words, remote_index + 1).is_none() {
            proposal.outcome = Outcome::NeedsClarification;
            proposal.summary = "Add remote".into();
            proposal.clarification = "What remote name and URL should I use?".into();
            proposal.steps.clear();
        }
    }
    proposal
}

fn remote_name_at<'a>(words: &'a [&str], index: usize) -> Option<&'a str> {
    let name = *words.get(index)?;
    if matches!(
        name.to_ascii_lowercase().as_str(),
        "url" | "with" | "to" | "for"
    ) || validate_remote_name(name).is_err()
    {
        return None;
    }
    Some(name)
}

fn remote_url_after<'a>(words: &'a [&str], index: usize) -> Option<&'a str> {
    let mut next = index;
    if words.get(next)?.eq_ignore_ascii_case("with") {
        next += 1;
    }
    if words.get(next)?.eq_ignore_ascii_case("url") {
        next += 1;
    }
    if words
        .get(next)
        .is_some_and(|word| word.eq_ignore_ascii_case("to"))
    {
        next += 1;
    }
    let url = *words.get(next)?;
    validate_remote_url(url).ok()?;
    Some(url)
}

fn remote_names_in(repository: &Path) -> Result<HashSet<String>, InvalidPlan> {
    let output = git(repository, &["remote"])
        .map_err(|_| invalid("could not inspect configured remotes"))?;
    if !output.status.success() {
        return Err(invalid("could not inspect configured remotes"));
    }
    let names = String::from_utf8(output.stdout)
        .map_err(|_| invalid("configured remote names are not UTF-8"))?;
    Ok(names.lines().map(str::to_owned).collect())
}

/// Accept 7–40 hexadecimal characters. Git object names are case-insensitive,
/// so mixed case is preserved and passed through to the printed command.
fn validate_sha(sha: &str) -> Result<(), InvalidPlan> {
    if !(7..=40).contains(&sha.len()) || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(invalid("commit SHA must be 7 to 40 hexadecimal characters"));
    }
    Ok(())
}

/// When the user named a commit SHA and the model answered with a lone
/// `showReflog` recovery plan, rewrite to `createBranchAt` for that SHA.
/// Deterministic: never invents a SHA that was not in the request text.
pub fn rewrite_named_sha_recovery(request: &str, mut proposal: Proposal) -> Proposal {
    if proposal.outcome != Outcome::Ready || proposal.steps.len() != 1 {
        return proposal;
    }
    if !matches!(proposal.steps[0].action, Action::ShowReflog) {
        return proposal;
    }
    let Some(sha) = first_hex_sha_in_request(request) else {
        return proposal;
    };
    proposal.steps[0] = Step {
        action: Action::CreateBranchAt {
            name: format!("recovered/{sha}"),
            sha: sha.clone(),
        },
        value: String::new(),
        paths: Vec::new(),
        sha: String::new(),
        url: String::new(),
    };
    if !text(&proposal.summary, 1_000) {
        proposal.summary = format!("Recover commit {sha}");
    }
    proposal
}

/// First 7–40 character hexadecimal token in `request` bounded by non-hex characters.
fn first_hex_sha_in_request(request: &str) -> Option<String> {
    let bytes = request.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_hexdigit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                i += 1;
            }
            let len = i - start;
            if (7..=40).contains(&len) {
                // Reject if adjacent to a longer hex run already consumed; token is exact.
                return Some(request[start..i].to_string());
            }
        } else {
            i += 1;
        }
    }
    None
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
                url: String::new(),
            }],
        }
    }

    #[test]
    fn trailing_sentence_punctuation_is_a_path_boundary() {
        assert!(path_mentioned("discard README.md.", "README.md"));
        assert!(path_mentioned("discard README.md, please", "README.md"));
        assert!(path_mentioned("stage README.md!", "README.md"));
        assert!(path_mentioned("unstage README.md?", "README.md"));
        assert!(!path_mentioned("discard README.md.bak", "README.md"));
        assert!(!path_mentioned("discard README.mdx", "README.md"));
    }

    #[test]
    fn named_paths_must_appear_in_user_text() {
        let request = "revert my changes to README.md";
        let rejected = proposal(Action::Discard, "", &["README.md", "src/main.rs"])
            .validate_for_request(Path::new("."), request, &[])
            .unwrap();
        assert_eq!(rejected.outcome, Outcome::NeedsClarification);
        assert!(rejected.commands.is_empty());
        assert!(!rejected.changes_repository);

        let accepted = proposal(Action::Discard, "", &["README.md"])
            .validate_for_request(Path::new("."), request, &[])
            .unwrap();
        assert_eq!(accepted.outcome, Outcome::Ready);
        assert_eq!(
            accepted.commands[0].args,
            strings(&["--literal-pathspecs", "restore", "--", "README.md"])
        );

        for (action, request) in [
            (Action::Stage, "stage README.md"),
            (Action::Unstage, "unstage README.md"),
        ] {
            let rejected = proposal(action, "", &["src/main.rs"])
                .validate_for_request(Path::new("."), request, &[])
                .unwrap();
            assert_eq!(rejected.outcome, Outcome::NeedsClarification);
            assert!(rejected.commands.is_empty());
        }

        let stage_all = proposal(Action::StageAll, "", &[])
            .validate_for_request(Path::new("."), "stage all changes", &[])
            .unwrap();
        assert_eq!(stage_all.outcome, Outcome::Ready);
        assert_eq!(stage_all.commands[0].args, strings(&["add", "--all"]));
    }

    #[test]
    fn path_scope_uses_typed_answers_and_normalized_steps() {
        let history = [ClarificationTurn {
            question: "Which file?".into(),
            answer: "src/main.rs".into(),
        }];
        let accepted = proposal(Action::Unstage, "", &["src/main.rs"])
            .validate_for_request(Path::new("."), "unstage a file", &history)
            .unwrap();
        assert_eq!(accepted.outcome, Outcome::Ready);

        let answer_only_in_question = [ClarificationTurn {
            question: "Did you mean src/main.rs?".into(),
            answer: "yes".into(),
        }];
        let rejected = proposal(Action::Discard, "", &["src/main.rs"])
            .validate_for_request(
                Path::new("."),
                "revert my changes",
                &answer_only_in_question,
            )
            .unwrap();
        assert_eq!(rejected.outcome, Outcome::NeedsClarification);

        let misfiled = proposal(Action::Stage, "src/main.rs", &[])
            .validate_for_request(Path::new("."), "stage README.md", &[])
            .unwrap();
        assert_eq!(misfiled.outcome, Outcome::NeedsClarification);
        assert!(!path_mentioned("README.md.backup", "README.md"));
        assert!(!path_mentioned("readme.md", "README.md"));
        assert!(path_mentioned("stage 'README.md', please", "README.md"));
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
            (UndoLastCommit, "", &[][..], "nothing is lost", true),
            (Discard, "", &["README.md"][..], "named paths", true),
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
            url: String::new(),
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
        let mut p = proposal(Action::Status, "", &[]);
        p.outcome = Outcome::NeedsClarification;
        assert!(p.validate(Path::new(".")).is_err());
        for outcome in [Outcome::NeedsClarification, Outcome::Unsupported] {
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
    fn stage_moves_misfiled_value_into_paths() {
        let plan = proposal(Action::Stage, "notes with spaces.txt", &[])
            .validate(Path::new("."))
            .unwrap();
        assert_eq!(
            plan.commands[0].args,
            strings(&["--literal-pathspecs", "add", "--", "notes with spaces.txt"])
        );
        let literal = proposal(Action::Stage, "--all", &[])
            .validate(Path::new("."))
            .unwrap();
        assert_eq!(
            literal.commands[0].args,
            strings(&["--literal-pathspecs", "add", "--", "--all"])
        );
        let redundant = proposal(Action::Stage, "README.md", &["README.md"])
            .validate(Path::new("."))
            .unwrap();
        assert_eq!(
            redundant.commands[0].args,
            strings(&["--literal-pathspecs", "add", "--", "README.md"])
        );
    }

    #[test]
    fn unsupported_outcome_drops_leftover_steps() {
        let mut p = proposal(Action::Push, "", &[]);
        p.outcome = Outcome::Unsupported;
        p.summary = "Force push is not supported.".into();
        p.clarification = "  ".into();
        let plan = p.validate(Path::new(".")).unwrap();
        assert!(plan.commands.is_empty());
        assert_eq!(plan.outcome, Outcome::Unsupported);
        assert_eq!(
            plan.clarification.as_deref(),
            Some("Force push is not supported.")
        );
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
    fn dumps_raw_proposal_json_when_validation_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("raw-model-proposal.json");
        let bad = Proposal {
            outcome: Outcome::Ready,
            summary: "stage escape".into(),
            clarification: String::new(),
            steps: vec![Step {
                action: Action::Stage,
                value: String::new(),
                paths: vec!["../secret".into()],
                sha: String::new(),
                url: String::new(),
            }],
        };
        let err = bad
            .validate_writing_dump_on_failure(Path::new("."), "stage README.md", &[], Some(&path))
            .expect_err("escape path must fail validation");
        assert!(
            err.to_string()
                .contains("paths must stay inside the repository")
        );
        let dumped = std::fs::read_to_string(&path).expect("dump written");
        let value: serde_json::Value = serde_json::from_str(&dumped).expect("valid json");
        assert_eq!(value["steps"][0]["paths"][0], "../secret");
        // Successful validation must not write the dump file.
        let ok_path = dir.path().join("should-not-exist.json");
        proposal(Action::Stage, "", &[":(glob)*"])
            .validate_writing_dump_on_failure(Path::new("."), "stage :(glob)*", &[], Some(&ok_path))
            .expect("literal pathspec filename is valid");
        assert!(!ok_path.exists(), "success must not create dump");
        // No dump path: failure still returns the same error and writes nothing.
        let missing = dir.path().join("never.json");
        let err = proposal(Action::Stage, "", &["../secret"])
            .validate_writing_dump_on_failure(Path::new("."), "stage ../secret", &[], None)
            .expect_err("escape path must fail");
        assert!(
            err.to_string()
                .contains("paths must stay inside the repository")
        );
        assert!(!missing.exists());
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
        let empty_commit = proposal(Action::Commit, "", &[])
            .validate(Path::new("."))
            .unwrap();
        assert_eq!(empty_commit.outcome, Outcome::NeedsClarification);
        assert!(empty_commit.commands.is_empty());
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

    fn remote_proposal(action: Action, name: &str, url: &str) -> Proposal {
        let mut proposed = proposal(action, name, &[]);
        proposed.steps[0].url = url.into();
        proposed
    }

    #[test]
    fn remote_commands_have_exact_arguments_and_state_flags() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--quiet"])
                .arg(dir.path())
                .status()
                .unwrap()
                .success()
        );
        for (action, name, url, args, mutates) in [
            (
                Action::ListRemotes,
                "",
                "",
                strings(&["remote", "-v"]),
                false,
            ),
            (
                Action::AddRemote,
                "upstream",
                "https://example.com/repo.git",
                strings(&["remote", "add", "upstream", "https://example.com/repo.git"]),
                true,
            ),
            (
                Action::SetRemoteUrl,
                "origin",
                "git@example.com:team/repo.git",
                strings(&[
                    "remote",
                    "set-url",
                    "origin",
                    "git@example.com:team/repo.git",
                ]),
                true,
            ),
        ] {
            let plan = remote_proposal(action, name, url)
                .validate(dir.path())
                .unwrap();
            assert_eq!(plan.outcome, Outcome::Ready);
            assert_eq!(plan.commands[0].args, args);
            assert_eq!(plan.changes_repository, mutates);
            assert!(!plan.commands[0].destructive);
        }
    }

    #[test]
    fn remote_inputs_fail_closed_or_clarify() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--quiet"])
                .arg(dir.path())
                .status()
                .unwrap()
                .success()
        );
        for url in [
            "http://example.com/repo",
            "file:///tmp/repo",
            "git@host",
            "https://",
            "ssh://",
            "https://host/a b",
        ] {
            assert!(
                remote_proposal(Action::AddRemote, "upstream", url)
                    .validate(dir.path())
                    .is_err(),
                "{url}"
            );
        }
        for name in ["", "-origin", "bad name", "bad\nname", "a..b", "a/.b"] {
            let result = remote_proposal(Action::AddRemote, name, "https://example.com/repo.git")
                .validate(dir.path());
            if name.is_empty() {
                assert_eq!(result.unwrap().outcome, Outcome::NeedsClarification);
            } else {
                assert!(result.is_err(), "{name}");
            }
        }
        for action in [Action::AddRemote, Action::SetRemoteUrl] {
            assert_eq!(
                remote_proposal(action, "origin", "")
                    .validate(dir.path())
                    .unwrap()
                    .outcome,
                Outcome::NeedsClarification
            );
        }
        let output = Command::new("git")
            .args(["remote", "add", "origin", "https://example.com/old.git"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(output.status.success());
        let plan = remote_proposal(Action::AddRemote, "origin", "https://example.com/new.git")
            .validate(dir.path())
            .unwrap();
        assert_eq!(plan.outcome, Outcome::NeedsClarification);
        assert!(plan.clarification.unwrap().contains("set-url"));
        assert!(plan.commands.is_empty());
        let without_url = remote_proposal(Action::AddRemote, "origin", "")
            .validate(dir.path())
            .unwrap();
        assert!(without_url.clarification.unwrap().contains("set-url"));
    }

    #[test]
    fn remove_remote_proposal_stays_unsupported() {
        let mut proposed = proposal(Action::ListRemotes, "", &[]);
        proposed.outcome = Outcome::Unsupported;
        proposed.clarification = "Removing remotes is not supported.".into();
        let plan = proposed.validate(Path::new(".")).unwrap();
        assert_eq!(plan.outcome, Outcome::Unsupported);
        assert!(plan.commands.is_empty());
        assert_eq!(
            plan.reason.as_deref(),
            Some("Removing remotes is not supported.")
        );
        assert_eq!(
            plan.manual_command.as_deref(),
            Some("git remote remove <name>")
        );
        assert_eq!(plan.feedback, Some(ISSUE_REQUEST));
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
        AddRemote => "upstream", &[];
        ListRemotes => "", &[];
        SetRemoteUrl => "upstream", &[];
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
        UndoLastCommit => "", &[];
        Discard => "", &["README.md"];
    }

    #[test]
    fn safety_metadata_is_paired_for_every_action() {
        each_action(|action, value, paths| {
            every_variant_is_listed(&action);
            assert_eq!(
                action.destructive(),
                action.warning().is_some(),
                "{action:?}"
            );
            let destructive = action.destructive();
            let warning = action.warning();
            let mut proposed = proposal(action.clone(), value, paths);
            if matches!(action, Action::AddRemote | Action::SetRemoteUrl) {
                proposed.steps[0].url = "https://example.com/repo.git".into();
            }
            let command = &proposed.validate(Path::new(".")).unwrap().commands[0];
            assert_eq!(command.destructive, destructive);
            assert_eq!(command.warning, warning);
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

    fn recovery_commit_repo() -> (tempfile::TempDir, String) {
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
    fn remote_set_url_rewrites_wrong_unsupported_proposal() {
        let mut wrong = proposal(Action::Status, "", &[]);
        wrong.outcome = Outcome::Unsupported;
        wrong.clarification = "URL unsupported".into();
        let rewritten = rewrite_remote_request(
            "set remote origin URL to https://example.com/new.git",
            wrong,
        );
        assert_eq!(rewritten.outcome, Outcome::Ready);
        assert!(rewritten.clarification.is_empty());
        assert_eq!(rewritten.steps.len(), 1);
        let step = &rewritten.steps[0];
        assert!(matches!(step.action, Action::SetRemoteUrl));
        assert_eq!(step.value, "origin");
        assert_eq!(step.url, "https://example.com/new.git");
        assert!(step.paths.is_empty());
        assert!(step.sha.is_empty());
        for request in [
            "set-url origin ssh://example.com/new.git",
            "change remote origin to git@example.com:new.git",
        ] {
            let rewritten = rewrite_remote_request(request, proposal(Action::Status, "", &[]));
            assert!(matches!(rewritten.steps[0].action, Action::SetRemoteUrl));
            assert_eq!(rewritten.steps[0].value, "origin");
        }
    }

    #[test]
    fn remote_add_rewrites_name_and_url_from_request() {
        for request in [
            "add remote upstream https://example.com/new.git",
            "add a remote named upstream with url https://example.com/new.git",
        ] {
            let rewritten = rewrite_remote_request(request, proposal(Action::Status, "", &[]));
            assert_eq!(rewritten.outcome, Outcome::Ready);
            assert_eq!(rewritten.steps.len(), 1);
            assert!(matches!(rewritten.steps[0].action, Action::AddRemote));
            assert_eq!(rewritten.steps[0].value, "upstream");
            assert_eq!(rewritten.steps[0].url, "https://example.com/new.git");
        }
    }

    #[test]
    fn remote_add_without_url_asks_for_name_and_url() {
        for request in ["add remote", "add a remote", "I want to add new remote"] {
            let rewritten = rewrite_remote_request(request, proposal(Action::Status, "", &[]));
            assert_eq!(rewritten.outcome, Outcome::NeedsClarification);
            assert_eq!(
                rewritten.clarification,
                "What remote name and URL should I use?"
            );
            assert!(rewritten.steps.is_empty());
        }
    }

    #[test]
    fn remote_remove_is_unsupported_even_if_model_is_ready() {
        for request in ["remove remote origin", "delete remote origin"] {
            let rewritten = rewrite_remote_request(request, proposal(Action::Status, "", &[]));
            assert_eq!(rewritten.outcome, Outcome::Unsupported);
            assert_eq!(
                rewritten.clarification,
                "Removing remotes is not supported."
            );
            assert!(rewritten.steps.is_empty());
        }
    }

    #[test]
    fn remote_list_rewrites_to_one_list_step() {
        for request in ["list remotes", "show remotes", "list my remotes"] {
            let rewritten = rewrite_remote_request(request, proposal(Action::Status, "", &[]));
            assert_eq!(rewritten.outcome, Outcome::Ready);
            assert_eq!(rewritten.steps.len(), 1);
            let step = &rewritten.steps[0];
            assert!(matches!(step.action, Action::ListRemotes));
            assert!(step.value.is_empty());
            assert!(step.url.is_empty());
        }
    }

    #[test]
    fn unrelated_request_keeps_original_proposal() {
        let original = proposal(Action::Status, "", &[]);
        let expected = serde_json::to_value(&original).unwrap();
        let actual = rewrite_remote_request("show my repository status", original);
        assert_eq!(serde_json::to_value(actual).unwrap(), expected);
    }

    #[test]
    fn named_sha_in_request_rewrites_lone_show_reflog_to_create_branch_at() {
        let request = "recover the commit abc1234";
        let mut show = proposal(Action::ShowReflog, "", &[]);
        show.summary = "Inspect the reflog".into();
        let rewritten = rewrite_named_sha_recovery(request, show);
        assert!(matches!(
            &rewritten.steps[0].action,
            Action::CreateBranchAt { name, sha }
                if name == "recovered/abc1234" && sha == "abc1234"
        ));
        // Requests without a SHA stay on showReflog.
        let kept =
            rewrite_named_sha_recovery("I lost a commit", proposal(Action::ShowReflog, "", &[]));
        assert!(matches!(kept.steps[0].action, Action::ShowReflog));
        // Already-correct createBranchAt is left alone.
        let ready = recovery("abc1234", "abc1234");
        let same = rewrite_named_sha_recovery(request, ready.clone());
        assert_eq!(
            format!("{:?}", same.steps[0].action),
            format!("{:?}", ready.steps[0].action)
        );
        // Hex shorter than 7 is ignored.
        let short =
            rewrite_named_sha_recovery("recover ab12", proposal(Action::ShowReflog, "", &[]));
        assert!(matches!(short.steps[0].action, Action::ShowReflog));
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
        let (dir, sha) = recovery_commit_repo();
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
        let (dir, sha) = recovery_commit_repo();
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
            .validate(Path::new("."))
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
                    .validate(Path::new("."))
                    .is_err(),
                "{value}"
            );
        }
        assert!(
            proposal(Action::UndoLastCommit, "", &["README.md"])
                .validate(Path::new("."))
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
            sha: String::new(),
            url: String::new(),
        });
        assert!(repeated.validate(Path::new(".")).is_err());
    }

    #[test]
    fn undo_root_commit_clarifies_and_published_commit_warns() {
        let empty = tempfile::tempdir().unwrap();
        git_ok(empty.path(), &["init", "--initial-branch=main"]);
        let plan = validate_repository(
            proposal(Action::UndoLastCommit, "", &[])
                .validate(Path::new("."))
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
                .validate(Path::new("."))
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
                .validate(Path::new("."))
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
                .validate(Path::new("."))
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
                .validate(Path::new("."))
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
                .validate(Path::new("."))
                .unwrap(),
            published.path(),
        )
        .unwrap();
        assert!(!plan.commands[0].destructive);
        assert_eq!(plan.commands[0].warning, None);
    }

    #[test]
    fn discard_validates_paths_and_marks_permanent_loss() {
        let plan = proposal(Action::Discard, "", &[])
            .validate(Path::new("."))
            .unwrap();
        assert_eq!(plan.outcome, Outcome::NeedsClarification);
        assert!(plan.commands.is_empty());
        assert!(!plan.changes_repository);
        assert!(plan.clarification.unwrap().to_lowercase().contains("file"));

        for path in [".", "./", "./."] {
            let plan = proposal(Action::Discard, "", &[path])
                .validate(Path::new("."))
                .unwrap();
            assert_eq!(plan.outcome, Outcome::Unsupported, "{path}");
            assert!(plan.commands.is_empty(), "{path}");
            assert!(!plan.changes_repository, "{path}");
        }
        let nested = proposal(Action::Discard, "", &["./README.md"])
            .validate(Path::new("."))
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
                proposal(Action::Discard, "", &[path])
                    .validate(Path::new("."))
                    .is_err(),
                "{path}"
            );
        }
        let stray_value = proposal(Action::Discard, "--hard", &["README.md"])
            .validate(Path::new("."))
            .unwrap();
        assert_eq!(stray_value.outcome, Outcome::Ready);
        assert_eq!(
            stray_value.commands[0].args,
            strings(&["--literal-pathspecs", "restore", "--", "README.md"])
        );
        assert!(stray_value.commands[0].destructive);
        assert_eq!(stray_value.commands[0].warning, Some(DISCARD_WARNING));

        let plan = proposal(Action::Discard, "", &["README.md", "a b.txt"])
            .validate(Path::new("."))
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
                .validate(Path::new("."))
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
                .validate(Path::new("."))
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
                .validate(Path::new("."))
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
                .validate(Path::new("."))
                .unwrap(),
            dir.path(),
        )
        .unwrap();
        assert_eq!(missing.outcome, Outcome::NeedsClarification);
        assert!(missing.clarification.unwrap().contains("missing.txt"));
        let partial = validate_repository(
            proposal(Action::Discard, "", &["file.txt", "missing.txt"])
                .validate(Path::new("."))
                .unwrap(),
            dir.path(),
        )
        .unwrap();
        assert_eq!(partial.outcome, Outcome::NeedsClarification);
        assert!(partial.commands.is_empty());
        assert!(partial.clarification.unwrap().contains("missing.txt"));

        let literal = validate_repository(
            proposal(Action::Discard, "", &["--all"])
                .validate(Path::new("."))
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

#[cfg(test)]
#[path = "recovery_edge_tests.rs"]
mod recovery_edge_tests;
