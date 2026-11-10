//! Slice `pins`: pin an issue to a map node or seam (an issue is never attached only to a repo).
//! Owns table `pins`.

mod handlers;
mod store;

pub use handlers::{add_pin, remove_pin};
pub use store::list_pins as list;
