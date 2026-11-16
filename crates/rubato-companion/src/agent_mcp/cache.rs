//! The newest `PlanStatus` per launch, in memory (plan 220 §4). Empty after a companion restart:
//! `plan_status` answers `unknown` until the server pushes again.

use rubato_protocol::PlanStatus;
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Default)]
pub(super) struct Cache(Mutex<HashMap<Uuid, PlanStatus>>);

impl Cache {
    /// Keep `s` unless the cache already holds a newer one for its launch: a replayed push never
    /// overwrites a newer status.
    pub fn put(&self, s: PlanStatus) {
        let mut m = self.0.lock().unwrap();
        match m.get(&s.launch_id) {
            Some(old) if old.as_of > s.as_of => {}
            _ => {
                m.insert(s.launch_id, s);
            }
        }
    }

    pub fn get(&self, launch_id: Uuid) -> Option<PlanStatus> {
        self.0.lock().unwrap().get(&launch_id).cloned()
    }
}
