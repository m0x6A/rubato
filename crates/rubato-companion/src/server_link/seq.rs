//! The companion's envelope sequence, reserved in blocks in `seq.json` (plan 29, S1–S4).
//!
//! The server drops an envelope whose `(companion_id, seq)` it has already recorded, so a
//! companion must never reuse a number, across restarts too. Numbers are reserved before they
//! are used: the file holds `reserved_to`, the highest number this companion may have sent, and
//! a new run starts above it. A crash wastes at most one block; gaps are harmless.
//!
//! `seq.json` is the only file a running companion rewrites. `companion.json` holds the pairing
//! token and is written by `pair` alone.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Sequence numbers reserved per write of `seq.json`.
pub const BLOCK: u64 = 1000;

/// A reservation that could not be read or written. Not a lost connection: retrying the
/// websocket does not help, so the server link ends with it (S4).
#[derive(Debug, thiserror::Error)]
#[error("reserving sequence numbers in {}: {source}", path.display())]
pub struct ReservationError {
    pub path: PathBuf,
    #[source]
    pub source: std::io::Error,
}

#[derive(Serialize, Deserialize)]
struct SeqFile {
    reserved_to: u64,
}

#[derive(Debug)]
pub struct Reservation {
    path: PathBuf,
    start_seq: u64,
    last: u64,
    reserved_to: u64,
}

impl Reservation {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("seq.json")
    }

    /// Reserve the first block of this run. The run starts above the last reservation in
    /// `seq.json`; with no file yet it starts from `now_unix_secs`, which is above anything a
    /// companion that counted from 1 has sent (S2).
    pub fn open(dir: &Path, now_unix_secs: u64) -> Result<Self, ReservationError> {
        let path = Self::path(dir);
        let err = |source| ReservationError {
            path: path.clone(),
            source,
        };
        let start_seq = match std::fs::read(&path) {
            Ok(bytes) => {
                serde_json::from_slice::<SeqFile>(&bytes)
                    .map_err(|e| err(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?
                    .reserved_to
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => now_unix_secs,
            Err(e) => return Err(err(e)),
        };
        let mut r = Self {
            path: path.clone(),
            start_seq,
            last: start_seq,
            reserved_to: start_seq,
        };
        r.reserve()?;
        Ok(r)
    }

    /// The number this run started above: its first envelope is `start_seq + 1`.
    pub fn start_seq(&self) -> u64 {
        self.start_seq
    }

    /// The highest number reserved so far.
    pub fn reserved_to(&self) -> u64 {
        self.reserved_to
    }

    /// The next sequence number. Writes the next block to `seq.json` before handing out a
    /// number above the current reservation.
    pub fn next_seq(&mut self) -> Result<u64, ReservationError> {
        if self.last >= self.reserved_to {
            self.reserve()?;
        }
        self.last += 1;
        Ok(self.last)
    }

    fn reserve(&mut self) -> Result<(), ReservationError> {
        let to = self.reserved_to + BLOCK;
        write_atomic(&self.path, to).map_err(|source| ReservationError {
            path: self.path.clone(),
            source,
        })?;
        self.reserved_to = to;
        Ok(())
    }
}

/// Temp file with mode 0600, synced, then renamed over `seq.json`: a reader sees the old
/// reservation or the new one, never a partial file.
fn write_atomic(path: &Path, reserved_to: u64) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let tmp = path.with_extension("json.tmp");
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&tmp)?;
    // In case a leftover temp file had wider permissions.
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    f.write_all(&serde_json::to_vec(&SeqFile { reserved_to })?)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::config::State;

    fn on_disk(dir: &Path) -> u64 {
        let text = std::fs::read_to_string(Reservation::path(dir)).unwrap();
        serde_json::from_str::<SeqFile>(&text).unwrap().reserved_to
    }

    #[test]
    fn a_new_run_starts_above_the_last_reservation() {
        let dir = tempfile::tempdir().unwrap();
        let mut first = Reservation::open(dir.path(), 100).unwrap();
        let sent: Vec<u64> = (0..3).map(|_| first.next_seq().unwrap()).collect();
        assert_eq!(sent, [101, 102, 103]);
        let reserved = first.reserved_to();
        drop(first);

        // The clock is ignored once there is a file, even a clock that went backwards.
        let mut second = Reservation::open(dir.path(), 5).unwrap();
        assert_eq!(second.start_seq(), reserved);
        let next = second.next_seq().unwrap();
        assert!(next > reserved, "{next} reuses the earlier run's block");
        assert_eq!(on_disk(dir.path()), reserved + BLOCK);
    }

    #[test]
    fn reserves_the_next_block_before_using_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = Reservation::open(dir.path(), 0).unwrap();
        assert_eq!(on_disk(dir.path()), BLOCK, "the first block is on disk");
        for _ in 0..BLOCK {
            let seq = r.next_seq().unwrap();
            assert!(seq <= on_disk(dir.path()));
        }
        assert_eq!(on_disk(dir.path()), BLOCK, "one block, one write");
        let seq = r.next_seq().unwrap();
        assert_eq!(seq, BLOCK + 1);
        assert_eq!(on_disk(dir.path()), 2 * BLOCK);
        assert_eq!(r.reserved_to(), 2 * BLOCK);
    }

    #[test]
    fn an_unwritable_state_dir_fails_the_first_reservation() {
        let dir = tempfile::tempdir().unwrap();
        // A state dir that is a regular file: nothing can be written into it, as root too.
        let not_a_dir = dir.path().join("state");
        std::fs::write(&not_a_dir, "").unwrap();
        let e = Reservation::open(&not_a_dir, 100).unwrap_err();
        assert_eq!(e.path, not_a_dir.join("seq.json"));
        assert!(e.to_string().contains("seq.json"), "{e}");
    }

    #[test]
    fn without_a_file_it_starts_from_the_clock() {
        let dir = tempfile::tempdir().unwrap();
        let now = 1_794_000_000;
        let mut r = Reservation::open(dir.path(), now).unwrap();
        assert_eq!(r.start_seq(), now);
        assert_eq!(r.next_seq().unwrap(), now + 1);
        assert_eq!(on_disk(dir.path()), now + BLOCK);
    }

    #[test]
    fn a_rollover_does_not_touch_companion_json() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        State {
            companion_id: uuid::Uuid::nil(),
            server_url: "http://x".into(),
            user: "m0x6A".into(),
            pairing_token: "t".into(),
            last_seq: 0,
            last_server_seq: 0,
        }
        .save(dir.path())
        .unwrap();
        let before = std::fs::read(State::path(dir.path())).unwrap();
        let mut r = Reservation::open(dir.path(), 100).unwrap();
        for _ in 0..=BLOCK {
            r.next_seq().unwrap();
        }
        assert_eq!(on_disk(dir.path()), 100 + 2 * BLOCK, "a rollover happened");
        assert_eq!(std::fs::read(State::path(dir.path())).unwrap(), before);
        let mode = std::fs::metadata(Reservation::path(dir.path()))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 2, "no temp file left behind: {names:?}");
    }
}
