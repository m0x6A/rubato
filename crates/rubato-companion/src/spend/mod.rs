//! Slice `spend`: cost is a signal. Sum a session's transcript against the workspace pricing and
//! enforce the launch's spend cap.

mod apply;
mod arm;
mod sum;
mod task;
mod transcript;

pub use arm::after_hook;
pub use sum::{Spend, Usage, price_for, sum_file, sum_text};
pub use task::{SpendJob, run};
pub use transcript::{MAX_TRANSCRIPT_BYTES, Refusal, claude_projects_dir, read_transcript};
