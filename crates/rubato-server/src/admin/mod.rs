//! Slice `admin`: work on people and tenants that spans slices (ADR 0088). Today the admission
//! adapter that `auth` reaches through `kernel::admission`: the identity guard at login
//! (Decision 15) and a person's tenants for `Me`; an installation's life after it is attached
//! (design §3.3), which reaches `rooms` and `queue`; the members routes of a tenant's admins and
//! the operator's routes (design §4.5, §4.6, §5.4). Owns no table; it writes others only through their slices' roots.

mod admission;
mod lifecycle;
pub mod members;
pub mod operator;
mod people;

pub use admission::{Admission, identity_guard};
pub use lifecycle::{InstallationEvent, Refreshed, installation_changed, refresh_installations};
pub use people::{MEMBERS_CHANGED, remove_person};

#[cfg(test)]
mod tests;
