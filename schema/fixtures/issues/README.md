# Issue body fixtures

Bodies for the `Map:` line parser (ADRs 0026, 0046), read by `rubato-protocol`'s `issue_body` tests.
The expected placement of each file is in the test, `crates/rubato-protocol/src/issue_body.rs`.

| File | Placement |
| --- | --- |
| `voice-note.md` | seam `web-server` |
| `in-code-fence.md` | none: the lines are inside code fences |
| `two-lines.md` | node `rubato-server`: the last line wins |
| `unknown-id.md` | node `billing`: parsed; the server finds the id unknown |
| `malformed.md` | none |
| `qualified.md` | node `auth-api` in workspace `shop` (ADR 0046); a server without workspaces ignores the line |
| `qualified-bad.md` | none: a malformed qualifier never places the issue |
