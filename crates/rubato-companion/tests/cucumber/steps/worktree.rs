//! Steps for `features/worktree.feature`.

use super::launch_refusal::when_launch;
use crate::world::*;
use cucumber::{given, then, when};
use rubato_companion::kernel::gitx;
use rubato_companion::kernel::tmux::session_name;
use rubato_protocol::{CompanionMessage, IssueRef, LaunchResult};
use std::path::PathBuf;
use std::time::Duration;

#[given(expr = "a temporary repository {string} with a bare origin")]
async fn given_repo(w: &mut RubatoWorld, name: String) {
    w.make_repo(&name);
}

#[given(
    expr = "the server has launched issue {int} of {string} with slug {string} for user {string}"
)]
async fn given_launched(w: &mut RubatoWorld, issue: u64, repo: String, slug: String, user: String) {
    when_launch(w, user, issue, repo, slug).await;
    let id = w.last_launch.as_ref().unwrap().id;
    let got = w
        .server()
        .wait_for(Duration::from_secs(30), |m| match m {
            CompanionMessage::Launch(LaunchResult::Started { command_id, .. })
                if *command_id == id =>
            {
                Some(())
            }
            _ => None,
        })
        .await;
    assert!(got.is_some(), "first launch started");
}

#[then(expr = "the server receives a launch result {string}")]
async fn then_launch_started(w: &mut RubatoWorld, outcome: String) {
    assert_eq!(outcome, "started");
    let id = w.last_launch.as_ref().unwrap().id;
    let got = w
        .server()
        .wait_for(Duration::from_secs(60), |m| match m {
            CompanionMessage::Launch(LaunchResult::Started {
                command_id,
                worktree,
                ..
            }) if *command_id == id => Some(Ok(PathBuf::from(worktree))),
            CompanionMessage::Launch(LaunchResult::Failed {
                command_id, error, ..
            }) if *command_id == id => Some(Err(error.clone())),
            CompanionMessage::Launch(LaunchResult::Refused {
                command_id, detail, ..
            }) if *command_id == id => Some(Err(detail.clone())),
            _ => None,
        })
        .await;
    match got {
        Some(Ok(wt)) => w.started_worktree = Some(wt),
        other => panic!("launch did not start: {other:?}"),
    }
}

#[then(expr = "a worktree exists for branch {string} of {string} beside the clone")]
async fn then_worktree(w: &mut RubatoWorld, branch: String, repo: String) {
    let wt = worktree_of(w, &repo, &branch);
    assert_eq!(
        w.started_worktree.as_deref(),
        Some(wt.as_path()),
        "reported worktree is the sibling path"
    );
    assert!(wt.is_dir(), "{}", wt.display());
    assert_eq!(gitx::current_branch(&wt).as_deref(), Some(branch.as_str()));
}

#[then(expr = "a tmux session exists for {string} branch {string} with a pane in that worktree")]
async fn then_session(w: &mut RubatoWorld, repo: String, branch: String) {
    let name = session_name(&repo, &branch);
    assert!(
        w.tmux().has_session(&name).unwrap(),
        "no tmux session {name}"
    );
    let wt = worktree_of(w, &repo, &branch);
    let panes = w.tmux().panes().unwrap();
    assert!(
        panes.iter().any(|p| p.session == name && p.cwd == wt),
        "{panes:?}"
    );
}

#[then("the stand-in claude was started with the session id from the command")]
async fn then_session_id(w: &mut RubatoWorld) {
    let log = standin_log(w).await;
    let id = w.last_launch.as_ref().unwrap().session_id.to_string();
    assert!(log.contains(&format!("--session-id {id}")), "{log}");
}

#[then(expr = "the stand-in claude received {string} in OTEL_RESOURCE_ATTRIBUTES")]
async fn then_otel_attr(w: &mut RubatoWorld, attr: String) {
    let log = standin_log(w).await;
    let line = log
        .lines()
        .find(|l| l.starts_with("OTEL_RESOURCE_ATTRIBUTES="))
        .expect("OTEL_RESOURCE_ATTRIBUTES set");
    assert!(line.contains(&attr), "{line}");
    // Claude Code strips OTEL_* from its Bash tool, so the same value is mirrored (ADR 0061).
    let mirror = format!("RUBATO_{line}");
    assert!(log.lines().any(|l| l == mirror), "no {mirror} in {log}");
    assert!(log.contains("CLAUDE_CODE_ENABLE_TELEMETRY=1"));
    assert!(log.contains("OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf"));
}

#[then("the stand-in claude received a settings file whose hooks post to the companion's port")]
async fn then_settings(w: &mut RubatoWorld) {
    let log = standin_log(w).await;
    let args = log.lines().find(|l| l.starts_with("ARGS:")).unwrap();
    let path = args
        .split_whitespace()
        .skip_while(|a| *a != "--settings")
        .nth(1)
        .expect("--settings <file>");
    let text = std::fs::read_to_string(path).unwrap();
    let port = w.handles.as_ref().unwrap().hook_endpoint.port;
    assert!(
        text.contains(&format!("http://127.0.0.1:{port}/hook")),
        "{text}"
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600,
        "hooks file names the header file"
    );
}

#[then("that settings file holds no token and names a 0600 header file beside it")]
async fn then_settings_no_token(w: &mut RubatoWorld) {
    use std::os::unix::fs::PermissionsExt;
    let log = standin_log(w).await;
    let args = log.lines().find(|l| l.starts_with("ARGS:")).unwrap();
    let path = args
        .split_whitespace()
        .skip_while(|a| *a != "--settings")
        .nth(1)
        .expect("--settings <file>");
    let text = std::fs::read_to_string(path).unwrap();
    // ADR 0063: the token is in a 0600 header file beside it, never in a hook command.
    let token = &w.handles.as_ref().unwrap().hook_endpoint.token;
    assert!(!text.contains(token.as_str()), "{text}");
    let header = std::path::Path::new(path)
        .parent()
        .unwrap()
        .join(rubato_companion::kernel::config::HOOK_HEADER_FILE);
    assert!(text.contains(&format!("@'{}'", header.display())), "{text}");
    assert!(
        std::fs::read_to_string(&header)
            .unwrap()
            .contains(token.as_str())
    );
    assert_eq!(
        std::fs::metadata(&header).unwrap().permissions().mode() & 0o777,
        0o600,
        "header file holds the token"
    );
}

#[then("the launch directory has mode 0700 and its prompt file mode 0600")]
async fn then_launch_dir_private(w: &mut RubatoWorld) {
    use std::os::unix::fs::PermissionsExt;
    let log = standin_log(w).await;
    let args = log.lines().find(|l| l.starts_with("ARGS:")).unwrap();
    let settings = args
        .split_whitespace()
        .skip_while(|a| *a != "--settings")
        .nth(1)
        .expect("--settings <file>");
    let dir = std::path::Path::new(settings).parent().unwrap();
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(dir), 0o700, "{}", dir.display());
    assert_eq!(mode(&dir.join("prompt.txt")), 0o600, "prompt.txt");
}

#[then(
    expr = "the server receives a sessions snapshot with issue {int} of {string} from source {string}"
)]
async fn then_snapshot_source(w: &mut RubatoWorld, issue: u64, repo: String, source: String) {
    let want = IssueRef::new(repo, issue);
    let got = w
        .server()
        .wait_for(Duration::from_secs(10), |m| match m {
            CompanionMessage::Sessions { sessions } => sessions
                .iter()
                .find(|s| s.issue.as_ref() == Some(&want))
                .map(|s| {
                    serde_json::to_value(s.source)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_owned()
                }),
            _ => None,
        })
        .await;
    assert_eq!(got.as_deref(), Some(source.as_str()));
}

// ----- ADR 0070: the worktree with git, the tmux pin (plan 147) -------------------------------

/// A script at `fake_bin/<name>` with `body`, written by a child `sh` so this process never
/// holds it open for writing (ETXTBSY, #217).
fn script(w: &RubatoWorld, name: &str, body: &str) -> PathBuf {
    let path = w.fake_bin.join(name);
    let ok = std::process::Command::new("sh")
        .arg("-c")
        .arg("printf '%s' \"$1\" > \"$2\" && chmod +x \"$2\"")
        .arg("sh")
        .arg(body)
        .arg(&path)
        .status()
        .unwrap()
        .success();
    assert!(ok);
    path
}

fn git_calls(w: &RubatoWorld) -> PathBuf {
    w.fake_bin.join("git-calls")
}

/// Every git call the companion's launch makes is appended, one per line, then run by the real
/// git (`companion.git_command`).
#[given("the companion's git records its calls")]
async fn given_recording_git(w: &mut RubatoWorld) {
    let body = format!(
        "#!/bin/sh\necho \"$*\" >> '{}'\nexec git \"$@\"\n",
        git_calls(w).display()
    );
    let path = script(w, "git-recording", &body);
    w.workspace.companion.git_command = path.to_string_lossy().into_owned();
}

#[then(expr = "the companion's git ran {string} {int} time(s)")]
async fn then_git_ran(w: &mut RubatoWorld, call: String, times: usize) {
    let calls = std::fs::read_to_string(git_calls(w)).unwrap_or_default();
    let n = calls.lines().filter(|l| l.contains(&call)).count();
    assert_eq!(n, times, "{calls}");
}

#[given(expr = "the clone of {string} has a local branch {string}")]
async fn given_local_branch(w: &mut RubatoWorld, repo: String, branch: String) {
    RubatoWorld::git(w.repos.get(&repo).unwrap(), &["branch", &branch]);
}

/// Pushed from another machine: on origin, not in the clone.
#[given(expr = "{string} has a branch {string} on origin only")]
async fn given_origin_branch(w: &mut RubatoWorld, repo: String, branch: String) {
    RubatoWorld::git(
        w.repos.get(&repo).unwrap(),
        &["push", "-q", "origin", &format!("main:refs/heads/{branch}")],
    );
}

#[then(expr = "the worktree for branch {string} of {string} tracks {string}")]
async fn then_tracks(w: &mut RubatoWorld, branch: String, repo: String, upstream: String) {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "@{u}"])
        .current_dir(worktree_of(w, &repo, &branch))
        .output()
        .unwrap();
    let got = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    assert_eq!(got, upstream);
}

#[then(expr = "the worktree for branch {string} of {string} has no upstream")]
async fn then_no_upstream(w: &mut RubatoWorld, branch: String, repo: String) {
    let ok = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "@{u}"])
        .current_dir(worktree_of(w, &repo, &branch))
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success();
    assert!(!ok, "a new branch has no upstream until its first push -u");
}

/// What a `git worktree add` killed half-way leaves: the directory, not the registration.
#[given(expr = "a leftover directory at the worktree path for branch {string} of {string}")]
async fn given_leftover(w: &mut RubatoWorld, branch: String, repo: String) {
    let dir = worktree_of(w, &repo, &branch);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes.md"), "mine\n").unwrap();
}

#[then(expr = "the leftover directory for branch {string} of {string} is as it was")]
async fn then_leftover_kept(w: &mut RubatoWorld, branch: String, repo: String) {
    let dir = worktree_of(w, &repo, &branch);
    let names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["notes.md"]);
    assert_eq!(
        std::fs::read_to_string(dir.join("notes.md")).unwrap(),
        "mine\n"
    );
}

/// A registration whose directory is gone: `git worktree list` still lists it, `prunable`.
#[given(expr = "a stale worktree registration for branch {string} of {string}")]
async fn given_stale(w: &mut RubatoWorld, branch: String, repo: String) {
    let clone = w.repos.get(&repo).unwrap().clone();
    let dir = worktree_of(w, &repo, &branch);
    RubatoWorld::git(
        &clone,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            &branch,
            &dir.to_string_lossy(),
            "main",
        ],
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[then(expr = "the stale worktree registration for branch {string} of {string} is still listed")]
async fn then_stale_kept(w: &mut RubatoWorld, branch: String, repo: String) {
    let dir = worktree_of(w, &repo, &branch);
    let listed = gitx::worktrees(w.repos.get(&repo).unwrap()).unwrap();
    assert!(
        listed
            .iter()
            .any(|(p, b)| p == &dir && b.as_deref() == Some(branch.as_str())),
        "{listed:?}"
    );
}

/// `tmux -V` says `version`; everything else is the real tmux.
#[given(expr = "a tmux that reports version {string}")]
async fn given_tmux_version(w: &mut RubatoWorld, version: String) {
    let body = format!(
        "#!/bin/sh\n[ \"$1\" = -V ] && {{ echo 'tmux {version}'; exit 0; }}\nexec tmux \"$@\"\n"
    );
    let path = script(w, "tmux-versioned", &body);
    w.workspace.companion.tmux_command = path.to_string_lossy().into_owned();
}

#[then(expr = "the refusal's detail contains {string} and {string}")]
async fn then_refusal_detail(w: &mut RubatoWorld, a: String, b: String) {
    let id = w.last_launch.as_ref().unwrap().id;
    let got = w
        .server()
        .wait_for(Duration::from_secs(30), |m| match m {
            CompanionMessage::Launch(LaunchResult::Refused {
                command_id, detail, ..
            }) if *command_id == id => Some(detail.clone()),
            _ => None,
        })
        .await;
    let detail = got.expect("a refusal");
    assert!(detail.contains(&a) && detail.contains(&b), "{detail}");
}

/// The session's tmux session is gone; the next reconcile marks it ended.
#[when(expr = "the tmux session for {string} branch {string} ends")]
async fn when_session_ends(w: &mut RubatoWorld, repo: String, branch: String) {
    w.tmux()
        .kill_session(&session_name(&repo, &branch))
        .unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;
}
