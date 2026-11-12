//! The vertical-slice layout check (ADR 0015, design S1–S3, S6, D3). Test-only.
//!
//! Reads every file under `src/`, removes comments and string literals, and expands every
//! `crate::` and `super::` path (plain, grouped `use crate::{…}`, inline) to an absolute module
//! path. Then it checks:
//!
//! - every directory is `kernel` or a declared slice, every declared slice exists, and the
//!   top-level files are exactly the composition root;
//! - the declared list equals the fenced list in AGENTS.md's `<!-- slices: rubato-server -->`;
//! - a slice names another slice only at its root (`crate::<slice>::<item>`), the kernel at most
//!   `crate::kernel::<module>::<item>`, and never the composition root;
//! - the kernel names no slice and no composition-root module;
//! - the slice graph has no cycle.
//!
//! A path stops at its first capitalised segment, so `crate::kernel::github::FakeGitHub::new`
//! counts as `kernel::github::FakeGitHub`. Edges made by inherent `impl` blocks are invisible to
//! this check (design D7). `slices.rs` itself is not scanned.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const DEPLOYABLE: &str = "rubato-server";

/// Design D1, in the order AGENTS.md lists them.
const SLICES: &[&str] = &[
    "auth",
    "pairing",
    "companion_link",
    "pins",
    "launch",
    "layout",
    "map",
    "map_draft",
    "new_issue",
    "plans",
    "queue",
    "github_poll",
    "overview",
    "web_link",
];

/// The composition root plus this check.
const ROOT_FILES: &[&str] = &["lib.rs", "main.rs", "app.rs", "tasks.rs", "slices.rs"];

#[derive(Debug, Default)]
struct Report {
    slices: usize,
    edges: BTreeSet<(String, String)>,
    problems: Vec<String>,
}

/// Replace comments and string or char literals with spaces, keeping newlines.
fn strip(src: &str) -> String {
    let c: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let blank = |out: &mut String, ch: char| out.push(if ch == '\n' { '\n' } else { ' ' });
    while i < c.len() {
        let next = c.get(i + 1).copied();
        if c[i] == '/' && next == Some('/') {
            while i < c.len() && c[i] != '\n' {
                out.push(' ');
                i += 1;
            }
        } else if c[i] == '/' && next == Some('*') {
            let mut depth = 0;
            while i < c.len() {
                if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    blank(&mut out, c[i]);
                    i += 1;
                }
            }
        } else if c[i] == 'r'
            && (next == Some('"') || next == Some('#'))
            && (i == 0 || !(c[i - 1].is_alphanumeric() || c[i - 1] == '_'))
        {
            // Raw string r#"…"#.
            let mut j = i + 1;
            let mut hashes = 0;
            while c.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if c.get(j) != Some(&'"') {
                out.push(c[i]);
                i += 1;
                continue;
            }
            j += 1;
            while j < c.len() {
                if c[j] == '"' && (0..hashes).all(|k| c.get(j + 1 + k) == Some(&'#')) {
                    j += 1 + hashes;
                    break;
                }
                j += 1;
            }
            for &ch in &c[i..j.min(c.len())] {
                blank(&mut out, ch);
            }
            i = j;
        } else if c[i] == '"' {
            out.push(' ');
            i += 1;
            while i < c.len() && c[i] != '"' {
                if c[i] == '\\' {
                    out.push(' ');
                    i += 1;
                }
                if i < c.len() {
                    blank(&mut out, c[i]);
                    i += 1;
                }
            }
            out.push(' ');
            i += 1;
        } else if c[i] == '\'' && (next == Some('\\') || c.get(i + 2) == Some(&'\'')) {
            // A char literal, not a lifetime.
            let mut j = i + 1;
            if c.get(j) == Some(&'\\') {
                j += 2;
            } else {
                j += 1;
            }
            while j < c.len() && c[j] != '\'' {
                j += 1;
            }
            for _ in i..=j.min(c.len() - 1) {
                out.push(' ');
            }
            i = j + 1;
        } else {
            out.push(c[i]);
            i += 1;
        }
    }
    out
}

fn is_ident_start(ch: char) -> bool {
    ch.is_alphabetic() || ch == '_'
}

fn is_ident(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// Paths starting at `crate` or `super`, with grouped `use` expanded. Each comes with the
/// byte offset where it starts.
fn paths(text: &str) -> Vec<(usize, Vec<String>)> {
    let c: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        if is_ident_start(c[i]) && (i == 0 || !(is_ident(c[i - 1]) || c[i - 1] == ':')) {
            let start = i;
            let mut j = i;
            while j < c.len() && is_ident(c[j]) {
                j += 1;
            }
            let word: String = c[i..j].iter().collect();
            if word == "crate" || word == "super" {
                let mut k = j;
                skip_ws(&c, &mut k);
                if c.get(k) == Some(&':') && c.get(k + 1) == Some(&':') {
                    let mut found = Vec::new();
                    parse_tail(&c, &mut j, vec![word], &mut found);
                    for p in found {
                        out.push((start, p));
                    }
                    i = j.max(i + 1);
                    continue;
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

fn skip_ws(c: &[char], k: &mut usize) {
    while *k < c.len() && c[*k].is_whitespace() {
        *k += 1;
    }
}

/// After a segment: `::ident`, `::{group}`, or the end of the path.
fn parse_tail(c: &[char], i: &mut usize, prefix: Vec<String>, out: &mut Vec<Vec<String>>) {
    let mut path = prefix;
    loop {
        let mut k = *i;
        skip_ws(c, &mut k);
        if !(c.get(k) == Some(&':') && c.get(k + 1) == Some(&':')) {
            out.push(path);
            return;
        }
        k += 2;
        skip_ws(c, &mut k);
        match c.get(k) {
            Some('{') => {
                k += 1;
                loop {
                    skip_ws(c, &mut k);
                    match c.get(k) {
                        Some('}') => {
                            *i = k + 1;
                            return;
                        }
                        Some(',') => k += 1,
                        Some(ch) if is_ident_start(*ch) || *ch == '*' => {
                            let mut j = k;
                            if *ch == '*' {
                                j += 1;
                            } else {
                                while j < c.len() && is_ident(c[j]) {
                                    j += 1;
                                }
                            }
                            let mut sub = path.clone();
                            sub.push(c[k..j].iter().collect());
                            let mut j2 = j;
                            parse_tail(c, &mut j2, sub, out);
                            // Skip an `as alias`.
                            k = j2;
                            skip_ws(c, &mut k);
                            if c[k..].starts_with(&['a', 's']) && !is_ident(c[k + 2]) {
                                k += 2;
                                skip_ws(c, &mut k);
                                while k < c.len() && is_ident(c[k]) {
                                    k += 1;
                                }
                            }
                        }
                        None => {
                            *i = k;
                            return;
                        }
                        Some(_) => k += 1,
                    }
                }
            }
            Some('*') => {
                path.push("*".into());
                *i = k + 1;
                out.push(path);
                return;
            }
            Some(ch) if is_ident_start(*ch) => {
                let mut j = k;
                while j < c.len() && is_ident(c[j]) {
                    j += 1;
                }
                path.push(c[k..j].iter().collect());
                *i = j;
            }
            _ => {
                out.push(path);
                return;
            }
        }
    }
}

/// `pins/handlers.rs` → `[pins, handlers]`; `pins/mod.rs` → `[pins]`; `lib.rs` → `[]`.
fn module_of(rel: &str) -> Vec<String> {
    let mut parts: Vec<String> = rel
        .trim_end_matches(".rs")
        .split('/')
        .map(str::to_owned)
        .collect();
    if parts
        .last()
        .is_some_and(|p| p == "mod" || p == "lib" || p == "main")
    {
        parts.pop();
    }
    parts
}

/// Absolute path from `crate::…` or `super::…`, cut after the first capitalised segment.
fn resolve(module: &[String], raw: &[String]) -> Option<Vec<String>> {
    let mut abs: Vec<String>;
    let mut rest = raw;
    if raw.first().map(String::as_str) == Some("crate") {
        abs = Vec::new();
        rest = &raw[1..];
    } else {
        abs = module.to_vec();
        while rest.first().map(String::as_str) == Some("super") {
            abs.pop()?;
            rest = &rest[1..];
        }
    }
    for seg in rest {
        if seg == "self" {
            continue;
        }
        abs.push(seg.clone());
        if seg.chars().next().is_some_and(|ch| ch.is_uppercase()) {
            break;
        }
    }
    Some(abs)
}

fn line_of(text: &str, offset: usize) -> usize {
    text.chars().take(offset).filter(|&c| c == '\n').count() + 1
}

/// Check one source tree given as (path relative to `src/`, contents).
fn check(files: &[(String, String)], slices: &[&str], root_files: &[&str]) -> Report {
    let mut report = Report {
        slices: slices.len(),
        ..Report::default()
    };
    let slice_set: BTreeSet<&str> = slices.iter().copied().collect();
    let root_mods: BTreeSet<String> = root_files
        .iter()
        .map(|f| f.trim_end_matches(".rs").to_owned())
        .filter(|m| m != "lib" && m != "main")
        .collect();

    // Layout.
    let mut dirs = BTreeSet::new();
    let mut top = BTreeSet::new();
    for (rel, _) in files {
        match rel.split_once('/') {
            Some((d, _)) => {
                dirs.insert(d.to_owned());
            }
            None => {
                top.insert(rel.clone());
            }
        }
    }
    for d in &dirs {
        if d != "kernel" && !slice_set.contains(d.as_str()) {
            report.problems.push(format!(
                "undeclared directory src/{d}/: not kernel and not a slice"
            ));
        }
    }
    for s in slices {
        if !files.iter().any(|(r, _)| r == &format!("{s}/mod.rs")) {
            report
                .problems
                .push(format!("declared slice {s} has no src/{s}/mod.rs"));
        }
    }
    let declared_top: BTreeSet<String> = root_files.iter().map(|s| (*s).to_owned()).collect();
    for extra in top.difference(&declared_top) {
        report
            .problems
            .push(format!("src/{extra} is not part of the composition root"));
    }
    for missing in declared_top.difference(&top) {
        report
            .problems
            .push(format!("composition-root file src/{missing} is missing"));
    }

    // Imports.
    for (rel, src) in files {
        if rel == "slices.rs" {
            continue;
        }
        let module = module_of(rel);
        let owner = module.first().cloned().unwrap_or_default();
        let is_root = module.is_empty() || root_mods.contains(&owner);
        if is_root {
            continue;
        }
        let text = strip(src);
        for (offset, raw) in paths(&text) {
            let at = format!("src/{rel}:{}", line_of(&text, offset));
            let Some(abs) = resolve(&module, &raw) else {
                report
                    .problems
                    .push(format!("{at}: super:: reaches above the crate root"));
                continue;
            };
            let Some(target) = abs.first() else { continue };
            if target == "*" || target == &owner {
                continue;
            }
            let shown = format!("crate::{}", abs.join("::"));
            if owner == "kernel" {
                if slice_set.contains(target.as_str()) {
                    report
                        .problems
                        .push(format!("{at}: kernel → slice edge {shown}"));
                } else if root_mods.contains(target) {
                    report
                        .problems
                        .push(format!("{at}: kernel → composition root {shown}"));
                }
                continue;
            }
            if target == "kernel" {
                if abs.len() > 3 {
                    report.problems.push(format!("{at}: deep import {shown}; name kernel items as crate::kernel::<module>::<item>"));
                }
            } else if slice_set.contains(target.as_str()) {
                if abs.len() > 2 {
                    report.problems.push(format!(
                        "{at}: deep import {shown}; name another slice only at its root"
                    ));
                }
                report.edges.insert((owner.clone(), target.clone()));
            } else {
                report
                    .problems
                    .push(format!("{at}: slice → composition root {shown}"));
            }
        }
    }

    // Cycles.
    let mut graph: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (a, b) in &report.edges {
        graph.entry(a).or_default().push(b);
    }
    if let Some(cycle) = find_cycle(&graph) {
        report
            .problems
            .push(format!("cycle: {}", cycle.join(" → ")));
    }
    report
}

fn find_cycle<'a>(graph: &BTreeMap<&'a str, Vec<&'a str>>) -> Option<Vec<&'a str>> {
    fn visit<'a>(
        n: &'a str,
        graph: &BTreeMap<&'a str, Vec<&'a str>>,
        stack: &mut Vec<&'a str>,
        done: &mut BTreeSet<&'a str>,
    ) -> Option<Vec<&'a str>> {
        if let Some(pos) = stack.iter().position(|s| *s == n) {
            let mut c = stack[pos..].to_vec();
            c.push(n);
            return Some(c);
        }
        if done.contains(n) {
            return None;
        }
        stack.push(n);
        for m in graph.get(n).into_iter().flatten() {
            if let Some(c) = visit(m, graph, stack, done) {
                return Some(c);
            }
        }
        stack.pop();
        done.insert(n);
        None
    }
    let mut done = BTreeSet::new();
    for n in graph.keys() {
        if let Some(c) = visit(n, graph, &mut Vec::new(), &mut done) {
            return Some(c);
        }
    }
    None
}

fn read_tree(root: &Path) -> Vec<(String, String)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
        entries.sort_by_key(|e| e.path());
        for e in entries {
            let p = e.path();
            if p.is_dir() {
                walk(&p, root, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let rel = p
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, std::fs::read_to_string(&p).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out
}

/// The first word of each line in the fenced block inside `<!-- slices: <name> -->`.
fn agents_block(agents: &str, name: &str) -> Option<Vec<String>> {
    let open = format!("<!-- slices: {name} -->");
    let start = agents.find(&open)? + open.len();
    let end = start + agents[start..].find("<!-- /slices -->")?;
    let block = &agents[start..end];
    let fence = block.find("```")?;
    let body = &block[fence + 3..];
    let body = &body[body.find('\n')? + 1..];
    let body = &body[..body.find("```")?];
    Some(
        body.lines()
            .filter_map(|l| l.split_whitespace().next())
            .map(str::to_owned)
            .collect(),
    )
}

/// A fixture file holds several source files, each after a `// file: <path>` line.
fn fixture(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if let Some(p) = line.strip_prefix("// file: ") {
            out.push((p.trim().to_owned(), String::new()));
        } else if let Some(last) = out.last_mut() {
            last.1.push_str(line);
            last.1.push('\n');
        }
    }
    out
}

#[test]
fn layout_matches_declared_slices() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let files = read_tree(&src);
    let report = check(&files, SLICES, ROOT_FILES);
    let layout: Vec<_> = report
        .problems
        .iter()
        .filter(|p| !p.contains(": deep") && !p.contains(" edge ") && !p.starts_with("cycle"))
        .collect();
    assert!(
        layout.is_empty(),
        "layout problems:\n{}",
        report.problems.join("\n")
    );
    let agents =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../AGENTS.md"))
            .unwrap();
    let block = agents_block(&agents, DEPLOYABLE)
        .unwrap_or_else(|| panic!("AGENTS.md has no fenced list in <!-- slices: {DEPLOYABLE} -->"));
    assert_eq!(
        block, SLICES,
        "AGENTS.md's slices block for {DEPLOYABLE} differs from the declared list"
    );
    println!("checked {} slices", report.slices);
}

#[test]
fn imports_are_acyclic_and_shallow() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let files = read_tree(&src);
    let report = check(&files, SLICES, ROOT_FILES);
    assert!(report.problems.is_empty(), "{}", report.problems.join("\n"));
    println!(
        "checked {} slices, {} edges in {} files: {}",
        report.slices,
        report.edges.len(),
        files.len(),
        report
            .edges
            .iter()
            .map(|(a, b)| format!("{a}→{b}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
}

#[test]
fn must_fail_fixtures_are_rejected() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/slices");
    let cases = [
        ("cycle.rs", "cycle: "),
        ("deep_crate.rs", ": deep import crate::b::store"),
        ("inline_path.rs", ": deep import crate::b::store"),
        ("grouped_use.rs", ": deep import crate::b::store"),
        ("super_reach.rs", ": deep import crate::b::store"),
    ];
    for (file, expect) in cases {
        let text = std::fs::read_to_string(dir.join(file)).unwrap();
        let files = fixture(&text);
        let report = check(&files, &["a", "b"], &["lib.rs"]);
        assert!(
            report.problems.iter().any(|p| p.contains(expect)),
            "{file} must be rejected with {expect:?}; got {:?}",
            report.problems
        );
    }
    // The kernel naming a slice, and a slice naming the composition root.
    let kernel = fixture(
        "// file: lib.rs\n// file: kernel/mod.rs\nuse crate::a::X;\n// file: a/mod.rs\npub struct X;\n// file: b/mod.rs\n",
    );
    let r = check(&kernel, &["a", "b"], &["lib.rs"]);
    assert!(
        r.problems.iter().any(|p| p.contains("kernel → slice edge")),
        "{:?}",
        r.problems
    );
    let root = fixture(
        "// file: lib.rs\n// file: app.rs\npub fn f() {}\n// file: a/mod.rs\nfn g() { crate::app::f() }\n// file: b/mod.rs\n",
    );
    let r = check(&root, &["a", "b"], &["lib.rs", "app.rs"]);
    assert!(
        r.problems
            .iter()
            .any(|p| p.contains("slice → composition root")),
        "{:?}",
        r.problems
    );
    println!("rejected {} must-fail fixtures", cases.len() + 2);
}

#[test]
fn the_lexer_ignores_comments_strings_and_lifetimes() {
    let text = strip(
        "// crate::b::store::x\nlet s = \"crate::b::store\"; fn f<'a>(x: &'a str) -> char { 'c' } /* crate::b::x::y */ crate::b::Item",
    );
    let found: Vec<_> = paths(&text)
        .into_iter()
        .map(|(_, p)| p.join("::"))
        .collect();
    assert_eq!(found, ["crate::b::Item"]);
    let grouped = paths("use crate::{a::X, kernel::{db::ts, state::App as S}, b};");
    let found: Vec<_> = grouped.into_iter().map(|(_, p)| p.join("::")).collect();
    assert_eq!(
        found,
        [
            "crate::a::X",
            "crate::kernel::db::ts",
            "crate::kernel::state::App",
            "crate::b"
        ]
    );
}
