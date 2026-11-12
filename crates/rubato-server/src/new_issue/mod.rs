//! Slice `new_issue`: create an issue from Rubato as the GitHub App, under #71's headings,
//! pinned where it was started (ADR 0025), and finish creations GitHub did not confirm from the
//! listing. Owns table `issue_creations`.

mod handlers;
mod reconcile;
mod store;

pub use handlers::create;
pub use reconcile::{Found, OPENING_LIMIT, reconcile};
