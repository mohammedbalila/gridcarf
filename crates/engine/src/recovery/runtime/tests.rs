use super::*;
use crate::{
    Session,
    recovery::{RecoveryAttemptError, RecoveryProvenance},
};
use std::{
    sync::{Arc, mpsc},
    time::Duration,
};
const WAIT: Duration = Duration::from_secs(10);
fn session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s
}
fn make_job(s: &mut Session) -> RecoveryJob {
    let ticket = s.recovery_ticket(s.doc().unwrap().uid).unwrap();
    s.capture_recovery(ticket, "0123456789abcdef0123456789abcdef".parse().unwrap(), 1).unwrap()
}
fn stale(job: &RecoveryJob) {
    let mut called = false;
    assert!(matches!(
        job.with_storage(|| {
            called = true;
            Ok::<_, ()>(())
        }),
        Err(RecoveryAttemptError::Stale | RecoveryAttemptError::Unavailable)
    ));
    assert!(!called);
}
#[test]
fn raw_mutable_access_is_unknown() {
    for doc_mut in [false, true] {
        let mut s = session();
        let job = make_job(&mut s);
        if doc_mut {
            let _ = s.doc_mut().unwrap();
        } else {
            let _ = s.active_mut().unwrap();
        }
        assert_eq!(s.doc().unwrap().recovery_provenance(), RecoveryProvenance::Unknown);
        stale(&job);
    }
}
#[test]
fn lifecycle_invalidation_wins_before_commit() {
    let mut s = session();
    let job = make_job(&mut s);
    let uid = s.doc().unwrap().uid;
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        rx.recv_timeout(WAIT).unwrap();
        stale(&job);
    });
    s.invalidate_recovery(uid);
    tx.send(()).unwrap();
    worker.join().unwrap();
}
#[test]
fn commit_wins_before_lifecycle_mutation() {
    let mut s = session();
    let job = make_job(&mut s);
    let uid = s.doc().unwrap().uid;
    let (entered, entry) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let (done, finished) = mpsc::channel();
    let committed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let committed_writer = committed.clone();
    let worker = std::thread::spawn(move || {
        job.with_storage(|| {
            entered.send(()).unwrap();
            wait.recv_timeout(WAIT).unwrap();
            committed_writer.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok::<_, ()>(())
        })
        .unwrap()
    });
    entry.recv_timeout(WAIT).unwrap();
    let mutation = std::thread::spawn(move || {
        s.invalidate_recovery(uid);
        assert!(committed.load(std::sync::atomic::Ordering::SeqCst));
        done.send(()).unwrap();
        s
    });
    assert!(matches!(finished.recv_timeout(Duration::from_millis(20)), Err(mpsc::RecvTimeoutError::Timeout)));
    release.send(()).unwrap();
    finished.recv_timeout(WAIT).unwrap();
    worker.join().unwrap();
    mutation.join().unwrap();
}
#[test]
fn origin_downgrade_invalidates_before_transfer() {
    let mut s = session();
    let job = make_job(&mut s);
    let uid = s.doc().unwrap().uid;
    s.receive_recovery_origin(uid, RecoveryProvenance::EncryptedOrigin).unwrap();
    stale(&job);
    s.execute("cell.set", serde_json::json!({"cell":"A1","input":"1"})).unwrap();
    assert_eq!(s.doc().unwrap().recovery_provenance(), RecoveryProvenance::EncryptedOrigin);
}
#[test]
fn closed_or_dropped_session_cannot_publish() {
    let mut s = session();
    let job = make_job(&mut s);
    s.close_document(0);
    stale(&job);
    let mut s = session();
    let job = make_job(&mut s);
    drop(s);
    stale(&job);
}
#[test]
fn gate_retains_no_workbook_payload() {
    let mut s = session();
    let weak = Arc::downgrade(&s.doc().unwrap().wb);
    let ticket = s.recovery_ticket(s.doc().unwrap().uid).unwrap();
    s.close_document(0);
    assert!(weak.upgrade().is_none());
    assert!(matches!(ticket.with_storage(|| Ok::<_, ()>(())), Err(RecoveryAttemptError::Stale)));
    let mut s = session();
    let weak = Arc::downgrade(&s.doc().unwrap().wb);
    let job = make_job(&mut s);
    drop(s);
    assert!(weak.upgrade().is_some());
    drop(job);
    assert!(weak.upgrade().is_none());
}

#[test]
fn epoch_overflow_and_poison_disable_only_recovery() {
    for poison in [false, true] {
        let mut s = session();
        let job = make_job(&mut s);
        let uid = s.doc().unwrap().uid;
        let gate = s.recovery.records[0].gate.clone();
        if poison {
            assert!(
                std::thread::spawn(move || {
                    let _held = gate.lock().unwrap();
                    panic!("test poison");
                })
                .join()
                .is_err()
            );
        } else {
            gate.lock().unwrap().epoch = u64::MAX;
            s.invalidate_recovery(uid);
        }
        stale(&job);
        assert!(s.recovery_ticket(uid).is_err());
        s.execute("cell.set", serde_json::json!({"cell":"A1","input":"7"})).unwrap();
        assert_eq!(s.doc().unwrap().wb.active().unwrap().value(gridcraft_core::CellRef::default()).display(), "7");
    }
}
#[test]
fn bounded_slots_reuse_and_duplicate_uid_cannot_revive_tickets() {
    let mut s = Session::new();
    for _ in 0..64 {
        s.new_workbook();
        s.recovery_ticket(s.doc().unwrap().uid).unwrap();
    }
    s.new_workbook();
    let uid = s.doc().unwrap().uid;
    assert!(s.recovery_ticket(uid).is_err());
    s.close_document(0);
    s.recovery_ticket(uid).unwrap();
    assert_eq!(s.recovery.records.len(), 64);
    let mut s = session();
    let job = make_job(&mut s);
    let doc = s.doc().unwrap().clone();
    let uid = doc.uid;
    s.add_document(doc);
    stale(&job);
    assert!(s.recovery_ticket(uid).is_err());
    s.close_document(1);
    assert!(s.recovery_ticket(uid).is_err());
    stale(&job);
}
#[test]
fn boundary_and_foreign_tickets_are_refused_without_binding_lineages() {
    let mut s = session();
    let uid = s.doc().unwrap().uid;
    let ticket = s.recovery_ticket(uid).unwrap();
    let group = s.group_begin("test").unwrap();
    assert!(s.recovery_ticket(uid).is_err());
    assert!(s.capture_recovery(ticket, "0123456789abcdef0123456789abcdef".parse().unwrap(), 1).is_err());
    s.group_rollback(group).unwrap();
    let job = make_job(&mut s);
    let mut other = session();
    assert!(other.capture_recovery(s.recovery_ticket(uid).unwrap(), "0123456789abcdef0123456789abcdef".parse().unwrap(), 1).is_err());
    let group = s.group_begin("post capture").unwrap();
    job.with_storage(|| Ok::<_, ()>(())).unwrap();
    s.group_rollback(group).unwrap();
}
#[test]
fn metadata_limits_and_eligibility_precede_ownership() {
    let mut s = session();
    let uid = s.doc().unwrap().uid;
    s.docs[0].title = "x".repeat(super::super::RecoveryLimits::default().metadata + 1);
    let ticket = s.recovery_ticket(uid).unwrap();
    assert!(matches!(
        s.capture_recovery(ticket, "0123456789abcdef0123456789abcdef".parse().unwrap(), 1),
        Err(super::super::RecoveryError::TooLarge("metadata"))
    ));
    s.receive_recovery_origin(uid, RecoveryProvenance::EncryptedOrigin).unwrap();
    assert!(matches!(s.recovery_ticket(uid), Err(super::super::RecoveryError::Ineligible(RecoveryProvenance::EncryptedOrigin))));
}

#[test]
fn ineligible_public_entries_do_zero_capture_or_serialization_work() {
    let bytes = crate::io::save_bytes(&gridcraft_model::Workbook::new(), "source.xlsx").unwrap();
    let bytes = gridcraft_xlsx::testing::encrypt_workbook(&bytes, "synthetic");
    for encrypted in [false, true] {
        let mut s = Session::new();
        if encrypted {
            s.execute("file.open", serde_json::json!({"name":"source.dat","base64":crate::io::base64_encode(&bytes),"password":"synthetic"}))
                .unwrap();
        } else {
            s.add_document(crate::DocState::new(gridcraft_model::Workbook::new(), None, "unknown".into(), crate::Trust::Local));
        }
        let before = super::super::manifest::CAPTURE_WORK.with(|work| work.get());
        assert!(s.recovery_ticket(s.doc().unwrap().uid).is_err());
        assert!(s.recovery.records.is_empty());
        assert_eq!(super::super::manifest::CAPTURE_WORK.with(|work| work.get()), before);
    }
    let mut s = session();
    let uid = s.doc().unwrap().uid;
    let ticket = s.recovery_ticket(uid).unwrap();
    s.receive_unknown_recovery_origin(uid).unwrap();
    let before = super::super::manifest::CAPTURE_WORK.with(|work| work.get());
    assert!(s.capture_recovery(ticket, "0123456789abcdef0123456789abcdef".parse().unwrap(), 1).is_err());
    assert_eq!(super::super::manifest::CAPTURE_WORK.with(|work| work.get()), before);
    assert!(s.recovery.records[0].lineage.is_none());
}

#[test]
fn only_last_window_close_retires_the_lineage() {
    let mut s = session();
    let first = s.active_window().unwrap();
    let job = make_job(&mut s);
    let second = s.new_window().unwrap();
    s.close_window(second);
    job.with_storage(|| Ok::<_, ()>(())).unwrap();
    s.close_window(first);
    stale(&job);
}
