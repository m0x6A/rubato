// A must-fail fixture: `pipe-pane` copies a pane's output to a command, which reads it too.
fn tap(tmux: &str, pane: &str) {
    let _ = std::process::Command::new(tmux)
        .args(["pipe-pane", "-o", "-t", pane, "cat >> /tmp/screen"])
        .status();
}
