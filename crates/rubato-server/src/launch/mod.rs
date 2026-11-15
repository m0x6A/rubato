//! Slice `launch`: start a session on an issue through one of the caller's own connected
//! companions (ADR 0004), and record what the companion reports back. Owns tables `launches`
//! and `cap_hits`.

mod handler;
mod prompt;
mod store;

pub use handler::launch;
pub use prompt::{PROMPT_TEMPLATE, render_prompt, slugify};
pub use store::{
    issues_with_live_cap_hit, launch_map_versions, launched_issues, launched_issues_in,
    store_cap_hit as record_cap_hit, store_launch_result as record_result,
};
