//! The tenant scoping check (plan 345, design §11.1). Test-only.
//!
//! Reads every `sqlx::query*` call under `src/` outside test code, takes its SQL (a string
//! literal, or a `format!` template whose `{…}` holes become placeholder names), and parses it
//! with `sqlparser`'s SQLite dialect. Every table is in exactly one class (design §3.1). For each
//! reference to a Workspace, Repo-fact or Launch table it requires:
//!
//! - in a `SELECT`, `UPDATE` or `DELETE` scope, a `tenant_id` predicate on that table in `WHERE`
//!   or in a join's `ON` (qualified by its name or alias, or unqualified when it is the scope's
//!   only checked table);
//! - in an `INSERT`, the `tenant_id` column, and, with `ON CONFLICT (…)`, `tenant_id` in the
//!   conflict target.
//!
//! A table still in [`NOT_YET_SCOPED`] is counted, not checked: its `tenant_id` column comes
//! with the pull request that scopes its queries (design §13), and a test fails if a table on
//! that list has the column or one off it lacks it, so the list can only shrink. A query whose
//! SQL is built at run time is allowed only per [`DYNAMIC`]. It prints what it checked.

use sqlparser::ast::{
    ConflictTarget, Expr, FromTable, JoinConstraint, JoinOperator, OnConflict, OnInsert, Select,
    Statement, TableFactor, TableObject, TableWithJoins, Visit, Visitor,
};
use sqlparser::dialect::SQLiteDialect;
use sqlparser::parser::Parser;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::ControlFlow;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Tenancy,
    Person,
    Companion,
    Workspace,
    RepoFact,
    Launch,
    Server,
}

impl Class {
    fn checked(self) -> bool {
        matches!(self, Class::Workspace | Class::RepoFact | Class::Launch)
    }
}

/// Design §3.1, one row per table in the migrated schema.
const CLASSES: &[(&str, Class)] = &[
    ("tenants", Class::Tenancy),
    ("tenant_installations", Class::Tenancy),
    ("tenant_members", Class::Tenancy),
    ("tenant_repos", Class::Tenancy),
    ("users", Class::Person),
    ("web_sessions", Class::Person),
    ("user_tokens", Class::Person),
    ("pairing_codes", Class::Person),
    ("pairings", Class::Person),
    ("session_snapshots", Class::Companion),
    ("companion_events", Class::Companion),
    ("stale_events", Class::Companion),
    ("map_layout", Class::Workspace),
    ("map_layout_grouped", Class::Workspace),
    ("map_cache", Class::Workspace),
    ("map_proposal", Class::Workspace),
    ("pins", Class::Workspace),
    ("map_line_pins", Class::Workspace),
    ("map_draft_ops", Class::Workspace),
    ("issue_creations", Class::Workspace),
    ("issue_suggestions", Class::Workspace),
    ("traffic_dismissals", Class::Workspace),
    ("workspace_settings", Class::Workspace),
    ("open_issues", Class::RepoFact),
    ("backlog_repos", Class::RepoFact),
    ("uncovered_repos", Class::RepoFact),
    ("pull_requests", Class::RepoFact),
    ("decisions", Class::RepoFact),
    ("issue_meta", Class::RepoFact),
    ("plan_signals", Class::RepoFact),
    ("repo_pulls", Class::RepoFact),
    ("repo_pulls_status", Class::RepoFact),
    ("plans", Class::RepoFact),
    ("plan_approvals", Class::RepoFact),
    ("rubato_approvals", Class::RepoFact),
    ("plan_links", Class::RepoFact),
    ("plan_drafts", Class::RepoFact),
    ("plan_draft_revisions", Class::RepoFact),
    ("pr_merges", Class::RepoFact),
    ("room_messages", Class::RepoFact),
    ("room_agents", Class::RepoFact),
    ("room_requests", Class::RepoFact),
    ("spend_sessions", Class::RepoFact),
    ("spend_points", Class::RepoFact),
    ("session_log_entries", Class::RepoFact),
    ("launches", Class::Launch),
    ("companion_queue", Class::Launch),
    ("session_messages", Class::Launch),
    ("agent_reports", Class::Launch),
    ("fleet_requests", Class::Launch),
    ("cap_hits", Class::Launch),
    ("webhook_deliveries", Class::Server),
    ("sqlite_sequence", Class::Server),
    ("_sqlx_migrations", Class::Server),
    ("sqlite_master", Class::Server),
];

/// Checked tables whose `tenant_id` has not landed yet (design §13): each pull request that
/// scopes a table's queries adds its column and takes it off this list.
const NOT_YET_SCOPED: &[&str] = &[];

/// Queries whose SQL is a run-time string, by file: how many, and why that is allowed. A file
/// that gains one fails until it is listed here with a reason, or uses a literal.
const DYNAMIC: &[(&str, usize, &str)] = &[
    (
        "kernel/db.rs",
        2,
        "PRAGMA page counts after the purge migration, and PRAGMA table_info over every table for the progress-is-never-stored test",
    ),
    (
        "kernel/workspaces.rs",
        4,
        "the orphan and id-reuse counts over SCOPED_TABLES and WORKSPACE_ONLY_TABLES, one table name per query; each names tenant_id where its table has it",
    ),
    (
        "launch/store.rs",
        1,
        "a filter list built from fixed fragments; scoped with the table in its PR",
    ),
    (
        "session_log/store.rs",
        1,
        "a filter list built from fixed fragments; scoped with the table in its PR",
    ),
    (
        "session_messages/store.rs",
        1,
        "a filter list built from fixed fragments; scoped with the table in its PR",
    ),
    (
        "tenants/store.rs",
        1,
        "the id-reuse count over the tenancy tables, one table name per query",
    ),
];

/// Queries on a tenant-owned table that act for no tenant, by file and by how their SQL starts:
/// the server's own bookkeeping (the registry, the poll's work list). Each says why. A query
/// that no longer exists must leave the list.
const GLOBAL: &[(&str, &str, &str)] = &[
    (
        "launch/store.rs",
        "SELECT tenant_id, companion_id, user, repo, number, command, workspace_id, role, lead_launch_id FROM launches WHERE id = ?",
        "a launch found by its own id: a companion's envelope and a hook name it by id alone. The row says its tenant, and a request handler compares that with its scope (launch_in)",
    ),
    (
        "session_messages/store.rs",
        "SELECT fmt_COLUMNS FROM session_messages WHERE id = ?",
        "a message found by its own id: the companion's answer names it by id alone. The row says its tenant and its companion, and the caller checks the companion",
    ),
    (
        "fleet/store.rs",
        "DELETE FROM fleet_requests WHERE claimed_at < ?",
        "housekeeping: claims older than the keep time go, whatever their tenant; nothing is read back",
    ),
    (
        "fleet/store.rs",
        "SELECT COUNT(*) FROM fleet_requests",
        "a gauge over the whole table for the server's own dashboard: a count, no row",
    ),
    (
        "workspace_settings/store.rs",
        "SELECT COUNT(*) FROM workspace_settings",
        "the seed asks whether the table is empty at all, before any tenant has a workspace",
    ),
    (
        "workspace_settings/store.rs",
        "SELECT * FROM workspace_settings ORDER BY tenant_id",
        "the registry of every tenant's workspaces, loaded whole at start and after a change; requests read the registry by WsKey",
    ),
    (
        "pins/store.rs",
        "SELECT DISTINCT tenant_id, workspace_id FROM pins",
        "the poll's work list: an issue pinned in any tenant is read, through its own tenant's installation",
    ),
    (
        "new_issue/store.rs",
        "SELECT * FROM issue_creations WHERE state = 'opening'",
        "the poll's work list of creations GitHub did not confirm; each row carries its tenant and is settled inside it",
    ),
];

fn class_of(table: &str) -> Option<Class> {
    CLASSES
        .iter()
        .find(|(t, _)| t.eq_ignore_ascii_case(table))
        .map(|(_, c)| *c)
}

/// Whether a checked table's references must carry `tenant_id` now.
fn enforced(table: &str) -> bool {
    class_of(table).is_some_and(Class::checked) && !NOT_YET_SCOPED.contains(&table)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Sql {
    Literal(String),
    Format(String),
    Dynamic,
}

#[derive(Debug)]
struct Found {
    file: String,
    line: usize,
    sql: Sql,
}

/// Everything before the file's trailing test module: production code only.
fn production(src: &str) -> &str {
    src.find("#[cfg(test)]\nmod tests")
        .map_or(src, |i| &src[..i])
}

/// Parse a Rust string literal at `s[i..]` (normal or raw); its value and the index after it.
fn literal(s: &[char], mut i: usize) -> Option<(String, usize)> {
    if s.get(i) == Some(&'r') {
        i += 1;
        let mut hashes = 0;
        while s.get(i) == Some(&'#') {
            hashes += 1;
            i += 1;
        }
        if s.get(i) != Some(&'"') {
            return None;
        }
        i += 1;
        let start = i;
        loop {
            if i >= s.len() {
                return None;
            }
            if s[i] == '"' && (0..hashes).all(|k| s.get(i + 1 + k) == Some(&'#')) {
                return Some((s[start..i].iter().collect(), i + 1 + hashes));
            }
            i += 1;
        }
    }
    if s.get(i) != Some(&'"') {
        return None;
    }
    i += 1;
    let mut out = String::new();
    while i < s.len() {
        match s[i] {
            '"' => return Some((out, i + 1)),
            '\\' => {
                i += 1;
                match s.get(i) {
                    Some('\n') => {
                        i += 1;
                        while s.get(i).is_some_and(|c| c.is_whitespace()) {
                            i += 1;
                        }
                        continue;
                    }
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(c) => out.push(*c),
                    None => return None,
                }
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    None
}

fn skip_ws(s: &[char], mut i: usize) -> usize {
    while s.get(i).is_some_and(|c| c.is_whitespace()) {
        i += 1;
    }
    i
}

fn starts(s: &[char], i: usize, word: &str) -> bool {
    word.chars()
        .enumerate()
        .all(|(k, c)| s.get(i + k) == Some(&c))
}

/// Every `sqlx::query*` call in `src`, with its SQL.
fn extract(file: &str, src: &str) -> Vec<Found> {
    let s: Vec<char> = production(src).chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1;
    while i < s.len() {
        if s[i] == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        if s[i] == '/' && s.get(i + 1) == Some(&'/') {
            while i < s.len() && s[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if !starts(&s, i, "sqlx::query") {
            i += 1;
            continue;
        }
        let at = line;
        let mut j = i + "sqlx::query".len();
        for suffix in ["_scalar", "_as"] {
            if starts(&s, j, suffix) {
                j += suffix.len();
            }
        }
        if starts(&s, j, "::<") {
            let mut depth = 0;
            while j < s.len() {
                match s[j] {
                    '<' => depth += 1,
                    '>' => {
                        depth -= 1;
                        if depth == 0 {
                            j += 1;
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
        }
        if s.get(j) != Some(&'(') {
            i = j;
            continue;
        }
        j = skip_ws(&s, j + 1);
        let sql = if let Some((text, _)) = literal(&s, j) {
            Sql::Literal(text)
        } else if starts(&s, j, "&format!(") {
            let k = skip_ws(&s, j + "&format!(".len());
            match literal(&s, k) {
                Some((text, _)) => Sql::Format(text),
                None => Sql::Dynamic,
            }
        } else {
            Sql::Dynamic
        };
        out.push(Found {
            file: file.to_owned(),
            line: at,
            sql,
        });
        i = j;
    }
    out
}

/// A `format!` template as SQL: each `{name}` hole becomes the identifier `fmt_name`, `{{` and
/// `}}` their brace. A hole where a table name goes then names no known table.
fn fill_template(t: &str) -> String {
    let mut out = String::new();
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                let mut name = String::new();
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                    name.push(c);
                }
                let name: String = name
                    .chars()
                    .take_while(|c| *c != ':')
                    .map(|c| if c.is_alphanumeric() { c } else { '_' })
                    .collect();
                out.push_str("fmt_");
                out.push_str(&name);
            }
            c => out.push(c),
        }
    }
    out
}

/// Identifiers in `e`: `(qualifier, name)`, lower case.
fn idents(e: &Expr) -> Vec<(Option<String>, String)> {
    let mut out = Vec::new();
    let _ = sqlparser::ast::visit_expressions(e, |x| {
        match x {
            Expr::Identifier(i) => out.push((None, i.value.to_lowercase())),
            Expr::CompoundIdentifier(v) if v.len() >= 2 => out.push((
                Some(v[v.len() - 2].value.to_lowercase()),
                v[v.len() - 1].value.to_lowercase(),
            )),
            _ => {}
        }
        ControlFlow::<()>::Continue(())
    });
    out
}

fn last_part(n: &sqlparser::ast::ObjectName) -> String {
    n.0.last()
        .and_then(|p| p.as_ident())
        .map(|i| i.value.to_lowercase())
        .unwrap_or_default()
}

/// Tables named in a scope's FROM, with the name a predicate qualifies them by.
fn tables(from: &[TableWithJoins]) -> (Vec<(String, String)>, Vec<Expr>) {
    let mut t = Vec::new();
    let mut on = Vec::new();
    let add = |f: &TableFactor, t: &mut Vec<(String, String)>| {
        if let TableFactor::Table { name, alias, .. } = f {
            let table = last_part(name);
            let by = alias
                .as_ref()
                .map(|a| a.name.value.to_lowercase())
                .unwrap_or_else(|| table.clone());
            t.push((table, by));
        }
    };
    for twj in from {
        add(&twj.relation, &mut t);
        for j in &twj.joins {
            add(&j.relation, &mut t);
            let c = match &j.join_operator {
                JoinOperator::Join(c)
                | JoinOperator::Inner(c)
                | JoinOperator::Left(c)
                | JoinOperator::LeftOuter(c)
                | JoinOperator::Right(c)
                | JoinOperator::RightOuter(c)
                | JoinOperator::FullOuter(c) => Some(c),
                _ => None,
            };
            if let Some(JoinConstraint::On(e)) = c {
                on.push(e.clone());
            }
        }
    }
    (t, on)
}

/// Whether a table's rows are also one companion's (design §3.1, the Launch class): a launch, a
/// queued issue, a message to a session, an agent's report and a lead's request each name the
/// companion they belong to. A query that names `companion_id` reads or writes one person's own
/// companion, the pairing's, whichever tenants that person is in: the Companion-class key. It is
/// scoped by that, as a query that names `tenant_id` is by the tenant.
fn companions_own(table: &str) -> bool {
    class_of(table) == Some(Class::Launch) && table != "cap_hits"
}

/// One scope: the checked tables it names must each have a `tenant_id` predicate, or, for a
/// table of a companion's own rows, a `companion_id` one.
fn scope(
    what: &str,
    named: &[(String, String)],
    predicates: &[&Expr],
    enforced: &dyn Fn(&str) -> bool,
    problems: &mut Vec<String>,
) {
    let ids: Vec<(Option<String>, String)> = predicates.iter().flat_map(|e| idents(e)).collect();
    let checked: Vec<&(String, String)> = named.iter().filter(|(t, _)| enforced(t)).collect();
    for (table, by) in &checked {
        let ok = ids.iter().any(|(q, n)| {
            (n == "tenant_id" || (n == "companion_id" && companions_own(table)))
                && (q.as_deref() == Some(by.as_str()) || (q.is_none() && checked.len() == 1))
        });
        if !ok {
            problems.push(format!("{what} on {table} has no tenant_id predicate"));
        }
    }
}

struct Checker<'a> {
    enforced: &'a dyn Fn(&str) -> bool,
    tables: BTreeSet<String>,
    problems: Vec<String>,
}

impl Visitor for Checker<'_> {
    type Break = ();

    fn pre_visit_relation(&mut self, r: &sqlparser::ast::ObjectName) -> ControlFlow<()> {
        self.tables.insert(last_part(r));
        ControlFlow::Continue(())
    }

    fn pre_visit_select(&mut self, s: &Select) -> ControlFlow<()> {
        let (named, on) = tables(&s.from);
        let mut preds: Vec<&Expr> = on.iter().collect();
        preds.extend(s.selection.as_ref());
        scope("SELECT", &named, &preds, self.enforced, &mut self.problems);
        ControlFlow::Continue(())
    }

    fn pre_visit_statement(&mut self, st: &Statement) -> ControlFlow<()> {
        match st {
            Statement::Update(u) => {
                let (named, on) = tables(std::slice::from_ref(&u.table));
                let mut preds: Vec<&Expr> = on.iter().collect();
                preds.extend(u.selection.as_ref());
                scope("UPDATE", &named, &preds, self.enforced, &mut self.problems);
            }
            Statement::Delete(d) => {
                let from = match &d.from {
                    FromTable::WithFromKeyword(f) | FromTable::WithoutKeyword(f) => f,
                };
                let (named, on) = tables(from);
                let mut preds: Vec<&Expr> = on.iter().collect();
                preds.extend(d.selection.as_ref());
                scope("DELETE", &named, &preds, self.enforced, &mut self.problems);
            }
            Statement::Insert(ins) => {
                let TableObject::TableName(name) = &ins.table else {
                    return ControlFlow::Continue(());
                };
                let table = last_part(name);
                if (self.enforced)(&table) {
                    let has = ins.columns.iter().any(|c| last_part(c) == "tenant_id");
                    if !has {
                        self.problems
                            .push(format!("INSERT into {table} has no tenant_id column"));
                    }
                    if let Some(OnInsert::OnConflict(OnConflict {
                        conflict_target: Some(ConflictTarget::Columns { columns, .. }),
                        ..
                    })) = &ins.on
                        && !columns
                            .iter()
                            .any(|c| c.value.eq_ignore_ascii_case("tenant_id"))
                    {
                        self.problems.push(format!(
                            "INSERT into {table} has a conflict target without tenant_id"
                        ));
                    }
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }
}

/// The tables `sql` names and what is wrong with its scoping; `Err` when it does not parse.
fn check_sql(
    sql: &str,
    enforced: &dyn Fn(&str) -> bool,
) -> Result<(BTreeSet<String>, Vec<String>), String> {
    let statements = Parser::parse_sql(&SQLiteDialect {}, sql).map_err(|e| e.to_string())?;
    let mut c = Checker {
        enforced,
        tables: BTreeSet::new(),
        problems: Vec::new(),
    };
    let _ = statements.visit(&mut c);
    Ok((c.tables, c.problems))
}

fn sources() -> Vec<Found> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let rel = p
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if !rel.ends_with(".rs")
                || name == "tests.rs"
                || name.ends_with("_tests.rs")
                || rel.contains("/tests/")
                || rel == "scoping.rs"
            {
                continue;
            }
            out.extend(extract(&rel, &std::fs::read_to_string(&p).unwrap()));
        }
    }
    out.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    out
}

#[test]
fn every_query_on_a_tenant_owned_table_is_scoped() {
    let found = sources();
    let mut problems = Vec::new();
    let mut dynamic: BTreeMap<String, usize> = BTreeMap::new();
    let mut global_used: std::collections::BTreeSet<usize> = Default::default();
    let (mut parsed, mut checked, mut pending) = (0, 0, 0);
    let mut tables_seen = BTreeSet::new();
    for f in &found {
        let sql = match &f.sql {
            Sql::Literal(s) => s.clone(),
            Sql::Format(t) => fill_template(t),
            Sql::Dynamic => {
                *dynamic.entry(f.file.clone()).or_default() += 1;
                continue;
            }
        };
        // A template with a hole where a table name goes, or one that only reads once filled
        // in, has a scope its text cannot show: it counts as run-time SQL.
        if matches!(f.sql, Sql::Format(_))
            && check_sql(&sql, &enforced)
                .map_or(true, |(t, _)| t.iter().any(|t| t.starts_with("fmt_")))
        {
            *dynamic.entry(f.file.clone()).or_default() += 1;
            continue;
        }
        match check_sql(&sql, &enforced) {
            Ok((tables, p)) => {
                parsed += 1;
                if tables.iter().any(|t| enforced(t)) {
                    checked += 1;
                }
                if tables.iter().any(|t| NOT_YET_SCOPED.contains(&t.as_str())) {
                    pending += 1;
                }
                for t in &tables {
                    if class_of(t).is_none() && !t.starts_with("pragma") {
                        problems.push(format!("{}:{}: unknown table {t}", f.file, f.line));
                    }
                }
                tables_seen.extend(tables);
                let flat = sql.split_whitespace().collect::<Vec<_>>().join(" ");
                match GLOBAL
                    .iter()
                    .position(|(file, start, _)| *file == f.file && flat.starts_with(start))
                {
                    Some(i) if !p.is_empty() => {
                        global_used.insert(i);
                    }
                    Some(i) => problems.push(format!(
                        "{}:{}: listed in scoping::GLOBAL but scoped now; take it off: {}",
                        f.file, f.line, GLOBAL[i].1
                    )),
                    None => problems
                        .extend(p.into_iter().map(|p| format!("{}:{}: {p}", f.file, f.line))),
                }
            }
            Err(e) => problems.push(format!(
                "{}:{}: does not parse ({e}): {sql}",
                f.file, f.line
            )),
        }
    }
    for (i, (file, start, _)) in GLOBAL.iter().enumerate() {
        if !global_used.contains(&i) {
            problems.push(format!(
                "scoping::GLOBAL lists a query in {file} that is not there any more: {start}"
            ));
        }
    }
    for (file, n) in &dynamic {
        match DYNAMIC.iter().find(|(f, _, _)| f == file) {
            Some((_, allowed, _)) if n <= allowed => {}
            _ => problems.push(format!(
                "{file}: {n} queries with run-time SQL; list them in scoping::DYNAMIC with a reason, or use a literal"
            )),
        }
    }
    for (file, allowed, _) in DYNAMIC {
        if dynamic.get(*file).copied().unwrap_or(0) < *allowed {
            problems.push(format!(
                "scoping::DYNAMIC allows {allowed} for {file}, which has fewer; lower it"
            ));
        }
    }
    println!(
        "rubato.scoping.check: {} queries, {parsed} parsed, {} run-time, {checked} on scoped tables, {} global by allowance, {pending} on tables not yet scoped ({} tables), {} tables named",
        found.len(),
        dynamic.values().sum::<usize>(),
        global_used.len(),
        NOT_YET_SCOPED.len(),
        tables_seen.len()
    );
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

#[tokio::test]
async fn every_table_has_a_class_and_the_pending_list_matches_the_schema() {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let pool = crate::kernel::db::connect(&url).await.unwrap();
    let names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .fetch_all(&pool)
            .await
            .unwrap();
    let mut problems = Vec::new();
    for n in &names {
        if class_of(n).is_none() {
            problems.push(format!(
                "table {n} has no class in scoping::CLASSES (design §3.1)"
            ));
        }
    }
    for (t, c) in CLASSES {
        if !names.iter().any(|n| n == t) && *t != "sqlite_master" {
            problems.push(format!(
                "scoping::CLASSES names {t}, which the schema does not have"
            ));
            continue;
        }
        if !c.checked() {
            continue;
        }
        let cols: Vec<String> =
            sqlx::query_scalar(&format!("SELECT name FROM pragma_table_info('{t}')"))
                .fetch_all(&pool)
                .await
                .unwrap();
        let has = cols.iter().any(|c| c == "tenant_id");
        match (NOT_YET_SCOPED.contains(t), has) {
            (true, true) => problems.push(format!("{t} has tenant_id: take it off NOT_YET_SCOPED")),
            (false, false) => problems.push(format!("{t} is checked but has no tenant_id column")),
            _ => {}
        }
    }
    for t in NOT_YET_SCOPED {
        if class_of(t).is_none_or(|c| !c.checked()) {
            problems.push(format!(
                "NOT_YET_SCOPED names {t}, which is not a checked table"
            ));
        }
    }
    println!(
        "rubato.scoping.tables: {} tables, {} checked, {} not yet scoped",
        names.len(),
        CLASSES.iter().filter(|(_, c)| c.checked()).count(),
        NOT_YET_SCOPED.len()
    );
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The check run as if every checked table were scoped: what it must catch, and what it must let
/// through.
fn all_enforced(t: &str) -> bool {
    class_of(t).is_some_and(Class::checked)
}

#[test]
fn must_fail_fixtures_are_rejected() {
    let bad = [
        // No predicate at all.
        "SELECT * FROM pins WHERE workspace_id = ?",
        // A join that scopes one of its two tables.
        "SELECT * FROM plans p JOIN pull_requests r ON r.repo = p.repo WHERE p.tenant_id = ?",
        // An upsert whose conflict target leaves the tenant out.
        "INSERT INTO plans (tenant_id, repo, number) VALUES (?, ?, ?) ON CONFLICT(repo, number) DO NOTHING",
        // An insert without the column.
        "INSERT INTO pins (workspace_id, repo) VALUES (?, ?)",
        // A delete in a subquery's table.
        "DELETE FROM launches WHERE id IN (SELECT launch_id FROM session_messages WHERE id = ?) AND tenant_id = ?",
        // `tenant_id` only in the projection is not a predicate.
        "SELECT tenant_id FROM decisions WHERE repo = ?",
    ];
    let mut caught = 0;
    for sql in bad {
        let (_, p) = check_sql(sql, &all_enforced).unwrap();
        assert!(!p.is_empty(), "must fail: {sql}");
        caught += 1;
    }
    let good = [
        "SELECT * FROM pins WHERE tenant_id = ? AND workspace_id = ?",
        "SELECT * FROM plans p JOIN pull_requests r ON r.repo = p.repo AND r.tenant_id = p.tenant_id WHERE p.tenant_id = ?",
        "INSERT INTO plans (tenant_id, repo, number) VALUES (?, ?, ?) ON CONFLICT(tenant_id, repo, number) DO UPDATE SET number = excluded.number",
        "UPDATE launches SET state = ? WHERE tenant_id = ? AND id = ?",
        "SELECT * FROM users WHERE login = ?",
    ];
    for sql in good {
        let (_, p) = check_sql(sql, &all_enforced).unwrap();
        assert!(p.is_empty(), "{sql}: {p:?}");
    }
    println!(
        "rubato.scoping.selftest: {caught} must-fail fixtures caught, {} passed",
        good.len()
    );
}

#[test]
fn the_extractor_reads_literals_templates_and_run_time_sql() {
    let src = r##"
fn a() {
    sqlx::query("SELECT 1 FROM pins \
                 WHERE tenant_id = ?");
    sqlx::query_scalar::<_, i64>(r#"SELECT COUNT(*) FROM "plans""#);
    // sqlx::query("SELECT * FROM commented_out")
    sqlx::query(&format!("SELECT {COLS} FROM launches WHERE id = {{x}}"));
    sqlx::query(&sql);
}
#[cfg(test)]
mod tests {
    fn t() { sqlx::query("SELECT * FROM in_tests"); }
}
"##;
    let f = extract("x.rs", src);
    let sql: Vec<&Sql> = f.iter().map(|f| &f.sql).collect();
    assert_eq!(
        sql,
        [
            &Sql::Literal("SELECT 1 FROM pins WHERE tenant_id = ?".into()),
            &Sql::Literal("SELECT COUNT(*) FROM \"plans\"".into()),
            &Sql::Format("SELECT {COLS} FROM launches WHERE id = {{x}}".into()),
            &Sql::Dynamic,
        ]
    );
    assert_eq!(f[0].line, 3);
    assert_eq!(
        fill_template("SELECT {COLS} FROM launches WHERE id = {{x}}"),
        "SELECT fmt_COLS FROM launches WHERE id = {x}"
    );
}
