//! Slice `auth`: GitHub OAuth login, the session cookie, and the `CurrentUser` extractor every
//! handler slice uses. Owns tables `users` and `web_sessions`.

mod current_user;
mod handlers;
mod store;

pub use current_user::CurrentUser;
pub use handlers::{callback, login, logout, me};
