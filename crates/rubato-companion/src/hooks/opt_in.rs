//! Opting a session the developer starts by hand in to our hooks (ADR 0078): the hooks it gets,
//! the merge into a repo's or worktree's `.claude/settings.local.json`, and whether a directory
//! is opted in. A launched session keeps its own `--settings` hooks (ADR 0013); the hooks written
//! here stand down inside one, so a worktree a launch reuses still delivers each event once.

use super::template::render_hooks;
use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// Every command written for a manual session starts with this: inside a launched session
/// (`launch.sh` exports `RUBATO_LAUNCH_ID`) the launch's own hooks deliver, so this one does not.
pub const STAND_DOWN: &str = "[ -n \"${RUBATO_LAUNCH_ID:-}\" ] || ";

/// The hooks for a session started by hand: the launch's template, each command behind
/// [`STAND_DOWN`]. The token stays in the header file (ADR 0063).
pub fn render_manual_hooks(template: &str, port: u16, header_file: &Path) -> Result<String> {
    let mut v: Value = serde_json::from_str(&render_hooks(template, port, header_file)?)?;
    for_each_command(&mut v, |c| *c = format!("{STAND_DOWN}{c}"));
    Ok(serde_json::to_string_pretty(&v)?)
}

fn for_each_command(v: &mut Value, mut f: impl FnMut(&mut String)) {
    let Some(events) = v.get_mut("hooks").and_then(Value::as_object_mut) else {
        return;
    };
    for entries in events.values_mut() {
        for entry in entries.as_array_mut().into_iter().flatten() {
            for hook in entry
                .get_mut("hooks")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                if let Some(Value::String(c)) = hook.get_mut("command") {
                    f(c);
                }
            }
        }
    }
}

/// The port of one of our hook commands: it posts to `http://127.0.0.1:<port>/hook` with the
/// companion's header file (`-H @…hook-auth.header`). `None` for anything else, such as the
/// developer's own hooks (plan 248 A1).
pub fn our_port(command: &str) -> Option<u16> {
    if !command.contains("-H @") || !command.contains(crate::kernel::config::HOOK_HEADER_FILE) {
        return None;
    }
    let rest = command.split("http://127.0.0.1:").nth(1)?;
    let (port, tail) = rest.split_once('/')?;
    tail.starts_with("hook").then_some(())?;
    port.parse().ok()
}

/// `existing` (a `settings.local.json`, if there is one) with our hooks from `ours` in place of
/// any of ours it had, event by event. The developer's own hooks and every other key are kept,
/// and a second merge of the same `ours` changes nothing.
pub fn merge_hooks(existing: Option<&str>, ours: &str) -> Result<String> {
    let mut doc: Map<String, Value> = match existing {
        Some(text) if !text.trim().is_empty() => match serde_json::from_str(text)
            .context("the settings file is not JSON; it was left as it is")?
        {
            Value::Object(m) => m,
            _ => bail!("the settings file is not a JSON object; it was left as it is"),
        },
        _ => Map::new(),
    };
    let ours: Value = serde_json::from_str(ours)?;
    let Some(our_events) = ours.get("hooks").and_then(Value::as_object) else {
        bail!("no hooks to merge");
    };
    let hooks = doc
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(hooks) = hooks.as_object_mut() else {
        bail!("the settings file's hooks is not an object; it was left as it is");
    };
    for (event, our_entries) in our_events {
        let mut kept: Vec<Value> = Vec::new();
        for mut entry in hooks
            .get(event)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            if let Some(list) = entry.get_mut("hooks").and_then(Value::as_array_mut) {
                list.retain(|h| {
                    h.get("command")
                        .and_then(Value::as_str)
                        .is_none_or(|c| our_port(c).is_none())
                });
                if list.is_empty() {
                    continue;
                }
            }
            kept.push(entry);
        }
        kept.extend(our_entries.as_array().cloned().unwrap_or_default());
        hooks.insert(event.clone(), Value::Array(kept));
    }
    Ok(serde_json::to_string_pretty(&Value::Object(doc))? + "\n")
}

/// Write our hooks into `path` (a `.claude/settings.local.json`), merged as [`merge_hooks`] says:
/// through a temp file and a rename, so a reader sees the old file or the new; the file keeps its
/// mode, and one created here is 0600 in a `.claude/` created if needed.
pub fn write_into(path: &Path, ours: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let existing = match std::fs::read_to_string(path) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let merged =
        merge_hooks(existing.as_deref(), ours).with_context(|| format!("{}", path.display()))?;
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let mode = match std::fs::metadata(path) {
        Ok(m) => m.permissions().mode() & 0o777,
        Err(_) => 0o600,
    };
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".rubato-tmp");
    let tmp = path.with_file_name(tmp_name);
    std::fs::write(&tmp, merged).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming over {}", path.display()))?;
    Ok(())
}

/// Where a directory's opt-in lives.
pub fn settings_path(dir: &Path) -> PathBuf {
    dir.join(".claude").join("settings.local.json")
}

/// Whether a directory's sessions report through our hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptIn {
    /// Our hooks, on the companion's port.
    In,
    /// Our hooks, on another port: a companion restart moved it (`hook port changed`).
    StalePort(u16),
    /// None of ours (no file, no hooks, or only the developer's own).
    Out,
}

pub fn opt_in(dir: &Path, port: u16) -> OptIn {
    let Ok(text) = std::fs::read_to_string(settings_path(dir)) else {
        return OptIn::Out;
    };
    let Ok(mut v) = serde_json::from_str::<Value>(&text) else {
        return OptIn::Out;
    };
    let mut ports = Vec::new();
    for_each_command(&mut v, |c| ports.extend(our_port(c)));
    match ports.iter().find(|p| **p != port) {
        _ if ports.is_empty() => OptIn::Out,
        Some(p) => OptIn::StalePort(*p),
        None => OptIn::In,
    }
}

/// The lines `rubato-companion status` prints for the directories a developer may start a
/// session in: each with its opt-in state, or the command that opts it in, and a count line.
pub fn report(dirs: &[PathBuf], port: u16) -> Vec<String> {
    let (mut opted, mut stale) = (0, 0);
    let mut lines: Vec<String> = dirs
        .iter()
        .map(|d| match opt_in(d, port) {
            OptIn::In => {
                opted += 1;
                format!("{}: opted in", d.display())
            }
            OptIn::StalePort(p) => {
                stale += 1;
                format!(
                    "{}: opted in, stale port {p}; run: rubato-companion render-hooks --into {}",
                    d.display(),
                    settings_path(d).display()
                )
            }
            OptIn::Out => format!(
                "{}: not opted in; run: rubato-companion render-hooks --into {}",
                d.display(),
                settings_path(d).display()
            ),
        })
        .collect();
    lines.push(format!(
        "checked {} directories: {opted} opted in, {stale} stale",
        dirs.len()
    ));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::HOOKS_TEMPLATE;

    const HEADER: &str = "/state/hook-auth.header";

    fn ours(port: u16) -> String {
        render_manual_hooks(HOOKS_TEMPLATE, port, Path::new(HEADER)).unwrap()
    }

    fn commands(text: &str) -> Vec<String> {
        let mut v: Value = serde_json::from_str(text).unwrap();
        let mut out = Vec::new();
        for_each_command(&mut v, |c| out.push(c.clone()));
        out
    }

    #[test]
    fn manual_hooks_stand_down_in_a_launched_session_and_carry_no_token() {
        let cmds = commands(&ours(4321));
        assert_eq!(cmds.len(), 9, "one per hook event");
        for c in &cmds {
            assert!(c.starts_with(STAND_DOWN), "{c}");
            assert!(c.contains("-H @'/state/hook-auth.header'"), "{c}");
            assert_eq!(our_port(c), Some(4321), "{c}");
        }
        // The guard does what it says under `sh -c`: with the launch id set, curl never runs.
        let probe = cmds[0].replacen("curl", "echo ran", 1);
        let run = |env: Option<&str>| {
            let mut c = std::process::Command::new("sh");
            c.args(["-c", &probe]).env_remove("RUBATO_LAUNCH_ID");
            if let Some(id) = env {
                c.env("RUBATO_LAUNCH_ID", id);
            }
            String::from_utf8(c.output().unwrap().stdout).unwrap()
        };
        assert!(run(None).starts_with("ran"), "a manual session posts");
        assert_eq!(
            run(Some("0f8fad5b")),
            "",
            "a launched session leaves it to --settings"
        );
    }

    /// Plan 248 "must not break": `render-hooks` with no flag is the launch's rendering, each
    /// command behind the prefix and nothing else changed.
    #[test]
    fn manual_hooks_are_the_launch_hooks_behind_the_prefix() {
        let launch = render_hooks(HOOKS_TEMPLATE, 4321, Path::new(HEADER)).unwrap();
        let mut want: Value = serde_json::from_str(&launch).unwrap();
        for_each_command(&mut want, |c| *c = format!("{STAND_DOWN}{c}"));
        let got: Value = serde_json::from_str(&ours(4321)).unwrap();
        assert_eq!(got, want);
    }

    #[test]
    fn our_port_recognises_only_our_commands() {
        assert_eq!(our_port(&commands(&ours(7))[0]), Some(7));
        assert_eq!(our_port("say done"), None);
        assert_eq!(
            our_port("curl -H @/etc/x http://127.0.0.1:9/hook"),
            None,
            "not the companion's header file"
        );
        assert_eq!(
            our_port("curl -H @/s/hook-auth.header http://127.0.0.1:9/other"),
            None
        );
    }

    /// Plan 248: the developer's own hooks and keys stay; ours are replaced, once per event.
    #[test]
    fn merging_keeps_the_developers_settings_and_is_idempotent() {
        let mine = r#"{
  "permissions": { "allow": ["Bash(npm test)"] },
  "hooks": {
    "Stop": [ { "hooks": [ { "type": "command", "command": "say done" } ] } ]
  }
}"#;
        let old = merge_hooks(Some(mine), &ours(1111)).unwrap();
        let once = merge_hooks(Some(&old), &ours(2222)).unwrap();
        let twice = merge_hooks(Some(&once), &ours(2222)).unwrap();
        assert_eq!(once, twice, "a second run changes nothing");
        let v: Value = serde_json::from_str(&once).unwrap();
        assert_eq!(v["permissions"]["allow"][0], "Bash(npm test)");
        let stop: Vec<String> = v["hooks"]["Stop"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|e| e["hooks"].as_array().unwrap().clone())
            .map(|h| h["command"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(stop.len(), 2, "{stop:?}");
        assert_eq!(stop[0], "say done");
        assert_eq!(our_port(&stop[1]), Some(2222));
        let ports: Vec<u16> = commands(&once).iter().filter_map(|c| our_port(c)).collect();
        assert_eq!(
            ports.len(),
            9,
            "one of ours per event, the old port gone: {ports:?}"
        );
        assert!(ports.iter().all(|p| *p == 2222));
    }

    #[test]
    fn a_file_that_is_not_json_is_refused_and_left_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        let e = write_into(&path, &ours(1)).unwrap_err();
        assert!(format!("{e:#}").contains("not JSON"), "{e:#}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    }

    #[test]
    fn a_new_file_is_private_and_an_existing_one_keeps_its_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(dir.path());
        write_into(&path, &ours(5)).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_into(&path, &ours(6)).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644
        );
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["settings.local.json"], "no temp file left");
    }

    #[test]
    fn the_report_names_each_directory_and_counts() {
        let root = tempfile::tempdir().unwrap();
        let (a, b, c) = (
            root.path().join("a"),
            root.path().join("b"),
            root.path().join("c"),
        );
        write_into(&settings_path(&a), &ours(5)).unwrap();
        write_into(&settings_path(&c), &ours(1)).unwrap();
        let lines = report(&[a.clone(), b.clone(), c.clone()], 5);
        assert_eq!(lines[0], format!("{}: opted in", a.display()));
        assert!(
            lines[1].starts_with(&format!(
                "{}: not opted in; run: rubato-companion render-hooks --into ",
                b.display()
            )),
            "{}",
            lines[1]
        );
        assert!(
            lines[2].starts_with(&format!("{}: opted in, stale port 1", c.display())),
            "{}",
            lines[2]
        );
        assert_eq!(lines[3], "checked 3 directories: 1 opted in, 1 stale");
    }

    #[test]
    fn opt_in_reads_in_stale_and_out() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(opt_in(dir.path(), 5), OptIn::Out);
        write_into(&settings_path(dir.path()), &ours(5)).unwrap();
        assert_eq!(opt_in(dir.path(), 5), OptIn::In);
        assert_eq!(opt_in(dir.path(), 6), OptIn::StalePort(5));
        std::fs::write(
            settings_path(dir.path()),
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"say done"}]}]}}"#,
        )
        .unwrap();
        assert_eq!(
            opt_in(dir.path(), 5),
            OptIn::Out,
            "only the developer's own"
        );
    }
}
