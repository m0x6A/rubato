//! The process table, to find Claude Code started under a wrapper: a pane whose own command
//! is `sh` or a script, with `claude` among its descendants (design 80 §5, A8). One `ps` per
//! reconcile, only when some pane's command did not already say `claude`.

use super::proc;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::time::Duration;

/// `ps` must answer within this, or discovery goes on without it.
pub const PS_TIMEOUT: Duration = Duration::from_secs(2);

/// Whether a command line is Claude Code: its program is `claude` or `claude-code`, or it is
/// Node running a script whose path names `claude`.
pub fn runs_claude(args: &str) -> bool {
    let mut words = args.split_whitespace();
    let base = |w: &str| w.rsplit('/').next().unwrap_or(w).to_owned();
    match words.next().map(base).as_deref() {
        Some("claude" | "claude-code") => true,
        Some("node") => words.next().is_some_and(|s| s.contains("claude")),
        _ => false,
    }
}

/// From `ps -A -o pid=,ppid=,args=` text: which of `roots` have a descendant (or are
/// themselves) running Claude Code.
pub fn roots_running_claude(table: &str, roots: &[u32]) -> HashSet<u32> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut claude: HashSet<u32> = HashSet::new();
    for line in table.lines() {
        let mut it = line.split_whitespace();
        let (Some(pid), Some(ppid)) = (
            it.next().and_then(|p| p.parse().ok()),
            it.next().and_then(|p| p.parse().ok()),
        ) else {
            continue;
        };
        children.entry(ppid).or_default().push(pid);
        if runs_claude(&it.collect::<Vec<_>>().join(" ")) {
            claude.insert(pid);
        }
    }
    roots
        .iter()
        .copied()
        .filter(|root| {
            let mut stack = vec![*root];
            let mut seen = HashSet::new();
            while let Some(p) = stack.pop() {
                if !seen.insert(p) {
                    continue;
                }
                if claude.contains(&p) {
                    return true;
                }
                stack.extend(children.get(&p).into_iter().flatten().copied());
            }
            false
        })
        .collect()
}

/// The process table now, on Linux and macOS alike.
pub fn table() -> Result<String> {
    let mut cmd = Command::new("ps");
    cmd.args(["-A", "-o", "pid=,ppid=,args="]);
    let out = proc::output(cmd, "ps", PS_TIMEOUT)?;
    anyhow::ensure!(out.status.success(), "ps exited with {}", out.status);
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_claude_under_a_wrapper_and_nothing_else() {
        let table = "\
    1     0 /sbin/init
  100     1 -zsh
  101   100 sh ./wrapper.sh
  102   101 /home/x/.local/bin/claude --resume
  200     1 bash
  201   200 node /usr/lib/node_modules/@anthropic-ai/claude-code/cli.js
  300     1 bash
  301   300 node server.js
  400     1 claude
";
        let got = roots_running_claude(table, &[100, 200, 300, 400, 999]);
        assert_eq!(got, HashSet::from([100, 200, 400]));
        assert!(!runs_claude("vim claude.md"));
        assert!(!runs_claude("node server.js"));
        println!("checked 5 roots");
    }
}
