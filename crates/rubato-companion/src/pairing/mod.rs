//! Slice `pairing`: pair this machine with a GitHub login through the device-code flow
//! (ADR 0011). Outbound only.

mod pair;

pub use pair::{Paired, pair, pair_and_save};

/// Shown once pairing succeeds (ADR 0052): what this companion sends to the server.
pub const WHAT_LEAVES: &str = "What leaves this machine: the sessions this companion sees (repo, branch, issue, status, \
spend), heartbeats, launch results, and each hook's event name and metadata (session, cwd, \
tool name). For sessions started from Rubato, also your prompts and the agent's replies, which \
the whole team reads in the session log, the plan the session writes (the plan file, its design \
note and the ADRs it proposes) as a draft, and the agent's answers to turns in the plan's room \
when you called it in, with what a fork turn cost, and the one line the agent says it is doing \
with the map parts it names (ADR 0071). Tool input and output stay here.";

#[cfg(test)]
mod what_leaves_tests {
    /// ADR 0071 supersedes ADR 0052 in part: the agent's report leaves the machine, and the pair
    /// text says so.
    #[test]
    fn what_leaves_names_the_agents_line_and_parts() {
        assert!(crate::pairing::WHAT_LEAVES.contains("the one line the agent says it is doing"));
        assert!(crate::pairing::WHAT_LEAVES.contains("the map parts it names"));
    }
}
