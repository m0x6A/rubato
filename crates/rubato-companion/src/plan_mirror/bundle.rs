//! Reading a plan bundle from a worktree (ADR 0053): the plan at its path, then every file
//! `bundle_paths` names. Paths are repository-relative and never leave the worktree: one that is
//! absolute, has `..`, or resolves (through a link) outside the worktree is not read, and counts as
//! missing. Blocking: call from `spawn_blocking`.

use anyhow::{Context, Result};
use rubato_protocol::bundle_paths;
use std::path::{Component, Path, PathBuf};

/// What a worktree holds for one plan path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Read {
    /// No plan file there: nothing is sent.
    NoPlan,
    /// The plan first, then the rest of the bundle that exists; the absent paths in `missing`.
    Bundle {
        plan_path: String,
        files: Vec<(String, String)>,
        missing: Vec<String>,
        bytes: u64,
    },
    /// The files together are larger than the limit; none were read past the plan.
    TooLarge { bytes: u64, missing: Vec<String> },
}

enum Found {
    File(PathBuf, u64),
    Missing,
    Outside,
}

/// A relative path of plain components only.
fn plain_relative(p: &str) -> bool {
    !p.is_empty()
        && Path::new(p)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

/// `rel` under `root` (already canonical), as a regular file inside it.
fn find(root: &Path, rel: &str) -> Found {
    if !plain_relative(rel) {
        return Found::Outside;
    }
    let Ok(real) = std::fs::canonicalize(root.join(rel)) else {
        return Found::Missing;
    };
    if !real.starts_with(root) {
        return Found::Outside;
    }
    match std::fs::metadata(&real) {
        Ok(m) if m.is_file() => Found::File(real, m.len()),
        _ => Found::Missing,
    }
}

fn text(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(String::from_utf8(bytes)
        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()))
}

/// Read the bundle of the plan at `plan_path` in `worktree`, up to `max_bytes` in all.
pub fn read_bundle(worktree: &Path, plan_path: &str, max_bytes: u64) -> Result<Read> {
    let root = std::fs::canonicalize(worktree)
        .with_context(|| format!("worktree {}", worktree.display()))?;
    let (plan, plan_len) = match find(&root, plan_path) {
        Found::File(p, n) => (p, n),
        Found::Missing => return Ok(Read::NoPlan),
        Found::Outside => {
            tracing::warn!(
                plan_path,
                "plan path outside the worktree; nothing mirrored"
            );
            return Ok(Read::NoPlan);
        }
    };
    if plan_len > max_bytes {
        return Ok(Read::TooLarge {
            bytes: plan_len,
            missing: Vec::new(),
        });
    }
    let plan_text = text(&plan)?;
    let paths = bundle_paths(&plan_text, plan_path);
    let plan_path = paths[0].clone();
    let mut found = Vec::new();
    let mut missing = Vec::new();
    let mut bytes = plan_len;
    for rel in &paths[1..] {
        match find(&root, rel) {
            Found::File(p, n) => {
                bytes += n;
                found.push((rel.clone(), p));
            }
            Found::Missing => missing.push(rel.clone()),
            Found::Outside => {
                tracing::warn!(path = %rel, "draft file outside the worktree not read");
                missing.push(rel.clone());
            }
        }
    }
    if bytes > max_bytes {
        return Ok(Read::TooLarge { bytes, missing });
    }
    let mut files = vec![(plan_path.clone(), plan_text)];
    for (rel, p) in found {
        files.push((rel, text(&p)?));
    }
    Ok(Read::Bundle {
        plan_path,
        files,
        missing,
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAN: &str =
        "# Plan\n\nSee [the design](7-restock/design.md) and [notes](7-restock/notes.md).\n";

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn the_plan_and_its_linked_files_are_read_and_the_absent_ones_listed() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            read_bundle(dir.path(), "docs/plans/7.md", 1000).unwrap(),
            Read::NoPlan
        );
        write(dir.path(), "docs/plans/7.md", PLAN);
        write(dir.path(), "docs/plans/7-restock/design.md", "# Design\n");
        match read_bundle(dir.path(), "docs/plans/7.md", 1000).unwrap() {
            Read::Bundle {
                plan_path,
                files,
                missing,
                bytes,
            } => {
                assert_eq!(plan_path, "docs/plans/7.md");
                let paths: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
                assert_eq!(paths, ["docs/plans/7.md", "docs/plans/7-restock/design.md"]);
                assert_eq!(files[1].1, "# Design\n");
                assert_eq!(missing, ["docs/plans/7-restock/notes.md"]);
                assert_eq!(bytes, (PLAN.len() + "# Design\n".len()) as u64);
            }
            r => panic!("expected a bundle, got {r:?}"),
        }
    }

    #[test]
    fn a_bundle_over_the_limit_is_too_large() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "docs/plans/7.md", PLAN);
        write(
            dir.path(),
            "docs/plans/7-restock/design.md",
            &"x".repeat(500),
        );
        let total = (PLAN.len() + 500) as u64;
        assert!(matches!(
            read_bundle(dir.path(), "docs/plans/7.md", total).unwrap(),
            Read::Bundle { .. }
        ));
        assert_eq!(
            read_bundle(dir.path(), "docs/plans/7.md", total - 1).unwrap(),
            Read::TooLarge {
                bytes: total,
                missing: vec!["docs/plans/7-restock/notes.md".into()]
            }
        );
        // The plan alone over the limit: not even parsed.
        assert_eq!(
            read_bundle(dir.path(), "docs/plans/7.md", 10).unwrap(),
            Read::TooLarge {
                bytes: PLAN.len() as u64,
                missing: vec![]
            }
        );
    }

    /// A link out of the worktree is never followed: the file counts as missing, and a plan path
    /// that leaves the worktree mirrors nothing.
    #[test]
    fn nothing_outside_the_worktree_is_read() {
        let outside = tempfile::tempdir().unwrap();
        write(outside.path(), "secret.md", "TOKEN=abc");
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "docs/plans/7.md", PLAN);
        std::fs::create_dir_all(dir.path().join("docs/plans/7-restock")).unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.md"),
            dir.path().join("docs/plans/7-restock/design.md"),
        )
        .unwrap();
        match read_bundle(dir.path(), "docs/plans/7.md", 1000).unwrap() {
            Read::Bundle { files, missing, .. } => {
                assert_eq!(files.len(), 1, "{files:?}");
                assert!(missing.contains(&"docs/plans/7-restock/design.md".to_owned()));
                assert!(files.iter().all(|(_, t)| !t.contains("TOKEN")));
            }
            r => panic!("expected a bundle, got {r:?}"),
        }
        for bad in ["../x/7.md", "/etc/passwd", "docs/../../x.md", ""] {
            assert_eq!(
                read_bundle(dir.path(), bad, 1000).unwrap(),
                Read::NoPlan,
                "{bad}"
            );
        }
    }
}
