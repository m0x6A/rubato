//! Slice `web_link`: fan-out to browsers. `WebEvent::Changed` whenever a signal lands; clients
//! refetch the overview. Owns no table.

mod ws;

pub use ws::upgrade;
