//! A launched session's conversation, as the team sees it (ADR 0052, plan 122 design §D2): the
//! prompt of each turn and the turn's last reply, from the `UserPromptSubmit` and `Stop` hooks,
//! joined by `prompt_id`. Never tool input or output, and never from a session Rubato did not
//! launch.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Who said it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LogRole {
    /// A prompt: typed in the terminal, or a session message from Rubato.
    User,
    /// The last assistant text of the turn.
    Assistant,
}

impl LogRole {
    pub fn name(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

/// What a reply is, by its first line (ADR 0056): the design review, the discussion summary, or
/// any other reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LogKind {
    Prompt,
    Reply,
    /// First line starts `Review:`.
    Review,
    /// First line starts `Summary:`.
    Summary,
}

impl LogKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Prompt => "prompt",
            Self::Reply => "reply",
            Self::Review => "review",
            Self::Summary => "summary",
        }
    }

    /// The kind of an assistant reply, from its first non-empty line.
    pub fn of_reply(text: &str) -> Self {
        let first = text
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim_start();
        if first.starts_with("Review:") {
            Self::Review
        } else if first.starts_with("Summary:") {
            Self::Summary
        } else {
            Self::Reply
        }
    }
}

/// One line of a launched session's log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionLogEntry {
    pub session_id: String,
    /// Claude Code's `prompt_id`: the prompt and the reply of one turn share it.
    pub prompt_id: String,
    pub role: LogRole,
    pub kind: LogKind,
    pub text: String,
    /// The text was cut to `Limits.session_log_max_chars` on the machine.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    /// The session message this prompt carried, when it was one (ADR 0051).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<Uuid>,
    /// The prompt held text before the message: a person was typing in the terminal when it was
    /// pasted.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mixed: bool,
    pub at: DateTime<Utc>,
}

/// A launch's log, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LaunchLog {
    pub launch_id: Uuid,
    pub entries: Vec<SessionLogEntry>,
}

pub mod routes {
    /// `GET` a launch's log. Everyone who may log in reads it.
    pub const LAUNCH_LOG: &str = "/api/launches/{launch_id}/log";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_is_a_review_or_a_summary_by_its_first_line() {
        assert_eq!(
            LogKind::of_reply("Review: full — passed\n\n## Plan"),
            LogKind::Review
        );
        assert_eq!(
            LogKind::of_reply("\n  Summary: we chose b"),
            LogKind::Summary
        );
        assert_eq!(
            LogKind::of_reply("Here is the Review: not first"),
            LogKind::Reply
        );
        assert_eq!(LogKind::of_reply(""), LogKind::Reply);
    }
}
