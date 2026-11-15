//! Slice `pairing`: pair this machine with a GitHub login through the device-code flow
//! (ADR 0011). Outbound only.

mod pair;

pub use pair::{Paired, pair, pair_and_save};

/// Shown once pairing succeeds (ADR 0052): what this companion sends to the server.
pub const WHAT_LEAVES: &str = "What leaves this machine: the sessions this companion sees (repo, branch, issue, status, \
spend), heartbeats, launch results, and each hook's event name and metadata (session, cwd, \
tool name). Tool input and output, prompts and replies stay here.";
