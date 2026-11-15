// A must-fail fixture for `kernel::tmux::tests::the_companion_never_reads_a_pane`: the scan over
// this file reports a pane read. Never compiled.
fn screen(tmux: &str, pane: &str) -> std::process::Output {
    std::process::Command::new(tmux)
        .args(["capture-pane", "-p", "-t", pane])
        .output()
        .unwrap()
}
