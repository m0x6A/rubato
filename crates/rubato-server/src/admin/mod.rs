//! Slice `admin`: work on people and tenants that spans slices (ADR 0088). Today the admission
//! adapter that `auth` reaches through `kernel::admission`: the identity guard at login
//! (Decision 15) and a person's tenants for `Me`. The operator and members routes join it
//! (design §13, PR 6). Owns no table; it writes others only through their slices' roots.

mod admission;

pub use admission::{Admission, identity_guard};

#[cfg(test)]
mod tests;
