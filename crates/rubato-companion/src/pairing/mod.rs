//! Slice `pairing`: pair this machine with a GitHub login through the device-code flow
//! (ADR 0011). Outbound only.

mod pair;

pub use pair::{Paired, pair, pair_and_save};

/// Shown once pairing succeeds (ADR 0052): what this companion sends to the server.
pub const WHAT_LEAVES: &str = "What leaves this machine: the sessions this companion sees (repo, branch, issue, status, \
spend), heartbeats, launch results, and each hook's event name and metadata (session, cwd, \
tool name). For sessions started from Rubato, also your prompts and the agent's replies, which \
the whole team reads in the session log, and the plan the session writes (the plan file, its \
design note and the ADRs it proposes) as a draft. Tool input and output stay here.";
