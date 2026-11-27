//! The app.
pub mod store;
mod util;

use crate::store::Store;
use other_crate::helper;
use serde::Serialize;
use std::sync::Arc;

/// Opens the store at `url`.
pub fn open(url: &str) -> Store {
    util::trim(url);
    helper();
    Store::connect(url)
}

#[derive(Serialize)]
pub struct Config {
    pub url: Arc<String>,
}

pub trait Named {
    fn name(&self) -> String;
}

impl Named for Config {
    fn name(&self) -> String {
        self.url.to_string()
    }
}

macro_rules! twice {
    ($e:expr) => {
        ($e, $e)
    };
}
