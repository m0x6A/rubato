//! Slice `map`: load, validate and cache the system map; serve it. The current map lives in
//! kernel state, which `pins`, `launch` and `overview` read. Owns table `map_cache`.

mod handlers;
mod load;
mod store;

pub use handlers::{map, map_reload};
pub use load::{reload, warm};
