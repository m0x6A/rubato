//! What a verified delivery names (design 208 D5): a pure function of the event header, the
//! payload's `action` and a few fields, and of which repos Rubato reads. No payload is kept;
//! only the [`Delivery`] it is reduced to.

use rubato_protocol::IssueRef;
use serde_json::Value;

/// The events Rubato subscribes to. Anything else is ignored and counted as `other`.
pub const EVENTS: [&str; 8] = [
    "issues",
    "issue_comment",
    "pull_request",
    "check_suite",
    "check_run",
    "push",
    "installation",
    "installation_repositories",
];

/// `installation` actions that change what Rubato reads (design §3.3, §5.4).
const INSTALLATION_ACTIONS: [&str; 3] = ["suspend", "unsuspend", "deleted"];

/// The events about an installation itself, not one of its repositories.
pub fn is_installation_event(event: &str) -> bool {
    matches!(event, "installation" | "installation_repositories")
}

/// `issues` actions that change what Rubato shows (title, state, the `Map:` line).
const ISSUE_ACTIONS: [&str; 6] = [
    "opened",
    "edited",
    "closed",
    "reopened",
    "deleted",
    "transferred",
];

/// The event as a bounded metric attribute value.
pub fn event_label(event: &str) -> &'static str {
    EVENTS
        .iter()
        .copied()
        .chain(["ping"])
        .find(|e| *e == event)
        .unwrap_or("other")
}

/// A delivery reduced to what is stored: never the payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub event: String,
    pub action: Option<String>,
    pub repo: Option<String>,
    pub issue_number: Option<u64>,
    pub branch: Option<String>,
    /// The App installation the delivery came from (`installation.id`), ADR 0087.
    pub installation: Option<u64>,
    /// `None`: accepted. `Some(reason)`: stored as ignored and answered 200.
    pub ignored: Option<&'static str>,
}

/// What the reader of the delivery knows: the repos Rubato reads and the map branches.
pub struct Context<'a> {
    /// Lower case: the union of workspace repos and the repos of tracked issues.
    pub read_repos: &'a dyn Fn(&str) -> bool,
    /// Whether `(repo, branch)` is a workspace's `map_repo` and `map_ref`.
    pub map_branch: &'a dyn Fn(&str, &str) -> bool,
}

fn cut(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn text(v: &Value, ptr: &str, max: usize) -> Option<String> {
    v.pointer(ptr).and_then(Value::as_str).map(|s| cut(s, max))
}

/// Reduce a verified delivery to its row (D5).
pub fn read(event: &str, body: &Value, ctx: &Context<'_>) -> Delivery {
    let action = text(body, "/action", 64);
    let repo = text(body, "/repository/full_name", 200);
    let mut d = Delivery {
        event: cut(event, 64),
        action: action.clone(),
        repo: repo.clone(),
        issue_number: None,
        branch: None,
        installation: body.pointer("/installation/id").and_then(Value::as_u64),
        ignored: None,
    };
    // An installation's own events name no repository Rubato reads: what they change is which
    // repositories it reads at all.
    match event {
        "installation" => {
            if !action
                .as_deref()
                .is_some_and(|a| INSTALLATION_ACTIONS.contains(&a))
            {
                d.ignored = Some("action");
            }
            return d;
        }
        "installation_repositories" => {
            if !matches!(action.as_deref(), Some("added" | "removed")) {
                d.ignored = Some("action");
            }
            return d;
        }
        _ => {}
    }
    let from_branch = |b: &Option<String>| b.as_deref().and_then(IssueRef::number_from_branch);
    match event {
        "issues" => {
            d.issue_number = body.pointer("/issue/number").and_then(Value::as_u64);
            if !action
                .as_deref()
                .is_some_and(|a| ISSUE_ACTIONS.contains(&a))
            {
                d.ignored = Some("action");
            }
        }
        "issue_comment" => {
            d.issue_number = body.pointer("/issue/number").and_then(Value::as_u64);
            if body.pointer("/issue/pull_request").is_some() {
                d.ignored = Some("pull_request_comment");
            }
        }
        "pull_request" => {
            d.branch = text(body, "/pull_request/head/ref", 255);
            d.issue_number = from_branch(&d.branch);
        }
        "check_suite" => {
            d.branch = text(body, "/check_suite/head_branch", 255);
            d.issue_number = from_branch(&d.branch);
        }
        "check_run" => {
            d.branch = text(body, "/check_run/check_suite/head_branch", 255);
            d.issue_number = from_branch(&d.branch);
            if action.as_deref() != Some("completed") {
                d.ignored = Some("action");
            }
        }
        "push" => {
            let r = body.pointer("/ref").and_then(Value::as_str).unwrap_or("");
            match r.strip_prefix("refs/heads/") {
                Some(b) => {
                    d.branch = Some(cut(b, 255));
                    d.issue_number = IssueRef::number_from_branch(b);
                    // A map branch first: its repo need not be one Rubato reads otherwise.
                    if let Some(repo) = &repo
                        && (ctx.map_branch)(repo, b)
                    {
                        return d;
                    }
                    if d.issue_number.is_none() {
                        d.ignored = Some("branch");
                    }
                }
                None => d.ignored = Some("tag"),
            }
        }
        "ping" => d.ignored = Some("ping"),
        _ => d.ignored = Some("event"),
    }
    if d.ignored.is_none() {
        match &repo {
            Some(r) if (ctx.read_repos)(&r.to_ascii_lowercase()) => {}
            _ => d.ignored = Some("repo"),
        }
    }
    if d.ignored.is_none() && d.issue_number.is_none() && !d.names_repo_pulls() {
        d.ignored = Some("nothing");
    }
    d
}

impl Delivery {
    /// The repo's open pull requests are listed again (ADR 0034), whatever the branch.
    pub fn names_repo_pulls(&self) -> bool {
        matches!(
            self.event.as_str(),
            "pull_request" | "check_suite" | "check_run"
        )
    }
}

/// What one stored, accepted delivery asks the poll loop to read (D5, D7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub repo: String,
    pub issue: Option<IssueRef>,
    /// The issue on the full lane (metadata and comments), not the fast one.
    pub full: bool,
    /// The repo's open issues listing.
    pub list_issues: bool,
    /// The repo's open pull requests, with checks of pull requests no tracked issue owns.
    pub list_pulls: bool,
    /// A push to this branch: the maps reload when it is a map branch.
    pub pushed_branch: Option<String>,
}

/// The target of a stored row; `None` for a row with no repo.
pub fn target(
    event: &str,
    repo: Option<&str>,
    issue: Option<u64>,
    branch: Option<&str>,
) -> Option<Target> {
    let repo = repo?.to_owned();
    Some(Target {
        issue: issue.map(|n| IssueRef::new(repo.clone(), n)),
        full: matches!(event, "issues" | "issue_comment"),
        list_issues: event == "issues",
        list_pulls: matches!(event, "pull_request" | "check_suite" | "check_run"),
        pushed_branch: (event == "push").then(|| branch.unwrap_or("").to_owned()),
        repo,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx_read(repo: &str) -> bool {
        repo == "m0x6a/rubato"
    }
    fn ctx_map(repo: &str, branch: &str) -> bool {
        repo.eq_ignore_ascii_case("m0x6A/maps") && branch == "main"
    }

    /// Design 208 D5, row by row, with each way a delivery is ignored.
    #[test]
    fn what_a_delivery_names() {
        let ctx = Context {
            read_repos: &ctx_read,
            map_branch: &ctx_map,
        };
        let repo = json!({"full_name": "m0x6A/rubato"});
        let other = json!({"full_name": "someone/else"});
        let maps = json!({"full_name": "m0x6A/maps"});
        // (event, body, issue, branch, ignored)
        // (event, body, issue, branch, ignored)
        type Case<'a> = (
            &'a str,
            Value,
            Option<u64>,
            Option<&'a str>,
            Option<&'a str>,
        );
        let cases: Vec<Case> = vec![
            (
                "issues",
                json!({"action":"opened","issue":{"number":5},"repository":repo}),
                Some(5),
                None,
                None,
            ),
            (
                "issues",
                json!({"action":"edited","issue":{"number":5},"repository":repo}),
                Some(5),
                None,
                None,
            ),
            (
                "issues",
                json!({"action":"closed","issue":{"number":5},"repository":repo}),
                Some(5),
                None,
                None,
            ),
            (
                "issues",
                json!({"action":"labeled","issue":{"number":5},"repository":repo}),
                Some(5),
                None,
                Some("action"),
            ),
            (
                "issue_comment",
                json!({"action":"created","issue":{"number":8},"repository":repo}),
                Some(8),
                None,
                None,
            ),
            (
                "issue_comment",
                json!({"action":"created","issue":{"number":9,"pull_request":{}},"repository":repo}),
                Some(9),
                None,
                Some("pull_request_comment"),
            ),
            (
                "pull_request",
                json!({"action":"closed","pull_request":{"head":{"ref":"issue/5-x"}},"repository":repo}),
                Some(5),
                Some("issue/5-x"),
                None,
            ),
            (
                "pull_request",
                json!({"action":"opened","pull_request":{"head":{"ref":"feature/y"}},"repository":repo}),
                None,
                Some("feature/y"),
                None,
            ),
            (
                "check_suite",
                json!({"action":"completed","check_suite":{"head_branch":"issue/5-x"},"repository":repo}),
                Some(5),
                Some("issue/5-x"),
                None,
            ),
            (
                "check_run",
                json!({"action":"completed","check_run":{"check_suite":{"head_branch":"issue/5-x"}},"repository":repo}),
                Some(5),
                Some("issue/5-x"),
                None,
            ),
            (
                "check_run",
                json!({"action":"created","check_run":{"check_suite":{"head_branch":"issue/5-x"}},"repository":repo}),
                Some(5),
                Some("issue/5-x"),
                Some("action"),
            ),
            (
                "push",
                json!({"ref":"refs/heads/issue/7-x","repository":repo}),
                Some(7),
                Some("issue/7-x"),
                None,
            ),
            (
                "push",
                json!({"ref":"refs/heads/main","repository":repo}),
                None,
                Some("main"),
                Some("branch"),
            ),
            (
                "push",
                json!({"ref":"refs/tags/v1","repository":repo}),
                None,
                None,
                Some("tag"),
            ),
            (
                "push",
                json!({"ref":"refs/heads/main","repository":maps}),
                None,
                Some("main"),
                None,
            ),
            (
                "issues",
                json!({"action":"opened","issue":{"number":5},"repository":other}),
                Some(5),
                None,
                Some("repo"),
            ),
            (
                "ping",
                json!({"zen":"hi","repository":repo}),
                None,
                None,
                Some("ping"),
            ),
            (
                "star",
                json!({"action":"created","repository":repo}),
                None,
                None,
                Some("event"),
            ),
            (
                "issues",
                json!({"action":"opened"}),
                None,
                None,
                Some("repo"),
            ),
        ];
        for (event, body, issue, branch, ignored) in &cases {
            let d = read(event, body, &ctx);
            assert_eq!(d.issue_number, *issue, "{event} {body}");
            assert_eq!(d.branch.as_deref(), *branch, "{event} {body}");
            assert_eq!(d.ignored, *ignored, "{event} {body}");
        }
        println!("checked {} deliveries", cases.len());
    }

    /// ADR 0087: an installation's own events name no repository; their installation is read.
    #[test]
    fn installation_events_carry_their_installation() {
        let ctx = Context {
            read_repos: &ctx_read,
            map_branch: &ctx_map,
        };
        let cases = [
            ("installation", "suspend", None),
            ("installation", "unsuspend", None),
            ("installation", "deleted", None),
            ("installation", "created", Some("action")),
            ("installation", "new_permissions_accepted", Some("action")),
            ("installation_repositories", "added", None),
            ("installation_repositories", "removed", None),
        ];
        for (event, action, ignored) in cases {
            let d = read(
                event,
                &json!({"action": action, "installation": {"id": 22}}),
                &ctx,
            );
            assert_eq!(d.installation, Some(22), "{event} {action}");
            assert_eq!(d.ignored, ignored, "{event} {action}");
            assert!(is_installation_event(event));
        }
        let d = read(
            "issues",
            &json!({"action":"opened","issue":{"number":5},"repository":{"full_name":"m0x6A/rubato"},"installation":{"id":11}}),
            &ctx,
        );
        assert_eq!((d.installation, d.ignored), (Some(11), None));
        assert!(!is_installation_event("issues"));
        println!("checked {} installation deliveries", cases.len() + 1);
    }

    #[test]
    fn targets_and_labels() {
        let t = target("issue_comment", Some("m0x6A/rubato"), Some(8), None).unwrap();
        assert!(t.full && !t.list_issues && !t.list_pulls);
        let t = target("issues", Some("m0x6A/rubato"), Some(8), None).unwrap();
        assert!(t.full && t.list_issues);
        let t = target("check_run", Some("m0x6A/rubato"), None, Some("feature/y")).unwrap();
        assert!(!t.full && t.list_pulls && t.issue.is_none());
        let t = target("push", Some("m0x6A/maps"), None, Some("main")).unwrap();
        assert_eq!(t.pushed_branch.as_deref(), Some("main"));
        assert!(target("push", None, None, None).is_none());
        assert_eq!(event_label("check_run"), "check_run");
        assert_eq!(event_label("ping"), "ping");
        assert_eq!(event_label("x".repeat(500).as_str()), "other");
    }
}
