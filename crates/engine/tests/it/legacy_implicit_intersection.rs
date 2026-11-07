//! Legacy XLSX formulas intersect at the formula cell; modern and CSE formulas retain arrays.

use std::io::{Cursor, Read, Write};

use gridcraft_engine::core::{CellError, CellRef, Value};
use gridcraft_engine::model::{Cell, Formula, FormulaEvalMode, Workbook};
use gridcraft_engine::{DocState, Session, Trust, io};
use serde_json::json;

fn at(cell: &str) -> CellRef {
    CellRef::parse(cell).unwrap()
}

fn open(bytes: &[u8]) -> Session {
    let (wb, _) = io::open_bytes("intersection.xlsx", bytes).unwrap();
    let mut session = Session::new();
    session.add_document(DocState::new(wb, None, "intersection".into(), Trust::FromFile));
    session
}

/// Build the package in code, replacing only the sheet with deliberately ordinary/CSE/dynamic XML.
fn fixture(formula: &str, cell_attrs: &str, formula_attrs: &str) -> Vec<u8> {
    sheet_fixture(&format!(
        r#"<sheetData><row r="1"><c r="A1"><v>1</v></c></row><row r="2"><c r="A2"><v>4</v></c><c r="B2" {cell_attrs}><f {formula_attrs}>{formula}</f><v>2</v></c></row><row r="3"><c r="A3"><v>9</v></c></row></sheetData>"#
    ))
}

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

fn value(session: &Session, cell: &str) -> Value {
    session.doc().unwrap().wb.sheet(0).unwrap().value(at(cell))
}

fn assert_scalar(session: &Session, expected: Value) {
    assert_eq!(value(session, "B2"), expected);
    assert_eq!(value(session, "B3"), Value::Empty);
    assert_eq!(value(session, "B4"), Value::Empty);
}

#[test]
fn legacy_scalar_sqrt_uses_formula_row() {
    assert_scalar(&open(&fixture("SQRT(A1:A3)", "", "")), Value::Number(2.0));
}

#[test]
fn modern_sqrt_still_spills() {
    let session = open(&fixture("SQRT(A1:A3)", r#"cm="1""#, r#"t="array" ref="B2:B4""#));
    for (cell, expected) in [("B2", 1.0), ("B3", 2.0), ("B4", 3.0)] {
        assert_eq!(value(&session, cell), Value::Number(expected));
    }
}

#[test]
fn legacy_formula_roundtrip_keeps_mode() {
    let session = open(&fixture("SQRT(A1:A3)", "", ""));
    let bytes = io::save_bytes(&session.doc().unwrap().wb, "intersection.xlsx").unwrap();
    let mut zip = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let mut sheet = String::new();
    zip.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_string(&mut sheet).unwrap();
    assert!(!sheet.contains("cm="), "{sheet}");
    assert!(!sheet.contains("t=\"array\""), "{sheet}");
    assert_scalar(&open(&bytes), Value::Number(2.0));
    let mut session = session;
    session.execute("cell.set", json!({"cell": "B2", "input": "=SQRT(A1:A3)"})).unwrap();
    assert_eq!(value(&session, "B2"), Value::Number(1.0));
    assert_eq!(value(&session, "B4"), Value::Number(3.0));
}

#[test]
fn legacy_scalar_and_range_contexts_remain_distinct() {
    for (formula, expected) in [
        ("A1:A3", 4.0),
        ("A1:A3*2", 8.0),
        ("-A1:A3", -4.0),
        ("SUM(A1:A3)", 14.0),
        ("SUMPRODUCT(A1:A3*A1:A3)", 98.0),
        ("SUMPRODUCT(SQRT(A1:A3))", 6.0),
        ("SUMIFS(A1:A3,A1:A3,A1:A3)", 4.0),
        ("IF(ISBLANK(A1:A3),0,SQRT(A1:A3))", 2.0),
    ] {
        let session = open(&fixture(formula, "", ""));
        assert_eq!(value(&session, "B2"), Value::Number(expected), "{formula}");
        assert_eq!(value(&session, "B3"), Value::Empty, "{formula}");
    }
}

#[test]
fn cse_sqrt_retains_array_evaluation() {
    let session = open(&fixture("SQRT(A1:A3)", "", r#"t="array" ref="B2:B4""#));
    for (cell, expected) in [("B2", 1.0), ("B3", 2.0), ("B4", 3.0)] {
        assert_eq!(value(&session, cell), Value::Number(expected));
    }
}

#[test]
fn shared_legacy_formulas_intersect_at_each_formula_row() {
    let bytes = sheet_fixture(
        r#"<sheetData><row r="1"><c r="A1"><v>1</v></c></row><row r="2"><c r="A2"><v>4</v></c><c r="B2"><f t="shared" si="0" ref="B2:B3">SQRT($A$1:$A$3)</f></c></row><row r="3"><c r="A3"><v>9</v></c><c r="B3"><f t="shared" si="0"/></c></row></sheetData>"#,
    );
    let session = open(&bytes);
    assert_eq!(value(&session, "B2"), Value::Number(2.0));
    assert_eq!(value(&session, "B3"), Value::Number(3.0));
    assert_eq!(value(&session, "B4"), Value::Empty);
}

#[test]
fn json_defaults_preserve_array_behavior_and_legacy_mode_roundtrips() {
    let old: Formula = serde_json::from_value(json!({"text": "SQRT(A1:A3)", "array": null})).unwrap();
    assert_eq!(old.eval_mode, FormulaEvalMode::Array);
    assert_eq!(Formula::new("SQRT(A1:A3)").eval_mode, FormulaEvalMode::Array);
    let session = open(&fixture("SQRT(A1:A3)", "", ""));
    let bytes = io::save_bytes(&session.doc().unwrap().wb, "intersection.json").unwrap();
    let (mut wb, _) = io::open_bytes("intersection.json", &bytes).unwrap();
    assert_eq!(wb.sheet(0).unwrap().cell(at("B2")).unwrap().formula.as_ref().unwrap().eval_mode, FormulaEvalMode::Legacy);
    let mut restored = Session::new();
    restored.add_document(DocState::new(wb.clone(), None, "intersection".into(), Trust::FromFile));
    assert_scalar(&restored, Value::Number(2.0));
    wb.sheet_mut(0).unwrap().cells.set(at("B2"), Cell::formula(old));
    let mut restored = Session::new();
    restored.add_document(DocState::new(wb, None, "old json".into(), Trust::FromFile));
    assert_eq!(value(&restored, "B4"), Value::Number(3.0));
}

#[test]
fn legacy_mode_survives_edits_and_dirty_recalculation() {
    let mut session = open(&fixture("SQRT($A$1:$A$3)", "", ""));
    session.execute("edit.fillDown", json!({"range": "B2:B3"})).unwrap();
    assert_eq!(value(&session, "B3"), Value::Number(3.0));
    session.execute("cell.set", json!({"cell": "A2", "input": "16"})).unwrap();
    assert_eq!(value(&session, "B2"), Value::Number(4.0));
    session.execute("home.insertRows", json!({"rows": "1:1"})).unwrap();
    assert_eq!(value(&session, "B3"), Value::Number(4.0));
    assert_eq!(value(&session, "B4"), Value::Number(3.0));
    session.execute("data.sortAscending", json!({"range": "A3:B4", "column": "A"})).unwrap();
    assert_eq!(value(&session, "B3"), Value::Number(3.0));
    assert_eq!(value(&session, "B4"), Value::Number(4.0));
    session.execute("selection.set", json!({"range": "B3"})).unwrap();
    session.execute("edit.copy", json!({})).unwrap();
    session.execute("selection.set", json!({"range": "C3"})).unwrap();
    session.execute("edit.paste", json!({})).unwrap();
    assert_eq!(value(&session, "C3"), Value::Number(3.0));
}

#[test]
fn legacy_reference_and_nested_array_contexts() {
    for (formula, expected) in [
        ("ROW(A1:A3)", Value::Number(1.0)),
        ("INDEX(A1:A3*2,2)", Value::Number(8.0)),
        ("SUM(INDEX(A1:A3,0))", Value::Number(14.0)),
        ("SUM(A1:A3*2)", Value::Number(8.0)),
        ("SQRT(A1:C3)", Value::Error(CellError::Value)),
        ("SQRT(A3:A4)", Value::Error(CellError::Value)),
        ("EOMONTH(A1:A3,0)", Value::Error(CellError::Value)),
        ("SUMPRODUCT(IFERROR(A1:A3/0,2))", Value::Number(6.0)),
        ("SUM(MAP(A1:A3,LAMBDA(x,SQRT(x))))", Value::Number(6.0)),
        ("SQRT(@A1:A3)", Value::Number(2.0)),
    ] {
        let session = open(&fixture(formula, "", ""));
        assert_eq!(value(&session, "B2"), expected, "{formula}");
        assert_eq!(value(&session, "B3"), Value::Empty, "{formula}");
    }
}

#[test]
fn paste_arithmetic_preserves_legacy_mode() {
    let mut session = open(&fixture("SQRT(A1:A3)", "", ""));
    session.execute("cell.set", json!({"cell": "D1", "input": "2"})).unwrap();
    session.execute("selection.set", json!({"range": "D1"})).unwrap();
    session.execute("edit.copy", json!({})).unwrap();
    session.execute("selection.set", json!({"range": "B2"})).unwrap();
    session.execute("edit.pasteSpecial", json!({"operation": "add"})).unwrap();
    assert_scalar(&session, Value::Number(4.0));
}
