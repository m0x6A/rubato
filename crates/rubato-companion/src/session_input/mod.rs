//! Slice `session_input`: type a `SessionMessage` into a session this companion launched for
//! its paired user, once, and only when a hook says the session waits for input (ADR 0051,
//! plan 122 §D1). Answers each message with one `SessionMessageResult`. A room turn in fork mode
//! is never typed: it runs in a read-only fork of the session and answers with a `RoomReply`
//! (ADR 0057). Nothing here reads a pane: replies come back as hooks or from the fork's output.

mod arm;
mod decide;
mod deliveries;
mod fork;

pub use arm::{SessionInputTask, run};
pub use decide::{Decision, decide};
pub use deliveries::{Deliveries, Delivery, DeliveryRecord, KEPT};
pub use fork::{ForkCtx, ForkTurn, Forks};
