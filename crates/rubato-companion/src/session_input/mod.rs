//! Slice `session_input`: type a `SessionMessage` into a session this companion launched for
//! its paired user, once, and only when a hook says the session waits for input (ADR 0051,
//! plan 122 §D1). Answers each message with one `SessionMessageResult`. Nothing here reads a
//! pane: replies come back as hooks.

mod arm;
mod decide;
mod deliveries;

pub use arm::{SessionInputTask, run};
pub use decide::{Decision, decide};
pub use deliveries::{Deliveries, Delivery, DeliveryRecord, KEPT};
