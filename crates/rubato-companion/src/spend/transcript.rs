//! Which transcript a hook may make the companion read (ADR 0063). A hook's `transcript_path` is
//! read only when it resolves, symlinks followed, to a regular file under the Claude projects
//! dir, and only up to a cap; anything else is refused before a byte is read, so a forged or
//! mistaken path can neither feed the spend cap nor make the read grow without bound
//! (`/dev/zero`).

use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

/// The largest transcript read for spend. A long session's transcript is tens of MiB.
pub const MAX_TRANSCRIPT_BYTES: u64 = 256 * 1024 * 1024;

/// Why a transcript was not read; `reason()` is the `rubato.reason` value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Resolves outside the Claude projects dir, or that dir does not exist.
    OutsideRoot,
    /// Not a regular file: a device, a FIFO, a directory.
    NotAFile,
    /// Bigger than the cap, by its metadata or by what could be read.
    TooLarge,
    /// Missing, unreadable or not UTF-8.
    Unreadable(String),
}

impl Refusal {
    pub fn reason(&self) -> &'static str {
        match self {
            Refusal::OutsideRoot => "outside_root",
            Refusal::NotAFile => "not_a_file",
            Refusal::TooLarge => "too_large",
            Refusal::Unreadable(_) => "unreadable",
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Unreadable(e) => write!(f, "unreadable: {e}"),
            other => f.write_str(other.reason()),
        }
    }
}

impl std::error::Error for Refusal {}

/// Where Claude Code keeps transcripts: `$CLAUDE_CONFIG_DIR/projects`, else
/// `$HOME/.claude/projects`, from the companion's environment.
pub fn claude_projects_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d).join("projects"));
    }
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join(".claude").join("projects"))
}

/// Read `path` if it is a regular file under `root` (both resolved) of at most `cap` bytes.
pub fn read_transcript(path: &Path, root: &Path, cap: u64) -> Result<String, Refusal> {
    let root = root.canonicalize().map_err(|_| Refusal::OutsideRoot)?;
    let real = path
        .canonicalize()
        .map_err(|e| Refusal::Unreadable(e.to_string()))?;
    if !real.starts_with(&root) || real == root {
        return Err(Refusal::OutsideRoot);
    }
    // stat before open: opening a FIFO for reading would block until a writer comes.
    let meta = std::fs::metadata(&real).map_err(|e| Refusal::Unreadable(e.to_string()))?;
    if !meta.is_file() {
        return Err(Refusal::NotAFile);
    }
    let file = std::fs::File::open(&real).map_err(|e| Refusal::Unreadable(e.to_string()))?;
    // Again on the open handle, in case the path changed in between.
    let meta = file
        .metadata()
        .map_err(|e| Refusal::Unreadable(e.to_string()))?;
    if !meta.is_file() {
        return Err(Refusal::NotAFile);
    }
    if meta.len() > cap {
        return Err(Refusal::TooLarge);
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    // One byte past the cap tells a file that grew after the check from one that fits.
    file.take(cap + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Refusal::Unreadable(e.to_string()))?;
    if bytes.len() as u64 > cap {
        return Err(Refusal::TooLarge);
    }
    String::from_utf8(bytes).map_err(|e| Refusal::Unreadable(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
        outside: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(".claude").join("projects");
        std::fs::create_dir_all(root.join("-w-issue-7")).unwrap();
        let outside = dir.path().join("elsewhere");
        std::fs::create_dir_all(&outside).unwrap();
        Fixture {
            _dir: dir,
            root,
            outside,
        }
    }

    #[test]
    fn a_transcript_under_the_root_is_read() {
        let f = fixture();
        let p = f.root.join("-w-issue-7").join("s1.jsonl");
        std::fs::write(&p, "{\"a\":1}\n").unwrap();
        assert_eq!(read_transcript(&p, &f.root, 1024).unwrap(), "{\"a\":1}\n");
    }

    /// The issue's own case: today this read grows until the companion runs out of memory.
    #[test]
    fn dev_zero_is_refused_before_any_read() {
        let f = fixture();
        assert_eq!(
            read_transcript(Path::new("/dev/zero"), &f.root, 1024),
            Err(Refusal::OutsideRoot)
        );
        // Linked from inside the root: the resolved path is what is checked.
        let link = f.root.join("-w-issue-7").join("zero.jsonl");
        symlink("/dev/zero", &link).unwrap();
        assert_eq!(
            read_transcript(&link, &f.root, 1024),
            Err(Refusal::OutsideRoot)
        );
        // Even with a root that holds it, a device is not a file.
        assert_eq!(
            read_transcript(Path::new("/dev/zero"), Path::new("/dev"), 1024),
            Err(Refusal::NotAFile)
        );
    }

    #[test]
    fn paths_that_leave_the_root_are_refused() {
        let f = fixture();
        let secret = f.outside.join("secret.jsonl");
        std::fs::write(&secret, "x").unwrap();
        assert_eq!(
            read_transcript(&secret, &f.root, 1024),
            Err(Refusal::OutsideRoot)
        );
        let dotdot = f
            .root
            .join("..")
            .join("..")
            .join("elsewhere")
            .join("secret.jsonl");
        assert_eq!(
            read_transcript(&dotdot, &f.root, 1024),
            Err(Refusal::OutsideRoot)
        );
        let link = f.root.join("-w-issue-7").join("link.jsonl");
        symlink(&secret, &link).unwrap();
        assert_eq!(
            read_transcript(&link, &f.root, 1024),
            Err(Refusal::OutsideRoot)
        );
        // A root that does not exist admits nothing.
        assert_eq!(
            read_transcript(&secret, &f.outside.join("nope"), 1024),
            Err(Refusal::OutsideRoot)
        );
    }

    #[test]
    fn directories_and_fifos_are_not_files() {
        let f = fixture();
        assert_eq!(
            read_transcript(&f.root.join("-w-issue-7"), &f.root, 1024),
            Err(Refusal::NotAFile)
        );
        let fifo = f.root.join("-w-issue-7").join("fifo.jsonl");
        let ok = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success();
        assert!(ok, "mkfifo");
        // Would block forever if opened before the check.
        assert_eq!(
            read_transcript(&fifo, &f.root, 1024),
            Err(Refusal::NotAFile)
        );
    }

    #[test]
    fn a_transcript_over_the_cap_is_refused() {
        let f = fixture();
        let p = f.root.join("-w-issue-7").join("big.jsonl");
        std::fs::write(&p, vec![b'x'; 2048]).unwrap();
        assert_eq!(read_transcript(&p, &f.root, 1024), Err(Refusal::TooLarge));
        assert_eq!(read_transcript(&p, &f.root, 2048).unwrap().len(), 2048);
    }

    #[test]
    fn a_missing_transcript_is_unreadable() {
        let f = fixture();
        let r = read_transcript(&f.root.join("-w-issue-7").join("none.jsonl"), &f.root, 1024);
        assert_eq!(r.unwrap_err().reason(), "unreadable");
    }
}
