//! Slice `layout`: where each part sits on the shared map (ADR 0016). One position per part,
//! last write wins, presentation only. Owns table `map_layout`.

mod handlers;
mod store;

pub use handlers::{clear_layout, put_layout};
pub use store::all;
