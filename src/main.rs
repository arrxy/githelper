mod commit_message;
mod doctor;
mod interaction;
mod model;
mod plan;
mod repository;
mod setup_text;

use clap::Parser;
use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
    time::Instant,
};

fn timings_enabled() -> bool {
    matches!(
        std::env::var("GITHELPER_TIMINGS").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    )
}

fn timing(label: &str, start: Instant) {
    if timings_enabled() {
        eprintln!(
            "githelper.timing {label}={:.3}ms",
            start.elapsed().as_secs_f64() * 1000.0
        );
    }
}

#[derive(Parser)]
#[command(
    version,
    about = "Plain language → Git commands, using Apple's on-device model.",
    after_help = "Examples:\n  githelper \"show my staged changes\"\n  githelper --repo ~/code/app \"create a branch called fix/login\"\n  githelper --json \"stage README.md and commit with message update docs\"\n  githelper \"undo my last commit\"\n  githelper \"discard changes to README.md\"\n  githelper --doctor\n  githelper --interactive \"commit my changes\"\n  githelper --draft-commit --json\n  githelper \"I lost a commit\"\n\nCommands are printed for review. Nothing is executed."
)]
struct Cli {
    /// Request in plain language (quotes recommended)
    request: Vec<String>,
    /// Repository directory; defaults to the current directory
    #[arg(short = 'C', long, default_value = ".")]
    repo: PathBuf,
    /// Print structured JSON, including command argument arrays
    #[arg(long)]
    json: bool,
    /// Check macOS, Xcode, Apple Intelligence, Git, and the current repository
    #[arg(long, conflicts_with = "request")]
    doctor: bool,
    /// Ask up to three clarification questions using terminal input
    #[arg(long, conflicts_with_all = ["json", "doctor", "draft_commit"])]
    interactive: bool,
    /// Draft a commit subject from the staged patch without committing
    #[arg(long, conflicts_with_all = ["request", "doctor", "interactive"])]
    draft_commit: bool,
}

fn main() {
    let process_started = Instant::now();
    let cli = Cli::parse();
    timing("cli_parse_ms", process_started);
    if let Err(error) = run(cli) {
        eprintln!("githelper: {error}");
        std::process::exit(1);
    }
    timing("process_wall_ms", process_started);
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let run_started = Instant::now();
    let bridge = model::AppleModel;
    if cli.doctor {
        let report = doctor::diagnose(&cli.repo);
        if cli.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            print!("{}", doctor::render_text(&report));
        }
        if !report.ok {
            return Err(report.failure_summary().into());
        }
        return Ok(());
    }
    if cli.draft_commit {
        let diff = repository::staged_diff(&cli.repo)?;
        let message = bridge.draft_commit(&diff)?;
        render_commit(&mut io::stdout().lock(), &message.subject, cli.json)?;
        return Ok(());
    }
    if cli.interactive && !io::stdin().is_terminal() {
        return Err("--interactive requires terminal input".into());
    }
    let request = cli.request.join(" ");
    if request.trim().is_empty() {
        return Err("provide a request, for example: githelper \"show my changes\"".into());
    }
    if request.len() > 4_000 || request.chars().any(char::is_control) {
        return Err("request must be at most 4,000 bytes and contain no control characters".into());
    }
    let context_started = Instant::now();
    let context = repository::inspect(&cli.repo)?;
    timing("repo_context_ms", context_started);
    let repository = PathBuf::from(&context.root);
    let plan = if cli.interactive {
        let result = interaction::clarify(
            |history| {
                let proposal = bridge.translate(&request, &context, history)?;
                Ok(plan::rewrite_named_sha_recovery(&request, proposal))
            },
            &repository,
            &request,
            &mut io::stdin().lock(),
            &mut io::stderr().lock(),
        )?;
        let Some(plan) = result else { return Ok(()) };
        plan
    } else {
        let model_started = Instant::now();
        let proposal = bridge.translate(&request, &context, &[])?;
        timing("model_call_ms", model_started);
        let proposal = plan::rewrite_named_sha_recovery(&request, proposal);
        let validate_started = Instant::now();
        let plan = proposal.validate_dumping_on_failure(&repository, &request, &[])?;
        timing("plan_validation_ms", validate_started);
        plan
    };
    let repo_validate_started = Instant::now();
    let plan = plan::validate_repository(plan, std::path::Path::new(&context.root))?;
    timing("repo_validation_ms", repo_validate_started);
    render_plan(&mut io::stdout().lock(), &plan, &context.root, cli.json)?;
    timing("total_after_startup_ms", run_started);
    Ok(())
}

fn render_commit(output: &mut impl Write, subject: &str, json: bool) -> io::Result<()> {
    if json {
        writeln!(
            output,
            "{}",
            serde_json::json!({"kind": "commit_message", "subject": subject})
        )
    } else {
        writeln!(output, "{subject}")
    }?;
    output.flush()
}

fn render_plan(
    output: &mut impl Write,
    plan: &plan::Plan,
    root: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if json {
        writeln!(output, "{}", serde_json::to_string_pretty(plan)?)?;
    } else if let Some(question) = &plan.clarification {
        let label = if matches!(plan.outcome, plan::Outcome::Unsupported) {
            "Unsupported request"
        } else {
            "Clarification needed"
        };
        writeln!(output, "{label}: {question}")?;
    } else {
        writeln!(output, "{}\n", plan.summary)?;
        for command in &plan.commands {
            writeln!(output, "{}", command.explanation)?;
            if let Some(warning) = command.warning {
                writeln!(output, "  Warning: {warning}")?;
            }
            writeln!(output, "  {}", command.display())?;
        }
        writeln!(
            output,
            "\nReview these commands before running them in {root}."
        )?;
        if plan.changes_repository {
            writeln!(
                output,
                "This plan changes repository state or contacts a remote."
            )?;
        }
    }
    output.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_pairs_explanation_with_command_and_json() {
        let proposal: plan::Proposal = serde_json::from_value(serde_json::json!({
            "outcome":"ready", "summary":"Show changes", "clarification":"",
            "steps":[{"action":"diff","value":"","paths":[]}]
        }))
        .unwrap();
        let plan = proposal.validate(std::path::Path::new(".")).unwrap();
        let mut output = Vec::new();
        render_plan(&mut output, &plan, "/repo", false).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains(&format!(
            "{}\n  {}",
            plan.commands[0].explanation,
            plan.commands[0].display()
        )));
        assert!(!text.contains("Warning:"));
        let mut output = Vec::new();
        render_plan(&mut output, &plan, "/repo", true).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(
            json["commands"][0]["explanation"],
            plan.commands[0].explanation
        );
        assert!(json["commands"][0].get("destructive").is_none());
        assert!(json["commands"][0].get("warning").is_none());
    }

    #[test]
    fn destructive_step_shows_warning_in_text_and_json() {
        let plan = plan::Plan {
            outcome: plan::Outcome::Ready,
            summary: "Discard local edits".into(),
            clarification: None,
            commands: vec![
                plan::GitCommand {
                    program: "git",
                    explanation: "Discards uncommitted changes in the working tree.".into(),
                    args: vec![
                        "restore".into(),
                        "--worktree".into(),
                        "--".into(),
                        ".".into(),
                    ],
                    destructive: true,
                    warning: Some("This permanently discards uncommitted work."),
                },
                plan::GitCommand {
                    program: "git",
                    explanation: "Shows short repository status and the current branch.".into(),
                    args: vec!["status".into(), "--short".into(), "--branch".into()],
                    destructive: false,
                    warning: None,
                },
            ],
            changes_repository: true,
        };
        let mut output = Vec::new();
        render_plan(&mut output, &plan, "/repo", false).unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "\
Discard local edits

Discards uncommitted changes in the working tree.
  Warning: This permanently discards uncommitted work.
  git restore --worktree -- .
Shows short repository status and the current branch.
  git status --short --branch

Review these commands before running them in /repo.
This plan changes repository state or contacts a remote.
"
        );
        let mut output = Vec::new();
        render_plan(&mut output, &plan, "/repo", true).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["commands"][0]["destructive"], true);
        assert_eq!(
            json["commands"][0]["warning"],
            "This permanently discards uncommitted work."
        );
        assert!(json["commands"][1].get("destructive").is_none());
        assert!(json["commands"][1].get("warning").is_none());
        assert_eq!(
            json["commands"][1]["args"],
            serde_json::json!(["status", "--short", "--branch"])
        );
    }

    #[test]
    fn draft_output_has_distinct_json_shape() {
        let mut output = Vec::new();
        render_commit(&mut output, "Fix login", true).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"kind":"commit_message","subject":"Fix login"})
        );
        let mut output = Vec::new();
        render_commit(&mut output, "Fix login", false).unwrap();
        assert_eq!(output, b"Fix login\n");
    }

    #[test]
    fn recovery_reflog_renders_inspection_without_reset_or_warning() {
        let proposal: plan::Proposal = serde_json::from_value(serde_json::json!({
            "outcome": "ready",
            "summary": "Recover the lost commit without moving the current branch.",
            "clarification": "",
            "steps": [{"action": "showReflog", "value": "", "paths": []}]
        }))
        .unwrap();
        let plan = proposal.validate(std::path::Path::new(".")).unwrap();
        let mut output = Vec::new();
        render_plan(&mut output, &plan, "/repo", false).unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "\
Recover the lost commit without moving the current branch.

Shows the 20 most recent reflog entries. Find the lost commit or deleted branch in that list and note its SHA, then ask to recover that SHA.
  git reflog -n 20

Review these commands before running them in /repo.
"
        );
        let mut output = Vec::new();
        render_plan(&mut output, &plan, "/repo", true).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(
            json["commands"][0]["args"],
            serde_json::json!(["reflog", "-n", "20"])
        );
        assert!(json["commands"][0].get("destructive").is_none());
        assert!(json["commands"][0].get("warning").is_none());
        assert_eq!(json["changes_repository"], false);
        assert!(!json.to_string().contains("reset"));
    }

    #[test]
    fn missing_commit_recovery_renders_clarification_without_commands() {
        let directory = tempfile::tempdir().unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "--quiet", "--initial-branch=main"])
                .arg(directory.path())
                .status()
                .unwrap()
                .success()
        );
        let proposal: plan::Proposal = serde_json::from_value(serde_json::json!({
            "outcome": "ready",
            "summary": "Recover the lost commit",
            "clarification": "",
            "steps": [{
                "action": "createBranchAt",
                "value": "login",
                "paths": [],
                "sha": "abc1234"
            }]
        }))
        .unwrap();
        let plan = proposal.validate(directory.path()).unwrap();
        let mut output = Vec::new();
        render_plan(&mut output, &plan, "/repo", false).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert_eq!(
            text,
            "Clarification needed: Commit abc1234 does not resolve to an existing commit. Which commit SHA should be recovered? Use `git reflog -n 20` to find it.\n"
        );
        assert!(!text.contains("reset"));
        assert!(!text.contains("Review these commands"));
        let mut output = Vec::new();
        render_plan(&mut output, &plan, "/repo", true).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["outcome"], "needsClarification");
        assert_eq!(json["commands"], serde_json::json!([]));
        assert_eq!(json["changes_repository"], false);
        assert!(json.get("destructive").is_none());
        assert!(!json.to_string().contains("reset"));
        let listed = std::process::Command::new("git")
            .args(["for-each-ref", "refs/heads"])
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert!(listed.status.success());
        assert!(listed.stdout.is_empty());
    }

    #[test]
    fn discard_plan_renders_permanent_loss_warning() {
        let proposal: plan::Proposal = serde_json::from_value(serde_json::json!({
            "outcome": "ready",
            "summary": "Discard README edits",
            "clarification": "",
            "steps": [{"action": "discard", "value": "", "paths": ["README.md"]}]
        }))
        .unwrap();
        let plan = proposal.validate(std::path::Path::new(".")).unwrap();
        let mut output = Vec::new();
        render_plan(&mut output, &plan, "/repo", false).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("lost permanently"));
        assert!(text.contains("Warning: "));
        assert!(text.contains("git --literal-pathspecs restore -- README.md"));
        let mut output = Vec::new();
        render_plan(&mut output, &plan, "/repo", true).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["commands"][0]["destructive"], true);
        assert!(
            json["commands"][0]["warning"]
                .as_str()
                .unwrap()
                .contains("lost permanently")
        );
    }
}
