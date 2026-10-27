mod commit_message;
mod interaction;
mod model;
mod plan;
mod repository;

use clap::Parser;
use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
};

#[derive(Parser)]
#[command(
    version,
    about = "Plain language → Git commands, using Apple's on-device model.",
    after_help = "Examples:\n  githelper \"show my staged changes\"\n  githelper --repo ~/code/app \"create a branch called fix/login\"\n  githelper --json \"stage README.md and commit with message update docs\"\n  githelper --doctor\n  githelper --interactive \"commit my changes\"\n  githelper --draft-commit --json\n\nCommands are printed for review. Nothing is executed."
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
    /// Check whether Apple's on-device model is available
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
    let cli = Cli::parse();
    if let Err(error) = run(cli) {
        eprintln!("githelper: {error}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let bridge = model::AppleModel;
    if cli.doctor {
        let status = bridge.status()?;
        if cli.json {
            println!("{}", serde_json::to_string_pretty(&status)?);
        } else {
            println!("{}", status.detail);
        }
        if !status.available {
            return Err("on-device model unavailable".into());
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
    let context = repository::inspect(&cli.repo)?;
    let plan = if cli.interactive {
        let result = interaction::clarify(
            |history| bridge.translate(&request, &context, history),
            &mut io::stdin().lock(),
            &mut io::stderr().lock(),
        )?;
        let Some(plan) = result else { return Ok(()) };
        plan
    } else {
        bridge.translate(&request, &context, &[])?.validate()?
    };
    render_plan(&mut io::stdout().lock(), &plan, &context.root, cli.json)?;
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
        let plan = proposal.validate().unwrap();
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
}
