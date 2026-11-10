//! Slice `pairing`: device-code pairing (ADR 0011), the caller's pairings, revoke; the token and
//! sequence bookkeeping `companion_link` uses. Owns tables `pairing_codes` and `pairings`.

mod codes;
mod handlers;
mod store;

pub use handlers::{approve, list, poll, revoke, start, to_info};
pub use store::{
    PairingRow, list_pairings, next_server_seq, pairing_by_token as authenticate,
    set_last_companion_seq, touch_pairing as touch,
};
