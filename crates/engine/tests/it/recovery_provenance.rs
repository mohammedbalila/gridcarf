//! Runtime origin is independent of execution Trust and untrusted snapshot labels.
use gridcraft_engine::{DocState, Session, Trust, io, model::Workbook, recovery::RecoveryProvenance as Origin};
use serde_json::json;
use std::sync::Arc;

fn origin(s: &Session) -> Origin {
    s.doc().unwrap().recovery_provenance()
}
fn opened(encrypted: bool) -> Session {
    let bytes = io::save_bytes(&Workbook::new(), "source.xlsx").unwrap();
    let bytes = if encrypted { gridcraft_xlsx::testing::encrypt_workbook(&bytes, "synthetic") } else { bytes };
    let mut s = Session::new();
    s.execute_headless("file.open", json!({"name":"source.dat", "base64":io::base64_encode(&bytes), "password":"synthetic"})).unwrap();
    s
}
#[test]
fn audited_entries_establish_origin_independently_of_trust() {
    let mut s = Session::new();
    s.new_workbook();
    assert_eq!(origin(&s), Origin::Plain);
    s.execute("cell.set", json!({"cell":"A1","input":"42"})).unwrap();
    assert_eq!(origin(&s), Origin::Plain);
    s.execute("file.new", json!({"sample":"sales"})).unwrap();
    assert_eq!(origin(&s), Origin::Plain);
    s.execute("file.new", json!({"sample":"budget"})).unwrap();
    assert_eq!(origin(&s), Origin::Plain);
    let s = opened(false);
    assert_eq!(s.doc().unwrap().trust, Trust::FromFile);
    assert_eq!(origin(&s), Origin::Plain);
    assert_eq!(origin(&opened(true)), Origin::EncryptedOrigin);
}
#[test]
fn generic_and_cloned_reassigned_adoption_is_unknown_or_worse() {
    for doc in [
        DocState::new(Workbook::new(), None, "local".into(), Trust::Local),
        DocState::from_saved(Workbook::new(), None, "saved".into(), Trust::FromFile),
    ] {
        assert_eq!(doc.recovery_provenance(), Origin::Unknown);
        let mut s = Session::new();
        s.add_document(doc);
        assert_eq!(origin(&s), Origin::Unknown);
    }
    for encrypted in [false, true] {
        let mut doc = opened(encrypted).doc().unwrap().clone();
        doc.wb = Arc::new(Workbook::new());
        let mut s = Session::new();
        s.add_document(doc);
        assert_eq!(origin(&s), if encrypted { Origin::EncryptedOrigin } else { Origin::Unknown });
    }
}

fn source(encrypted: bool) -> Session {
    let mut s = opened(encrypted);
    s.execute("range.setValues", json!({"range":"A1", "values":[[42,7],[3,9]]})).unwrap();
    s.execute("home.bold", json!({"range":"A1:B2"})).unwrap();
    s
}
#[test]
fn all_transfer_modes_taint_receiver() {
    // These are every actual What mode (including the notes alias), plus each orthogonal operation.
    let modes = [
        json!({"what":"all"}),
        json!({"what":"formulas"}),
        json!({"what":"values"}),
        json!({"what":"formats"}),
        json!({"what":"comments"}),
        json!({"what":"notes"}),
        json!({"what":"validation"}),
        json!({"what":"allExceptBorders"}),
        json!({"what":"columnWidths"}),
        json!({"what":"formulasAndNumberFormats"}),
        json!({"what":"valuesAndNumberFormats"}),
        json!({"link":true}),
        json!({"transpose":true}),
        json!({"skipBlanks":true}),
        json!({"operation":"add"}),
        json!({"operation":"subtract"}),
        json!({"operation":"multiply"}),
        json!({"operation":"divide"}),
    ];
    for encrypted in [false, true] {
        for mut mode in modes.clone() {
            let mut s = source(encrypted);
            if !encrypted {
                let doc = s.doc().unwrap().clone();
                s.close_document(0);
                s.add_document(doc);
            }
            let want = if encrypted { Origin::EncryptedOrigin } else { Origin::Unknown };
            s.execute("edit.copy", json!({"range":"A1:B2"})).unwrap();
            s.new_workbook();
            assert_eq!(origin(&s), Origin::Plain);
            let ticket = s.recovery_ticket(s.doc().unwrap().uid).unwrap();
            let job = s.capture_recovery(ticket, "0123456789abcdef0123456789abcdef".parse().unwrap(), 1).unwrap();
            mode["at"] = json!("C3");
            s.execute("edit.pasteSpecial", mode.clone()).unwrap();
            assert_eq!(origin(&s), want, "{mode}");
            let mut published = false;
            assert!(
                job.with_storage(|| {
                    published = true;
                    Ok::<_, ()>(())
                })
                .is_err()
            );
            assert!(!published);
            if matches!(mode["what"].as_str(), Some("values" | "formats" | "comments" | "notes" | "columnWidths" | "valuesAndNumberFormats")) {
                assert_eq!(s.doc().unwrap().trust, Trust::Local, "passive mode {mode} retains execution policy");
            }
        }
    }
}
#[test]
fn copy_cut_multiarea_and_insert_copied_cells_carry_origin() {
    for command in ["edit.copy", "edit.cut"] {
        for multiarea in [false, true] {
            if multiarea && command == "edit.cut" {
                continue;
            }
            let mut s = source(true);
            if multiarea {
                s.execute("selection.set", json!({"range":"A1:A2,C1:C2"})).unwrap();
            }
            let params = if multiarea { json!({}) } else { json!({"range":"A1:B2"}) };
            s.execute(command, params).unwrap();
            if multiarea {
                assert_eq!(s.clipboard.as_ref().unwrap().areas.len(), 2);
            }
            s.new_workbook();
            s.execute("edit.paste", json!({"at":"C3"})).unwrap();
            assert_eq!(origin(&s), Origin::EncryptedOrigin, "{command} multi={multiarea}");
            assert_eq!(s.doc().unwrap().wb.active().unwrap().value(gridcraft_engine::core::CellRef::parse("C3").unwrap()).display(), "42");
        }
    }
    for encrypted in [false, true] {
        let mut s = source(encrypted);
        s.execute("edit.copy", json!({"range":"A1:B2"})).unwrap();
        s.new_workbook();
        s.execute("home.insertCopiedCells", json!({"at":"C3","shift":"down"})).unwrap();
        assert_eq!(origin(&s), if encrypted { Origin::EncryptedOrigin } else { Origin::Plain });
    }
}
#[test]
fn clipboard_payload_replacement_cannot_retain_plain_authority() {
    for encrypted in [false, true] {
        let mut s = source(encrypted);
        s.execute("edit.copy", json!({"range":"A1"})).unwrap();
        let mut clip = s.clipboard.clone().unwrap();
        clip.wb = Arc::new(Workbook::new());
        s.clipboard = Some(clip);
        s.new_workbook();
        s.execute("edit.pasteSpecial", json!({"what":"values","at":"C3"})).unwrap();
        assert_eq!(origin(&s), Origin::Unknown);
    }
}
#[test]
fn painter_sticky_source_close_and_replaced_tuple_keep_origin() {
    for encrypted in [false, true] {
        let mut s = source(encrypted);
        s.execute("selection.set", json!({"range":"A1"})).unwrap();
        s.execute("edit.formatPainter", json!({"sticky":true})).unwrap();
        s.new_workbook();
        s.close_document(0);
        for at in ["C3", "D4"] {
            s.execute("edit.formatPainter", json!({"apply":at})).unwrap();
            assert_eq!(origin(&s), if encrypted { Origin::EncryptedOrigin } else { Origin::Plain });
            let d = s.doc().unwrap();
            let sh = d.wb.active().unwrap();
            assert!(d.wb.styles.get(sh.style_id(gridcraft_engine::core::CellRef::parse(at).unwrap())).font.bold);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(origin(&s), if encrypted { Origin::EncryptedOrigin } else { Origin::Plain });
        }
    }
    let mut s = source(false);
    s.execute("edit.formatPainter", json!({"sticky":true})).unwrap();
    let (_, sheet, range, sticky) = s.format_painter.take().unwrap();
    s.format_painter = Some((Arc::new(Workbook::new()), sheet, range, sticky));
    s.new_workbook();
    s.execute("edit.formatPainter", json!({"apply":"C3"})).unwrap();
    assert_eq!(origin(&s), Origin::Unknown, "equal style tables confer no origin authority");
}

#[test]
fn save_as_undo_and_rollback_do_not_downgrade_origin() {
    let mut s = source(true);
    s.execute("edit.copy", json!({"range":"A1"})).unwrap();
    s.new_workbook();
    s.execute("edit.pasteSpecial", json!({"what":"values","at":"B2"})).unwrap();
    for command in ["edit.undo", "edit.redo"] {
        s.execute(command, json!({})).unwrap();
        assert_eq!(origin(&s), Origin::EncryptedOrigin);
    }
    // A failure rolls back content, while the received source fact stays with the live document.
    s.new_workbook();
    let before = s.doc().unwrap().wb.clone();
    let result = s.grouped::<()>("failed transfer", |s| {
        s.execute("edit.pasteSpecial", json!({"what":"values","at":"B2"}))?;
        Err(gridcraft_engine::EngineError::Other("synthetic failure".into()))
    });
    assert!(result.is_err());
    assert!(Arc::ptr_eq(&s.doc().unwrap().wb, &before));
    assert_eq!(origin(&s), Origin::EncryptedOrigin);
    s.new_workbook();
    let before = s.doc().unwrap().wb.clone();
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        s.grouped::<()>("panic transfer", |s| {
            s.execute("edit.pasteSpecial", json!({"what":"formats","at":"B2"}))?;
            panic!("synthetic panic after transfer");
        })
    }));
    assert!(caught.is_err());
    assert!(Arc::ptr_eq(&s.doc().unwrap().wb, &before));
    assert_eq!(origin(&s), Origin::EncryptedOrigin);
    let path = std::env::temp_dir().join(format!("origin-{}.xlsx", s.doc().unwrap().uid));
    for command in ["file.saveAs", "file.save"] {
        s.execute_headless(command, json!({"path":path})).unwrap();
        assert_eq!(origin(&s), Origin::EncryptedOrigin);
    }
    std::fs::remove_file(path).unwrap();
}
#[test]
fn macro_dry_run_keeps_taint_but_restores_trust_and_painter_binding() {
    let mut s = source(true);
    s.execute("edit.copy", json!({"range":"A1"})).unwrap();
    s.new_workbook();
    s.execute("home.bold", json!({"range":"A1"})).unwrap();
    s.execute("edit.formatPainter", json!({"sticky":true})).unwrap();
    s.execute(
        "macro.save",
        json!({"name":"Receive", "steps":[
            {"command":"edit.pasteSpecial","params":{"what":"all","at":"B2"}},
            {"command":"edit.formatPainter","params":{"sticky":true}}
        ]}),
    )
    .unwrap();
    let before = s.doc().unwrap().wb.clone();
    let result = s.execute("macro.check", json!({"name":"Receive"})).unwrap();
    assert_eq!(result["ok"], json!(true), "{result}");
    assert!(Arc::ptr_eq(&s.doc().unwrap().wb, &before));
    assert_eq!(s.doc().unwrap().trust, Trust::Local);
    assert_eq!(origin(&s), Origin::EncryptedOrigin);
    // The original Plain painter is restored with its own binding, rather than the dry-run one.
    s.new_workbook();
    s.execute("edit.formatPainter", json!({"apply":"C3"})).unwrap();
    assert_eq!(origin(&s), Origin::Plain);
}
#[test]
fn system_text_equality_is_not_source_authority() {
    for encrypted in [false, true] {
        let mut s = source(encrypted);
        let copy = s.execute("edit.copy", json!({"range":"A1"})).unwrap();
        s.new_workbook();
        s.execute("edit.paste", json!({"text":copy["text"],"at":"C3"})).unwrap();
        assert_eq!(origin(&s), if encrypted { Origin::EncryptedOrigin } else { Origin::Unknown });
    }
    for params in [json!({"text":"42"}), json!({"text":"42","html":"<table><tr><td>42</td></tr></table>"})] {
        let mut s = Session::new();
        s.new_workbook();
        s.execute("edit.paste", params).unwrap();
        assert_eq!(origin(&s), Origin::Unknown);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(origin(&s), Origin::Unknown);
    }
}
#[test]
fn retained_fact_save_as_and_generic_restore_stay_ineligible() {
    let mut s = opened(true);
    let path = std::env::temp_dir().join(format!("origin-open-{}.xlsx", s.doc().unwrap().uid));
    let analysis = s.execute_headless("file.analyzeSave", json!({"path":path})).unwrap();
    let codes: Vec<_> = analysis["issues"].as_array().unwrap().iter().map(|i| i["code"].clone()).collect();
    s.execute_headless(
        "file.saveAs",
        json!({"path":path,"confirmedLosses":codes,"expectedRevision":analysis["revision"],"expectedDocument":analysis["document"]}),
    )
    .unwrap();
    assert_eq!(origin(&s), Origin::EncryptedOrigin);
    assert!(s.doc().unwrap().import_issues().iter().any(|i| i.effect == gridcraft_engine::model::CompatibilityEffect::EncryptionRemoved));
    std::fs::remove_file(path).unwrap();
    const SNAPSHOT: &[u8] = include_bytes!("../../src/recovery/manifest/complete/tests/goldens/fixtures/minimal.gcr");
    let doc = gridcraft_engine::recovery::restore(gridcraft_engine::recovery::decode(SNAPSHOT).unwrap()).unwrap();
    assert_eq!(doc.recovery_provenance(), Origin::Unknown);
    s.add_document(doc);
    assert_eq!(origin(&s), Origin::Unknown);
    let uid = s.doc().unwrap().uid;
    s.receive_unknown_recovery_origin(uid).unwrap();
    assert_eq!(origin(&s), Origin::Unknown);
    assert!(s.receive_unknown_recovery_origin(u64::MAX).is_err());
}

#[test]
fn external_imports_and_images_receive_unknown_before_content_commit() {
    let png = {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255])).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    };
    let path = std::env::temp_dir().join(format!("origin-import-{}.csv", std::process::id()));
    std::fs::write(&path, b"Name,Score\nAda,42").unwrap();
    for (command, params) in [
        ("data.fromTextCsv", json!({"text":"Name,Score\nAda,42"})),
        ("data.getData", json!({"text":"[{\"Name\":\"Ada\",\"Score\":42}]"})),
        ("data.fromTextCsv", json!({"path":path})),
        ("data.getData", json!({"path":path})),
        ("insert.picture", json!({"base64":io::base64_encode(&png)})),
        ("insert.picture", json!({"base64":io::base64_encode(&png),"placement":"cell","at":"B2"})),
        ("pageLayout.background", json!({"base64":io::base64_encode(&png)})),
    ] {
        let mut s = Session::new();
        s.new_workbook();
        s.execute(command, params.clone()).unwrap();
        assert_eq!(origin(&s), Origin::Unknown, "{command} {params}");
        assert_eq!(s.doc().unwrap().trust, Trust::Local);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(origin(&s), Origin::Unknown);
    }
    std::fs::remove_file(&path).unwrap();
    let image_path = path.with_extension("png");
    std::fs::write(&image_path, &png).unwrap();
    for command in ["insert.picture", "pageLayout.background"] {
        let mut s = Session::new();
        s.new_workbook();
        s.execute(command, json!({"path":image_path})).unwrap();
        assert_eq!(origin(&s), Origin::Unknown);
        assert_eq!(s.doc().unwrap().wb.active().unwrap().images[0].data.as_ref(), &png);
    }
    std::fs::remove_file(image_path).unwrap();
}
#[test]
fn same_document_table_range_and_sheet_copy_keep_lineage() {
    for encrypted in [false, true] {
        let mut s = source(encrypted);
        s.execute("data.fromTableRange", json!({"range":"A1:B2", "text":"Hostile,Secret\nattacker,999", "path":"unreadable-external-origin.csv"}))
            .unwrap();
        assert_eq!(
            s.doc().unwrap().wb.active().unwrap().value(gridcraft_engine::core::CellRef::default()).display(),
            "42",
            "only the owning document is serialized"
        );
        assert_eq!(origin(&s), if encrypted { Origin::EncryptedOrigin } else { Origin::Plain });
        s.execute("sheet.move", json!({"to":0,"copy":true})).unwrap();
        assert_eq!(origin(&s), if encrypted { Origin::EncryptedOrigin } else { Origin::Plain });
    }
}

#[test]
fn expired_clipboard_binding_and_changed_source_identity_are_unknown() {
    for close in [false, true] {
        let mut s = source(false);
        s.execute("edit.copy", json!({"range":"A1"})).unwrap();
        if close {
            s.clipboard.as_mut().unwrap().wb = Arc::new(Workbook::new());
            s.close_document(0);
        } else {
            s.clipboard.as_mut().unwrap().doc_uid = u64::MAX;
        }
        s.new_workbook();
        s.execute("edit.pasteSpecial", json!({"what":"formats","at":"C3"})).unwrap();
        assert_eq!(origin(&s), Origin::Unknown);
    }
    let mut s = opened(true);
    let uid = s.doc().unwrap().uid;
    s.receive_unknown_recovery_origin(uid).unwrap();
    assert_eq!(origin(&s), Origin::EncryptedOrigin);
}
#[test]
fn command_panic_rollback_preserves_received_origin() {
    gridcraft_engine::register_test_command("test.recoveryTransferPanic", |s, _| {
        s.execute("edit.pasteSpecial", json!({"what":"values","at":"B2"}))?;
        panic!("synthetic command panic after transfer");
    });
    let mut s = source(true);
    s.execute("edit.copy", json!({"range":"A1"})).unwrap();
    s.new_workbook();
    let before = s.doc().unwrap().wb.clone();
    assert!(s.execute("test.recoveryTransferPanic", json!({})).is_err());
    assert!(Arc::ptr_eq(&s.doc().unwrap().wb, &before));
    assert_eq!(origin(&s), Origin::EncryptedOrigin);
}

#[test]
fn encrypted_source_never_writes_plaintext_recovery() {
    for mut s in [opened(true), {
        let mut s = Session::new();
        s.add_document(DocState::new(Workbook::new(), None, "unknown".into(), Trust::Local));
        s
    }] {
        let uid = s.doc().unwrap().uid;
        let mut storage_calls = 0;
        if let Ok(ticket) = s.recovery_ticket(uid) {
            ticket
                .with_storage(|| {
                    storage_calls += 1;
                    Ok::<_, ()>(())
                })
                .unwrap();
            panic!("ineligible source issued authority");
        }
        assert_eq!(storage_calls, 0);
    }
}

#[test]
fn direct_engine_close_save_and_headless_transfer_cancel_job() {
    use gridcraft_engine::recovery::RecoveryAttemptError;
    let files = super::recovery_snapshots::RestoreFiles::new();
    for route in 0..5 {
        for action in ["save", "close", "transfer"] {
            let mut s = Session::new();
            s.new_workbook();
            let uid = s.doc().unwrap().uid;
            let path = files.0.join(format!("recovery-life-{uid}.xlsx"));
            s.execute("file.save", json!({"path":path})).unwrap();
            let ticket = s.recovery_ticket(uid).unwrap();
            let job = s.capture_recovery(ticket, "0123456789abcdef0123456789abcdef".parse().unwrap(), 1).unwrap();
            let (command, params) = match action {
                "save" => ("file.save", json!({})),
                "close" => ("file.close", json!({"discard":true})),
                _ => ("edit.paste", json!({"text":"secret","at":"A1"})),
            };
            match route {
                0 => {
                    s.execute(command, params).unwrap();
                }
                1 => {
                    s.run(command, params).unwrap();
                }
                2 => {
                    s.execute_headless(command, params).unwrap();
                }
                3 => {
                    s.execute_in(gridcraft_engine::Target::Window(s.active_window().unwrap()), command, params).unwrap();
                }
                _ => {
                    if action == "save" {
                        s.autosave_document(uid).unwrap();
                    } else if action == "close" {
                        s.close_document(0);
                    } else {
                        s.receive_unknown_recovery_origin(uid).unwrap();
                    }
                }
            }
            let mut calls = 0;
            assert!(
                matches!(
                    job.with_storage(|| {
                        calls += 1;
                        Ok::<_, ()>(())
                    }),
                    Err(RecoveryAttemptError::Stale | RecoveryAttemptError::Unavailable)
                ),
                "{route} {action}"
            );
            assert_eq!(calls, 0);
        }
    }
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn authorized_publication_finishes_before_each_public_lifecycle_route() {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::{Duration, Instant},
    };
    let files = super::recovery_snapshots::RestoreFiles::new();
    let wait = Duration::from_secs(10);
    for route in 0..5 {
        for action in ["save", "close", "transfer"] {
            let mut s = Session::new();
            s.new_workbook();
            let uid = s.doc().unwrap().uid;
            let path = files.0.join(format!("recovery-race-{uid}.xlsx"));
            s.execute("file.save", json!({"path":path})).unwrap();
            let ticket = s.recovery_ticket(uid).unwrap();
            let job = s.capture_recovery(ticket, "0123456789abcdef0123456789abcdef".parse().unwrap(), 1).unwrap();
            let committed = Arc::new(AtomicBool::new(false));
            let committed_writer = committed.clone();
            let (entered, entry) = mpsc::channel();
            let (release, resume) = mpsc::channel();
            let (attempt, attempted) = mpsc::channel();
            let (done, finished) = mpsc::channel();
            let publish = std::thread::spawn(move || {
                job.with_storage(|| {
                    entered.send(()).unwrap();
                    resume.recv_timeout(wait).unwrap();
                    committed_writer.store(true, Ordering::SeqCst);
                    Ok::<_, ()>(())
                })
                .unwrap()
            });
            entry.recv_timeout(wait).unwrap();
            let lifecycle = std::thread::spawn(move || {
                attempt.send(()).unwrap();
                #[allow(clippy::disallowed_methods, reason = "this gate-order measurement test is native-only")]
                let started = Instant::now();
                let (command, params) = match action {
                    "save" => ("file.save", json!({})),
                    "close" => ("file.close", json!({"discard":true})),
                    _ => ("edit.paste", json!({"text":"secret","at":"A1"})),
                };
                match route {
                    0 => {
                        s.execute(command, params).unwrap();
                    }
                    1 => {
                        s.run(command, params).unwrap();
                    }
                    2 => {
                        s.execute_headless(command, params).unwrap();
                    }
                    3 => {
                        s.execute_in(gridcraft_engine::Target::Window(s.active_window().unwrap()), command, params).unwrap();
                    }
                    _ => {
                        if action == "save" {
                            s.autosave_document(uid).unwrap();
                        } else if action == "close" {
                            s.close_document(0);
                        } else {
                            s.receive_unknown_recovery_origin(uid).unwrap();
                        }
                    }
                }
                assert!(committed.load(Ordering::SeqCst));
                done.send(started.elapsed()).unwrap();
            });
            attempted.recv_timeout(wait).unwrap();
            assert!(matches!(finished.recv_timeout(Duration::from_millis(20)), Err(mpsc::RecvTimeoutError::Timeout)));
            release.send(()).unwrap();
            let elapsed = finished.recv_timeout(wait).unwrap();
            publish.join().unwrap();
            lifecycle.join().unwrap();
            eprintln!("P5B_FAKE_GATE_ORDER route={route} action={action} lifecycle_elapsed_us={}", elapsed.as_micros());
        }
    }
}
