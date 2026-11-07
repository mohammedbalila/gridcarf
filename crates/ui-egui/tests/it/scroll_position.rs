//! The saved scroll position (`sheetView topLeftCell`) is restored on open and saved back (#378).

use gridcraft_engine::core::{CellRef, Value};
use gridcraft_engine::model::Workbook;
use gridcraft_engine::{DocState, Session};
use gridcraft_ui_egui::{SheetApp, grid::Geo};
use serde_json::json;

use crate::owned_directory::Directory;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness(freeze: Option<(u32, u32)>) -> Harness {
    let mut wb = Workbook::new();
    let sh = wb.sheet_mut(0).unwrap();
    for r in 0..60 {
        sh.set_value(CellRef::new(r, 0), Value::text(format!("row {}", r + 1)));
    }
    sh.view_top_left = CellRef::parse("A31").unwrap();
    sh.view_active = CellRef::parse("A31").unwrap();
    sh.freeze = freeze;
    let mut session = Session::new();
    session.add_document(DocState::new(wb, None, "Book1".into(), gridcraft_engine::Trust::Local));
    let mut h = crate::common::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        crate::common::app(session),
    );
    h.run_steps(4);
    h
}

/// The first row painted at the top of the scrolling area.
fn first_scrolled_row(h: &Harness) -> u32 {
    let app = h.state();
    let sheet = app.session.active().unwrap().wb.active().unwrap();
    let geo = Geo::new(sheet, app.grid.rect.unwrap(), app.view().scroll);
    let (fr, _) = sheet.freeze.unwrap_or((0, 0));
    let top = geo.cells.top() + sheet.row_top(fr) as f32 * geo.z;
    (fr..200).find(|r| geo.y(sheet, *r) <= top + 0.5 && geo.y(sheet, *r + 1) > top + 0.5).unwrap()
}

#[test]
fn saved_scroll_position_is_restored_and_saved_back() {
    for freeze in [None, Some((1, 0))] {
        let mut h = harness(freeze);
        assert_eq!(first_scrolled_row(&h), 30, "row 31 is the first visible row ({freeze:?})");
        // Scroll to row 51 and save: the file gets the new position, the document stays clean.
        {
            let app = h.state_mut();
            let sh = app.session.active().unwrap().wb.active().unwrap().clone();
            let fr = freeze.map_or(0, |f| f.0);
            app.view_mut().unwrap().scroll.y = (sh.row_top(50) - sh.row_top(fr)) as f32;
            app.run("file.saveBytes", json!({"format": "xlsx"})).unwrap();
        }
        let d = h.state().session.active().unwrap();
        assert_eq!(d.wb.active().unwrap().view_top_left, CellRef::parse("A51").unwrap());
        assert!(!d.is_dirty(), "scrolling and saving view state does not dirty the workbook");
    }
}

// These cases isolate index-changing commands from painting so each failure identifies
// the scroll owner or saved-sheet mapping, rather than a later layout side effect.
fn identity_app() -> SheetApp {
    let mut session = Session::new();
    session.new_workbook();
    let mut app = crate::common::app(session);
    app.ui.autosave = false;
    app.run("home.insertSheet", json!({"name":"Two"})).unwrap();
    for (sheet, row) in [("Sheet1", 50), ("Two", 10)] {
        app.run("sheet.activate", json!({"sheet":sheet})).unwrap();
        let y = app.session.doc().unwrap().wb.active().unwrap().row_top(row) as f32;
        app.view_mut().unwrap().scroll.y = y;
    }
    app
}

fn assert_scroll_row(app: &SheetApp, row: u32) {
    let sh = app.session.doc().unwrap().wb.active().unwrap();
    assert_eq!(app.view().scroll.y, sh.row_top(row) as f32, "scroll belongs to {}", sh.name);
}

#[test]
fn m03a_reordered_sheets_keep_scroll_and_save_to_their_actual_identity() {
    let mut app = identity_app();
    app.run("sheet.move", json!({"sheet":"Two", "to":0})).unwrap();
    app.store_view_state();
    let d = app.session.doc().unwrap();
    for (name, row) in [("Sheet1", 50), ("Two", 10)] {
        let sh = d.wb.sheets.iter().find(|sh| sh.name == name).unwrap();
        assert_eq!(sh.view_top_left, CellRef::new(row, 0), "saved mapping for {name}");
    }
    for (name, row) in [("Sheet1", 50), ("Two", 10)] {
        app.run("sheet.activate", json!({"sheet":name})).unwrap();
        assert_scroll_row(&app, row);
    }
}

#[test]
fn m03a_deleted_sheet_scroll_survives_undo_without_moving_to_its_neighbor() {
    let mut app = identity_app();
    let deleted_uid = app.session.doc().unwrap().wb.sheets.iter().find(|sh| sh.name == "Two").unwrap().uid;
    // Delete the first sheet so the surviving sheet actually changes index.
    app.run("sheet.move", json!({"sheet":"Two", "to":0})).unwrap();
    app.run("home.deleteSheet", json!({"sheet":"Two"})).unwrap();
    app.run("sheet.activate", json!({"sheet":"Sheet1"})).unwrap();
    assert_scroll_row(&app, 50);
    app.store_view_state();
    assert_eq!(app.session.doc().unwrap().wb.active().unwrap().view_top_left, CellRef::new(50, 0));
    app.run("edit.undo", json!({})).unwrap();
    app.run("sheet.activate", json!({"sheet":"Two"})).unwrap();
    assert_eq!(app.session.doc().unwrap().wb.active().unwrap().uid, deleted_uid);
    assert_scroll_row(&app, 10);
    app.store_view_state();
    for (name, row) in [("Sheet1", 50), ("Two", 10)] {
        let sh = app.session.doc().unwrap().wb.sheets.iter().find(|sh| sh.name == name).unwrap();
        assert_eq!(sh.view_top_left, CellRef::new(row, 0), "saved mapping after Undo for {name}");
    }
}

#[test]
fn m03a_inserted_sheet_does_not_inherit_neighbor_scroll() {
    let mut app = identity_app();
    app.run("home.insertSheet", json!({"name":"Fresh", "before":0})).unwrap();
    assert_scroll_row(&app, 0);
    app.run("sheet.activate", json!({"sheet":"Sheet1"})).unwrap();
    assert_scroll_row(&app, 50);
}

#[test]
fn m03a_active_cell_storage_and_empty_session_are_preserved() {
    let mut app = identity_app();
    app.run("selection.set", json!({"cell":"E51"})).unwrap();
    app.store_view_state();
    assert_eq!(app.session.doc().unwrap().wb.active().unwrap().view_active, CellRef::new(50, 4));
    let mut empty = crate::common::app(Session::new());
    assert!(empty.session.active_window().is_none());
    empty.store_view_state();
    assert!(empty.session.documents().is_empty());
}

#[test]
fn m03a_windows_on_one_document_keep_independent_scroll_and_release_closed_entries() {
    let mut app = identity_app();
    let first = app.session.active_window().unwrap();
    app.run("view.newWindow", json!({})).unwrap();
    let second = app.session.active_window().unwrap();
    let y = app.session.doc().unwrap().wb.active().unwrap().row_top(20) as f32;
    app.view_mut().unwrap().scroll.y = y;
    app.enter_window(first);
    assert_scroll_row(&app, 10);
    app.enter_window(second);
    assert_scroll_row(&app, 20);
    app.enter_window(first);
    app.run("window.close", json!({"window":second,"force":true})).unwrap();
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.session.active_window(), Some(first));
    assert_scroll_row(&app, 10);
    assert!(app.views.keys().all(|(window, _)| *window == first), "closed window's scroll entries remain");
}

fn has_sheet_uid(wb: &Workbook, uid: u64) -> bool {
    wb.sheets.iter().any(|sh| sh.uid.get() == uid)
}

fn logic_tick(app: &mut SheetApp) {
    let ctx = egui::Context::default();
    ctx.set_fonts(gridcraft_ui_egui::theme::pinned_font_definitions());
    gridcraft_ui_egui::theme::apply(&ctx, false);
    let _ = ctx.run_logic(&egui::RawInput::default(), |ctx| app.logic(ctx));
}

fn assert_saved_is_only_sheet_root(app: &SheetApp, doc: u64, deleted_uid: u64) {
    let d = app.session.doc().unwrap();
    assert_eq!(d.undo.len(), 100);
    assert!(!has_sheet_uid(&d.wb, deleted_uid));
    assert!(d.undo.iter().chain(&d.redo).all(|entry| !has_sheet_uid(&entry.wb, deleted_uid)));
    assert!(has_sheet_uid(&d.saved, deleted_uid), "saved is the final restorable root");
    assert!(!app.session.in_group(doc));
}

fn assert_saved_and_history_released_sheet(app: &SheetApp, doc: u64, deleted_uid: u64) {
    let d = app.session.doc().unwrap();
    assert!(!d.is_dirty());
    assert!(!has_sheet_uid(&d.wb, deleted_uid));
    assert!(!has_sheet_uid(&d.saved, deleted_uid));
    assert!(d.undo.iter().chain(&d.redo).all(|entry| !has_sheet_uid(&entry.wb, deleted_uid)));
    assert!(!app.session.in_group(doc));
}

#[test]
fn m03c2_deleted_sheet_scroll_survives_undo_and_expires_after_last_root() {
    let dir = Directory::acquire((0..).map(|id| std::env::temp_dir().join(format!("gridcraft-scroll-roots-{}-{id}", std::process::id())))).unwrap();
    let path = dir.join("views.json");
    let mut app = identity_app();
    let window = app.session.active_window().unwrap();
    let doc = app.session.doc().unwrap().uid;
    let deleted_uid = app.session.doc().unwrap().wb.sheets[0].uid.get();
    assert_eq!(app.session.doc().unwrap().wb.sheets[0].name, "Sheet1");
    let deleted_key = (window, deleted_uid);
    let deleted_scroll = app.views[&deleted_key].scroll;
    let saved = app.run("file.save", json!({"path":path})).unwrap();
    assert!(saved["bytes"].as_u64().unwrap() > 0);
    assert_eq!(std::fs::read(&path).unwrap().len() as u64, saved["bytes"].as_u64().unwrap());
    assert!(has_sheet_uid(&app.session.doc().unwrap().wb, deleted_uid));
    assert!(has_sheet_uid(&app.session.doc().unwrap().saved, deleted_uid));
    assert!(!app.session.doc().unwrap().is_dirty());
    assert!(!app.session.in_group(doc));

    app.run("home.deleteSheet", json!({"sheet":"Sheet1"})).unwrap();
    let d = app.session.doc().unwrap();
    assert!(!has_sheet_uid(&d.wb, deleted_uid));
    assert!(d.undo.iter().any(|entry| has_sheet_uid(&entry.wb, deleted_uid)));
    assert!(has_sheet_uid(&d.saved, deleted_uid));
    assert_eq!(d.wb.sheets[0].name, "Two");
    logic_tick(&mut app);
    assert_eq!(app.views[&deleted_key].scroll, deleted_scroll);
    assert_scroll_row(&app, 10);

    app.run("edit.undo", json!({})).unwrap();
    app.run("sheet.activate", json!({"sheet":"Sheet1"})).unwrap();
    assert_eq!(app.session.doc().unwrap().wb.active().unwrap().uid.get(), deleted_uid);
    assert_scroll_row(&app, 50);
    app.run("edit.redo", json!({})).unwrap();
    assert!(!has_sheet_uid(&app.session.doc().unwrap().wb, deleted_uid));
    assert!(has_sheet_uid(&app.session.doc().unwrap().saved, deleted_uid));
    logic_tick(&mut app);
    assert_eq!(app.views[&deleted_key].scroll, deleted_scroll);
    assert_scroll_row(&app, 10);

    for value in 0..101 {
        app.run("cell.set", json!({"cell":"A1", "input":value.to_string()})).unwrap();
    }
    assert_saved_is_only_sheet_root(&app, doc, deleted_uid);
    logic_tick(&mut app);
    assert_eq!(app.views[&deleted_key].scroll, deleted_scroll, "saved alone must preserve deleted-sheet scroll");

    // Store view metadata first, then bypass app.run cleanup so logic alone must evict.
    app.store_view_state();
    let before_save_revision = app.session.doc().unwrap().revision;
    let saved = app.session.run("file.save", json!({"path":path})).unwrap();
    assert!(saved["bytes"].as_u64().unwrap() > 0);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.len() as u64, saved["bytes"].as_u64().unwrap());
    let (written, _) = gridcraft_engine::io::open_bytes("views.json", &bytes).unwrap();
    assert_eq!(written.sheets.len(), 1);
    assert_eq!(written.sheets[0].name, "Two");
    let d = app.session.doc().unwrap();
    assert_eq!(d.revision, before_save_revision, "same-path save replaces saved without a revision change");
    assert_saved_and_history_released_sheet(&app, doc, deleted_uid);
    assert!(app.views.contains_key(&deleted_key), "no UI cleanup has run since saved released the UID");
    logic_tick(&mut app);
    assert_eq!(app.session.active_window(), Some(window));
    assert_eq!(app.session.doc().unwrap().uid, doc);
    assert_scroll_row(&app, 10);
    assert!(!app.views.contains_key(&deleted_key), "deleted-sheet scroll remains after every restorable root released its UID");
}

#[test]
fn m03c2_same_name_new_sheet_at_deleted_index_does_not_inherit_scroll() {
    let mut app = identity_app();
    let deleted_uid = app.session.doc().unwrap().wb.sheets[0].uid.get();
    app.run("home.deleteSheet", json!({"sheet":"Sheet1"})).unwrap();
    app.run("home.insertSheet", json!({"name":"Sheet1", "before":0})).unwrap();
    let d = app.session.doc().unwrap();
    assert_eq!(d.wb.active_sheet(), 0);
    assert_eq!(d.wb.sheets[0].name, "Sheet1");
    assert_ne!(d.wb.sheets[0].uid.get(), deleted_uid);
    assert_scroll_row(&app, 0);
    app.run("sheet.activate", json!({"sheet":"Two"})).unwrap();
    assert_scroll_row(&app, 10);
}

#[test]
fn m03c2_engine_close_releases_scroll_on_logic_without_app_cleanup() {
    let mut app = identity_app();
    app.run("selection.set", json!({"cell":"E11"})).unwrap();
    let sentinel = app.session.active_window().unwrap();
    let sentinel_doc = app.session.doc().unwrap().uid;
    let selection = app.session.doc().unwrap().selection.clone();
    let sentinel_views = app.views.clone();
    app.run("file.new", json!({})).unwrap();
    let target = app.session.active_window().unwrap();
    let target_doc = app.session.doc().unwrap().uid;
    app.view_mut().unwrap().scroll.y = 100.0;
    app.enter_window(sentinel);
    app.session.hide_window(target);
    let target_index = app.session.document_index(target_doc).unwrap();
    app.session.close_document(target_index);
    assert!(app.session.document_index(target_doc).is_none());
    assert!(app.session.window(target).is_none());
    assert!(app.views.keys().any(|(window, _)| *window == target), "engine close must precede any UI cleanup");
    logic_tick(&mut app);
    assert!(app.views.keys().all(|(window, _)| *window == sentinel));
    assert_eq!(app.views.len(), sentinel_views.len());
    for (key, view) in sentinel_views {
        assert_eq!(app.views[&key].scroll, view.scroll);
    }
    assert_eq!(app.session.active_window(), Some(sentinel));
    assert_eq!(app.ui_window(), Some(sentinel));
    assert_eq!(app.session.doc().unwrap().uid, sentinel_doc);
    assert_eq!(app.session.doc().unwrap().selection, selection);
    assert_scroll_row(&app, 10);
}
