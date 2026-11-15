//! Restoring a parked draft into the launch's worktree (ADR 0053, plan 122 §D5). The files are
//! written only into a worktree this launch created, before the session starts, so the agent
//! finds the draft on its first turn. A reused worktree is never written: if its files at those
//! paths already equal the draft, the draft counts as restored; otherwise it is left alone
//! (`worktree_differs`). Every path is checked before the first write: relative, plain
//! components only, and every part that exists resolves inside the worktree, so a symlink never
//! leads a write out. Any violation writes nothing (`path_outside`). Blocking.

use anyhow::{Context, Result, bail};
use rubato_protocol::{DraftRestore, RestoreSkipped, bundle_hash};
use std::path::{Component, Path, PathBuf};

/// What happened to a launch's draft.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Restored {
    /// The draft is in the worktree (written, or already there).
    pub restored: bool,
    pub skipped: Option<RestoreSkipped>,
    /// The bundle hash of the restored files, so the plan mirror's first settle does not send
    /// the same bundle back.
    pub bundle_hash: Option<String>,
}

fn plain_relative(p: &str) -> bool {
    !p.is_empty()
        && Path::new(p)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

/// Every existing prefix of `rel` under `root` resolves inside `root` (which is canonical).
fn stays_inside(root: &Path, rel: &str) -> bool {
    let mut cur = root.to_path_buf();
    for c in Path::new(rel).components() {
        cur.push(c);
        match std::fs::symlink_metadata(&cur) {
            Ok(_) => match std::fs::canonicalize(&cur) {
                Ok(real) if real.starts_with(root) => {}
                // A link out of the worktree, or a dangling one.
                _ => return false,
            },
            // The rest is created under a parent already checked.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) => return false,
        }
    }
    true
}

/// Restore `draft` into `worktree`. `created` says whether this launch made the worktree.
pub fn restore(worktree: &Path, draft: &DraftRestore, created: bool) -> Result<Restored> {
    let root = std::fs::canonicalize(worktree)
        .with_context(|| format!("worktree {}", worktree.display()))?;
    let outside = draft
        .files
        .iter()
        .find(|f| !plain_relative(&f.path) || !stays_inside(&root, &f.path));
    if let Some(f) = outside {
        tracing::warn!(draft_id = %draft.draft_id, rev = draft.rev, path = %f.path, reason = RestoreSkipped::PathOutside.name(), "draft not restored");
        return Ok(Restored {
            skipped: Some(RestoreSkipped::PathOutside),
            ..Restored::default()
        });
    }
    let files: Vec<(String, String)> = draft
        .files
        .iter()
        .map(|f| (f.path.clone(), f.text.clone()))
        .collect();
    let hash = files.first().map(|(plan, _)| bundle_hash(plan, &files));
    if !created {
        let same = draft
            .files
            .iter()
            .all(|f| std::fs::read(root.join(&f.path)).is_ok_and(|b| b == f.text.as_bytes()));
        if !same {
            tracing::warn!(draft_id = %draft.draft_id, rev = draft.rev, reason = RestoreSkipped::WorktreeDiffers.name(), "draft not restored");
            return Ok(Restored {
                skipped: Some(RestoreSkipped::WorktreeDiffers),
                ..Restored::default()
            });
        }
        tracing::info!(draft_id = %draft.draft_id, rev = draft.rev, files = draft.files.len(), written = false, "draft restored");
        return Ok(Restored {
            restored: true,
            skipped: None,
            bundle_hash: hash,
        });
    }
    for f in &draft.files {
        let path: PathBuf = root.join(&f.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
            // Again after creating: nothing moved the parent out meanwhile.
            let real = std::fs::canonicalize(parent)?;
            if !real.starts_with(&root) {
                bail!("{} left the worktree while restoring", f.path);
            }
        }
        std::fs::write(&path, &f.text).with_context(|| format!("writing {}", path.display()))?;
    }
    tracing::info!(draft_id = %draft.draft_id, rev = draft.rev, files = draft.files.len(), written = true, "draft restored");
    Ok(Restored {
        restored: true,
        skipped: None,
        bundle_hash: hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rubato_protocol::DraftFile;
    use uuid::Uuid;

    fn draft(files: &[(&str, &str)]) -> DraftRestore {
        DraftRestore {
            draft_id: Uuid::from_u128(1),
            rev: 3,
            files: files
                .iter()
                .map(|(p, t)| DraftFile {
                    path: (*p).into(),
                    text: (*t).into(),
                })
                .collect(),
        }
    }

    const PLAN: (&str, &str) = ("docs/plans/7.md", "# Plan\n\n[d](7-restock/design.md)\n");
    const DESIGN: (&str, &str) = ("docs/plans/7-restock/design.md", "# Design\n");

    #[test]
    fn a_created_worktree_gets_every_file() {
        let dir = tempfile::tempdir().unwrap();
        let r = restore(dir.path(), &draft(&[PLAN, DESIGN]), true).unwrap();
        assert!(r.restored && r.skipped.is_none());
        assert_eq!(
            std::fs::read_to_string(dir.path().join(DESIGN.0)).unwrap(),
            DESIGN.1
        );
        let files = vec![
            (PLAN.0.to_owned(), PLAN.1.to_owned()),
            (DESIGN.0.to_owned(), DESIGN.1.to_owned()),
        ];
        assert_eq!(r.bundle_hash, Some(bundle_hash(PLAN.0, &files)));
    }

    #[test]
    fn a_reused_worktree_is_compared_never_written() {
        let dir = tempfile::tempdir().unwrap();
        // Missing files differ.
        let r = restore(dir.path(), &draft(&[PLAN]), false).unwrap();
        assert_eq!(r.skipped, Some(RestoreSkipped::WorktreeDiffers));
        assert!(!dir.path().join(PLAN.0).exists(), "nothing written");
        std::fs::create_dir_all(dir.path().join("docs/plans")).unwrap();
        std::fs::write(dir.path().join(PLAN.0), "# Someone's edit\n").unwrap();
        let r = restore(dir.path(), &draft(&[PLAN]), false).unwrap();
        assert_eq!(r.skipped, Some(RestoreSkipped::WorktreeDiffers));
        assert_eq!(
            std::fs::read_to_string(dir.path().join(PLAN.0)).unwrap(),
            "# Someone's edit\n"
        );
        std::fs::write(dir.path().join(PLAN.0), PLAN.1).unwrap();
        let r = restore(dir.path(), &draft(&[PLAN]), false).unwrap();
        assert!(r.restored && r.skipped.is_none() && r.bundle_hash.is_some());
    }

    /// One bad path and nothing at all is written, the good paths before it included.
    #[test]
    fn a_path_out_of_the_worktree_writes_nothing() {
        for bad in [
            "../x.md",
            "/tmp/rubato-abs.md",
            "a/../../x.md",
            "./x.md",
            "",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let r = restore(dir.path(), &draft(&[PLAN, (bad, "x")]), true).unwrap();
            assert_eq!(r.skipped, Some(RestoreSkipped::PathOutside), "{bad:?}");
            assert!(!r.restored);
            assert!(
                !dir.path().join(PLAN.0).exists(),
                "{bad:?}: the good path was written"
            );
        }
    }

    /// A link inside the worktree that points out of it is never followed.
    #[test]
    fn a_symlink_out_of_the_worktree_writes_nothing() {
        let outside = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("docs")).unwrap();
        let r = restore(dir.path(), &draft(&[PLAN, DESIGN]), true).unwrap();
        assert_eq!(r.skipped, Some(RestoreSkipped::PathOutside));
        assert_eq!(
            std::fs::read_dir(outside.path()).unwrap().count(),
            0,
            "nothing written outside"
        );
        // A link that stays inside is fine.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("real")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("docs")).unwrap();
        let r = restore(dir.path(), &draft(&[PLAN]), true).unwrap();
        assert!(r.restored, "{r:?}");
        assert!(dir.path().join("real/plans/7.md").exists());
    }
}
