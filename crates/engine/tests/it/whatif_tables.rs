//! Synthetic what-if import through public commands; cached preservation only.
use gridcraft_engine::model::Workbook;
use gridcraft_engine::{DocState, Session, Trust, io};
use serde_json::json;
use std::io::{Cursor, Write};

fn sheet_fixture(body: &str) -> Vec<u8> {
    let bytes = io::save_bytes(&Workbook::new(), "intersection.xlsx").unwrap();
    let mut source = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..source.len() {
        let mut entry = source.by_index(i).unwrap();
        out.start_file(entry.name(), zip::write::SimpleFileOptions::default()).unwrap();
        if entry.name() == "xl/worksheets/sheet1.xml" {
            write!(out, r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{body}</worksheet>"#).unwrap();
        } else {
            std::io::copy(&mut entry, &mut out).unwrap();
        }
    }
    out.finish().unwrap().into_inner()
}

fn open() -> Session {
    let bytes = sheet_fixture(
        r#"<sheetData><row r="1"><c r="A1"><v>1</v></c><c r="C1"><f>A1*2</f><v>2</v></c></row><row r="2"><c r="B2"><v>3</v></c><c r="C2"><f t="dataTable" ref="C2:C3" r1="A1"/><v>6</v></c></row><row r="3"><c r="B3"><v>4</v></c><c r="C3"><v>8</v></c></row></sheetData>"#,
    );
    let (wb, _) = io::open_bytes("whatif.xlsx", &bytes).unwrap();
    let mut s = Session::new();
    s.add_document(DocState::new(wb, None, "whatif".into(), Trust::FromFile));
    s
}

#[test]
fn unsupported_edit_refused() {
    for (command, params) in [
        ("cell.set", json!({"cell":"C2", "input":"99"})),
        ("cell.set", json!({"cell":"C3", "input":"99"})),
        ("cell.set", json!({"cell":"C1", "input":"=A1*3"})),
        ("cell.set", json!({"cell":"B2", "input":"99"})),
        ("cell.set", json!({"cell":"A1", "input":"99"})),
        ("edit.clearContents", json!({"range":"C2:C3"})),
        ("range.setValues", json!({"range":"C2", "values":[[99],[100]]})),
        ("home.deleteRows", json!({"rows":"2"})),
        ("home.insertRows", json!({"rows":"2"})),
        ("home.deleteColumns", json!({"cols":"B"})),
        ("home.insertCells", json!({"range":"B2", "shift":"right"})),
        ("range.move", json!({"range":"C2:C3", "to":"F5"})),
    ] {
        let mut s = open();
        let before = s.doc().unwrap().wb.clone();
        let undo_before = s.doc().unwrap().undo.len();
        let error = s.execute(command, params).expect_err(command);
        assert!(error.to_string().contains("what-if"), "{command}: {error}");
        assert_eq!(&*s.doc().unwrap().wb, &*before, "refusal must be atomic: {command}");
        assert_eq!(s.doc().unwrap().undo.len(), undo_before, "refusal must not add history");
    }
}

#[test]
fn unrelated_edit_keeps_definition() {
    let mut s = open();
    let definitions = s.doc().unwrap().wb.sheet(0).unwrap().what_if_tables.clone();
    assert_eq!(definitions.len(), 1);
    for (command, params) in [
        ("cell.set", json!({"cell":"Z20", "input":"99"})),
        ("home.bold", json!({"range":"C2:C3"})),
        ("home.insertRows", json!({"rows":"50"})),
        ("sheet.rename", json!({"name":"Renamed"})),
    ] {
        s.execute(command, params).unwrap();
        assert_eq!(s.doc().unwrap().wb.sheet(0).unwrap().what_if_tables, definitions);
    }
    let bytes = io::save_bytes(&s.doc().unwrap().wb, "whatif.xlsx").unwrap();
    let (back, report) = io::open_bytes_with_report("whatif.xlsx", &bytes, None, false).unwrap();
    assert_eq!(back.sheet(0).unwrap().what_if_tables, definitions);
    assert_eq!(back.sheet(0).unwrap().value(gridcraft_core::CellRef::new(1, 2)), gridcraft_core::Value::Number(6.0));
    assert!(report.issues.iter().any(|i| i.feature == "what-if data table" && i.effect == gridcraft_model::CompatibilityEffect::Static));
}

fn native_session(range: &str, anchor: &str, attributes: &str) -> Session {
    let bytes =
        sheet_fixture(&format!(r#"<sheetData><row><c r="{anchor}"><f t="dataTable" ref="{range}" {attributes}/><v>12</v></c></row></sheetData>"#));
    let (wb, _) = io::open_bytes("whatif.xlsx", &bytes).unwrap();
    let mut s = Session::new();
    s.add_document(DocState::new(wb, None, "whatif".into(), Trust::FromFile));
    s
}

#[test]
fn row_column_two_variable_and_edge_footprints() {
    // The native ref excludes the headers. One-variable row and column tables both need
    // the top and left edges, while only two-variable tables use the top-left corner.
    for attrs in [r#"r1="J10""#, r#"r1="J10" dtr="1""#, r#"r1="J10" r2="K10" dt2D="1""#] {
        for cell in ["C2", "D3", "C1", "D1", "B2", "B3", "J10"] {
            let mut s = native_session("C2:D3", "C2", attrs);
            assert!(s.execute("cell.set", json!({"cell":cell,"input":"99"})).unwrap_err().to_string().contains("what-if"), "{attrs}: {cell}");
        }
        let mut s = native_session("C2:D3", "C2", attrs);
        let result = s.execute("cell.set", json!({"cell":"B1","input":"99"}));
        assert_eq!(result.is_err(), attrs.contains("dt2D"));
        if attrs.contains("dt2D") {
            assert!(s.execute("cell.set", json!({"cell":"K10","input":"99"})).is_err());
        }
        s.execute("cell.set", json!({"cell":"E2","input":"99"})).unwrap();
    }
    // Edge imports are preserved as static native records; no nonexistent header wraps.
    for (range, anchor, blocked, allowed) in [("A1:B2", "A1", "B2", "C1"), ("A2:B3", "A2", "B1", "C2"), ("B1:C2", "B1", "A2", "D1")] {
        let mut s = native_session(range, anchor, r#"r1="J10" dt2D="1""#);
        assert!(s.execute("cell.set", json!({"cell":blocked,"input":"99"})).is_err());
        s.execute("cell.set", json!({"cell":allowed,"input":"99"})).unwrap();
    }
}

#[test]
fn whole_sheet_copy_reorder_and_delete_keep_definition_ownership() {
    let mut s = open();
    s.execute("sheet.move", json!({"copy":true,"to":1})).unwrap();
    let wb = &s.doc().unwrap().wb;
    assert_eq!(wb.sheets.len(), 2);
    assert_eq!(wb.sheet(0).unwrap().what_if_tables, wb.sheet(1).unwrap().what_if_tables);
    assert_ne!(wb.sheet(0).unwrap().uid.get(), wb.sheet(1).unwrap().uid.get());
    s.execute("sheet.move", json!({"to":0})).unwrap();
    s.execute("home.deleteSheet", json!({"sheet":0})).unwrap();
    assert_eq!(s.doc().unwrap().wb.sheet(0).unwrap().what_if_tables.len(), 1);
}

#[test]
fn recalculation_does_not_spill_into_empty_cached_table_cells() {
    let bytes = sheet_fixture(r#"<sheetData><row r="4"><c r="C4"><f t="dataTable" ref="C4:D5" r1="J10"/><v>12</v></c></row></sheetData>"#);
    let (wb, _) = io::open_bytes("whatif.xlsx", &bytes).unwrap();
    let mut s = Session::new();
    s.add_document(DocState::new(wb, None, "whatif".into(), Trust::FromFile));
    s.execute("cell.set", json!({"cell":"A1", "input":"=D1*2"})).unwrap();
    s.execute("cell.set", json!({"cell":"A2", "input":"=SUM(D1:D5)"})).unwrap();
    s.execute("cell.set", json!({"cell":"D1", "input":"=SEQUENCE(5)"})).unwrap();
    let sh = s.doc().unwrap().wb.sheet(0).unwrap();
    assert_eq!(sh.value(gridcraft_core::CellRef::parse("D1").unwrap()), gridcraft_core::Value::Error(gridcraft_core::CellError::Spill));
    assert_eq!(sh.value(gridcraft_core::CellRef::parse("D5").unwrap()), gridcraft_core::Value::Empty);
    for at in ["A1", "A2"] {
        assert_eq!(sh.value(gridcraft_core::CellRef::parse(at).unwrap()), gridcraft_core::Value::Error(gridcraft_core::CellError::Spill));
    }
    s.execute("cell.set", json!({"cell":"F1", "input":"=SEQUENCE(5)"})).unwrap();
    assert_eq!(s.doc().unwrap().wb.sheet(0).unwrap().value(gridcraft_core::CellRef::parse("F5").unwrap()), gridcraft_core::Value::Number(5.0));
    let mut wb = (*s.doc().unwrap().wb).clone();
    gridcraft_calc::Calc::new().recalc_all(&mut wb);
    assert_eq!(wb.sheet(0).unwrap().value(gridcraft_core::CellRef::parse("D5").unwrap()), gridcraft_core::Value::Empty);
}

#[test]
fn renaming_a_sheet_can_rewrite_formula_headers_without_invalidating_tables() {
    let bytes = sheet_fixture(
        r#"<sheetData><row r="1"><c r="A1"><v>1</v></c><c r="C1"><f>Sheet1!A1*2</f><v>2</v></c></row><row r="2"><c r="C2"><f t="dataTable" ref="C2:C3" r1="A1"/><v>6</v></c></row></sheetData>"#,
    );
    let (wb, _) = io::open_bytes("whatif.xlsx", &bytes).unwrap();
    let mut s = Session::new();
    s.add_document(DocState::new(wb, None, "whatif".into(), Trust::FromFile));
    s.execute("sheet.rename", json!({"name":"Renamed"})).unwrap();
    assert_eq!(s.doc().unwrap().wb.sheet(0).unwrap().what_if_tables.len(), 1);
}

#[test]
fn json_roundtrip_and_older_documents_keep_compatible_defaults() {
    let s = open();
    let wb = &s.doc().unwrap().wb;
    let encoded = serde_json::to_value(wb).unwrap();
    let back: Workbook = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(back.sheet(0).unwrap().what_if_tables, wb.sheet(0).unwrap().what_if_tables);
    let mut older_table = encoded.clone();
    older_table["sheets"][0]["what_if_tables"][0].as_object_mut().unwrap().remove("formula_text");
    let back: Workbook = serde_json::from_value(older_table).unwrap();
    assert!(back.sheet(0).unwrap().what_if_tables[0].formula_text.is_empty());
    let mut old = encoded;
    old["sheets"][0].as_object_mut().unwrap().remove("what_if_tables");
    let back: Workbook = serde_json::from_value(old).unwrap();
    assert!(back.sheet(0).unwrap().what_if_tables.is_empty());
}

#[test]
fn outside_precedent_changes_recalculate_headers_but_leave_table_caches_static() {
    let bytes = sheet_fixture(
        r#"<sheetData><row r="1"><c r="A1"><v>1</v></c><c r="C1"><f>A1+Z1</f><v>6</v></c><c r="Z1"><v>5</v></c></row><row r="2"><c r="C2"><f t="dataTable" ref="C2:C3" r1="A1"/><v>8</v></c></row><row r="3"><c r="C3"><v>9</v></c></row></sheetData>"#,
    );
    let (wb, report) = io::open_bytes_with_report("whatif.xlsx", &bytes, None, false).unwrap();
    assert!(report.issues.iter().any(|i| i.feature == "what-if data table" && i.description.contains("outside precedent edits")));
    let mut s = Session::new();
    s.add_document(DocState::new(wb, None, "whatif".into(), Trust::FromFile));
    s.execute("cell.set", json!({"cell":"Z1", "input":"20"})).unwrap();
    let sh = s.doc().unwrap().wb.sheet(0).unwrap();
    for (at, expected) in [("C1", 21.0), ("C2", 8.0), ("C3", 9.0)] {
        assert_eq!(sh.value(gridcraft_core::CellRef::parse(at).unwrap()), gridcraft_core::Value::Number(expected));
    }
}

#[test]
fn retyping_a_header_cannot_change_its_evaluation_mode() {
    let mut s = open();
    let before = s.doc().unwrap().wb.clone();
    // The same text is legacy on import, but a freshly entered formula uses array evaluation.
    assert!(s.execute("cell.set", json!({"cell":"C1", "input":"=A1*2"})).is_err());
    assert_eq!(&*s.doc().unwrap().wb, &*before);
}

#[test]
fn unrelated_structure_can_rewrite_an_outside_header_precedent() {
    let bytes = sheet_fixture(
        r#"<sheetData><row r="1"><c r="A1"><v>1</v></c><c r="C1"><f>A1+Z100</f><v>6</v></c></row><row r="2"><c r="C2"><f t="dataTable" ref="C2:C3" r1="A1"/><v>8</v></c></row><row r="100"><c r="Z100"><v>5</v></c></row></sheetData>"#,
    );
    let (wb, _) = io::open_bytes("whatif.xlsx", &bytes).unwrap();
    let definitions = wb.sheet(0).unwrap().what_if_tables.clone();
    let mut s = Session::new();
    s.add_document(DocState::new(wb, None, "whatif".into(), Trust::FromFile));
    for command in ["home.insertRows", "home.deleteRows"] {
        s.execute(command, json!({"rows":"50"})).unwrap();
        let sh = s.doc().unwrap().wb.sheet(0).unwrap();
        assert_eq!(sh.what_if_tables, definitions);
        assert_eq!(sh.value(gridcraft_core::CellRef::parse("C1").unwrap()), gridcraft_core::Value::Number(6.0));
        assert_eq!(sh.value(gridcraft_core::CellRef::parse("C2").unwrap()), gridcraft_core::Value::Number(8.0));
    }
}

#[test]
fn moving_a_blank_cached_result_is_refused_before_publishing() {
    let mut s = native_session("C2:D3", "C2", r#"r1="J10""#);
    let before = s.doc().unwrap().wb.clone();
    let error = s.execute("range.move", json!({"range":"D3", "to":"Z20"})).unwrap_err();
    assert!(error.to_string().contains("what-if"));
    assert_eq!(&*s.doc().unwrap().wb, &*before);
    assert!(s.doc().unwrap().undo.is_empty());
    s.execute("range.move", json!({"range":"Z20", "to":"Z21"})).unwrap();
    assert_eq!(s.doc().unwrap().wb.sheet(0).unwrap().what_if_tables, before.sheet(0).unwrap().what_if_tables);
}

#[test]
fn cross_sheet_move_cannot_overwrite_blank_native_results() {
    let mut s = native_session("C2:D3", "C2", r#"r1="J10""#);
    s.execute("home.insertSheet", json!({"name":"Source"})).unwrap();
    s.execute("edit.cut", json!({"range":"A1"})).unwrap();
    let before = s.doc().unwrap().wb.clone();
    let error = s.execute("edit.pasteSpecial", json!({"sheet":"Sheet1", "at":"D3", "what":"all"})).unwrap_err();
    assert!(error.to_string().contains("what-if"));
    assert_eq!(&*s.doc().unwrap().wb, &*before);
}

#[test]
fn native_text_references_cannot_be_left_stale_by_address_changes() {
    for text in ["TABLE(Z100,)", "TABLE(Z100,)&amp;&lt;☃"] {
        let bytes = sheet_fixture(&format!(
            r#"<sheetData><row r="2"><c r="C2"><f t="dataTable" ref="C2:D3" r1="J10">{text}</f><v>12</v></c></row></sheetData>"#
        ));
        let (wb, _) = io::open_bytes("whatif.xlsx", &bytes).unwrap();
        let mut s = Session::new();
        s.add_document(DocState::new(wb, None, "whatif".into(), Trust::FromFile));
        let before = s.doc().unwrap().wb.clone();
        assert!(s.execute("home.insertRows", json!({"rows":"50"})).unwrap_err().to_string().contains("what-if"));
        assert_eq!(&*s.doc().unwrap().wb, &*before);
        s.execute("cell.set", json!({"cell":"Z20", "input":"3"})).unwrap();
        if text == "TABLE(Z100,)" {
            // Pure reference adjustment proves a later row insertion leaves this text intact.
            s.execute("home.insertRows", json!({"rows":"150"})).unwrap();
        }
    }
}

#[test]
fn native_text_qualified_references_are_not_silently_renamed() {
    let bytes = sheet_fixture(
        r#"<sheetData><row r="2"><c r="C2"><f t="dataTable" ref="C2:D3" r1="J10">TABLE(Sheet1!Z100,)</f><v>12</v></c></row></sheetData>"#,
    );
    let (wb, _) = io::open_bytes("whatif.xlsx", &bytes).unwrap();
    let mut s = Session::new();
    s.add_document(DocState::new(wb, None, "whatif".into(), Trust::FromFile));
    assert!(s.execute("sheet.rename", json!({"name":"Renamed"})).unwrap_err().to_string().contains("what-if"));
    s.execute("home.insertSheet", json!({"name":"Unrelated"})).unwrap();
    s.execute("sheet.rename", json!({"name":"Other"})).unwrap();
}

#[test]
fn copying_native_text_requires_unchanged_owner_references() {
    for (text, allowed) in [("", true), (" \n ", true), ("TABLE(J10,)", true), ("TABLE('Sheet1'!J10,)", false), ("TABLE(J10,)&amp;&lt;☃", false)] {
        let bytes = sheet_fixture(&format!(
            r#"<sheetData><row r="2"><c r="C2"><f t="dataTable" ref="C2:D3" r1="J10">{text}</f><v>12</v></c></row></sheetData>"#
        ));
        let (wb, _) = io::open_bytes("whatif.xlsx", &bytes).unwrap();
        let mut s = Session::new();
        s.add_document(DocState::new(wb, None, "whatif".into(), Trust::FromFile));
        if !allowed {
            s.execute("cell.set", json!({"cell":"Z20", "input":"3"})).unwrap();
            s.execute("edit.undo", json!({})).unwrap();
        }
        let before = s.doc().unwrap().wb.clone();
        let before_selection = s.doc().unwrap().selection.clone();
        let history = (s.doc().unwrap().undo.len(), s.doc().unwrap().redo.len());
        let dirty = s.doc().unwrap().is_dirty();
        let result = s.execute("sheet.move", json!({"copy":true,"to":1}));
        if allowed {
            result.unwrap();
            let wb = &s.doc().unwrap().wb;
            assert_eq!(wb.sheets.len(), 2);
            assert_eq!(wb.sheet(0).unwrap().what_if_tables, wb.sheet(1).unwrap().what_if_tables);
            assert_ne!(wb.sheet(0).unwrap().uid.get(), wb.sheet(1).unwrap().uid.get());
            s.execute("home.insertRows", json!({"rows":"150"})).unwrap();
        } else {
            assert!(result.unwrap_err().to_string().contains("what-if"));
            assert_eq!(&*s.doc().unwrap().wb, &*before);
            assert_eq!(s.doc().unwrap().selection, before_selection);
            assert_eq!((s.doc().unwrap().undo.len(), s.doc().unwrap().redo.len()), history);
            assert_eq!(s.doc().unwrap().is_dirty(), dirty);
        }
    }
}
