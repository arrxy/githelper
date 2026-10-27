mod model;
mod plan;
mod repository;

use clap::Parser;
use std::{
    io::{self, Write},
    path::PathBuf,
};

#[derive(Parser)]
#[command(
    version,
    about = "Plain language → Git commands, using Apple's on-device model.",
    after_help = "Examples:\n  githelper \"show my staged changes\"\n  githelper --repo ~/code/app \"create a branch called fix/login\"\n  githelper --json \"stage README.md and commit with message update docs\"\n  githelper --doctor\n\nCommands are printed for review. Nothing is executed."
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
    let request = cli.request.join(" ");
    if request.trim().is_empty() {
        return Err("provide a request, for example: githelper \"show my changes\"".into());
    }
    if request.len() > 4_000 || request.chars().any(char::is_control) {
        return Err("request must be at most 4,000 bytes and contain no control characters".into());
    }
    let context = repository::inspect(&cli.repo)?;
    let proposal = bridge.translate(&request, &context)?;
    let plan = proposal.validate()?;
    if cli.json {
        println!("{}", serde_json::to_string_pretty(&plan)?);
    } else if let Some(question) = &plan.clarification {
        println!("Clarification needed: {question}");
    } else {
        println!("{}\n", plan.summary);
        for command in &plan.commands {
            println!("  {}", command.display());
        }
        println!(
            "\nReview these commands before running them in {}.",
            context.root
        );
        if plan.changes_repository {
            println!("This plan changes repository state or contacts a remote.");
        }
    }
    io::stdout().flush()?;
    Ok(())
}
