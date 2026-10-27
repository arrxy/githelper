use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Serialize)]
pub struct Context {
    pub root: String,
    pub current_branch: String,
    pub local_branches: Vec<String>,
    pub tracked_status: String,
}

fn git(directory: &Path, args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args(["--no-optional-locks", "-c", "core.fsmonitor=false"])
        .args(args)
        .current_dir(directory)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "Git could not inspect {}: {}",
            directory.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?
        .trim_end_matches(['\r', '\n'])
        .into())
}

pub fn inspect(directory: &Path) -> Result<Context, Box<dyn std::error::Error>> {
    let directory = directory.canonicalize()?;
    let root = git(&directory, &["rev-parse", "--show-toplevel"])?;
    let root_path = PathBuf::from(&root);
    let current_branch = git(&root_path, &["branch", "--show-current"])?;
    let branches = git(
        &root_path,
        &[
            "for-each-ref",
            "--count=32",
            "--format=%(refname:short)",
            "refs/heads/",
        ],
    )?;
    let status = git(
        &root_path,
        &["status", "--porcelain=v1", "--untracked-files=no"],
    )?;
    // Keep context small enough for the on-device model. No file contents or diffs are read.
    let tracked_status: String = status.chars().take(4_000).collect();
    Ok(Context {
        root,
        current_branch: if current_branch.is_empty() {
            "(detached HEAD)".into()
        } else {
            current_branch
        },
        local_branches: branches.lines().map(str::to_owned).collect(),
        tracked_status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_an_empty_repository_from_a_subdirectory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--initial-branch=main"])
                .arg(dir.path())
                .output()
                .unwrap()
                .status
                .success()
        );
        let child = dir.path().join("src");
        std::fs::create_dir(&child).unwrap();
        let context = inspect(&child).unwrap();
        assert_eq!(
            PathBuf::from(context.root),
            dir.path().canonicalize().unwrap()
        );
        assert_eq!(context.current_branch, "main");
        assert!(context.local_branches.is_empty());
        assert!(context.tracked_status.is_empty());
    }

    #[test]
    fn rejects_non_repositories() {
        assert!(inspect(tempfile::tempdir().unwrap().path()).is_err());
    }
}
