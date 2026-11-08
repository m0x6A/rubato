//! git as a CLI: worktrees, branches, origin. Identity comes from the branch (ADR 0006).

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn git(cwd: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    if !out.status.success() {
        bail!(
            "git {} in {} failed: {}",
            args.join(" "),
            cwd.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// The worktree root containing `path`, if it is inside a git worktree.
pub fn worktree_root(path: &Path) -> Option<PathBuf> {
    git(path, &["rev-parse", "--show-toplevel"])
        .ok()
        .map(PathBuf::from)
}

pub fn current_branch(worktree: &Path) -> Option<String> {
    git(worktree, &["rev-parse", "--abbrev-ref", "HEAD"])
        .ok()
        .filter(|b| b != "HEAD")
}

/// `owner/name` from the origin URL.
pub fn origin_repo(worktree: &Path) -> Option<String> {
    let url = git(worktree, &["remote", "get-url", "origin"]).ok()?;
    repo_from_url(&url)
}

pub fn repo_from_url(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/').trim_end_matches(".git");
    let tail = if let Some(i) = trimmed.rfind(':') {
        if trimmed[i + 1..].contains('/')
            && !trimmed[..i].ends_with("http")
            && !trimmed[..i].ends_with("https")
        {
            &trimmed[i + 1..]
        } else {
            trimmed
        }
    } else {
        trimmed
    };
    let parts: Vec<&str> = tail.rsplit('/').take(2).collect();
    if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
        Some(format!("{}/{}", parts[1], parts[0]))
    } else {
        None
    }
}

/// `git worktree list --porcelain` → (path, branch) pairs.
pub fn worktrees(repo: &Path) -> Result<Vec<(PathBuf, Option<String>)>> {
    let out = git(repo, &["worktree", "list", "--porcelain"])?;
    let mut list = Vec::new();
    let mut cur: Option<PathBuf> = None;
    let mut branch: Option<String> = None;
    for line in out.lines().chain(std::iter::once("")) {
        if let Some(p) = line.strip_prefix("worktree ") {
            cur = Some(PathBuf::from(p));
        } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
            branch = Some(b.to_owned());
        } else if line.is_empty()
            && let Some(p) = cur.take()
        {
            list.push((p, branch.take()));
        }
    }
    Ok(list)
}

pub fn fetch(repo: &Path) -> Result<()> {
    git(repo, &["fetch", "--quiet", "origin"]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_from_urls() {
        assert_eq!(
            repo_from_url("https://github.com/m0x6A/rubato.git").as_deref(),
            Some("m0x6A/rubato")
        );
        assert_eq!(
            repo_from_url("git@github.com:acme/pantry.git").as_deref(),
            Some("acme/pantry")
        );
        assert_eq!(
            repo_from_url("/tmp/x/origin.git").as_deref(),
            Some("x/origin")
        );
        assert_eq!(repo_from_url("nonsense"), None);
    }
}
