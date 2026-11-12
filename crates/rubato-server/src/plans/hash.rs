//! The approval hash of a plan file (ADR 0027): task checkboxes cleared, then the git blob sha1.
//! Ticking a task keeps an approval current; any other edit does not. The skill's `plan.ts sha`
//! prints the same value; both test against `schema/fixtures/plans/approval-hash/`.

use sha1::{Digest, Sha1};

/// `text` with every checkbox that opens a list item set to `[ ]`: per line,
/// `^(\s*(?:[-*+]|\d+[.)])\s+)\[[xX]\]` → `$1[ ]` (spaces and tabs). Everything else is left
/// byte for byte.
pub fn clear_checkboxes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        out.push_str(&clear_line(line));
    }
    out
}

fn clear_line(line: &str) -> std::borrow::Cow<'_, str> {
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    let rest = &line[indent..];
    let marker = if rest.starts_with(['-', '*', '+']) {
        1
    } else {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 && rest[digits..].starts_with(['.', ')']) {
            digits + 1
        } else {
            return line.into();
        }
    };
    let after = &rest[marker..];
    let spaces = after.len() - after.trim_start_matches([' ', '\t']).len();
    if spaces == 0 {
        return line.into();
    }
    let at = indent + marker + spaces;
    if line[at..].starts_with("[x]") || line[at..].starts_with("[X]") {
        format!("{}[ ]{}", &line[..at], &line[at + 3..]).into()
    } else {
        line.into()
    }
}

/// The git blob sha1 of `text` with its checkboxes cleared, as lower-case hex.
pub fn approval_hash(text: &str) -> String {
    let cleared = clear_checkboxes(text);
    let mut h = Sha1::new();
    h.update(format!("blob {}\0", cleared.len()).as_bytes());
    h.update(cleared.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether an approved sha (7 to 40 hex characters, from a comment) names `current`.
pub fn names(approved: &str, current: &str) -> bool {
    approved.len() >= 7 && current.starts_with(&approved.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/fixtures/plans/approval-hash")
    }

    /// Every `<name>.md` hashes to what `<name>.txt` says. The fixtures are shared with the
    /// skill's `plan.ts sha` test.
    #[test]
    fn matches_the_shared_fixtures() {
        let dir = fixtures();
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "md") {
                let text = std::fs::read_to_string(&path).unwrap();
                let want = std::fs::read_to_string(path.with_extension("txt")).unwrap();
                assert_eq!(approval_hash(&text), want.trim(), "{}", path.display());
                checked += 1;
            }
        }
        assert_eq!(checked, 3, "fixtures checked");
    }

    #[test]
    fn ticking_a_task_keeps_the_hash_and_editing_does_not() {
        let dir = fixtures();
        let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap();
        assert_eq!(
            approval_hash(&read("ticked.md")),
            approval_hash(&read("unticked.md"))
        );
        assert_ne!(
            approval_hash(&read("ticked.md")),
            approval_hash(&read("edited.md"))
        );
    }

    #[test]
    fn plain_blob_sha_when_there_are_no_boxes() {
        // `printf 'hello\n' | git hash-object --stdin`
        assert_eq!(
            approval_hash("hello\n"),
            "ce013625030ba8dba906f756967f9e9ca394464a"
        );
    }

    #[test]
    fn clears_only_list_item_boxes() {
        assert_eq!(clear_line("- [x] a\n"), "- [ ] a\n");
        assert_eq!(clear_line("\t12) [X] b"), "\t12) [ ] b");
        assert_eq!(clear_line("-\t[x] tab"), "-\t[ ] tab");
        assert_eq!(clear_line("-[x] no space"), "-[x] no space");
        assert_eq!(clear_line("see [x] here"), "see [x] here");
        assert_eq!(clear_line("1. [ ] open"), "1. [ ] open");
        assert_eq!(clear_line("12 [x] no dot"), "12 [x] no dot");
    }

    #[test]
    fn a_short_sha_names_its_full_hash() {
        let full = "f3c5e84f2bf82c2a409ad90e83f3236012b1dfb9";
        assert!(names("f3c5e84", full));
        assert!(names("F3C5E84F2B", full));
        assert!(
            !names("f3c5e8", full),
            "fewer than 7 characters names nothing"
        );
        assert!(!names("c438e00", full));
    }
}
