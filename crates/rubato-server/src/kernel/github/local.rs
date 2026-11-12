//! A GitHub stand-in over local bare repositories, for end-to-end runs without GitHub
//! (`RUBATO_LOCAL_REPOS=owner/name=/path/to/repo.git,...`). Files and branches are real git;
//! issues, comments and pull requests do not exist here.

use super::{
    BranchHead, Comment, GitHubApi, IssueInfo, OpenIssueInfo, OpenIssues, OpenPulls, PullInfo,
};
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use rubato_protocol::ChecksStatus;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::process::Command;

pub struct LocalGit {
    repos: HashMap<String, PathBuf>,
    /// Open issues per repo, from `RUBATO_DEV_ISSUES`. Git has none of its own.
    open: HashMap<String, Vec<OpenIssueInfo>>,
    /// Pull requests opened through this stand-in, by (repo, head branch). Git has none.
    pulls: std::sync::Mutex<HashMap<(String, String), PullInfo>>,
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
        Ok(Self {
            repos,
            open: HashMap::new(),
            pulls: Default::default(),
        })
    }

    /// Give the stand-in its open issues.
    pub fn with_open_issues(mut self, open: HashMap<String, Vec<OpenIssueInfo>>) -> Self {
        self.open = open;
        self
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

    /// A git command that writes: a failure is an error with git's own words, never `None`.
    async fn git_write(
        &self,
        repo: &str,
        args: &[&str],
        index: Option<&Path>,
        stdin: Option<&str>,
    ) -> Result<String> {
        use tokio::io::AsyncWriteExt;
        let mut cmd = Command::new("git");
        cmd.arg("-C")
            .arg(self.path(repo)?)
            .args([
                "-c",
                "user.name=rubato",
                "-c",
                "user.email=rubato@localhost",
            ])
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if let Some(i) = index {
            cmd.env("GIT_INDEX_FILE", i);
        }
        let mut child = cmd.spawn().context("running git")?;
        if let Some(mut pipe) = child.stdin.take() {
            if let Some(text) = stdin {
                pipe.write_all(text.as_bytes()).await?;
            }
            drop(pipe);
        }
        let out = child.wait_with_output().await?;
        if !out.status.success() {
            bail!(
                "git {} failed: {}",
                args.first().copied().unwrap_or_default(),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
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

    async fn branches(&self, repo: &str, prefix: &str) -> Result<Vec<BranchHead>> {
        let out = self
            .git(
                repo,
                &[
                    "for-each-ref",
                    "--format=%(refname:short) %(objectname)",
                    &format!("refs/heads/{prefix}*"),
                ],
            )
            .await?
            .unwrap_or_default();
        Ok(out
            .lines()
            .filter_map(|l| l.trim().split_once(' '))
            .map(|(name, sha)| BranchHead {
                name: name.to_owned(),
                sha: sha.to_owned(),
            })
            .collect())
    }

    async fn open_pulls(&self, repo: &str) -> Result<OpenPulls> {
        let pulls = self
            .pulls
            .lock()
            .unwrap()
            .iter()
            .filter(|((r, _), p)| r == repo && p.open)
            .map(|((_, branch), p)| (branch.clone(), p.clone()))
            .collect();
        Ok(OpenPulls {
            pulls,
            truncated: false,
        })
    }

    async fn issue(&self, _repo: &str, _number: u64) -> Result<Option<IssueInfo>> {
        Ok(None)
    }

    async fn issue_comments(&self, _repo: &str, _number: u64) -> Result<Vec<Comment>> {
        Ok(vec![])
    }

    async fn pull_for_branch(&self, repo: &str, branch: &str) -> Result<Option<PullInfo>> {
        Ok(self
            .pulls
            .lock()
            .unwrap()
            .get(&(repo.to_owned(), branch.to_owned()))
            .cloned())
    }

    async fn checks(&self, _repo: &str, _sha: &str) -> Result<ChecksStatus> {
        Ok(ChecksStatus::None)
    }

    async fn rate_limit(&self) -> Result<(u64, u64)> {
        Ok((5000, 5000))
    }

    async fn open_issues(&self, repo: &str, max: usize) -> Result<OpenIssues> {
        let all = self.open.get(repo).cloned().unwrap_or_default();
        let truncated = all.len() > max;
        Ok(OpenIssues {
            issues: all.into_iter().take(max).collect(),
            truncated,
        })
    }

    /// Real git underneath: branches and files can be written (the map proposal, ADR 0018).
    fn can_write(&self) -> bool {
        true
    }

    async fn create_ref(&self, repo: &str, branch: &str, sha: &str) -> Result<()> {
        if self.branch_sha(repo, branch).await?.is_some() {
            return Ok(());
        }
        self.git_write(
            repo,
            &["update-ref", &format!("refs/heads/{branch}"), sha],
            None,
            None,
        )
        .await?;
        Ok(())
    }

    /// One commit on `branch` that changes one file, made with plumbing: the repository is
    /// bare, so there is no working tree to commit from.
    async fn put_file(
        &self,
        repo: &str,
        branch: &str,
        path: &str,
        content: &str,
        message: &str,
    ) -> Result<()> {
        let head = format!("refs/heads/{branch}");
        let parent = self
            .git_write(repo, &["rev-parse", "--verify", &head], None, None)
            .await?;
        let blob = self
            .git_write(repo, &["hash-object", "-w", "--stdin"], None, Some(content))
            .await?;
        let index = std::env::temp_dir().join(format!("rubato-index-{}", uuid::Uuid::new_v4()));
        let made = async {
            self.git_write(repo, &["read-tree", &parent], Some(&index), None)
                .await?;
            self.git_write(
                repo,
                &[
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    &format!("100644,{blob},{path}"),
                ],
                Some(&index),
                None,
            )
            .await?;
            let tree = self
                .git_write(repo, &["write-tree"], Some(&index), None)
                .await?;
            self.git_write(
                repo,
                &["commit-tree", &tree, "-p", &parent, "-m", message],
                None,
                None,
            )
            .await
        }
        .await;
        let _ = tokio::fs::remove_file(&index).await;
        // Only if the branch is still where it was read: a commit that landed meanwhile is a
        // failed write, not a lost one.
        self.git_write(repo, &["update-ref", &head, &made?, &parent], None, None)
            .await?;
        Ok(())
    }

    /// Git has no pull requests: this one lives in memory, so the proposal flow can be walked.
    async fn create_pull(
        &self,
        repo: &str,
        head: &str,
        _base: &str,
        _title: &str,
        _body: &str,
    ) -> Result<(u64, String)> {
        let head_sha = self
            .branch_sha(repo, head)
            .await?
            .with_context(|| format!("branch {head} does not exist in {repo}"))?;
        let mut pulls = self.pulls.lock().unwrap();
        let number = pulls.len() as u64 + 1;
        let url = format!("local-git://{repo}/pull/{number}");
        pulls.insert(
            (repo.to_owned(), head.to_owned()),
            PullInfo {
                number,
                open: true,
                draft: false,
                head_sha,
                merged_at: None,
                updated_at: chrono::Utc::now(),
                url: url.clone(),
            },
        );
        Ok((number, url))
    }

    async fn close_pull(&self, repo: &str, number: u64) -> Result<()> {
        for ((r, _), p) in self.pulls.lock().unwrap().iter_mut() {
            if r == repo && p.number == number {
                p.open = false;
            }
        }
        Ok(())
    }

    async fn delete_ref(&self, repo: &str, branch: &str) -> Result<()> {
        if self.branch_sha(repo, branch).await?.is_none() {
            return Ok(());
        }
        self.git_write(
            repo,
            &["update-ref", "-d", &format!("refs/heads/{branch}")],
            None,
            None,
        )
        .await?;
        Ok(())
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
        let heads = g.branches("a/r", "issue/").await.unwrap();
        assert_eq!(
            heads.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
            ["issue/4-x"]
        );
        assert_eq!(
            Some(heads[0].sha.clone()),
            g.branch_sha("a/r", "issue/4-x").await.unwrap()
        );
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

    #[tokio::test]
    async fn writes_a_branch_a_commit_and_a_pull_request_into_a_bare_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let seed = tmp.path().join("seed");
        std::fs::create_dir_all(seed.join("docs")).unwrap();
        git(&seed, &["init", "-q", "-b", "main"]);
        std::fs::write(seed.join("docs/map.yml"), "version: 1\n").unwrap();
        std::fs::write(seed.join("README.md"), "kept\n").unwrap();
        git(&seed, &["add", "-A"]);
        git(&seed, &["commit", "-q", "-m", "map"]);
        let bare = tmp.path().join("r.git");
        git(
            tmp.path(),
            &[
                "clone",
                "-q",
                "--bare",
                seed.to_str().unwrap(),
                bare.to_str().unwrap(),
            ],
        );
        let g = LocalGit::from_spec(&format!("a/r={}", bare.display())).unwrap();
        assert!(g.can_write());
        let main = g.branch_sha("a/r", "main").await.unwrap().unwrap();

        g.create_ref("a/r", "map/draft-1", &main).await.unwrap();
        assert_eq!(
            g.branch_sha("a/r", "map/draft-1").await.unwrap(),
            Some(main.clone())
        );
        // Creating it again is not an error and does not move it.
        g.create_ref("a/r", "map/draft-1", &main).await.unwrap();

        g.put_file(
            "a/r",
            "map/draft-1",
            "docs/map.yml",
            "version: 2\n",
            "Propose a map change",
        )
        .await
        .unwrap();
        let head = g.branch_sha("a/r", "map/draft-1").await.unwrap().unwrap();
        assert_ne!(head, main, "one new commit on the branch");
        assert_eq!(
            g.file("a/r", "docs/map.yml", "map/draft-1")
                .await
                .unwrap()
                .as_deref(),
            Some("version: 2\n")
        );
        // The other files of the tree are still there, and main is untouched.
        assert_eq!(
            g.file("a/r", "README.md", "map/draft-1")
                .await
                .unwrap()
                .as_deref(),
            Some("kept\n")
        );
        assert_eq!(
            g.file("a/r", "docs/map.yml", "main")
                .await
                .unwrap()
                .as_deref(),
            Some("version: 1\n")
        );
        assert_eq!(g.branch_sha("a/r", "main").await.unwrap(), Some(main));

        let (number, url) = g
            .create_pull("a/r", "map/draft-1", "main", "t", "b")
            .await
            .unwrap();
        let pr = g
            .pull_for_branch("a/r", "map/draft-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (pr.number, pr.open, pr.url.as_str()),
            (number, true, url.as_str())
        );
        assert_eq!(pr.head_sha, head);
        g.close_pull("a/r", number).await.unwrap();
        assert!(
            !g.pull_for_branch("a/r", "map/draft-1")
                .await
                .unwrap()
                .unwrap()
                .open
        );
        g.delete_ref("a/r", "map/draft-1").await.unwrap();
        assert_eq!(g.branch_sha("a/r", "map/draft-1").await.unwrap(), None);
        g.delete_ref("a/r", "map/draft-1").await.unwrap();

        // A write that git refuses is an error, not a quiet nothing.
        let e = g
            .put_file("a/r", "no-such-branch", "x", "y", "m")
            .await
            .unwrap_err();
        assert!(e.to_string().contains("git rev-parse failed"), "{e}");
        assert!(
            g.create_pull("a/r", "no-such-branch", "main", "t", "b")
                .await
                .is_err()
        );
    }
}
