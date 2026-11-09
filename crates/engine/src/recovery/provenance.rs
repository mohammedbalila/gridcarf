//! Private payload identity for retained clipboard and format-painter snapshots.
use std::sync::{Arc, Weak};

use gridcraft_model::Workbook;

use super::RecoveryProvenance;

#[derive(Clone, Debug)]
pub(crate) struct OriginBinding {
    payload: Weak<Workbook>,
    pub(crate) source_uid: u64,
    origin: RecoveryProvenance,
}
impl OriginBinding {
    pub(crate) fn new(payload: &Arc<Workbook>, source_uid: u64, origin: RecoveryProvenance) -> Self {
        Self { payload: Arc::downgrade(payload), source_uid, origin }
    }
    pub(crate) fn origin(&self, payload: &Arc<Workbook>, source_uid: u64) -> RecoveryProvenance {
        if source_uid == self.source_uid && self.payload.upgrade().is_some_and(|source| Arc::ptr_eq(&source, payload)) {
            self.origin
        } else {
            RecoveryProvenance::Unknown
        }
    }
}
