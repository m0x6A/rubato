//! Slice `plan_mirror`: mirror a launched session's plan bundle to the server at its settle
//! points, so the plan lives in Rubato as a draft until it is approved (ADR 0053, plan 122 §D4).
//! Only for a session launched from Rubato with a plan path, only when the bundle changed, and
//! only to a server that announced `drafts`. Nothing is written to the worktree here.

mod arm;
mod bundle;

pub use arm::{PlanMirrorTask, Target, is_settle_point, on_hook, run};
pub use bundle::{Read, read_bundle};
