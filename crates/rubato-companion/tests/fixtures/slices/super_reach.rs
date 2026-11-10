// Must fail: super::super:: from an inner module of a reaches into b's inner module.
// file: lib.rs
// file: a/mod.rs
mod handlers;
// file: a/handlers.rs
use super::super::b::store::rows;
// file: b/mod.rs
mod store;
// file: b/store.rs
pub fn rows() {}
