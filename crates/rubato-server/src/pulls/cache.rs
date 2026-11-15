//! What GitHub said about a pull request, in memory for [`TTL`] per (workspace, repo, number),
//! one fetch in flight per key (design 82 §2). The person's part is kept per (login, repo).

use crate::kernel::github::RepoAccess;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::OnceCell;

/// How long a view's reads are reused.
pub const TTL: Duration = Duration::from_secs(30);

pub type Key = (String, String, u64);

/// Whether a view was answered from memory or read from GitHub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Cached,
    Fetched,
}

struct Entry<T> {
    at: Instant,
    cell: Arc<OnceCell<Arc<T>>>,
}

pub struct Cache<T> {
    entries: Mutex<HashMap<Key, Entry<T>>>,
    person: Mutex<HashMap<(String, String), (Instant, RepoAccess)>>,
}

impl<T> Default for Cache<T> {
    fn default() -> Self {
        Self {
            entries: Mutex::default(),
            person: Mutex::default(),
        }
    }
}

impl<T: Send + Sync> Cache<T> {
    /// The value for `key`: from memory while younger than [`TTL`], else fetched once however
    /// many callers wait for it. `fresh` reads again and replaces the entry (a merge's own read).
    /// A failed fetch leaves nothing behind.
    pub async fn get<F, Fut>(
        &self,
        key: Key,
        fresh: bool,
        fetch: F,
    ) -> anyhow::Result<(Arc<T>, Outcome)>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = anyhow::Result<T>>,
    {
        let cell = {
            let mut m = self.entries.lock().unwrap();
            let stale = m.get(&key).is_none_or(|e| fresh || e.at.elapsed() >= TTL);
            if stale {
                m.insert(
                    key.clone(),
                    Entry {
                        at: Instant::now(),
                        cell: Arc::new(OnceCell::new()),
                    },
                );
            }
            m[&key].cell.clone()
        };
        if let Some(v) = cell.get() {
            return Ok((v.clone(), Outcome::Cached));
        }
        let mut fetched = false;
        let v = cell
            .get_or_try_init(|| async {
                fetched = true;
                fetch().await.map(Arc::new)
            })
            .await;
        match v {
            Ok(v) => Ok((
                v.clone(),
                if fetched {
                    Outcome::Fetched
                } else {
                    Outcome::Cached
                },
            )),
            Err(e) => {
                let mut m = self.entries.lock().unwrap();
                if m.get(&key).is_some_and(|e| Arc::ptr_eq(&e.cell, &cell)) {
                    m.remove(&key);
                }
                Err(e)
            }
        }
    }

    /// Forget every view of `repo#number`, in every workspace, so the next one reads GitHub
    /// (any merge outcome, design 82 §2).
    pub fn evict(&self, repo: &str, number: u64) {
        self.entries
            .lock()
            .unwrap()
            .retain(|(_, r, n), _| !(r.eq_ignore_ascii_case(repo) && *n == number));
    }

    pub fn person(&self, login: &str, repo: &str) -> Option<RepoAccess> {
        let m = self.person.lock().unwrap();
        m.get(&(login.to_owned(), repo.to_owned()))
            .filter(|(at, _)| at.elapsed() < TTL)
            .map(|(_, a)| a.clone())
    }

    pub fn keep_person(&self, login: &str, repo: &str, access: RepoAccess) {
        self.person.lock().unwrap().insert(
            (login.to_owned(), repo.to_owned()),
            (Instant::now(), access),
        );
    }
}
