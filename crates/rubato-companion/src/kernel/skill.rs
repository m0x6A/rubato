//! The installed rubato skill as the companion sees it: the version in its `package.json`
//! (ADR 0055). The skill installs at user level, `~/.claude/skills/rubato`
//! (`skills/rubato/scripts/install.ts`); `companion.skill_dir` in `local.yml` points elsewhere.

use std::path::{Path, PathBuf};

/// Where the skill installs by default.
pub fn default_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"));
    home.join(".claude").join("skills").join("rubato")
}

/// The `version` in `<dir>/package.json`. None when the file is missing, does not parse, or has
/// no version: then the companion announces no `skill:` capability.
pub fn installed_version(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("package.json")).ok()?;
    let pkg: serde_json::Value = serde_json::from_str(&text).ok()?;
    let v = pkg.get("version")?.as_str()?.trim();
    (!v.is_empty()).then(|| v.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_comes_from_package_json_or_not_at_all() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(installed_version(dir.path()), None, "no file");
        let pkg = dir.path().join("package.json");
        std::fs::write(&pkg, r#"{"name": "rubato-skill", "version": " 0.2.0 "}"#).unwrap();
        assert_eq!(installed_version(dir.path()).as_deref(), Some("0.2.0"));
        // What install.ts wrote before it carried the version.
        std::fs::write(&pkg, r#"{"name": "rubato-skill", "private": true}"#).unwrap();
        assert_eq!(installed_version(dir.path()), None);
        std::fs::write(&pkg, "{not json").unwrap();
        assert_eq!(installed_version(dir.path()), None);
        std::fs::write(&pkg, r#"{"version": ""}"#).unwrap();
        assert_eq!(installed_version(dir.path()), None);
    }
}
