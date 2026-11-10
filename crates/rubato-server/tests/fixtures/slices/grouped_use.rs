// Must fail: a grouped use hides the deep import.
// file: lib.rs
// file: a/mod.rs
use crate::{
    b::store::rows,
    kernel::db::ts,
};
// file: b/mod.rs
mod store;
// file: b/store.rs
pub fn rows() {}
