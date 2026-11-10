// Must fail: slice a names b and b names a.
// file: lib.rs
// file: a/mod.rs
use crate::b::Thing;
pub struct Other;
// file: b/mod.rs
use crate::a::Other;
pub struct Thing;
