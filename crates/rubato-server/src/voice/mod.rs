//! Slice `voice`: a plan room's voice channel (ADR 0073, plan 146), behind `RUBATO_VOICE`, off by
//! default. Rubato mints a LiveKit join token and allows LiveKit's origin in the CSP, but never
//! starts, supervises or connects to LiveKit. A push-to-talk utterance is transcribed through the
//! kernel's `Speech` port and posted in the room as the speaker, marked `spoken`, through
//! `rooms::post_as`. An agent's reply to a spoken turn is spoken through the same port, a short
//! cut of it, from an in-memory cache. No audio is stored.
//!
//! Owns no table. Names `rooms` (the room, its post path, its agents, one message, whether a turn
//! was spoken) and `auth`. Nothing names it but the composition root.

mod cut;
mod gate;
mod handlers;
mod token;

#[cfg(test)]
mod tests;

pub use cut::spoken_cut;
pub use handlers::{join_failed, speech, token as voice_token, utterance};
pub use token::{mint, room_name};
