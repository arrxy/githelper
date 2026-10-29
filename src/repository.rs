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
    // An empty porcelain listing means a clean tracked worktree. Send an explicit marker so
    // the model does not treat a missing/blank field as "context unavailable".
    let tracked_status: String = {
        let clipped: String = status.chars().take(4_000).collect();
        if clipped.trim().is_empty() {
            "(clean: no modified tracked files)".into()
        } else {
            clipped
        }
    };
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
        assert_eq!(context.tracked_status, "(clean: no modified tracked files)");
    }

    #[test]
    fn rejects_non_repositories() {
        assert!(inspect(tempfile::tempdir().unwrap().path()).is_err());
    }
}

const PATCH_LIMIT: usize = 6_000;
const DIAGNOSTIC_LIMIT: usize = 4_096;

// Read both pipes concurrently. Keep only bounded buffers; terminate and reap Git
// as soon as stdout exceeds its budget, and drain stderr without retaining it.
fn bounded_git(
    directory: &Path,
    args: &[&str],
    limit: usize,
) -> Result<(Vec<u8>, bool), Box<dyn std::error::Error>> {
    use std::io::{Read, Write};
    use std::process::Stdio;
    let mut child = Command::new("git")
        .args(["--no-optional-locks", "-c", "core.fsmonitor=false"])
        .args(args)
        .current_dir(directory)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdout = child.stdout.take().ok_or("could not read Git output")?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or("could not read Git diagnostics")?;
    let diagnostics = std::thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut retained = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let count = stderr.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            let keep = count.min(DIAGNOSTIC_LIMIT.saturating_sub(retained.len()));
            retained.write_all(&buffer[..keep])?;
        }
        Ok(retained)
    });
    let mut bytes = Vec::new();
    let read = stdout
        .by_ref()
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes);
    let overflow = bytes.len() > limit;
    if overflow || read.is_err() {
        let _ = child.kill();
    }
    drop(stdout);
    let status = child.wait();
    let diagnostics = diagnostics
        .join()
        .map_err(|_| "Git diagnostic reader failed")??;
    read?;
    let status = status?;
    if !overflow && !status.success() {
        return Err(format!(
            "Git could not inspect {}: {}",
            directory.display(),
            String::from_utf8_lossy(&diagnostics).trim()
        )
        .into());
    }
    Ok((bytes, overflow))
}

/// Collect only staged changes. This never runs diff drivers or reads worktree content.
pub fn staged_diff(directory: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let directory = directory.canonicalize()?;
    let (root, overflow) = bounded_git(
        &directory,
        &["rev-parse", "--show-toplevel"],
        DIAGNOSTIC_LIMIT,
    )?;
    if overflow {
        return Err("repository root exceeds the path budget".into());
    }
    let root = String::from_utf8(root)?;
    let root = PathBuf::from(root.trim_end_matches(['\r', '\n']));
    let (conflicts, _) = bounded_git(&root, &["ls-files", "--unmerged", "-z"], 1)?;
    if !conflicts.is_empty() {
        return Err("resolve staged conflicts before drafting a commit message".into());
    }
    let (patch, overflow) = bounded_git(
        &root,
        &[
            "diff",
            "--cached",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--no-renames",
            "--submodule=short",
            "--ignore-submodules=none",
            "--unified=3",
            "--",
        ],
        PATCH_LIMIT,
    )?;
    if overflow {
        return Err("staged patch exceeds 6,000 bytes; stage a smaller change and retry".into());
    }
    if patch.is_empty() {
        return Err("no staged changes; stage changes before drafting a commit message".into());
    }
    String::from_utf8(patch).map_err(|_| "staged patch is not valid UTF-8".into())
}

#[cfg(test)]
mod staged_tests {
    use super::*;
    fn run(path: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "--initial-branch=main"]);
        run(dir.path(), &["config", "user.name", "Test"]);
        run(dir.path(), &["config", "user.email", "test@example.com"]);
        dir
    }
    #[test]
    fn initial_patch_uses_index_and_preserves_state_from_subdirectory() {
        let dir = repo();
        let p = dir.path();
        std::fs::write(p.join("file"), "staged-version\n").unwrap();
        run(p, &["add", "file"]);
        let index = std::fs::read(p.join(".git/index")).unwrap();
        std::fs::write(p.join("file"), "unstaged-version\n").unwrap();
        std::fs::write(p.join("secret"), "untracked-secret\n").unwrap();
        std::fs::create_dir(p.join("child")).unwrap();
        let patch = staged_diff(&p.join("child")).unwrap();
        assert!(patch.contains("staged-version"));
        assert!(!patch.contains("unstaged-version") && !patch.contains("untracked-secret"));
        assert_eq!(index, std::fs::read(p.join(".git/index")).unwrap());
        assert_eq!(
            std::fs::read_to_string(p.join("file")).unwrap(),
            "unstaged-version\n"
        );
    }
    #[test]
    fn rejects_empty_oversized_and_invalid_utf8() {
        let dir = repo();
        let p = dir.path();
        assert!(
            staged_diff(p)
                .unwrap_err()
                .to_string()
                .contains("no staged")
        );
        std::fs::write(p.join("large"), "x".repeat(PATCH_LIMIT)).unwrap();
        run(p, &["add", "large"]);
        assert!(staged_diff(p).unwrap_err().to_string().contains("6,000"));
        std::fs::write(p.join("large"), [b'a', 255, b'\n']).unwrap();
        run(p, &["add", "large"]);
        assert!(staged_diff(p).unwrap_err().to_string().contains("UTF-8"));
    }
    #[test]
    fn includes_binary_metadata_mode_changes_and_deletions() {
        let dir = repo();
        let p = dir.path();
        std::fs::write(p.join("binary"), [0, 1, 2]).unwrap();
        std::fs::write(p.join("script"), "echo hello\n").unwrap();
        std::fs::write(p.join("deleted"), "remove me\n").unwrap();
        run(p, &["add", "."]);
        run(p, &["commit", "-m", "initial"]);
        std::fs::write(p.join("binary"), [0, 3, 4]).unwrap();
        std::fs::remove_file(p.join("deleted")).unwrap();
        run(p, &["add", "--all"]);
        run(p, &["update-index", "--chmod=+x", "script"]);
        let patch = staged_diff(p).unwrap();
        assert!(patch.contains("Binary files"));
        assert!(!patch.contains("GIT binary patch"));
        assert!(patch.contains("new mode 100755"));
        assert!(patch.contains("deleted file mode"));
    }

    #[test]
    fn staged_gitlinks_are_not_hidden_by_local_configuration() {
        let dir = repo();
        let p = dir.path();
        std::fs::write(p.join("file"), "base\n").unwrap();
        run(p, &["add", "file"]);
        run(p, &["commit", "-m", "base"]);
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(p)
            .output()
            .unwrap();
        assert!(output.status.success());
        let commit = String::from_utf8(output.stdout).unwrap();
        let entry = format!("160000,{},nested", commit.trim());
        run(p, &["update-index", "--add", "--cacheinfo", &entry]);
        run(p, &["config", "diff.ignoreSubmodules", "all"]);
        let patch = staged_diff(p).unwrap();
        assert!(patch.contains("diff --git a/nested b/nested"));
        assert!(patch.contains("new file mode 160000"));
        assert!(patch.contains(&format!("+Subproject commit {}", commit.trim())));
    }
    #[test]
    fn configured_diff_helpers_never_run() {
        let dir = repo();
        let p = dir.path();
        std::fs::write(p.join(".gitattributes"), "file diff=custom\n").unwrap();
        std::fs::write(p.join("file"), "content\n").unwrap();
        run(p, &["add", "."]);
        let command = format!("touch {}", p.join("sentinel").display());
        run(p, &["config", "diff.external", &command]);
        run(p, &["config", "diff.custom.command", &command]);
        run(p, &["config", "diff.custom.textconv", &command]);
        assert!(staged_diff(p).unwrap().contains("content"));
        assert!(!p.join("sentinel").exists());
    }
    #[test]
    fn rejects_unmerged_entries() {
        let dir = repo();
        let p = dir.path();
        std::fs::write(p.join("file"), "base\n").unwrap();
        run(p, &["add", "."]);
        run(p, &["commit", "-m", "base"]);
        run(p, &["checkout", "-b", "other"]);
        std::fs::write(p.join("file"), "other\n").unwrap();
        run(p, &["commit", "-am", "other"]);
        run(p, &["checkout", "main"]);
        std::fs::write(p.join("file"), "main\n").unwrap();
        run(p, &["commit", "-am", "main"]);
        assert!(
            !Command::new("git")
                .args(["merge", "other"])
                .current_dir(p)
                .output()
                .unwrap()
                .status
                .success()
        );
        assert!(
            staged_diff(p)
                .unwrap_err()
                .to_string()
                .contains("conflicts")
        );
    }
}
