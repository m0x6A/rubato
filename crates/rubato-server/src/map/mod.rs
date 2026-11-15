//! Slice `map`: load, validate and cache the system map; serve it. The current map lives in
//! kernel state, which `pins`, `launch` and `overview` read. Owns table `map_cache`.

mod handlers;
mod load;
mod store;

pub use handlers::{list as list_workspaces, map, map_reload, workspaces};
pub use load::{reload, reload_ws, warm};
