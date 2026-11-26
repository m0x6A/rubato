//! Symbols, imports and references from source files (ADR 0085), with tree-sitter: first Rust,
//! TypeScript, TSX and JavaScript. A file in any other language stays a `file` node.
//!
//! This is a map of where to look, not a call graph a compiler would sign. Definitions come
//! from each grammar's tags query. A reference becomes an edge only when its name is exactly
//! one definition among the files the referencing file imports, or failing that exactly one in
//! the repository; every other reference is counted and not stored.

use super::object;
use rubato_protocol::graph::{GraphEdge, GraphNode, id, kind};
use rubato_protocol::limits;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator};
use tree_sitter_tags::{TagsConfiguration, TagsContext};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) enum Lang {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
}

impl Lang {
    pub(super) const ALL: [Lang; 4] = [Lang::Rust, Lang::TypeScript, Lang::Tsx, Lang::JavaScript];

    pub(super) fn of(path: &str) -> Option<Lang> {
        let name = path.rsplit('/').next().unwrap_or(path);
        Some(match name.rsplit_once('.')?.1 {
            "rs" => Lang::Rust,
            "ts" | "mts" | "cts" => Lang::TypeScript,
            "tsx" => Lang::Tsx,
            "js" | "mjs" | "cjs" | "jsx" => Lang::JavaScript,
            _ => return None,
        })
    }

    /// Rust resolves by module path, the others by file path: two families.
    fn is_rust(self) -> bool {
        self == Lang::Rust
    }
}

/// Paths that are tracked but are not the repository's own code.
pub(super) fn is_vendored(path: &str) -> bool {
    path.split('/')
        .any(|part| matches!(part, "node_modules" | "target" | "dist" | "vendor"))
        || path.ends_with(".min.js")
        || path.ends_with(".d.ts")
}

struct Grammar {
    tags: TagsConfiguration,
    language: Language,
    imports: Query,
}

const RUST_IMPORTS: &str =
    "(use_declaration argument: (_) @import)\n(mod_item name: (identifier) @module !body)";
const ES_IMPORTS: &str =
    "(import_statement source: (string) @import)\n(export_statement source: (string) @import)";

fn grammar(lang: Lang) -> Result<Grammar, String> {
    let js_tags = tree_sitter_javascript::TAGS_QUERY;
    // TypeScript's tags query adds to JavaScript's; its grammar is JavaScript's and more.
    let ts_tags = format!("{js_tags}\n{}", tree_sitter_typescript::TAGS_QUERY);
    let (language, tags, locals, imports): (Language, &str, &str, &str) = match lang {
        Lang::Rust => (
            tree_sitter_rust::LANGUAGE.into(),
            tree_sitter_rust::TAGS_QUERY,
            "",
            RUST_IMPORTS,
        ),
        Lang::TypeScript => (
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            &ts_tags,
            tree_sitter_typescript::LOCALS_QUERY,
            ES_IMPORTS,
        ),
        Lang::Tsx => (
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            &ts_tags,
            tree_sitter_typescript::LOCALS_QUERY,
            ES_IMPORTS,
        ),
        Lang::JavaScript => (
            tree_sitter_javascript::LANGUAGE.into(),
            js_tags,
            tree_sitter_javascript::LOCALS_QUERY,
            ES_IMPORTS,
        ),
    };
    Ok(Grammar {
        tags: TagsConfiguration::new(language.clone(), tags, locals)
            .map_err(|e| format!("{lang:?} tags query: {e}"))?,
        imports: Query::new(&language, imports)
            .map_err(|e| format!("{lang:?} imports query: {e}"))?,
        language,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Def {
    name: String,
    /// What the grammar calls it: `function`, `method`, `class`, `interface`, `module`, …
    symbol: String,
    line: u32,
    end_line: u32,
    signature: String,
    doc: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Import {
    /// `use a::b::C;`, as written up to a `{` or `*`.
    RustUse(Vec<String>),
    /// `mod x;`
    RustMod(String),
    /// `import … from "<specifier>"`.
    Es(String),
}

struct FileCode {
    path: String,
    lang: Lang,
    defs: Vec<Def>,
    imports: Vec<Import>,
    refs: BTreeSet<String>,
}

/// How much code one repository gave.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodeCounts {
    pub symbols: usize,
    pub imports: usize,
    pub references: usize,
    /// References whose name was no definition, or more than one: counted, not stored.
    pub unresolved: usize,
    /// Files tree-sitter found syntax errors in; what parsed around them is kept.
    pub with_errors: usize,
    /// Files that gave nothing at all.
    pub unparsed: usize,
    /// Languages whose queries did not compile against their grammar. A build fault: a test
    /// fails on it, and a build logs it, since those files then give no symbols.
    pub broken_grammars: Vec<String>,
}

pub(super) struct Code {
    grammars: &'static BTreeMap<Lang, Grammar>,
    context: TagsContext,
    parser: Parser,
    files: Vec<FileCode>,
    counts: CodeCounts,
}

fn one_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    match line.char_indices().nth(limits::GRAPH_SIGNATURE_MAX_CHARS) {
        Some((at, _)) => line[..at].to_owned(),
        None => line.to_owned(),
    }
}

impl Code {
    pub(super) fn new() -> Self {
        // Compiled once for the process: a build of a repository does not pay for it again.
        static GRAMMARS: std::sync::OnceLock<(BTreeMap<Lang, Grammar>, Vec<String>)> =
            std::sync::OnceLock::new();
        let (grammars, broken) = GRAMMARS.get_or_init(|| {
            let mut grammars = BTreeMap::new();
            let mut broken = Vec::new();
            for lang in Lang::ALL {
                match grammar(lang) {
                    Ok(g) => {
                        grammars.insert(lang, g);
                    }
                    Err(e) => broken.push(e),
                }
            }
            (grammars, broken)
        });
        let counts = CodeCounts {
            broken_grammars: broken.clone(),
            ..CodeCounts::default()
        };
        Self {
            grammars,
            context: TagsContext::new(),
            parser: Parser::new(),
            files: Vec::new(),
            counts,
        }
    }

    /// Read one source file: its definitions, what it imports, the names it refers to.
    pub(super) fn file(&mut self, path: &str, text: &str) {
        let Some(lang) = Lang::of(path) else { return };
        let Some(grammar) = self.grammars.get(&lang) else {
            return;
        };
        let source = text.as_bytes();
        let mut file = FileCode {
            path: path.to_owned(),
            lang,
            defs: Vec::new(),
            imports: Vec::new(),
            refs: BTreeSet::new(),
        };
        let lines: Vec<&str> = text.lines().collect();
        match self.context.generate_tags(&grammar.tags, source, None) {
            Err(_) => {
                self.counts.unparsed += 1;
                return;
            }
            Ok((tags, had_errors)) => {
                if had_errors {
                    self.counts.with_errors += 1;
                }
                for tag in tags.flatten() {
                    let Some(name) = text.get(tag.name_range.clone()) else {
                        continue;
                    };
                    if !tag.is_definition {
                        file.refs.insert(name.to_owned());
                        continue;
                    }
                    file.defs.push(Def {
                        name: name.to_owned(),
                        symbol: grammar.tags.syntax_type_name(tag.syntax_type_id).to_owned(),
                        line: tag.span.start.row as u32 + 1,
                        end_line: tag.span.end.row as u32 + 1,
                        signature: one_line(text.get(tag.range.clone()).unwrap_or(name)),
                        doc: doc_above(&lines, tag.span.start.row),
                    });
                }
            }
        }
        if self.parser.set_language(&grammar.language).is_ok()
            && let Some(tree) = self.parser.parse(source, None)
        {
            let module = grammar.imports.capture_index_for_name("module");
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&grammar.imports, tree.root_node(), source);
            while let Some(m) = matches.next() {
                for c in m.captures() {
                    let Ok(written) = c.node.utf8_text(source) else {
                        continue;
                    };
                    file.imports.push(if Some(c.index) == module {
                        Import::RustMod(written.to_owned())
                    } else if lang.is_rust() {
                        Import::RustUse(rust_path(written))
                    } else {
                        Import::Es(written.trim_matches(['"', '\'', '`']).to_owned())
                    });
                }
            }
        }
        self.files.push(file);
    }

    /// Resolve what was read against every path of the repository.
    pub(super) fn finish(
        mut self,
        repo: &str,
        paths: &BTreeSet<String>,
    ) -> (Vec<GraphNode>, Vec<GraphEdge>, CodeCounts) {
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let mut packages: BTreeSet<(&'static str, String)> = BTreeSet::new();
        let index: HashMap<&str, usize> = self
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| (f.path.as_str(), i))
            .collect();

        // A name defined twice in one file (two `new`s) is told apart by its line.
        let symbol_id = |f: &FileCode, d: &Def| {
            let twice = f.defs.iter().filter(|o| o.name == d.name).count() > 1;
            let qualified = if twice {
                format!("{}@L{}", d.name, d.line)
            } else {
                d.name.clone()
            };
            id::symbol(repo, &f.path, &qualified)
        };

        let mut imported: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); self.files.len()];
        for (i, f) in self.files.iter().enumerate() {
            let me = id::file(repo, &f.path);
            for d in &f.defs {
                let sym = symbol_id(f, d);
                nodes.push(GraphNode {
                    id: sym.clone(),
                    kind: kind::node::SYMBOL.into(),
                    name: d.name.clone(),
                    repo: Some(repo.to_owned()),
                    path: Some(f.path.clone()),
                    line: Some(d.line),
                    end_line: Some(d.end_line),
                    summary: Some(d.signature.clone()),
                    detail: object([
                        ("symbol", json!(d.symbol)),
                        ("doc", d.doc.as_ref().map_or(Value::Null, |x| json!(x))),
                    ]),
                });
                edges.push(GraphEdge {
                    from: me.clone(),
                    kind: kind::edge::DEFINES.into(),
                    to: sym,
                    detail: Default::default(),
                });
                self.counts.symbols += 1;
            }
            let mut targets: BTreeSet<String> = BTreeSet::new();
            for import in &f.imports {
                match resolve(&f.path, import, paths) {
                    Resolved::File(path) => {
                        if let Some(j) = index.get(path.as_str()) {
                            imported[i].insert(*j);
                        }
                        if path != f.path {
                            targets.insert(id::file(repo, &path));
                        }
                    }
                    Resolved::Package(ecosystem, name) => {
                        targets.insert(id::package(repo, ecosystem, &name));
                        packages.insert((ecosystem, name));
                    }
                    Resolved::Nothing => {}
                }
            }
            self.counts.imports += targets.len();
            edges.extend(targets.into_iter().map(|to| GraphEdge {
                from: me.clone(),
                kind: kind::edge::IMPORTS.into(),
                to,
                detail: Default::default(),
            }));
        }
        for (ecosystem, name) in packages {
            nodes.push(GraphNode {
                id: id::package(repo, ecosystem, &name),
                kind: kind::node::PACKAGE.into(),
                name,
                repo: Some(repo.to_owned()),
                path: None,
                line: None,
                end_line: None,
                summary: None,
                detail: object([("ecosystem", json!(ecosystem))]),
            });
        }

        // References, by name. Definitions of the same family of languages only: a Rust
        // `parse` is never a TypeScript `parse`.
        let mut by_name: HashMap<(bool, &str), Vec<(usize, usize)>> = HashMap::new();
        for (i, f) in self.files.iter().enumerate() {
            for (j, d) in f.defs.iter().enumerate() {
                by_name
                    .entry((f.lang.is_rust(), d.name.as_str()))
                    .or_default()
                    .push((i, j));
            }
        }
        for (i, f) in self.files.iter().enumerate() {
            let me = id::file(repo, &f.path);
            for name in &f.refs {
                // Defined here: a call inside the file, which the file's own symbols show.
                if f.defs.iter().any(|d| &d.name == name) {
                    continue;
                }
                let Some(all) = by_name.get(&(f.lang.is_rust(), name.as_str())) else {
                    self.counts.unresolved += 1;
                    continue;
                };
                let among_imports: Vec<&(usize, usize)> = all
                    .iter()
                    .filter(|(file, _)| imported[i].contains(file))
                    .collect();
                let hit = match (among_imports.as_slice(), all.as_slice()) {
                    ([one], _) => Some(**one),
                    ([], [one]) => Some(*one),
                    _ => None,
                };
                match hit {
                    Some((file, def)) => {
                        let target = &self.files[file];
                        edges.push(GraphEdge {
                            from: me.clone(),
                            kind: kind::edge::REFERENCES.into(),
                            to: symbol_id(target, &target.defs[def]),
                            detail: Default::default(),
                        });
                        self.counts.references += 1;
                    }
                    None => self.counts.unresolved += 1,
                }
            }
        }
        (nodes, edges, self.counts)
    }
}

/// The first line of the doc comment right above the item that starts at `row` (0-based):
/// `///` lines, or a `/** … */` block. Attributes and an `export` in between are stepped over.
fn doc_above(lines: &[&str], row: usize) -> Option<String> {
    let mut at = row;
    let mut first: Option<&str> = None;
    while at > 0 {
        let line = lines.get(at - 1)?.trim();
        if let Some(text) = line.strip_prefix("///") {
            first = Some(text.trim());
            at -= 1;
        } else if first.is_none() && (line.starts_with("#[") || line.starts_with('@')) {
            at -= 1;
        } else if first.is_none() && line.ends_with("*/") {
            // Walk up to the opening of the block; its first line with words is the doc.
            let mut top = at - 1;
            while top > 0 && !lines[top].trim_start().starts_with("/**") {
                top -= 1;
            }
            if !lines[top].trim_start().starts_with("/**") {
                return None;
            }
            return lines[top..at]
                .iter()
                .map(|l| {
                    l.trim()
                        .trim_start_matches("/**")
                        .trim_end_matches("*/")
                        .trim_start_matches('*')
                        .trim()
                })
                .find(|l| !l.is_empty())
                .map(one_line);
        } else {
            break;
        }
    }
    first.filter(|d| !d.is_empty()).map(one_line)
}

/// A `use` path as written, up to a group or a glob: `crate::a::{B, C}` is `[crate, a]`.
fn rust_path(written: &str) -> Vec<String> {
    let head = written.split(['{', '*']).next().unwrap_or("");
    let head = head.split(" as ").next().unwrap_or("");
    head.split("::")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

enum Resolved {
    File(String),
    Package(&'static str, String),
    Nothing,
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

fn join(dir: &str, rest: &str) -> String {
    if dir.is_empty() {
        rest.to_owned()
    } else {
        format!("{dir}/{rest}")
    }
}

/// `dir` + a relative specifier, with `.` and `..` folded. `None` when it leaves the repository.
fn normalize(dir: &str, relative: &str) -> Option<String> {
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// The `src` directory of the crate `path` is in: the nearest ancestor with a `Cargo.toml`.
fn crate_src(path: &str, paths: &BTreeSet<String>) -> Option<String> {
    let mut dir = dir_of(path);
    loop {
        if paths.contains(&join(dir, "Cargo.toml")) {
            return Some(join(dir, "src"));
        }
        if dir.is_empty() {
            return None;
        }
        dir = dir_of(dir);
    }
}

/// The file a module path names under `src`: the longest prefix that is a file.
fn rust_module_file(src: &str, segments: &[String], paths: &BTreeSet<String>) -> Option<String> {
    for len in (1..=segments.len()).rev() {
        let module = segments[..len].join("/");
        for candidate in [
            format!("{src}/{module}.rs"),
            format!("{src}/{module}/mod.rs"),
        ] {
            if paths.contains(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn resolve(from: &str, import: &Import, paths: &BTreeSet<String>) -> Resolved {
    match import {
        Import::RustMod(name) => {
            let file = from.rsplit('/').next().unwrap_or(from);
            let base = if matches!(file, "mod.rs" | "lib.rs" | "main.rs") {
                dir_of(from).to_owned()
            } else {
                join(dir_of(from), file.trim_end_matches(".rs"))
            };
            [
                join(&base, &format!("{name}.rs")),
                join(&base, &format!("{name}/mod.rs")),
            ]
            .into_iter()
            .find(|c| paths.contains(c))
            .map_or(Resolved::Nothing, Resolved::File)
        }
        Import::RustUse(segments) => {
            let Some((first, rest)) = segments.split_first() else {
                return Resolved::Nothing;
            };
            match first.as_str() {
                "crate" => crate_src(from, paths)
                    .and_then(|src| rust_module_file(&src, rest, paths))
                    .map_or(Resolved::Nothing, Resolved::File),
                // Relative paths and the standard library say little about where to look.
                "self" | "super" | "std" | "core" | "alloc" => Resolved::Nothing,
                name => {
                    // Another crate of this repository, by its directory name, or a package.
                    let dashed = name.replace('_', "-");
                    let lib = paths.iter().find(|p| {
                        p.strip_suffix("/src/lib.rs").is_some_and(|dir| {
                            let last = dir.rsplit('/').next().unwrap_or(dir);
                            last == dashed || last == name
                        })
                    });
                    match lib {
                        Some(lib) => {
                            let src = dir_of(lib);
                            Resolved::File(
                                rust_module_file(src, rest, paths).unwrap_or_else(|| lib.clone()),
                            )
                        }
                        None => Resolved::Package("cargo", name.to_owned()),
                    }
                }
            }
        }
        Import::Es(specifier) => {
            if !specifier.starts_with('.') {
                // `@scope/name/deep` is `@scope/name`; `name/deep` is `name`.
                let mut parts = specifier.split('/');
                let first = parts.next().unwrap_or("");
                let name = match (first.starts_with('@'), parts.next()) {
                    (true, Some(second)) => format!("{first}/{second}"),
                    _ => first.to_owned(),
                };
                return if name.is_empty() || name.contains(':') {
                    // `node:fs` and the like: the runtime, not a package.
                    Resolved::Nothing
                } else {
                    Resolved::Package("npm", name)
                };
            }
            let Some(base) = normalize(dir_of(from), specifier) else {
                return Resolved::Nothing;
            };
            // TypeScript sources are imported by the name they compile to.
            let stem = base
                .strip_suffix(".js")
                .or_else(|| base.strip_suffix(".mjs"))
                .unwrap_or(&base);
            let exts = ["ts", "tsx", "js", "jsx", "mjs", "cjs"];
            std::iter::once(base.clone())
                .chain(exts.iter().map(|e| format!("{stem}.{e}")))
                .chain(exts.iter().map(|e| format!("{base}/index.{e}")))
                .find(|c| paths.contains(c))
                .map_or(Resolved::Nothing, Resolved::File)
        }
    }
}
