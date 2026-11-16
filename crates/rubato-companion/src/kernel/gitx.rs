//! git as a CLI: worktrees, branches, origin. Identity comes from the branch (ADR 0006).

use crate::kernel::proc;
use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// A local git call, bounded by [`proc::LOCAL_TIMEOUT`].
pub fn git(cwd: &Path, args: &[&str]) -> Result<String> {
    git_within(cwd, args, proc::LOCAL_TIMEOUT)
}

pub fn git_within(cwd: &Path, args: &[&str], timeout: Duration) -> Result<String> {
    command_within("git", cwd, args, timeout)
}

/// As [`git_within`], with the git binary named (`companion.git_command`, ADR 0070).
fn command_within(command: &str, cwd: &Path, args: &[&str], timeout: Duration) -> Result<String> {
    let mut c = Command::new(command);
    c.arg("-C").arg(cwd).args(args).current_dir(cwd);
    let out = proc::output(c, &format!("git {}", args.join(" ")), timeout)?;
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
    remote_repo(worktree, "origin")
}

/// `owner/name` from a named remote's URL.
pub fn remote_repo(worktree: &Path, remote: &str) -> Option<String> {
    let url = git(worktree, &["remote", "get-url", remote]).ok()?;
    repo_from_url(&url)
}

/// The worktree's `HEAD` file (in a linked worktree, under the main repo's `.git/worktrees`).
pub fn head_file(worktree: &Path) -> Option<PathBuf> {
    let p = PathBuf::from(git(worktree, &["rev-parse", "--git-path", "HEAD"]).ok()?);
    Some(if p.is_absolute() { p } else { worktree.join(p) })
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

/// One entry of `git worktree list --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    /// Registered, but its directory is gone (`prunable`): a `worktree add` killed half-way, or
    /// a directory the developer deleted.
    pub prunable: bool,
}

fn parse_worktrees(out: &str) -> Vec<Worktree> {
    let mut list = Vec::new();
    let mut cur: Option<Worktree> = None;
    for line in out.lines().chain(std::iter::once("")) {
        if let Some(p) = line.strip_prefix("worktree ") {
            cur = Some(Worktree {
                path: PathBuf::from(p),
                branch: None,
                prunable: false,
            });
        } else if let Some(w) = cur.as_mut() {
            if let Some(b) = line.strip_prefix("branch refs/heads/") {
                w.branch = Some(b.to_owned());
            } else if line == "prunable" || line.starts_with("prunable ") {
                w.prunable = true;
            } else if line.is_empty() {
                list.extend(cur.take());
            }
        }
    }
    list
}

/// `git worktree list --porcelain` → (path, branch) pairs.
pub fn worktrees(repo: &Path) -> Result<Vec<(PathBuf, Option<String>)>> {
    let out = git(repo, &["worktree", "list", "--porcelain"])?;
    Ok(parse_worktrees(&out)
        .into_iter()
        .map(|w| (w.path, w.branch))
        .collect())
}

/// The git a launch runs: `companion.git_command` under `companion.command_timeout()` (plan 40,
/// ADR 0070). Worktrees for interactive and headless launches alike, without tmux-ide.
#[derive(Debug, Clone)]
pub struct Git {
    pub command: String,
    pub timeout: Duration,
}

impl Git {
    pub fn new(command: impl Into<String>, timeout: Duration) -> Self {
        Self {
            command: command.into(),
            timeout,
        }
    }

    fn run(&self, repo: &Path, args: &[&str]) -> Result<String> {
        command_within(&self.command, repo, args, self.timeout)
    }

    fn has_ref(&self, repo: &Path, name: &str) -> bool {
        self.run(repo, &["rev-parse", "--verify", "--quiet", name])
            .is_ok()
    }

    /// Network: `git fetch origin` (plan 40 S3).
    pub fn fetch(&self, repo: &Path) -> Result<()> {
        self.run(repo, &["fetch", "--quiet", "origin"]).map(|_| ())
    }

    pub fn worktrees(&self, repo: &Path) -> Result<Vec<Worktree>> {
        Ok(parse_worktrees(
            &self.run(repo, &["worktree", "list", "--porcelain"])?,
        ))
    }

    /// Drop the registration of one worktree whose directory is gone (plan 147 Decision 12).
    /// Never `git worktree prune`: that would drop the developer's other registrations too.
    pub fn remove_registration(&self, repo: &Path, path: &Path) -> Result<()> {
        let p = path.display().to_string();
        self.run(repo, &["worktree", "remove", "--force", "--", &p])
            .map(|_| ())
    }

    /// A worktree for `branch` at `path` (design 147 §2):
    /// 1. the local branch when it exists;
    /// 2. else `origin/<branch>`, tracked, when it was pushed from elsewhere;
    /// 3. else a new branch from `base_ref`, with no upstream until its first `push -u`.
    ///
    /// The branch and the base ref come from the server: `-b` takes the branch as its value and
    /// `--` ends the options before the path (as for tmux-ide, issue #155). A directory already
    /// at `path` is never written over or deleted: the developer may have work in it.
    pub fn worktree_add(
        &self,
        repo: &Path,
        path: &Path,
        branch: &str,
        base_ref: &str,
    ) -> Result<()> {
        if path.exists() {
            bail!(
                "{} exists but is not a worktree of {} for {branch}; move it away or remove it",
                path.display(),
                repo.display()
            );
        }
        let p = path.display().to_string();
        let remote = format!("origin/{branch}");
        if self.has_ref(repo, &format!("refs/heads/{branch}")) {
            self.run(repo, &["worktree", "add", "--", &p, branch])?;
        } else if self.has_ref(repo, &format!("refs/remotes/{remote}")) {
            self.run(
                repo,
                &[
                    "worktree", "add", "--track", "-b", branch, "--", &p, &remote,
                ],
            )?;
        } else {
            self.run(
                repo,
                &[
                    "worktree",
                    "add",
                    "--no-track",
                    "-b",
                    branch,
                    "--",
                    &p,
                    base_ref,
                ],
            )?;
        }
        Ok(())
    }
}

/// The worktree path tmux-ide used, kept so existing worktrees stay put: a sibling directory
/// `<repo>-worktrees/<branch>` (slashes kept as directories).
pub fn default_worktree_path(repo: &Path, branch: &str) -> PathBuf {
    let name = repo
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".into());
    let parent = repo.parent().unwrap_or(repo);
    parent.join(format!("{name}-worktrees")).join(branch)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plain git for the fixtures, with an identity so commits work on any runner.
    fn sh_git(cwd: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    /// A bare origin with `main`, and a clone of it at `<tmp>/pantry` whose origin has a branch
    /// pushed from elsewhere (`issue/8-remote`) and which has a local one (`issue/9-local`).
    fn fixture(tmp: &Path) -> PathBuf {
        let origin = tmp.join("origin.git");
        sh_git(tmp, &["init", "--bare", "-q", origin.to_str().unwrap()]);
        let seed = tmp.join("seed");
        sh_git(
            tmp,
            &[
                "clone",
                "-q",
                origin.to_str().unwrap(),
                seed.to_str().unwrap(),
            ],
        );
        sh_git(&seed, &["commit", "-q", "--allow-empty", "-m", "first"]);
        sh_git(&seed, &["push", "-q", "origin", "HEAD:main"]);
        sh_git(&seed, &["push", "-q", "origin", "HEAD:issue/8-remote"]);
        let repo = tmp.join("pantry");
        sh_git(
            tmp,
            &[
                "clone",
                "-q",
                origin.to_str().unwrap(),
                repo.to_str().unwrap(),
            ],
        );
        sh_git(&repo, &["branch", "issue/9-local"]);
        repo
    }

    /// A git that appends its argv, one argument per line and a blank line after, then runs the
    /// real git. Written by a child `sh` so this process never holds the file open for writing
    /// when it is executed (ETXTBSY, #217).
    fn recording_git(dir: &Path) -> (Git, PathBuf) {
        let script = dir.join("git-recording");
        let argv = dir.join("argv");
        let body = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{}'\necho >> '{}'\nexec git \"$@\"\n",
            argv.display(),
            argv.display()
        );
        let ok = Command::new("sh")
            .arg("-c")
            .arg("printf '%s' \"$1\" > \"$2\" && chmod +x \"$2\"")
            .arg("sh")
            .arg(body)
            .arg(&script)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        (
            Git::new(script.to_string_lossy(), proc::LOCAL_TIMEOUT),
            argv,
        )
    }

    /// The recorded `worktree add` calls, each as its argument list.
    fn adds(argv: &Path) -> Vec<Vec<String>> {
        std::fs::read_to_string(argv)
            .unwrap_or_default()
            .split("\n\n")
            .map(|call| call.lines().map(str::to_owned).collect::<Vec<_>>())
            .filter(|a| a.len() > 2 && a[2] == "worktree" && a[3] == "add")
            .collect()
    }

    /// Design 147 §2: the three cases, each at the path tmux-ide used, each with `--` before the
    /// path; a new branch has no upstream, a branch from `origin` tracks it.
    #[test]
    fn a_worktree_is_added_in_each_of_the_three_cases() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = fixture(tmp.path());
        let (git, argv) = recording_git(tmp.path());
        git.fetch(&repo).unwrap();
        let cases = [
            ("issue/9-local", "local"),
            ("issue/8-remote", "origin"),
            ("issue/7-new", "new"),
        ];
        for (branch, _) in cases {
            let path = default_worktree_path(&repo, branch);
            git.worktree_add(&repo, &path, branch, "origin/main")
                .unwrap();
            assert_eq!(
                sh_git(&path, &["rev-parse", "--abbrev-ref", "HEAD"]),
                branch
            );
        }
        let calls = adds(&argv);
        assert_eq!(calls.len(), cases.len(), "{calls:?}");
        for call in &calls {
            // `git -C <repo> worktree add … -- <path> <commit-ish>`
            let dash = call.iter().position(|a| a == "--").expect("a --");
            assert_eq!(
                call.len(),
                dash + 3,
                "only the path and the commit-ish follow --: {call:?}"
            );
            assert!(
                call[dash + 1].contains("pantry-worktrees/issue/"),
                "{call:?}"
            );
        }
        assert!(
            calls[0].ends_with(&["issue/9-local".to_owned()]),
            "{:?}",
            calls[0]
        );
        assert!(calls[1].contains(&"--track".to_owned()), "{:?}", calls[1]);
        assert!(
            calls[2].contains(&"--no-track".to_owned()),
            "{:?}",
            calls[2]
        );
        let upstream = |branch: &str| {
            Command::new("git")
                .args(["rev-parse", "--abbrev-ref", "@{u}"])
                .current_dir(default_worktree_path(&repo, branch))
                .output()
                .unwrap()
        };
        let tracked = upstream("issue/8-remote");
        assert_eq!(
            String::from_utf8_lossy(&tracked.stdout).trim(),
            "origin/issue/8-remote"
        );
        assert!(
            !upstream("issue/7-new").status.success(),
            "a new branch has no upstream until its first push -u"
        );
        println!("checked {} worktree add cases", calls.len());
    }

    /// A directory at the path that git does not list (a `worktree add` killed half-way) fails
    /// with the path, and is left exactly as it was.
    #[test]
    fn a_leftover_directory_is_refused_and_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = fixture(tmp.path());
        let git = Git::new("git", proc::LOCAL_TIMEOUT);
        let path = default_worktree_path(&repo, "issue/7-new");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("notes.md"), "mine").unwrap();
        let e = git
            .worktree_add(&repo, &path, "issue/7-new", "origin/main")
            .unwrap_err();
        assert!(
            format!("{e:#}").contains(&path.display().to_string()),
            "{e:#}"
        );
        assert_eq!(
            std::fs::read_to_string(path.join("notes.md")).unwrap(),
            "mine"
        );
        assert!(!git.worktrees(&repo).unwrap().iter().any(|w| w.path == path));
    }

    /// A registration whose directory is gone is listed `prunable`, and removing it drops only
    /// that one: the developer's other registrations stay (plan 147 Decision 12).
    #[test]
    fn a_stale_registration_is_removed_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = fixture(tmp.path());
        let git = Git::new("git", proc::LOCAL_TIMEOUT);
        let mine = default_worktree_path(&repo, "issue/7-new");
        let other = default_worktree_path(&repo, "issue/9-local");
        git.worktree_add(&repo, &mine, "issue/7-new", "origin/main")
            .unwrap();
        git.worktree_add(&repo, &other, "issue/9-local", "origin/main")
            .unwrap();
        std::fs::remove_dir_all(&mine).unwrap();
        std::fs::remove_dir_all(&other).unwrap();
        let listed = git.worktrees(&repo).unwrap();
        let stale: std::collections::BTreeSet<_> = listed
            .iter()
            .filter(|w| w.prunable)
            .map(|w| w.path.clone())
            .collect();
        assert_eq!(stale, [mine.clone(), other.clone()].into(), "{listed:?}");
        git.remove_registration(&repo, &mine).unwrap();
        let left: Vec<_> = git
            .worktrees(&repo)
            .unwrap()
            .into_iter()
            .map(|w| w.path)
            .collect();
        assert!(!left.contains(&mine), "{left:?}");
        assert!(
            left.contains(&other),
            "another stale registration is kept: {left:?}"
        );
        // The branch is free again, so the launch can add it at the same path.
        git.worktree_add(&repo, &mine, "issue/7-new", "origin/main")
            .unwrap();
    }

    #[test]
    fn worktree_path_is_sibling() {
        assert_eq!(
            default_worktree_path(Path::new("/w/pantry"), "issue/7-restock"),
            PathBuf::from("/w/pantry-worktrees/issue/7-restock")
        );
    }

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
