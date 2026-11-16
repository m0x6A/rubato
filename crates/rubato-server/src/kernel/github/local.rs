//! A GitHub stand-in over local bare repositories, for end-to-end runs without GitHub
//! (`RUBATO_LOCAL_REPOS=owner/name=/path/to/repo.git,...`). Files and branches are real git;
//! issues and pull requests live in memory (`RUBATO_DEV_ISSUES`, `RUBATO_DEV_PULLS`), comments do
//! not exist here.

use super::{
    BranchHead, Comment, GitHubApi, IssueInfo, OpenIssueInfo, OpenIssues, OpenPulls, PullFull,
    PullInfo, RepoAccess,
};
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use rubato_protocol::ChecksStatus;
use rubato_protocol::pulls::{ChangedFile, MergeMethod};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::process::Command;

pub struct LocalGit {
    repos: HashMap<String, PathBuf>,
    /// Open issues per repo, from `RUBATO_DEV_ISSUES`. Git has none of its own.
    open: std::sync::Mutex<HashMap<String, Vec<OpenIssueInfo>>>,
    /// Pull requests opened through this stand-in, by (repo, head branch). Git has none.
    pulls: std::sync::Mutex<HashMap<(String, String), PullInfo>>,
    /// Pull requests `create_pull` made so far: numbered from 1 on their own, so seeded ones
    /// (`RUBATO_DEV_PULLS`) do not shift the numbers a map proposal gets.
    created: std::sync::atomic::AtomicU64,
}

/// One seeded pull request of `RUBATO_DEV_PULLS`: `{"owner/name": [{number, branch, base,
/// title, author}]}`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DevPull {
    pub number: u64,
    pub branch: String,
    #[serde(default = "main_branch")]
    pub base: String,
    pub title: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub draft: bool,
}

fn main_branch() -> String {
    "main".into()
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
            open: Default::default(),
            pulls: Default::default(),
            created: Default::default(),
        })
    }

    /// Give the stand-in its open issues.
    pub fn with_open_issues(mut self, open: HashMap<String, Vec<OpenIssueInfo>>) -> Self {
        self.open = std::sync::Mutex::new(open);
        self
    }

    /// Give the stand-in open pull requests between its branches (`RUBATO_DEV_PULLS`, issue #82).
    /// Their heads are read from git whenever they are listed or viewed.
    pub fn with_pulls(self, seeds: HashMap<String, Vec<DevPull>>) -> Self {
        {
            let mut pulls = self.pulls.lock().unwrap();
            for (repo, list) in seeds {
                for d in list {
                    pulls.insert(
                        (repo.clone(), d.branch.clone()),
                        PullInfo {
                            number: d.number,
                            open: true,
                            draft: d.draft,
                            head_sha: String::new(),
                            merged_at: None,
                            updated_at: chrono::Utc::now(),
                            url: format!("local-git://{repo}/pull/{}", d.number),
                            base: Some(d.base),
                            title: d.title,
                            author: d.author,
                        },
                    );
                }
            }
        }
        self
    }

    /// The pull request as stored, with its head read from git now.
    async fn fresh(&self, repo: &str, branch: &str, mut p: PullInfo) -> PullInfo {
        if let Ok(Some(sha)) = self.branch_sha(repo, branch).await {
            p.head_sha = sha;
        }
        p
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

    async fn last_commit_author(
        &self,
        repo: &str,
        path: &str,
        sha: &str,
    ) -> Result<Option<String>> {
        Ok(self
            .git(repo, &["log", "-1", "--format=%an", sha, "--", path])
            .await?
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty()))
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
        let listed: Vec<(String, PullInfo)> = self
            .pulls
            .lock()
            .unwrap()
            .iter()
            .filter(|((r, _), p)| r == repo && p.open)
            .map(|((_, branch), p)| (branch.clone(), p.clone()))
            .collect();
        let mut pulls = Vec::with_capacity(listed.len());
        for (branch, p) in listed {
            let p = self.fresh(repo, &branch, p).await;
            pulls.push((branch, p));
        }
        pulls.sort_by_key(|(_, p)| p.number);
        Ok(OpenPulls {
            pulls,
            truncated: false,
        })
    }

    async fn pull(&self, repo: &str, number: u64) -> Result<Option<PullFull>> {
        let found = self
            .pulls
            .lock()
            .unwrap()
            .iter()
            .find(|((r, _), p)| r == repo && p.number == number)
            .map(|((_, branch), p)| (branch.clone(), p.clone()));
        let Some((branch, p)) = found else {
            return Ok(None);
        };
        let info = self.fresh(repo, &branch, p).await;
        let base = info.base.clone().unwrap_or_else(main_branch);
        let base_sha = self.branch_sha(repo, &base).await?.unwrap_or_default();
        Ok(Some(PullFull {
            info,
            branch,
            body: String::new(),
            fork: false,
            mergeable: Some(true),
            base_sha,
        }))
    }

    /// `git diff --numstat` between the base and the head, as GitHub's three-dot compare.
    async fn pull_files(&self, repo: &str, number: u64) -> Result<Vec<ChangedFile>> {
        let Some(p) = self.pull(repo, number).await? else {
            return Ok(vec![]);
        };
        let range = format!("{}...{}", p.base_sha, p.info.head_sha);
        let out = self
            .git(repo, &["diff", "--numstat", &range])
            .await?
            .unwrap_or_default();
        Ok(out
            .lines()
            .filter_map(|l| {
                let mut f = l.split('\t');
                let add = f.next()?.parse().unwrap_or(0);
                let del = f.next()?.parse().unwrap_or(0);
                Some(ChangedFile {
                    path: f.next()?.to_owned(),
                    additions: add,
                    deletions: del,
                })
            })
            .take(super::FILES_PER_PAGE)
            .collect())
    }

    fn acts_without_token(&self) -> bool {
        true
    }

    /// A merge into the bare repo, as GitHub would make it: the head must still be `sha` (409),
    /// the trees must merge cleanly (405); `merge` makes a commit with both parents, `squash` and
    /// `rebase` one with the base only. The base moves only from the commit read (`update-ref`
    /// with the old value), as GitHub guards it.
    async fn merge_as(
        &self,
        _token: &str,
        repo: &str,
        number: u64,
        sha: &str,
        method: MergeMethod,
    ) -> std::result::Result<String, super::MergeError> {
        use super::MergeError;
        let refused = |status: u16, message: String| MergeError::Refused { status, message };
        let unknown = |e: anyhow::Error| MergeError::Unknown(format!("{e:#}"));
        let found = self
            .pulls
            .lock()
            .unwrap()
            .iter()
            .find(|((r, _), p)| r == repo && p.number == number)
            .map(|((_, b), p)| (b.clone(), p.clone()));
        let Some((branch, p)) = found else {
            return Err(refused(404, "Not Found".into()));
        };
        if !p.open {
            return Err(refused(405, "Pull Request is not mergeable".into()));
        }
        let base = p.base.clone().unwrap_or_else(main_branch);
        let head = self.branch_sha(repo, &branch).await.map_err(unknown)?;
        if head.as_deref() != Some(sha) {
            return Err(refused(
                409,
                "Head branch was modified. Review and try the merge again.".into(),
            ));
        }
        let old = self
            .branch_sha(repo, &base)
            .await
            .map_err(unknown)?
            .ok_or_else(|| refused(422, format!("base branch {base} does not exist")))?;
        let tree = self
            .git_write(repo, &["merge-tree", "--write-tree", &old, sha], None, None)
            .await
            .map_err(|e| refused(405, format!("Merge conflict: {e:#}")))?;
        let tree = tree.lines().next().unwrap_or_default().to_owned();
        let message = format!("Merge pull request #{number} from {branch}");
        let mut args = vec!["commit-tree", tree.as_str(), "-p", old.as_str()];
        if method == MergeMethod::Merge {
            args.extend(["-p", sha]);
        }
        args.extend(["-m", message.as_str()]);
        let made = self
            .git_write(repo, &args, None, None)
            .await
            .map_err(unknown)?;
        self.git_write(
            repo,
            &["update-ref", &format!("refs/heads/{base}"), &made, &old],
            None,
            None,
        )
        .await
        .map_err(|e| refused(409, format!("Base branch was modified: {e:#}")))?;
        if let Some(p) = self
            .pulls
            .lock()
            .unwrap()
            .get_mut(&(repo.to_owned(), branch))
        {
            p.open = false;
            p.merged_at = Some(chrono::Utc::now());
        }
        Ok(made)
    }

    /// The local stand-in has one person, who may do everything.
    async fn repo_as(&self, _token: &str, repo: &str) -> Result<RepoAccess> {
        self.path(repo)?;
        Ok(RepoAccess {
            push: true,
            methods: vec![MergeMethod::Merge, MergeMethod::Squash, MergeMethod::Rebase],
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
        let all = self
            .open
            .lock()
            .unwrap()
            .get(repo)
            .cloned()
            .unwrap_or_default();
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

    /// Issues live in memory here (from `RUBATO_DEV_ISSUES`), so creating one is adding a row:
    /// dev mode can run the whole "New issue here…" flow (ADR 0025).
    fn can_create_issues(&self) -> bool {
        true
    }

    /// The author `create_issue` writes, as the App's login would be.
    fn app_login(&self) -> Option<String> {
        Some("rubato[bot]".into())
    }

    async fn create_issue(&self, repo: &str, title: &str, body: &str) -> Result<(u64, String)> {
        self.path(repo)?;
        let mut open = self.open.lock().unwrap();
        let issues = open.entry(repo.to_owned()).or_default();
        let number = issues.iter().map(|i| i.number).max().unwrap_or(0).max(1000) + 1;
        let url = format!("https://github.com/{repo}/issues/{number}");
        issues.push(OpenIssueInfo {
            number,
            title: title.to_owned(),
            url: url.clone(),
            labels: vec![],
            updated_at: chrono::Utc::now(),
            body: body.to_owned(),
            author: "rubato[bot]".into(),
            author_association: "NONE".into(),
            author_is_bot: true,
        });
        Ok((number, url))
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
        base: &str,
        title: &str,
        _body: &str,
    ) -> Result<(u64, String)> {
        let head_sha = self
            .branch_sha(repo, head)
            .await?
            .with_context(|| format!("branch {head} does not exist in {repo}"))?;
        let mut pulls = self.pulls.lock().unwrap();
        let number = self
            .created
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
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
                base: Some(base.to_owned()),
                title: title.to_owned(),
                author: "rubato[bot]".into(),
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

    /// `RUBATO_DEV_PULLS` (issue #82): a seeded pull request lists with its branch's head as
    /// git has it now, reads on its own with the base head, and its files are the three-dot diff.
    #[tokio::test]
    async fn seeded_pull_requests_list_and_read_from_git() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("r");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "base"]);
        git(&repo, &["checkout", "-q", "-b", "issue/7-x"]);
        std::fs::write(repo.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(repo.join("b.txt"), "b\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "work"]);
        let seeds: HashMap<String, Vec<DevPull>> = serde_json::from_str(
            r#"{"a/r": [{"number": 3, "branch": "issue/7-x", "title": "Work", "author": "ann"}]}"#,
        )
        .unwrap();
        let g = LocalGit::from_spec(&format!("a/r={}", repo.display()))
            .unwrap()
            .with_pulls(seeds);
        let head = g.branch_sha("a/r", "issue/7-x").await.unwrap().unwrap();
        let listed = g.open_pulls("a/r").await.unwrap();
        assert_eq!(listed.pulls.len(), 1);
        assert_eq!(listed.pulls[0].1.head_sha, head);
        assert_eq!(listed.pulls[0].1.base.as_deref(), Some("main"));
        let p = g.pull("a/r", 3).await.unwrap().unwrap();
        assert_eq!(
            (p.branch.as_str(), p.info.head_sha.as_str()),
            ("issue/7-x", head.as_str())
        );
        assert_eq!(Some(p.base_sha), g.branch_sha("a/r", "main").await.unwrap());
        assert!(g.pull("a/r", 4).await.unwrap().is_none());
        let mut files = g.pull_files("a/r", 3).await.unwrap();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(
            files
                .iter()
                .map(|f| (f.path.as_str(), f.additions, f.deletions))
                .collect::<Vec<_>>(),
            [("a.txt", 2, 0), ("b.txt", 1, 0)]
        );
        assert!(g.repo_as("", "a/r").await.unwrap().push);
        assert!(g.repo_as("", "x/y").await.is_err());
    }

    /// Issue #82: a merge moves the base of the bare repo once, refuses a moved head, and refuses a
    /// conflict without moving anything.
    #[tokio::test]
    async fn a_merge_moves_the_base_once_and_only_from_the_head_seen() {
        let tmp = tempfile::tempdir().unwrap();
        let seed = tmp.path().join("seed");
        std::fs::create_dir_all(&seed).unwrap();
        git(&seed, &["init", "-q", "-b", "main"]);
        std::fs::write(seed.join("a.txt"), "one\n").unwrap();
        git(&seed, &["add", "-A"]);
        git(&seed, &["commit", "-q", "-m", "base"]);
        for (b, file, text) in [
            ("issue/7-x", "b.txt", "b\n"),
            ("issue/8-y", "a.txt", "other\n"),
        ] {
            git(&seed, &["checkout", "-q", "-b", b, "main"]);
            std::fs::write(seed.join(file), text).unwrap();
            git(&seed, &["add", "-A"]);
            git(&seed, &["commit", "-q", "-m", b]);
        }
        git(&seed, &["checkout", "-q", "main"]);
        std::fs::write(seed.join("a.txt"), "changed on main\n").unwrap();
        git(&seed, &["commit", "-q", "-am", "main moves"]);
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
        let seeds: HashMap<String, Vec<DevPull>> = serde_json::from_str(
            r#"{"a/r": [{"number": 1, "branch": "issue/7-x", "title": "B"}, {"number": 2, "branch": "issue/8-y", "title": "C"}]}"#,
        )
        .unwrap();
        let g = LocalGit::from_spec(&format!("a/r={}", bare.display()))
            .unwrap()
            .with_pulls(seeds);
        let main = g.branch_sha("a/r", "main").await.unwrap().unwrap();
        let head = g.branch_sha("a/r", "issue/7-x").await.unwrap().unwrap();
        let refused = |r: std::result::Result<String, super::super::MergeError>| match r {
            Err(super::super::MergeError::Refused { status, .. }) => status,
            other => panic!("expected a refusal, got {other:?}"),
        };
        assert_eq!(
            refused(
                g.merge_as("", "a/r", 1, "0000000", MergeMethod::Merge)
                    .await
            ),
            409
        );
        assert_eq!(
            g.branch_sha("a/r", "main").await.unwrap(),
            Some(main.clone())
        );
        let made = g
            .merge_as("", "a/r", 1, &head, MergeMethod::Merge)
            .await
            .unwrap();
        assert_eq!(
            g.branch_sha("a/r", "main").await.unwrap(),
            Some(made.clone())
        );
        assert!(
            g.file("a/r", "b.txt", "main").await.unwrap().is_some(),
            "the head's file landed"
        );
        let p = g.pull("a/r", 1).await.unwrap().unwrap();
        assert!(!p.info.open && p.info.merged_at.is_some());
        assert_eq!(
            refused(g.merge_as("", "a/r", 1, &head, MergeMethod::Merge).await),
            405,
            "merged once"
        );
        // A conflict: issue/8-y and main both changed a.txt.
        let head8 = g.branch_sha("a/r", "issue/8-y").await.unwrap().unwrap();
        assert_eq!(
            refused(g.merge_as("", "a/r", 2, &head8, MergeMethod::Squash).await),
            405
        );
        assert_eq!(g.branch_sha("a/r", "main").await.unwrap(), Some(made));
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
