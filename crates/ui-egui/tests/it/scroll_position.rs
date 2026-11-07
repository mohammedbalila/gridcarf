//! The saved scroll position (`sheetView topLeftCell`) is restored on open and saved back (#378).

use gridcraft_engine::core::{CellRef, Value};
use gridcraft_engine::model::Workbook;
use gridcraft_engine::{DocState, Session};
use gridcraft_ui_egui::{SheetApp, grid::Geo};
use serde_json::json;

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
