// Must fail: slice a reaches into b's inner module with a plain use.
// file: lib.rs
// file: a/mod.rs
use crate::b::store::rows;
// file: b/mod.rs
mod store;
// file: b/store.rs
pub fn rows() {}
