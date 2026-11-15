//! Slice `import`: a plan written outside Rubato becomes a draft the importer owns (ADR 0058,
//! plan 122 §D9). No table. It names `plan_drafts`, `launch`, `plans` and `approve` (whether an
//! approval counts), so none of those names another back for it.

mod handler;

pub use handler::import;
