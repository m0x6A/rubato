//! Slice `layout`: where each part sits on the shared map (ADR 0016), and the By repo grouping's
//! own arrangement of parts and repo boxes (ADR 0068). One position per part (and per box), last
//! write wins, presentation only. Owns tables `map_layout` and `map_layout_grouped`.

mod handlers;
mod store;

pub use handlers::{clear_grouped_layout, clear_layout, put_layout};
pub use store::all;
