//! Synthetic AutoSave destinations; never externally authored workbook fixtures.
use egui_kittest::kittest::Queryable;
use gridcraft_engine::{DocState, Session, Trust, model::Workbook};
use serde_json::json;

fn directory(name: &str) -> std::path::PathBuf {
    let id = DocState::new(Workbook::new(), None, "id".into(), Trust::Local).uid;
    let dir = std::env::temp_dir().join(format!("gridcraft-autosave-{name}-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn window_context(session: &Session) -> serde_json::Value {
    let mut windows = session.window_list();
    for w in windows.as_array_mut().unwrap() {
        w.as_object_mut().unwrap().remove("dirty");
    }
    windows
}

fn tick(app: &mut gridcraft_ui_egui::SheetApp) {
    let ctx = egui::Context::default();
    let _ = ctx.run_logic(&egui::RawInput::default(), |ctx| app.logic(ctx));
}

#[test]
fn all_eligible_documents_are_considered() {
    let dir = directory("all");
    let mut app = crate::common::app(Session::new());
    for name in ["first.xlsx", "second.XLSX", "third.xlsm"] {
        app.session.new_workbook();
        app.run("cell.set", json!({"cell":"A1","input":name})).unwrap();
        app.session.active_mut().unwrap().path = Some(dir.join(name).to_str().unwrap().into());
        if name.ends_with(".xlsm") {
            std::sync::Arc::make_mut(&mut app.session.doc_mut().unwrap().wb).vba_project = Some(std::sync::Arc::new(vec![7, 8, 9]));
        }
    }
    let active = app.session.active().unwrap().uid;
    let window = app.session.active_window();
    let windows = window_context(&app.session);
    let selection = app.session.active().unwrap().selection.clone();
    app.ui.autosave = true;
    tick(&mut app);
    for name in ["first.xlsx", "second.XLSX", "third.xlsm"] {
        let bytes = std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("{name} was not saved: {e}"));
        let (wb, _) = gridcraft_engine::io::open_bytes(name, &bytes).unwrap();
        assert_eq!(wb.active().unwrap().value(gridcraft_engine::core::CellRef::new(0, 0)), gridcraft_engine::core::Value::text(name));
        if name.ends_with(".xlsm") {
            assert_eq!(wb.vba_project.as_deref().map(Vec::as_slice), Some(&[7, 8, 9][..]));
        }
    }
    assert!(app.session.documents().iter().all(|d| !d.is_dirty()));
    assert_eq!(app.session.active().unwrap().uid, active);
    assert_eq!(app.session.active_window(), window);
    assert_eq!(window_context(&app.session), windows, "no activation or MRU change");
    assert_eq!(app.session.active().unwrap().selection, selection);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn untitled_explicitly_reports_recovery_unavailable() {
    let mut session = Session::new();
    session.new_workbook();
    session.run("cell.set", json!({"cell":"A1","input":"unsaved"})).unwrap();
    let mut app = crate::common::app(session);
    app.ui.autosave = true;
    let mut h = crate::common::harness_with(crate::common::builder().with_size(egui::vec2(1200.0, 800.0)), app, 4);
    assert!(h.query_by_label("AutoSave paused for Book1: no save destination. Recovery is not available yet; choose Save As.").is_some());
    assert!(h.state().session.doc().unwrap().is_dirty());
    assert!(h.state().dialog.is_none());
    if let Some(path) = std::env::var_os("GRIDCRAFT_AUTOSAVE_SCREENSHOT") {
        h.render().unwrap().save(path).unwrap();
    }
    h.state_mut().ui.dark = true;
    h.run_steps(4);
    assert!(h.query_by_label("AutoSave paused for Book1: no save destination. Recovery is not available yet; choose Save As.").is_some());
    if let Some(path) = std::env::var_os("GRIDCRAFT_AUTOSAVE_DARK_SCREENSHOT") {
        h.render().unwrap().save(path).unwrap();
    }
}

#[test]
fn lossy_save_never_auto_confirms() {
    let dir = directory("loss");
    let path = dir.join("loss.xlsx");
    let mut app = crate::common::app(Session::new());
    app.session.new_workbook();
    app.run("range.setValues", json!({"range":"A1","values":[["Region"],["East"]]})).unwrap();
    app.run("insert.table", json!({"range":"A1:A2","name":"Sales","header":true})).unwrap();
    app.run("insert.slicer", json!({"table":"Sales","column":"Region","at":"D1"})).unwrap();
    app.session.active_mut().unwrap().path = Some(path.to_str().unwrap().into());
    std::fs::write(&path, b"original").unwrap();
    app.ui.autosave = true;
    tick(&mut app);
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert!(app.session.doc().unwrap().is_dirty());
    assert!(app.dialog.is_none());
    assert!(!app.session.journal.iter().any(|(id, _)| id == "file.save"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unwritable_destination_keeps_dirty_and_notifies() {
    let dir = directory("failure");
    let path = dir.join("missing").join("blocked.xlsx");
    let mut app = crate::common::app(Session::new());
    app.session.new_workbook();
    app.run("cell.set", json!({"cell":"A1","input":"keep me"})).unwrap();
    app.session.active_mut().unwrap().path = Some(path.to_str().unwrap().into());
    let uid = app.session.doc().unwrap().uid;
    let error = app.session.run("file.save", json!({})).unwrap_err();
    let label = format!("AutoSave paused for blocked.xlsx: {}", gridcraft_ui_egui::clean_error(&error));
    app.ui.autosave = true;
    let mut h = crate::common::harness_with(crate::common::builder().with_size(egui::vec2(1200.0, 800.0)), app, 4);
    assert!(h.query_by_label(&label).is_some(), "failure is visible");
    assert!(h.state().session.doc().unwrap().is_dirty());
    h.state_mut().notice = None;
    h.state_mut().session.new_workbook();
    h.run_steps(4);
    assert!(h.query_by_label(&label).is_some(), "ordinary notices cannot dismiss AutoSave failure");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    h.run_steps(4);
    assert!(!path.exists(), "failure retries are bounded, not once per frame");
    std::thread::sleep(std::time::Duration::from_millis(2100));
    h.run_steps(4);
    assert!(path.exists());
    assert!(!h.state().session.documents().iter().find(|d| d.uid == uid).unwrap().is_dirty());
    assert!(h.query_by_label(&label).is_none(), "a successful retry clears the failure");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn locked_and_failed_inactive_documents_do_not_block_another_save() {
    let dir = directory("locked");
    let mut app = crate::common::app(Session::new());
    app.session.new_workbook();
    app.run("cell.set", json!({"cell":"A1","input":"locked"})).unwrap();
    let locked = app.session.doc().unwrap().uid;
    app.session.doc_mut().unwrap().path = Some(dir.join("locked.xlsx").to_str().unwrap().into());
    let m = gridcraft_engine::macros::parse_macro(&json!({"name":"Paused", "steps":[{"command":"cell.set","params":{"cell":"B1","input":"step"}}]}))
        .unwrap();
    std::sync::Arc::make_mut(&mut app.session.doc_mut().unwrap().wb).macros.push(m);
    app.session.execute("macro.debug", json!({"name":"Paused","action":"start"})).unwrap();
    app.session.new_workbook();
    app.run("cell.set", json!({"cell":"A1","input":"failed"})).unwrap();
    app.session.doc_mut().unwrap().path = Some(dir.join("missing").join("failed.xlsx").to_str().unwrap().into());
    let failed = app.session.doc().unwrap().uid;
    app.session.new_workbook();
    app.run("cell.set", json!({"cell":"A1","input":"eligible"})).unwrap();
    let good = app.session.doc().unwrap().uid;
    app.session.doc_mut().unwrap().path = Some(dir.join("good.xlsx").to_str().unwrap().into());
    app.run("selection.set", json!({"cell":"D4"})).unwrap();
    let selected = app.session.doc().unwrap().selection.clone();
    app.session.new_window().unwrap();
    app.session.new_workbook();
    let active = app.session.doc().unwrap().uid;
    let windows = window_context(&app.session);
    let locked_error = app.session.autosave_document(locked).unwrap_err();
    assert!(matches!(locked_error, gridcraft_engine::EngineError::Disabled(_, _)));
    assert_eq!(app.session.doc().unwrap().uid, active);
    app.session.start_recording();
    app.ui.autosave = true;
    tick(&mut app);
    assert!(dir.join("good.xlsx").exists());
    assert!(!dir.join("locked.xlsx").exists(), "AutoSave must honor execute's macro lock");
    assert!(app.session.documents().iter().find(|d| d.uid == locked).unwrap().is_dirty());
    assert!(app.session.documents().iter().find(|d| d.uid == failed).unwrap().is_dirty());
    assert_eq!(app.session.doc().unwrap().uid, active);
    assert_eq!(window_context(&app.session), windows);
    assert_eq!(app.session.documents().iter().find(|d| d.uid == good).unwrap().selection, selected);
    let saves: Vec<_> = app.session.recorder.as_ref().unwrap().steps.iter().filter(|s| s.id == "file.save").collect();
    assert_eq!(saves.len(), 1);
    assert_eq!(saves[0].doc, good);
    assert_eq!(saves[0].selection, selected);
    assert_eq!(app.session.macro_debug.as_ref().unwrap().doc, locked);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn encrypted_own_file_never_becomes_unprotected_automatically() {
    let dir = directory("encrypted");
    let path = dir.join("encrypted.XLSX");
    let plain = gridcraft_engine::io::save_bytes(&Workbook::new(), "plain.xlsx").unwrap();
    let encrypted = gridcraft_xlsx::testing::encrypt_workbook(&plain, "synthetic password");
    std::fs::write(&path, &encrypted).unwrap();
    let mut session = Session::new();
    session.execute_headless("file.open", json!({"path":path,"password":"synthetic password"})).unwrap();
    session.execute("cell.set", json!({"cell":"A1","input":"dirty"})).unwrap();
    // Decryption normally clears the destination. Even if a host attaches the original path,
    // the retained encryption issue must still block the unacknowledged save.
    assert!(session.doc().unwrap().path.is_none());
    session.doc_mut().unwrap().path = Some(path.to_str().unwrap().into());
    let mut app = crate::common::app(session);
    app.ui.autosave = true;
    let h = crate::common::harness_with(crate::common::builder().with_size(egui::vec2(1200.0, 800.0)), app, 4);
    assert!(
        h.query_by_label(
            "AutoSave paused for encrypted.XLSX: saving needs your decision about content loss or compatibility. Choose Save to review it."
        )
        .is_some()
    );
    assert!(h.state().session.doc().unwrap().is_dirty());
    assert!(h.state().dialog.is_none());
    assert_eq!(std::fs::read(&path).unwrap(), encrypted);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn inactive_save_flushes_the_live_view_instead_of_mru_or_visible_window() {
    use gridcraft_engine::core::CellRef;
    use gridcraft_ui_egui::SheetView;
    let dir = directory("views");
    let path = dir.join("views.xlsx");
    let mut app = crate::common::app(Session::new());
    app.session.new_workbook();
    app.run("cell.set", json!({"cell":"A1","input":"data"})).unwrap();
    app.session.doc_mut().unwrap().path = Some(path.to_str().unwrap().into());
    let uid = app.session.doc().unwrap().uid;
    let first = app.session.active_window().unwrap();
    let second = app.session.new_window().unwrap();
    app.run("selection.set", json!({"cell":"E51"})).unwrap();
    app.views.insert((first, 0), SheetView { scroll: egui::vec2(0.0, 0.0) });
    let y = app.session.doc().unwrap().wb.active().unwrap().row_top(50) as f32;
    app.views.insert((second, 0), SheetView { scroll: egui::vec2(0.0, y) });
    app.session.activate_window(first);
    app.session.hide_window(second);
    app.session.show_window(second);
    assert_eq!(app.session.doc_window(uid), Some(first));
    assert_eq!(app.session.document_view_window(uid), Some(second));
    assert!(app.session.window(second).unwrap().hidden);
    let selection = app.session.doc().unwrap().selection.clone();
    app.session.new_workbook();
    let active = app.session.doc().unwrap().uid;
    let windows = window_context(&app.session);
    app.ui.autosave = true;
    tick(&mut app);
    let (wb, _) = gridcraft_engine::io::open_bytes("views.xlsx", &std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(wb.active().unwrap().view_top_left, CellRef::new(50, 0));
    assert_eq!(wb.active().unwrap().view_active, CellRef::new(50, 4));
    assert_eq!(app.session.documents().iter().find(|d| d.uid == uid).unwrap().selection, selection);
    assert_eq!(app.session.doc().unwrap().uid, active);
    assert_eq!(window_context(&app.session), windows);
    assert_eq!(app.session.doc_window(uid), Some(first));
    assert_eq!(app.session.document_view_window(uid), Some(second));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn current_and_parked_drafts_are_preserved_while_other_documents_save() {
    let dir = directory("drafts");
    let mut app = crate::common::app(Session::new());
    app.session.new_workbook();
    app.run("cell.set", json!({"cell":"A1","input":"committed"})).unwrap();
    app.session.doc_mut().unwrap().path = Some(dir.join("draft.xlsx").to_str().unwrap().into());
    app.begin_edit(Some("unfinished".into()), false);
    let uid = app.session.doc().unwrap().uid;
    app.session.new_workbook();
    app.follow_active_window(); // Park the draft without settling it, as native hosts can.
    app.run("cell.set", json!({"cell":"A1","input":"other"})).unwrap();
    app.session.doc_mut().unwrap().path = Some(dir.join("other.xlsx").to_str().unwrap().into());
    app.ui.autosave = true;
    tick(&mut app);
    assert!(!dir.join("draft.xlsx").exists());
    assert!(dir.join("other.xlsx").exists());
    let first = app.session.doc_window(uid).unwrap();
    app.session.show_window(first);
    app.follow_active_window();
    let before = app.editor.as_ref().unwrap().clone();
    tick(&mut app);
    let after = app.editor.as_ref().unwrap();
    assert_eq!(
        (&after.text, after.doc, after.window, after.caret, after.anchor),
        (&before.text, before.doc, before.window, before.caret, before.anchor)
    );
    assert!(!dir.join("draft.xlsx").exists());
    assert!(app.session.doc().unwrap().is_dirty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn engine_autosave_rejects_non_destination_and_preserves_context_on_failure() {
    let mut session = Session::new();
    assert!(session.autosave_document(u64::MAX).is_err());
    assert!(session.store_document_view(u64::MAX, &[]).is_err());
    assert_eq!(session.document_view_window(u64::MAX), None);
    session.new_workbook();
    let uid = session.doc().unwrap().uid;
    assert!(session.autosave_document(uid).is_err());
    assert!(session.ui_requests.is_empty(), "untitled must not open Save As");
    for extension in ["csv", "pdf", "json"] {
        session.doc_mut().unwrap().path = Some(format!("must-not-write.{extension}"));
        assert!(session.autosave_document(uid).is_err());
    }
    let window = session.active_window();
    session.new_workbook();
    let active = session.doc().unwrap().uid;
    assert!(session.autosave_document(uid).is_err());
    assert_eq!(session.doc().unwrap().uid, active);
    assert_ne!(session.active_window(), window);
    assert!(session.ui_requests.is_empty());
    let dir = directory("clean-helper");
    let path = dir.join("clean.xlsx");
    session.doc_mut().unwrap().path = Some(path.to_str().unwrap().into());
    assert!(!session.doc().unwrap().is_dirty());
    let result = session.autosave_document(active).unwrap();
    assert!(result["bytes"].as_u64().unwrap() > 0);
    assert!(std::fs::read(&path).unwrap().starts_with(b"PK"));
    assert!(!session.doc().unwrap().is_dirty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn active_autosave_keeps_trace_arrows_through_stored_view_changes() {
    use gridcraft_engine::core::CellRef;
    let dir = directory("trace");
    let path = dir.join("trace.xlsx");
    let mut app = crate::common::app(Session::new());
    app.session.new_workbook();
    app.run("file.save", json!({"path":path})).unwrap();
    app.run("cell.set", json!({"cell":"A1","input":"42"})).unwrap();
    app.run("cell.set", json!({"cell":"B1","input":"=A1"})).unwrap();
    app.run("selection.set", json!({"cell":"B1"})).unwrap();
    app.view_mut().unwrap().scroll.y = 1000.0;
    app.grid.trace = gridcraft_ui_egui::grid::Trace::new(&app, json!([{"range":"A1"}]));
    let arrows = app.grid.trace.as_ref().unwrap().arrows.clone();
    app.ui.autosave = true;
    tick(&mut app);
    let d = app.session.doc().unwrap();
    assert!(!d.is_dirty());
    assert_ne!(d.wb.active().unwrap().view_top_left, CellRef::new(0, 0));
    let trace = app.grid.trace.as_ref().unwrap();
    assert_eq!(trace.revision, d.revision, "pure stored view changes preserve arrows");
    assert_eq!(trace.arrows, arrows);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_active_draft_does_not_defer_another_document() {
    let dir = directory("active-draft");
    let mut app = crate::common::app(Session::new());
    app.session.new_workbook();
    app.run("cell.set", json!({"cell":"A1","input":"other"})).unwrap();
    app.session.doc_mut().unwrap().path = Some(dir.join("other.xlsx").to_str().unwrap().into());
    app.session.new_workbook();
    app.run("cell.set", json!({"cell":"A1","input":"committed"})).unwrap();
    app.session.doc_mut().unwrap().path = Some(dir.join("draft.xlsx").to_str().unwrap().into());
    app.begin_edit(Some("unfinished".into()), false);
    let uid = app.session.doc().unwrap().uid;
    let window = app.session.active_window();
    let mode = app.session.mode;
    let before = app.editor.as_ref().unwrap().clone();
    app.ui.autosave = true;
    tick(&mut app);
    assert!(dir.join("other.xlsx").exists());
    assert!(!dir.join("draft.xlsx").exists());
    assert_eq!((app.session.doc().unwrap().uid, app.session.active_window(), app.session.mode), (uid, window, mode));
    let after = app.editor.as_ref().unwrap();
    assert_eq!(
        (&after.text, after.doc, after.window, after.caret, after.anchor),
        (&before.text, before.doc, before.window, before.caret, before.anchor)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn textbox_draft_uses_its_document_before_the_first_window_sync() {
    let dir = directory("textbox-owner");
    let mut session = Session::new();
    session.new_workbook();
    let id = session.execute("insert.textBox", json!({"at":"B2","text":"committed"})).unwrap()["shape"].as_u64().unwrap() as u32;
    session.doc_mut().unwrap().path = Some(dir.join("draft.xlsx").to_str().unwrap().into());
    let draft_doc = session.doc().unwrap().uid;
    let mut app = crate::common::app(session);
    assert_eq!(app.ui_window(), None);
    assert!(app.begin_text_box_edit(id));
    app.text_box_editor.as_mut().unwrap().text = "unfinished text".into();
    // A newly embedded app can receive a Session switch before its first UI/window sync.
    app.session.new_workbook();
    app.session.execute("cell.set", json!({"cell":"A1","input":"other document"})).unwrap();
    app.session.doc_mut().unwrap().path = Some(dir.join("other.xlsx").to_str().unwrap().into());
    let active = app.session.doc().unwrap().uid;
    let window = app.session.active_window();
    app.ui.autosave = true;
    tick(&mut app);
    assert!(!dir.join("draft.xlsx").exists(), "the actual textbox-owning document must be deferred");
    assert!(dir.join("other.xlsx").exists(), "the unrelated active document remains eligible");
    assert!(app.session.documents().iter().find(|d| d.uid == draft_doc).unwrap().is_dirty());
    assert_eq!((app.session.doc().unwrap().uid, app.session.active_window()), (active, window));
    let draft = app.text_box_editor.as_ref().unwrap();
    assert_eq!((draft.id, draft.text.as_str()), (id, "unfinished text"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn inactive_autosave_rebases_valid_parked_traces_only_for_its_view_change() {
    use gridcraft_ui_egui::grid::Trace;
    let dir = directory("parked-traces");
    let path = dir.join("traced.xlsx");
    let mut app = crate::common::app(Session::new());
    app.session.new_workbook();
    // Establish source/destination metadata before tracing: this test isolates view storage.
    app.run("file.save", json!({"path":path})).unwrap();
    app.run("cell.set", json!({"cell":"A1","input":"42"})).unwrap();
    app.run("cell.set", json!({"cell":"B1","input":"=A1"})).unwrap();
    app.run("selection.set", json!({"cell":"B1"})).unwrap();
    let uid = app.session.doc().unwrap().uid;
    let stale_window = app.session.active_window().unwrap();
    app.grid.trace = Trace::new(&app, json!([{"range":"A1"}]));
    let stale_revision = app.grid.trace.as_ref().unwrap().revision;
    let arrows = app.grid.trace.as_ref().unwrap().arrows.clone();
    app.run("view.newWindow", json!({})).unwrap();
    app.run("cell.set", json!({"cell":"A1","input":"43"})).unwrap();
    let valid_window = app.session.active_window().unwrap();
    app.grid.trace = Trace::new(&app, json!([{"range":"A1"}]));
    app.run("view.newWindow", json!({})).unwrap();
    let live_window = app.session.active_window().unwrap();
    app.grid.trace = Trace::new(&app, json!([{"range":"A1"}]));
    let before_view = app.grid.trace.as_ref().unwrap().revision;
    assert!(stale_revision < before_view, "a real edit made only the old trace stale");
    app.run("selection.set", json!({"cell":"E5"})).unwrap();
    app.session.new_workbook();
    app.follow_active_window();
    app.run("cell.set", json!({"cell":"A1","input":"7"})).unwrap();
    app.run("cell.set", json!({"cell":"B1","input":"=A1"})).unwrap();
    app.run("selection.set", json!({"cell":"B1"})).unwrap();
    app.run("file.save", json!({"path":dir.join("other.xlsx")})).unwrap();
    // Use matching revisions so this control detects a missing document-identity guard.
    let other_before = app.session.doc().unwrap().revision;
    assert!(other_before <= before_view && before_view - other_before <= 8);
    for revision in other_before..before_view {
        app.run("cell.set", json!({"cell":"C1","input":revision.to_string()})).unwrap();
    }
    app.run("file.save", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().revision, before_view);
    let other_uid = app.session.doc().unwrap().uid;
    let other_window = app.session.active_window().unwrap();
    app.grid.trace = Trace::new(&app, json!([{"range":"A1"}]));
    let other_revision = app.grid.trace.as_ref().unwrap().revision;
    let other_arrows = app.grid.trace.as_ref().unwrap().arrows.clone();
    app.run("view.newWindow", json!({})).unwrap();
    let active_window = app.session.active_window();
    app.grid.trace = Trace::new(&app, json!([{"range":"A1"}]));
    let windows = window_context(&app.session);
    app.ui.autosave = true;
    tick(&mut app);
    let saved = app.session.documents().iter().find(|d| d.uid == uid).unwrap();
    let saved_revision = saved.revision;
    assert!(!saved.is_dirty());
    assert!(saved_revision > before_view, "the pending active-cell view was stored");
    assert_eq!(saved.wb.active().unwrap().view_active, gridcraft_engine::core::CellRef::new(4, 4));
    assert_eq!(app.session.doc().unwrap().uid, other_uid);
    assert_eq!(app.session.active_window(), active_window);
    assert_eq!(window_context(&app.session), windows);
    let other_trace = app.grid.trace.as_ref().unwrap();
    assert_eq!((other_trace.uid, other_trace.revision, &other_trace.arrows), (other_uid, other_revision, &other_arrows));
    for window in [live_window, valid_window] {
        app.enter_window(window);
        let trace = app.grid.trace.as_ref().unwrap();
        assert_eq!(trace.revision, saved_revision, "valid parked trace in window {window} survives pure view storage");
        assert_eq!(trace.arrows, arrows);
    }
    app.enter_window(stale_window);
    let stale = app.grid.trace.as_ref().unwrap();
    assert_eq!(stale.revision, stale_revision, "a content-invalidated trace must remain stale");
    assert_ne!(stale.revision, saved_revision);
    assert_eq!(stale.arrows, arrows);
    app.enter_window(other_window);
    let other_trace = app.grid.trace.as_ref().unwrap();
    assert_eq!((other_trace.uid, other_trace.revision, &other_trace.arrows), (other_uid, other_revision, &other_arrows));
    if let Some(path) = std::env::var_os("GRIDCRAFT_R01_I1_SCREENSHOT") {
        app.enter_window(live_window);
        let mut h = crate::common::harness_with(crate::common::builder().with_size(egui::vec2(1200.0, 800.0)), app, 4);
        h.render().unwrap().save(path).unwrap();
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn inactive_autosave_does_not_rebase_traces_across_new_source_metadata() {
    use gridcraft_ui_egui::grid::Trace;
    let dir = directory("trace-source");
    let path = dir.join("first-save.xlsx");
    let mut app = crate::common::app(Session::new());
    app.session.new_workbook();
    app.run("cell.set", json!({"cell":"A1","input":"42"})).unwrap();
    app.run("cell.set", json!({"cell":"B1","input":"=A1"})).unwrap();
    app.run("selection.set", json!({"cell":"B1"})).unwrap();
    app.session.doc_mut().unwrap().path = Some(path.to_str().unwrap().into());
    assert!(app.session.doc().unwrap().wb.source_path.is_none());
    let window = app.session.active_window().unwrap();
    app.grid.trace = Trace::new(&app, json!([{"range":"A1"}]));
    let before_view = app.grid.trace.as_ref().unwrap().revision;
    let arrows = app.grid.trace.as_ref().unwrap().arrows.clone();
    app.run("selection.set", json!({"cell":"E5"})).unwrap();
    app.session.new_workbook();
    app.follow_active_window();
    app.ui.autosave = true;
    tick(&mut app);
    app.enter_window(window);
    let d = app.session.doc().unwrap();
    assert!(!d.is_dirty());
    assert!(d.wb.source_path.is_some(), "ordinary save established new source metadata");
    let trace = app.grid.trace.as_ref().unwrap();
    assert!(trace.revision > before_view, "the pure stored-view change was rebased");
    assert!(trace.revision < d.revision, "the separate save metadata change still invalidates the trace");
    assert_eq!(trace.arrows, arrows);
    std::fs::remove_dir_all(dir).unwrap();
}
