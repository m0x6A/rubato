//! Slice `pairing`: pair this machine with a GitHub login through the device-code flow
//! (ADR 0011). Outbound only.

mod pair;

pub use pair::{Paired, pair};
