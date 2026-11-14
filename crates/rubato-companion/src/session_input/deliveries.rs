//! `deliveries.json`: what the companion did with each recent session message id (plan 122
//! §D1). It records `typing` just before it pastes and `delivered` right after, so a message
//! pushed again (the server pushes every unanswered one at each Hello) is never typed twice:
//!
//! | record      | answer                                   |
//! | ----------- | ---------------------------------------- |
//! | none        | handled as new                           |
//! | `delivered` | `delivered` again, without typing        |
//! | `typing`    | `interrupted`: it stopped while pasting  |
//!
//! The last [`KEPT`] ids are kept. Written atomically with mode 0600 on every change. A file
//! that cannot be read stops the companion at start, with its path in the error, as `seq.json`
//! does: reading it as empty could type a delivered message again.

use crate::kernel::config::write_atomic_private;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Ids kept; older ones fall off the front.
pub const KEPT: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    /// About to paste, or pasting. Still there after a restart: whether it arrived is unknown.
    Typing,
    /// Pasted and Enter sent.
    Delivered,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryRecord {
    pub id: Uuid,
    pub state: Delivery,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DeliveriesFile {
    deliveries: VecDeque<DeliveryRecord>,
}

#[derive(Debug)]
pub struct Deliveries {
    path: PathBuf,
    records: VecDeque<DeliveryRecord>,
}

impl Deliveries {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("deliveries.json")
    }

    /// Read the file the last run left, or start empty when there is none.
    pub fn open(dir: &Path) -> Result<Self> {
        let path = Self::path(dir);
        let records = match std::fs::read(&path) {
            Ok(bytes) => {
                serde_json::from_slice::<DeliveriesFile>(&bytes)
                    .with_context(|| {
                        format!(
                            "{} does not parse; it guards against typing a message twice, so \
                             move it aside only knowing that",
                            path.display()
                        )
                    })?
                    .deliveries
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => VecDeque::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Ok(Self { path, records })
    }

    pub fn get(&self, id: Uuid) -> Option<Delivery> {
        self.records.iter().find(|r| r.id == id).map(|r| r.state)
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Record `state` for `id` and write the file. On a write error nothing changes in memory
    /// either, so the caller can refuse to paste.
    pub fn record(&mut self, id: Uuid, state: Delivery, at: DateTime<Utc>) -> Result<()> {
        let mut next = self.records.clone();
        match next.iter_mut().find(|r| r.id == id) {
            Some(r) => {
                r.state = state;
                r.at = at;
            }
            None => next.push_back(DeliveryRecord { id, state, at }),
        }
        while next.len() > KEPT {
            next.pop_front();
        }
        self.write(next)
    }

    /// Forget `id`: a paste that failed typed nothing, so it is not `typing`.
    pub fn remove(&mut self, id: Uuid) -> Result<()> {
        let mut next = self.records.clone();
        next.retain(|r| r.id != id);
        self.write(next)
    }

    fn write(&mut self, next: VecDeque<DeliveryRecord>) -> Result<()> {
        let file = DeliveriesFile { deliveries: next };
        write_atomic_private(&self.path, &serde_json::to_vec(&file)?)?;
        self.records = file.deliveries;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> DateTime<Utc> {
        DateTime::from_timestamp(1_794_000_000, 0).unwrap()
    }

    #[test]
    fn records_survive_a_restart_with_private_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let mut d = Deliveries::open(dir.path()).unwrap();
        assert!(d.is_empty());
        d.record(a, Delivery::Typing, at()).unwrap();
        d.record(a, Delivery::Delivered, at()).unwrap();
        d.record(b, Delivery::Typing, at()).unwrap();
        let mode = std::fs::metadata(Deliveries::path(dir.path()))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        let again = Deliveries::open(dir.path()).unwrap();
        assert_eq!(again.get(a), Some(Delivery::Delivered));
        assert_eq!(again.get(b), Some(Delivery::Typing));
        assert_eq!(again.len(), 2, "one record per id");
        d.remove(b).unwrap();
        assert_eq!(Deliveries::open(dir.path()).unwrap().get(b), None);
    }

    #[test]
    fn only_the_last_thousand_ids_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = Deliveries::open(dir.path()).unwrap();
        for n in 0..(KEPT as u128 + 5) {
            d.record(Uuid::from_u128(n), Delivery::Delivered, at())
                .unwrap();
        }
        assert_eq!(d.len(), KEPT);
        assert_eq!(d.get(Uuid::from_u128(4)), None, "the oldest fell off");
        assert_eq!(d.get(Uuid::from_u128(5)), Some(Delivery::Delivered));
        assert_eq!(Deliveries::open(dir.path()).unwrap().len(), KEPT);
    }

    #[test]
    fn a_file_that_does_not_parse_stops_the_start_by_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(Deliveries::path(dir.path()), "{\"deliveries\": 7").unwrap();
        let e = Deliveries::open(dir.path()).unwrap_err();
        assert!(format!("{e:#}").contains("deliveries.json"), "{e:#}");
    }

    #[test]
    fn a_failed_write_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = Deliveries::open(dir.path()).unwrap();
        // The state dir is gone: nothing can be written into it.
        drop(dir);
        assert!(
            d.record(Uuid::from_u128(1), Delivery::Typing, at())
                .is_err()
        );
        assert_eq!(d.get(Uuid::from_u128(1)), None);
    }
}
