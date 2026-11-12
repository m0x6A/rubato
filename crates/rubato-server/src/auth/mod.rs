//! Slice `auth`: GitHub OAuth login, the session cookie, the `CurrentUser` extractor every
//! handler slice uses, and the person's user-to-server tokens (ADR 0030). Owns tables `users`,
//! `web_sessions` and `user_tokens`.

mod current_user;
mod handlers;
mod store;
mod team;
mod tokens;

pub use current_user::CurrentUser;
pub use handlers::{callback, login, logout, me};
pub use tokens::{
    Granted, TokenError, access_token, can_act, keep as keep_token, reason as token_reason,
    sweep as sweep_tokens,
};
