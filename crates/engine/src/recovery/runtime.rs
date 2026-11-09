//! Sealed lifetime authority; gate records never own document payloads.
use super::{RecoveryCapture, RecoveryError, RecoveryId, ValidatedRecovery, eligible_origin, manifest::Metadata};
use crate::{DocState, Session};
use std::sync::{Arc, Mutex, Weak};

#[derive(Default)]
pub(crate) struct RecoveryRuntime {
    records: Vec<Record>,
}
struct Record {
    uid: u64,
    gate: Arc<Mutex<GateState>>,
    lineage: Option<RecoveryId>,
}
struct GateState {
    epoch: u64,
    disabled: bool,
}

/// A refused publication or the concrete storage operation's error.
#[derive(Debug, PartialEq, Eq)]
pub enum RecoveryAttemptError<E> {
    Stale,
    Unavailable,
    Operation(E),
}
/// Non-cloneable, weak authority for one live document lifecycle.
pub struct RecoveryTicket {
    gate: Weak<Mutex<GateState>>,
    uid: u64,
    epoch: u64,
}
/// A frozen workbook owner and bounded metadata. Encoding runs outside the gate.
pub struct RecoveryJob {
    workbook: Arc<gridcraft_model::Workbook>,
    metadata: Metadata,
    ticket: RecoveryTicket,
}
/// Inert receipt identifying the actual captured revision, never a publication permit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureGuard {
    uid: u64,
    epoch: u64,
    revision: u64,
}
impl CaptureGuard {
    pub fn doc_uid(&self) -> u64 {
        self.uid
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn captured_revision(&self) -> u64 {
        self.revision
    }
}
impl RecoveryTicket {
    pub fn doc_uid(&self) -> u64 {
        self.uid
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    /// Run only a concrete storage operation while lifecycle invalidation is excluded.
    /// The callback must not reenter Session or recursively call either storage gate.
    pub fn with_storage<T, E>(&self, operation: impl FnOnce() -> Result<T, E>) -> Result<T, RecoveryAttemptError<E>> {
        let gate = self.gate.upgrade().ok_or(RecoveryAttemptError::Stale)?;
        let state = match gate.lock() {
            Ok(state) => state,
            Err(poison) => {
                poison.into_inner().disabled = true;
                return Err(RecoveryAttemptError::Unavailable);
            }
        };
        if state.disabled {
            return Err(RecoveryAttemptError::Unavailable);
        }
        if state.epoch != self.epoch {
            return Err(RecoveryAttemptError::Stale);
        }
        let result = operation().map_err(RecoveryAttemptError::Operation);
        drop(state);
        result
    }
}
impl RecoveryJob {
    pub fn capture(&self) -> RecoveryCapture<'_> {
        RecoveryCapture { workbook: &self.workbook, metadata: &self.metadata }
    }
    pub fn guard(&self) -> CaptureGuard {
        CaptureGuard { uid: self.ticket.uid, epoch: self.ticket.epoch, revision: self.metadata.captured_revision() }
    }
    /// Gate a concrete stage/commit operation. Never reenter Session or either gate here.
    /// Prepare and stream payloads outside this callback, then gate final publication again.
    pub fn with_storage<T, E>(&self, operation: impl FnOnce() -> Result<T, E>) -> Result<T, RecoveryAttemptError<E>> {
        self.ticket.with_storage(operation)
    }
}
impl RecoveryRuntime {
    fn record(&mut self, uid: u64) -> Result<&mut Record, RecoveryError> {
        if let Some(at) = self.records.iter().position(|r| r.uid == uid) {
            return Ok(&mut self.records[at]);
        }
        if self.records.len() >= 64 {
            return Err(RecoveryError::TooLarge("live_lineages"));
        }
        self.records.try_reserve(1).map_err(|_| RecoveryError::TooLarge("live_lineages"))?;
        self.records.push(Record { uid, gate: Arc::new(Mutex::new(GateState { epoch: 0, disabled: false })), lineage: None });
        self.records.last_mut().ok_or(RecoveryError::Unavailable("gate"))
    }
    fn invalidate_gate(gate: &Mutex<GateState>, disable: bool) {
        match gate.lock() {
            Ok(mut state) => {
                if disable {
                    state.disabled = true;
                } else if let Some(epoch) = state.epoch.checked_add(1) {
                    state.epoch = epoch;
                } else {
                    state.disabled = true;
                }
            }
            Err(poison) => poison.into_inner().disabled = true,
        }
    }
    pub(crate) fn invalidate(&mut self, uid: u64) {
        if let Some(r) = self.records.iter().find(|r| r.uid == uid) {
            Self::invalidate_gate(&r.gate, false);
        }
    }
    pub(crate) fn disable(&mut self, uid: u64) {
        if let Some(r) = self.records.iter().find(|r| r.uid == uid) {
            Self::invalidate_gate(&r.gate, true);
        }
    }
    pub(crate) fn retire(&mut self, uid: u64) {
        self.disable(uid);
        self.records.retain(|r| r.uid != uid);
    }
}
impl Drop for RecoveryRuntime {
    fn drop(&mut self) {
        for r in &self.records {
            Self::invalidate_gate(&r.gate, true);
        }
    }
}
impl Session {
    fn recovery_document(&self, uid: u64) -> Result<&DocState, RecoveryError> {
        let mut docs = self.docs.iter().filter(|d| d.uid == uid);
        let doc = docs.next().ok_or(RecoveryError::Unavailable("document"))?;
        if docs.next().is_some() {
            return Err(RecoveryError::Unavailable("duplicate_uid"));
        }
        if self.depth != 0 || self.selection_cleanup_depth != 0 || self.in_group(uid) || self.journal_paused != 0 || self.macro_debug.is_some() {
            return Err(RecoveryError::Unavailable("boundary"));
        }
        eligible_origin(doc)?;
        Ok(doc)
    }
    /// Issue weak storage authority only at a complete eligible document boundary.
    pub fn recovery_ticket(&mut self, uid: u64) -> Result<RecoveryTicket, RecoveryError> {
        self.recovery_document(uid)?;
        let record = self.recovery.record(uid)?;
        let state = match record.gate.lock() {
            Ok(state) => state,
            Err(poison) => {
                poison.into_inner().disabled = true;
                return Err(RecoveryError::Unavailable("gate"));
            }
        };
        if state.disabled {
            return Err(RecoveryError::Unavailable("gate"));
        }
        Ok(RecoveryTicket { gate: Arc::downgrade(&record.gate), uid, epoch: state.epoch })
    }
    /// Freeze the current eligible revision. Callers cannot supply revision or lifecycle epoch.
    pub fn capture_recovery(&mut self, ticket: RecoveryTicket, id: RecoveryId, sequence: u64) -> Result<RecoveryJob, RecoveryError> {
        let doc = self.recovery_document(ticket.uid)?;
        let record = self.recovery.records.iter().find(|r| r.uid == ticket.uid).ok_or(RecoveryError::Unavailable("gate"))?;
        if !Weak::ptr_eq(&ticket.gate, &Arc::downgrade(&record.gate)) {
            return Err(RecoveryError::Unavailable("ticket"));
        }
        ticket.with_storage(|| Ok::<_, ()>(())).map_err(|_| RecoveryError::Unavailable("ticket"))?;
        if record.lineage.is_some_and(|old| old != id)
            || self.recovery.records.iter().any(|r| r.uid != ticket.uid && r.lineage == Some(id))
            || self.docs.iter().any(|d| d.uid != ticket.uid && d.recovery_id() == Some(id))
        {
            return Err(RecoveryError::Unavailable("lineage"));
        }
        let metadata = Metadata::capture(doc, id, sequence)?;
        let workbook = doc.wb.clone();
        self.recovery.record(ticket.uid)?.lineage = Some(id);
        Ok(RecoveryJob { workbook, metadata, ticket })
    }
    /// Adopt validated semantic data as an untrusted, unsaved copy; decoded origin grants no authority.
    pub fn add_recovery_copy(&mut self, validated: ValidatedRecovery) -> Result<usize, RecoveryError> {
        let id = validated.id();
        if self.docs.iter().any(|d| d.recovery_id() == Some(id)) || self.recovery.records.iter().any(|r| r.lineage == Some(id)) {
            return Err(RecoveryError::Unavailable("lineage"));
        }
        let doc = super::restore(validated)?;
        Ok(self.add_document(doc))
    }
    pub(crate) fn invalidate_recovery(&mut self, uid: u64) {
        self.recovery.invalidate(uid);
    }
}
#[cfg(test)]
mod tests;
