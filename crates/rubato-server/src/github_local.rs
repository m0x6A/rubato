//! A GitHub stand-in over local bare repositories, for end-to-end runs without GitHub
//! (`RUBATO_LOCAL_REPOS=owner/name=/path/to/repo.git,...`). Files and branches are real git;
//! issues, comments and pull requests do not exist here.

use crate::github::{Comment, GitHubApi, IssueInfo, PullInfo};
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use rubato_protocol::ChecksStatus;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::process::Command;

pub struct LocalGit {
    repos: HashMap<String, PathBuf>,
}

impl LocalGit {
    /// Parse `owner/name=/path,owner/other=/path2`.
    pub fn from_spec(spec: &str) -> Result<Self> {
        let mut repos = HashMap::new();
        for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (name, path) = part
                .split_once('=')
                .with_context(|| format!("{part:?} is not owner/name=path"))?;
            repos.insert(name.trim().to_owned(), PathBuf::from(path.trim()));
        }
        if repos.is_empty() {
            bail!("RUBATO_LOCAL_REPOS is empty");
        }
        Ok(Self { repos })
    }

    fn path(&self, repo: &str) -> Result<&Path> {
        self.repos
            .get(repo)
            .map(PathBuf::as_path)
            .with_context(|| format!("{repo} is not a local repo"))
    }

    async fn git(&self, repo: &str, args: &[&str]) -> Result<Option<String>> {
        let out = Command::new("git")
            .arg("-C")
            .arg(self.path(repo)?)
            .args(args)
            .output()
            .await
            .context("running git")?;
        if !out.status.success() {
            return Ok(None);
        }
        Ok(Some(String::from_utf8_lossy(&out.stdout).into_owned()))
    }
}

#[async_trait]
impl GitHubApi for LocalGit {
    async fn file(&self, repo: &str, path: &str, git_ref: &str) -> Result<Option<String>> {
        self.git(repo, &["show", &format!("{git_ref}:{path}")])
            .await
    }

    async fn branch_sha(&self, repo: &str, branch: &str) -> Result<Option<String>> {
        Ok(self
            .git(
                repo,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("refs/heads/{branch}"),
                ],
            )
            .await?
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty()))
    }

    async fn branches_with_prefix(&self, repo: &str, prefix: &str) -> Result<Vec<String>> {
        let out = self
            .git(
                repo,
                &[
                    "for-each-ref",
                    "--format=%(refname:short)",
                    &format!("refs/heads/{prefix}*"),
                ],
            )
            .await?
            .unwrap_or_default();
        Ok(out
            .lines()
            .map(|l| l.trim().to_owned())
            .filter(|l| !l.is_empty())
            .collect())
    }

    async fn issue(&self, _repo: &str, _number: u64) -> Result<Option<IssueInfo>> {
        Ok(None)
    }

    async fn issue_comments(&self, _repo: &str, _number: u64) -> Result<Vec<Comment>> {
        Ok(vec![])
    }

    async fn pull_for_branch(&self, _repo: &str, _branch: &str) -> Result<Option<PullInfo>> {
        Ok(None)
    }

    async fn checks(&self, _repo: &str, _sha: &str) -> Result<ChecksStatus> {
        Ok(ChecksStatus::None)
    }

    async fn rate_limit(&self) -> Result<(u64, u64)> {
        Ok((5000, 5000))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as Std;

    fn git(cwd: &Path, args: &[&str]) {
        let out = Std::new("git")
            .arg("-C")
            .arg(cwd)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[tokio::test]
    async fn reads_branches_and_files_from_a_local_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("r");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::create_dir_all(repo.join("docs/plans")).unwrap();
        std::fs::write(repo.join("docs/plans/4.md"), "---\nissue: a/r#4\n---\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "plan"]);
        git(&repo, &["branch", "issue/4-x"]);
        let g = LocalGit::from_spec(&format!("a/r={}", repo.display())).unwrap();
        assert_eq!(
            g.branches_with_prefix("a/r", "issue/4-").await.unwrap(),
            vec!["issue/4-x".to_string()]
        );
        assert!(g.branch_sha("a/r", "issue/4-x").await.unwrap().is_some());
        assert_eq!(g.branch_sha("a/r", "nope").await.unwrap(), None);
        assert!(
            g.file("a/r", "docs/plans/4.md", "issue/4-x")
                .await
                .unwrap()
                .unwrap()
                .contains("a/r#4")
        );
        assert_eq!(g.file("a/r", "missing.md", "main").await.unwrap(), None);
        assert!(g.issue("a/r", 4).await.unwrap().is_none());
        assert!(LocalGit::from_spec("").is_err());
    }
}
