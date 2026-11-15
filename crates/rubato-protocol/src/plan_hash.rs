//! The hashes that name one version of a plan, and the files that travel with it (ADR 0027,
//! ADR 0054; plan 122, design §D3). Pure. The TypeScript twins are
//! `actions/plan-check/src/plan-sha.ts` and `actions/plan-check/src/bundle.ts`, held equal by the
//! shared fixtures under `schema/fixtures/plans/approval-hash/` and `schema/fixtures/plans/bundle/`.

use sha1::{Digest, Sha1};

/// The front-matter keys of the approval record (ADR 0054). They are left out of the approval
/// hash, so writing the record does not change the hash it records.
pub const APPROVAL_KEYS: [&str; 3] = ["approved_by", "approved_at", "approved_sha"];

/// `text` without the approval record: whole lines of the first front-matter block that start with
/// `approved_by:`, `approved_at:` or `approved_sha:`. Every other byte is kept, line endings
/// included. A text without front matter is returned as it is.
pub fn strip_approval_record(text: &str) -> String {
    let mut lines = text.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return String::new();
    };
    if first.trim() != "---" {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    out.push_str(first);
    let mut in_front = true;
    for line in lines {
        if in_front && line.trim() == "---" {
            in_front = false;
        } else if in_front && is_approval_line(line) {
            continue;
        }
        out.push_str(line);
    }
    out
}

fn is_approval_line(line: &str) -> bool {
    APPROVAL_KEYS.iter().any(|k| {
        line.strip_prefix(k)
            .is_some_and(|rest| rest.starts_with(':'))
    })
}

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

/// The text the approval hash is taken over: the approval record removed, checkboxes cleared.
pub fn normalise_plan(text: &str) -> String {
    clear_checkboxes(&strip_approval_record(text))
}

/// Git's blob sha1 of `bytes`, as lower-case hex: `sha1("blob <len>\0" + bytes)`.
pub fn git_blob_sha(bytes: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(format!("blob {}\0", bytes.len()).as_bytes());
    h.update(bytes);
    hex(&h.finalize())
}

/// The approval hash of a plan (ADR 0027, ADR 0054): the git blob sha1 of the normalised text.
/// Ticking a task or writing the approval record keeps it; any other edit changes it.
pub fn approval_hash(text: &str) -> String {
    git_blob_sha(normalise_plan(text).as_bytes())
}

/// Whether an approved sha (7 to 40 hex characters) names `current`.
pub fn approval_names(approved: &str, current: &str) -> bool {
    approved.len() >= 7 && current.starts_with(&approved.to_ascii_lowercase())
}

/// The files of a plan's bundle, as repository paths: the plan itself first, then, sorted and
/// without duplicates:
/// - every relative Markdown link in the plan (outside code fences) that resolves under the
///   plan's design folder, `<plan dir>/<plan stem>-…/`;
/// - every path in the File column of the plan's `## ADRs proposed` table.
///
/// Paths that leave the repository are dropped. Anchors and queries are cut.
pub fn bundle_paths(plan_text: &str, plan_path: &str) -> Vec<String> {
    let plan_path = normalise_path(plan_path).unwrap_or_else(|| plan_path.to_owned());
    let (dir, stem) = match plan_path.rsplit_once('/') {
        Some((d, f)) => (d.to_owned(), f.trim_end_matches(".md").to_owned()),
        None => (String::new(), plan_path.trim_end_matches(".md").to_owned()),
    };
    let design_prefix = if dir.is_empty() {
        format!("{stem}-")
    } else {
        format!("{dir}/{stem}-")
    };
    let mut found = std::collections::BTreeSet::new();
    let mut in_fence = false;
    for line in plan_text.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        for target in link_targets(line) {
            let joined = if dir.is_empty() {
                target.clone()
            } else {
                format!("{dir}/{target}")
            };
            if let Some(p) = normalise_path(&joined)
                && p.starts_with(&design_prefix)
                && p[design_prefix.len()..].contains('/')
            {
                found.insert(p);
            }
        }
    }
    for f in adr_files(plan_text) {
        if let Some(p) = normalise_path(&f) {
            found.insert(p);
        }
    }
    found.remove(&plan_path);
    let mut out = vec![plan_path];
    out.extend(found);
    out
}

/// The hash of a whole bundle: sha1 over the lines `<path>\0<blob sha>\n`, sorted by path. The
/// plan's blob sha is its approval hash, so its approval record and checkboxes do not count.
pub fn bundle_hash(plan_path: &str, files: &[(String, String)]) -> String {
    let mut lines: Vec<(String, String)> = files
        .iter()
        .map(|(path, text)| {
            let blob = if path == plan_path {
                approval_hash(text)
            } else {
                git_blob_sha(text.as_bytes())
            };
            (path.clone(), blob)
        })
        .collect();
    lines.sort();
    let mut h = Sha1::new();
    for (path, blob) in &lines {
        h.update(path.as_bytes());
        h.update([0u8]);
        h.update(blob.as_bytes());
        h.update(b"\n");
    }
    hex(&h.finalize())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Relative link targets `[text](target)` on one line: no scheme, not absolute, not an anchor.
fn link_targets(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(i) = rest.find("](") {
        let after = &rest[i + 2..];
        let Some(end) = after.find(')') else { break };
        let raw = after[..end].trim();
        let target = raw
            .split_once(' ')
            .map(|(t, _)| t)
            .unwrap_or(raw)
            .trim_matches(['<', '>']);
        let target = target.split(['#', '?']).next().unwrap_or("");
        if !target.is_empty() && !target.starts_with('/') && !target.contains("://") {
            out.push(target.to_owned());
        }
        rest = &after[end..];
    }
    out
}

/// `a/./b/../c` → `a/c`; `None` when the path leaves the repository or is empty.
fn normalise_path(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// One row of a plan's `## ADRs proposed` table: the ADR, its file, and the blob the plan pins
/// it at (ADR 0035, ADR 0054). The TypeScript twin is `pinnedAdrs` in `skills/rubato/scripts/adrs.ts`,
/// held equal by `schema/fixtures/plans/adr-pins/`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AdrPin {
    /// The four-digit number.
    pub adr: String,
    pub file: String,
    /// Lower-case; `None` when the Blob cell is empty or the table has no Blob column.
    pub blob: Option<String>,
}

/// The rows of `## ADRs proposed` whose ADR cell is a four-digit number, in order, backticks
/// removed. Empty when the section is absent or says None.
pub fn adr_pins(plan_text: &str) -> Vec<AdrPin> {
    let Some((header, rows)) = adrs_table(plan_text) else {
        return vec![];
    };
    let col = |name: &str| {
        header
            .iter()
            .position(|h| h.to_ascii_lowercase().starts_with(name))
    };
    let (Some(adr), Some(file)) = (col("adr"), col("file")) else {
        return vec![];
    };
    let blob = col("blob");
    let cell = |r: &[String], i: usize| {
        r.get(i)
            .map(|c| c.replace('`', "").trim().to_owned())
            .unwrap_or_default()
    };
    rows.iter()
        .filter_map(|r| {
            let number = cell(r, adr);
            if number.len() != 4 || !number.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let b = blob.map(|i| cell(r, i).to_ascii_lowercase());
            Some(AdrPin {
                adr: number,
                file: cell(r, file),
                blob: b.filter(|b| !b.is_empty()),
            })
        })
        .collect()
}

/// The File column of the `## ADRs proposed` table, backticks removed.
fn adr_files(text: &str) -> Vec<String> {
    let Some((header, rows)) = adrs_table(text) else {
        return vec![];
    };
    let Some(col) = header
        .iter()
        .position(|h| h.to_ascii_lowercase().starts_with("file"))
    else {
        return vec![];
    };
    rows.iter()
        .filter_map(|r| r.get(col))
        .map(|c| c.replace('`', "").trim().to_owned())
        .filter(|c| !c.is_empty())
        .collect()
}

/// The first table under the `ADRs proposed` heading: its header cells and its body rows.
fn adrs_table(text: &str) -> Option<(Vec<String>, Vec<Vec<String>>)> {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.iter().position(|l| {
        let h = l.trim_start_matches('#');
        h.len() < l.len()
            && h.starts_with(' ')
            && h.trim().to_ascii_lowercase().starts_with("adrs proposed")
    })?;
    let mut i = start + 1;
    while i < lines.len() && !lines[i].starts_with('#') && !lines[i].trim_start().starts_with('|') {
        i += 1;
    }
    let mut rows: Vec<Vec<String>> = lines[i..]
        .iter()
        .take_while(|l| l.trim_start().starts_with('|'))
        .map(|l| split_row(l))
        .collect();
    if rows.is_empty() {
        return None;
    }
    let header = rows.remove(0);
    Some((header, rows.into_iter().skip(1).collect()))
}

fn split_row(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut in_code = false;
    for ch in line.trim().chars() {
        if ch == '`' {
            in_code = !in_code;
        }
        if ch == '|' && !in_code {
            cells.push(std::mem::take(&mut cur));
        } else {
            cur.push(ch);
        }
    }
    cells.push(cur);
    if cells.first().is_some_and(|c| c.trim().is_empty()) {
        cells.remove(0);
    }
    if cells.last().is_some_and(|c| c.trim().is_empty()) {
        cells.pop();
    }
    cells.into_iter().map(|c| c.trim().to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixtures(sub: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../schema/fixtures/plans")
            .join(sub)
    }

    /// Every `<name>.md` hashes to what `<name>.txt` says. Shared with the TypeScript twins.
    #[test]
    fn matches_the_shared_hash_fixtures() {
        let dir = fixtures("approval-hash");
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
        assert_eq!(checked, 5, "fixtures checked");
        println!("checked {checked} approval-hash fixtures");
    }

    #[test]
    fn the_approval_record_does_not_change_the_hash() {
        let read = |f: &str| std::fs::read_to_string(fixtures("approval-hash").join(f)).unwrap();
        assert_eq!(
            approval_hash(&read("with-keys.md")),
            approval_hash(&read("unticked.md"))
        );
        // Only front matter lines count: the same line in the body is plan text.
        let body = format!("{}\napproved_by: m0x6A\n", read("unticked.md"));
        assert_ne!(approval_hash(&body), approval_hash(&read("unticked.md")));
        // A key without its colon is not the key.
        assert_eq!(
            strip_approval_record("---\napproved_byx: y\n---\n"),
            "---\napproved_byx: y\n---\n"
        );
        // Line endings are kept.
        assert_eq!(
            strip_approval_record("---\r\nissue: a/b#1\r\napproved_sha: x\r\n---\r\nbody\r\n"),
            "---\r\nissue: a/b#1\r\n---\r\nbody\r\n"
        );
    }

    #[test]
    fn ticking_a_task_keeps_the_hash_and_editing_does_not() {
        let read = |f: &str| std::fs::read_to_string(fixtures("approval-hash").join(f)).unwrap();
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
        assert!(approval_names("f3c5e84", full));
        assert!(approval_names("F3C5E84F2B", full));
        assert!(
            !approval_names("f3c5e8", full),
            "fewer than 7 names nothing"
        );
        assert!(!approval_names("c438e00", full));
    }

    #[derive(serde::Deserialize)]
    struct BundleCase {
        plan_path: String,
        paths: Vec<String>,
        files: Vec<(String, String)>,
        hash: String,
    }

    /// `bundle/plan.md` gives `paths` and, with `files`, `hash` (`bundle/case.json`), in Rust as in
    /// TypeScript.
    #[test]
    fn matches_the_shared_bundle_fixture() {
        let dir = fixtures("bundle");
        let case: BundleCase =
            serde_json::from_str(&std::fs::read_to_string(dir.join("case.json")).unwrap()).unwrap();
        let text = std::fs::read_to_string(dir.join("plan.md")).unwrap();
        assert_eq!(bundle_paths(&text, &case.plan_path), case.paths);
        assert_eq!(bundle_hash(&case.plan_path, &case.files), case.hash);
        // The approval record and a ticked box leave the bundle hash as it is; an edit does not.
        let mut keyed = case.files.clone();
        keyed[0].1 = keyed[0].1.replacen(
            "---\n",
            "---\napproved_by: m0x6A\napproved_at: x\napproved_sha: y\n",
            1,
        );
        assert_eq!(bundle_hash(&case.plan_path, &keyed), case.hash);
        let mut edited = case.files.clone();
        edited[1].1.push_str("one more line\n");
        assert_ne!(bundle_hash(&case.plan_path, &edited), case.hash);
        println!(
            "checked {} bundle paths and {} files",
            case.paths.len(),
            case.files.len()
        );
    }

    /// `adr-pins/<name>.md` gives the rows of `<name>.json`, in Rust as in TypeScript
    /// (`pinnedAdrs`), an empty Blob being `""` there and `None` here.
    #[test]
    fn matches_the_shared_adr_pin_fixtures() {
        #[derive(serde::Deserialize)]
        struct Row {
            adr: String,
            file: String,
            blob: String,
        }
        let dir = fixtures("adr-pins");
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "md") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let want: Vec<Row> = serde_json::from_str(
                &std::fs::read_to_string(path.with_extension("json")).unwrap(),
            )
            .unwrap();
            let want: Vec<AdrPin> = want
                .into_iter()
                .map(|r| AdrPin {
                    adr: r.adr,
                    file: r.file,
                    blob: (!r.blob.is_empty()).then_some(r.blob),
                })
                .collect();
            assert_eq!(adr_pins(&text), want, "{}", path.display());
            checked += 1;
        }
        assert_eq!(checked, 2, "fixtures checked");
        println!("checked {checked} adr-pin fixtures");
        // The bundle fixture's table: one row unpinned, one pinned.
        let bundle = std::fs::read_to_string(fixtures("bundle").join("plan.md")).unwrap();
        let pins = adr_pins(&bundle);
        assert_eq!(
            pins.iter().map(|p| p.blob.is_some()).collect::<Vec<_>>(),
            [false, true]
        );
    }

    #[test]
    fn paths_outside_the_repository_or_the_design_folder_are_not_bundled() {
        let text = "[a](../../../etc/passwd) [b](122-x/../../../../x) [c](other/design.md) [d](122-x/design.md#d1) [e](https://x.test/122-x/y.md) [f](/abs/122-x/z.md)\n";
        assert_eq!(
            bundle_paths(text, "docs/plans/122.md"),
            ["docs/plans/122.md", "docs/plans/122-x/design.md"]
        );
    }
}
