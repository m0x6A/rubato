//! Slice `pins`: pin an issue to a map node or seam (an issue is never attached only to a repo),
//! by hand or by its `Map:` line (ADR 0026). Owns tables `pins` and `map_line_pins`.

mod handlers;
mod map_lines;
mod store;

pub use handlers::{add_pin, remove_pin};
pub use map_lines::{Line, Refusal, apply_map_lines, check_target};
pub use store::list_pins as list;
