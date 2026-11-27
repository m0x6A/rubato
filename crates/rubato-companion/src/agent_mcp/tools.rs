//! The three tools (ADR 0071). No tool approves, reviews, decides or pins; `tools::tests` fails
//! when one that is not read-only has a name or description that sounds like it.

use super::{AgentMcp, report};
use crate::sessions::AssignmentView;
use chrono::Utc;
use rubato_protocol::{AgentReport, IssueRef};
use serde_json::{Value, json};

/// One tool as `tools/list` shows it.
pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    /// `annotations.readOnlyHint`.
    pub read_only: bool,
    pub(super) schema: fn() -> Value,
}

fn issue_arg() -> Value {
    json!({
        "type": "string",
        "description": "Optional, as owner/name#n. A mistake catcher: a call naming any issue but this session's own is refused."
    })
}

/// Every tool, in the order `tools/list` gives them.
pub const TOOLS: &[Tool] = &[
    Tool {
        name: "get_assignment",
        description: "What this session was launched for: repository, issue, branch, worktree, the map parts the issue was pinned to, the map version, the plan path, the planning mode, the spend cap and spend so far, the attempt and attempt limit.",
        read_only: true,
        schema: || json!({"type": "object", "properties": {"issue": issue_arg()}, "additionalProperties": false}),
    },
    Tool {
        name: "report_status",
        description: "Say in one line what you are doing and which map parts (node or seam ids) you are on. The team sees the newest line next to this session, with its age. It is a signal, not a progress state.",
        read_only: false,
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "doing": {"type": "string", "description": "One line, at most 200 characters."},
                    "parts": {"type": "array", "items": {"type": "string"}, "maxItems": 8, "description": "Map node or seam ids."},
                    "issue": issue_arg()
                },
                "required": ["doing"],
                "additionalProperties": false
            })
        },
    },
    Tool {
        name: "plan_status",
        description: "Where this session's plan stands as Rubato last sent it: the draft revision and whether it fits the map, the newest review, the counted approval with its version (it covers the plan only at version current), and the plan on the branch, with link_up: whether the companion is connected to Rubato now. Answers unknown until Rubato has sent one; unknown, or link_up false, means not known, not not approved.",
        read_only: true,
        schema: || json!({"type": "object", "properties": {"issue": issue_arg()}, "additionalProperties": false}),
    },
];

/// `tools/list`'s result: the three tools, the graph tools when this machine keeps a copy
/// (ADR 0086), and the fleet tools for a lead (ADR 0072).
pub(super) fn list(lead: bool, graph: bool) -> Value {
    let graph_tools: &[Tool] = if graph {
        super::graph_tools::GRAPH_TOOLS
    } else {
        &[]
    };
    let mut tools: Vec<Value> = TOOLS
        .iter()
        .chain(graph_tools)
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": (t.schema)(),
                "annotations": {"readOnlyHint": t.read_only},
            })
        })
        .collect();
    if lead {
        tools.extend(super::fleet::list());
    }
    json!({"tools": tools})
}

/// What a call came to, for the span and `rubato.mcp.calls`.
pub(crate) struct Called {
    pub result: Value,
    pub outcome: &'static str,
    pub reason: &'static str,
}

pub(super) fn text(v: &Value, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": v.to_string()}], "isError": is_error})
}

pub(super) fn refused(reason: &'static str, message: &str) -> Called {
    Called {
        result: text(&json!({"refused": reason, "message": message}), true),
        outcome: "refused",
        reason,
    }
}

/// Whether `issue`, when given, names this session's own issue.
fn own_issue(args: &Value, view: &AssignmentView) -> bool {
    match args.get("issue").and_then(Value::as_str) {
        None => true,
        Some(s) => s.parse::<IssueRef>().is_ok_and(|i| {
            i.number == view.issue.number && i.repo.eq_ignore_ascii_case(&view.issue.repo)
        }),
    }
}

/// Run tool `name` for the launch `view` describes. `None` for a tool that does not exist.
pub(super) fn call(
    st: &AgentMcp,
    view: &AssignmentView,
    name: &str,
    args: &Value,
) -> Option<Called> {
    let tool = TOOLS.iter().find(|t| t.name == name)?;
    if !own_issue(args, view) {
        return Some(refused(
            "identity",
            "this session reports only for its own issue",
        ));
    }
    let ok = |v: Value| Called {
        result: text(&v, false),
        outcome: "ok",
        reason: "",
    };
    Some(match tool.name {
        "get_assignment" => ok(serde_json::to_value(view).unwrap_or_default()),
        "report_status" => {
            let Some(doing) = args.get("doing").and_then(Value::as_str) else {
                return Some(Called {
                    result: text(&json!({"error": "doing is required"}), true),
                    outcome: "error",
                    reason: "arguments",
                });
            };
            let given: Vec<String> = args
                .get("parts")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let (parts, dropped) = report::parts(&given);
            let r = AgentReport {
                launch_id: view.launch_id,
                doing: report::one_line(doing),
                parts,
                at: Utc::now(),
            };
            let answer = json!({"accepted": true, "doing": r.doing, "parts": r.parts, "dropped_parts": dropped});
            let replaced = st.reports.put(r);
            Called {
                result: text(&answer, false),
                outcome: if replaced { "coalesced" } else { "ok" },
                reason: "",
            }
        }
        "plan_status" => {
            // `link_up: false` means "not known", whatever the cache holds (#197 design §2).
            let link_up = *st.link.borrow();
            match st.cache.get(view.launch_id) {
                Some(s) => ok(json!({"status": "known", "plan_status": s, "link_up": link_up})),
                None => ok(json!({"status": "unknown", "link_up": link_up})),
            }
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR 0071: no tool writes an approval. A tool that is not read-only may not sound like one.
    fn approval_problems(tools: &[(&str, &str, bool)]) -> Vec<String> {
        let re = |s: &str| {
            let s = s.to_ascii_lowercase();
            ["approv", "accept", "merge", "handoff"]
                .iter()
                .any(|w| s.contains(w))
        };
        tools
            .iter()
            .filter(|(_, _, read_only)| !read_only)
            .filter(|(name, description, _)| re(name) || re(description))
            .map(|(name, _, _)| format!("{name}: writes and sounds like an approval"))
            .collect()
    }

    fn as_rows(tools: &[Tool]) -> Vec<(&str, &str, bool)> {
        tools
            .iter()
            .map(|t| (t.name, t.description, t.read_only))
            .collect()
    }

    #[test]
    fn no_tool_that_writes_sounds_like_an_approval() {
        // The three of ADR 0071, the four fleet tools of ADR 0072, the five graph tools of
        // ADR 0086.
        let mut rows = as_rows(TOOLS);
        rows.extend(crate::agent_mcp::FLEET_TOOLS.iter().copied());
        rows.extend(as_rows(crate::agent_mcp::GRAPH_TOOLS));
        let problems = approval_problems(&rows);
        println!("checked {} tools", rows.len());
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(rows.len(), 3 + 4 + 5);
        // Every graph tool is read-only: none of them can sound like an approval, or be one.
        assert!(crate::agent_mcp::GRAPH_TOOLS.iter().all(|t| t.read_only));
    }

    #[test]
    fn the_guard_fails_on_a_write_tool_that_approves() {
        let mut rows = as_rows(TOOLS);
        rows.push(("approve_plan", "approve the plan for build", false));
        assert_eq!(
            approval_problems(&rows),
            vec!["approve_plan: writes and sounds like an approval"]
        );
    }

    #[test]
    fn a_doing_line_is_cut_to_one_line_of_200_characters() {
        assert_eq!(report::one_line("  designing\nthe seam"), "designing");
        let long = "x".repeat(300);
        let cut = report::one_line(&long);
        assert_eq!(cut.chars().count(), 200);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn only_map_ids_are_kept_and_at_most_eight() {
        let given: Vec<String> = [
            "rubato-companion",
            "Bad_Id",
            "a",
            "b",
            "c",
            "d",
            "e",
            "f",
            "g",
            "h",
        ]
        .map(String::from)
        .to_vec();
        let (kept, dropped) = report::parts(&given);
        assert_eq!(kept.len(), 8);
        assert_eq!(kept[0], "rubato-companion");
        assert!(dropped.contains(&"Bad_Id".to_owned()) && dropped.contains(&"h".to_owned()));
    }
}
