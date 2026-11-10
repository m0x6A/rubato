// Must fail: an inline path, no use line, into b's inner module.
// file: lib.rs
// file: a/mod.rs
pub fn f() {
    let _ = crate::b::store::rows();
}
// file: b/mod.rs
mod store;
// file: b/store.rs
pub fn rows() {}
