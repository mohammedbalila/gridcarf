use gridcraft_core::{CellRef, MAX_ROWS};
use gridcraft_engine::Session;
use serde_json::json;

use super::merged_cell_shift::assert_refused_unchanged;

fn refused(range: &str) {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("insert.table", json!({"range": range, "header": true})).unwrap();
    let before = s.doc().unwrap().wb.active().unwrap().tables[0].clone();
    assert_refused_unchanged(&mut s, "table.totalRow", json!({"on": true, "table": &before.name}));
    let sh = s.doc().unwrap().wb.active().unwrap();
    assert_eq!(sh.tables[0].range, before.range, "{range}");
    assert_eq!(sh.tables[0].range.end.row, MAX_ROWS - 1, "{range}");
    assert!(!sh.tables[0].totals_row, "{range}");
    assert!(sh.value(CellRef::new(MAX_ROWS, 0)).is_empty(), "{range}");
}

#[test]
fn total_row_is_refused_on_a_table_ending_at_the_last_worksheet_row() {
    refused("A1048575:B1048576");
    refused("A1048576:B1048576");
}

#[test]
fn total_row_still_grows_an_ordinary_table() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "A1", "values": [["Item", "Amount"], ["a", 2], ["b", 3]]})).unwrap();
    s.execute("insert.table", json!({"range": "A1:B3", "header": true})).unwrap();
    s.execute("table.totalRow", json!({"on": true, "table": "Table1"})).unwrap();
    let t = &s.doc().unwrap().wb.active().unwrap().tables[0];
    assert!(t.totals_row);
    assert_eq!(t.range.end.row, 3);
}

fn ordinary_table() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range":"A1", "values":[["Item","Middle","Amount"],["a",2,3],["b",4,5]]})).unwrap();
    s.execute("insert.table", json!({"range":"A1:C3", "header":true,"name":"T"})).unwrap();
    s
}

#[test]
fn exact_seed191_refuses_totals_over_adjacent_table() {
    let mut s = Session::new();
    s.execute("file.new", json!({"sample":"grades"})).unwrap();
    s.execute("home.deleteRows", json!({"rows":"5:5"})).unwrap();
    s.execute("home.deleteRows", json!({"rows":"6:8"})).unwrap();
    s.execute("insert.table", json!({"header":true,"range":"A6:B8"})).unwrap();
    let headers = [
        s.doc().unwrap().wb.active().unwrap().value(CellRef::parse("A6").unwrap()),
        s.doc().unwrap().wb.active().unwrap().value(CellRef::parse("B6").unwrap()),
    ];
    assert_refused_unchanged(&mut s, "table.totalRow", json!({"on":true,"table":"Grades"}));
    let sh = s.doc().unwrap().wb.active().unwrap();
    assert_eq!(sh.tables.iter().map(|t| t.range.a1()).collect::<Vec<_>>(), ["A1:G5", "A6:B8"]);
    assert_eq!([sh.value(CellRef::parse("A6").unwrap()), sh.value(CellRef::parse("B6").unwrap())], headers);
}

#[test]
fn totals_refuse_adjacent_table_with_explicit_or_toggle_enable() {
    for params in [json!({"table":"T","on":true}), json!({"table":"T"})] {
        let mut s = ordinary_table();
        s.execute("range.setValues", json!({"range":"B4", "values":[["Other","Header"],["kept",9]]})).unwrap();
        s.execute("insert.table", json!({"range":"B4:C5","header":true,"name":"U"})).unwrap();
        assert_refused_unchanged(&mut s, "table.totalRow", params);
        let sh = s.doc().unwrap().wb.active().unwrap();
        assert_eq!(sh.tables[1].columns[0].name, "Other");
        assert_eq!(sh.tables[1].columns[1].name, "Header");
        assert_eq!(sh.value(CellRef::parse("B4").unwrap()).display(), "Other");
    }
}

fn occupied_destination(cell: &str, input: &str) {
    let mut s = ordinary_table();
    s.execute("cell.set", json!({"cell":cell,"input":input})).unwrap();
    assert_refused_unchanged(&mut s, "table.totalRow", json!({"table":"T","on":true}));
    let sh = s.doc().unwrap().wb.active().unwrap();
    assert_eq!(sh.cell(CellRef::parse(cell).unwrap()).unwrap().bar_text(), input);
    assert!(!sh.tables[0].totals_row);
    assert_eq!(sh.tables[0].range.a1(), "A1:C3");
}

#[test]
fn totals_refuse_ordinary_middle_value() {
    occupied_destination("B4", "keep middle");
}
#[test]
fn totals_refuse_ordinary_formula() {
    occupied_destination("C4", "=40+2");
}

fn derived_destination(array: bool) {
    let mut s = ordinary_table();
    s.execute("range.setValues", json!({"range":"E1","values":[["X","Y"],[1,2],[3,4]]})).unwrap();
    s.execute("insert.table", json!({"range":"E1:F3","header":true,"name":"U"})).unwrap();
    s.execute("selection.set", json!({"range":"D4:G4"})).unwrap();
    // The formula anchor is outside the table columns: only derived results intersect.
    s.execute("cell.set", json!({"cell":"D4","input":"=SEQUENCE(1,4)","array":array})).unwrap();
    let destination = CellRef::parse("E4").unwrap();
    assert!(s.doc().unwrap().wb.active().unwrap().cell(destination).is_none());
    assert_eq!(s.doc().unwrap().wb.active().unwrap().value(destination).display(), "2");
    assert_refused_unchanged(&mut s, "table.totalRow", json!({"table":"U","on":true}));
}

#[test]
fn totals_refuse_dynamic_spill_children() {
    derived_destination(false);
}
#[test]
fn totals_refuse_cse_array_children() {
    derived_destination(true);
}

#[test]
fn totals_refuse_blank_displayed_spill_children() {
    let mut s = ordinary_table();
    s.execute("range.setValues", json!({"range":"E1","values":[["X","Y"],[1,2],[3,4]]})).unwrap();
    s.execute("insert.table", json!({"range":"E1:F3","header":true,"name":"U"})).unwrap();
    s.execute("cell.set", json!({"cell":"D4","input":"=IF(SEQUENCE(1,4)>0,\"\",\"\")"})).unwrap();
    let sh = s.doc().unwrap().wb.active().unwrap();
    let child = CellRef::parse("E4").unwrap();
    assert!(sh.cell(child).is_none());
    assert_eq!(sh.value(child).display(), "");
    assert!(sh.spill.contains_key(&child));
    assert_refused_unchanged(&mut s, "table.totalRow", json!({"table":"U","on":true}));
}

#[test]
fn totals_refuse_cse_range_without_spill_cache() {
    let mut s = ordinary_table();
    s.execute("range.setValues", json!({"range":"E1","values":[["X","Y"],[1,2],[3,4]]})).unwrap();
    s.execute("insert.table", json!({"range":"E1:F3","header":true,"name":"U"})).unwrap();
    s.execute("selection.set", json!({"range":"D4:G4"})).unwrap();
    s.execute("cell.set", json!({"cell":"D4","input":"=SEQUENCE(1,4)","array":true})).unwrap();
    // Derived caches are not the owner of an imported CSE range. Build a synthetic
    // uncached fixture to exercise the stored array metadata independently.
    let sh = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().wb).sheet_mut(0).unwrap();
    sh.spill.clear();
    sh.spill_ranges.clear();
    let sh = s.doc().unwrap().wb.active().unwrap();
    assert!(sh.spill.is_empty());
    assert!(sh.spill_ranges.is_empty());
    assert!(sh.cell(CellRef::parse("E4").unwrap()).is_none());
    assert_eq!(sh.cell(CellRef::parse("D4").unwrap()).unwrap().formula.as_ref().unwrap().array.unwrap().a1(), "D4:G4");
    assert_refused_unchanged(&mut s, "table.totalRow", json!({"table":"U","on":true}));
}

#[test]
fn totals_allow_formatting_outside_columns_and_safe_adjacent_tables() {
    for other in ["E4:F5", "A5:B6"] {
        let mut s = ordinary_table();
        s.execute("home.bold", json!({"range":"B4"})).unwrap();
        s.execute("cell.set", json!({"cell":"D4","input":"outside"})).unwrap();
        s.execute("insert.table", json!({"range":other,"header":true,"name":"U"})).unwrap();
        let before = s.doc().unwrap().wb.clone();
        s.execute("table.totalRow", json!({"table":"T","on":true})).unwrap();
        let enabled = s.doc().unwrap().wb.clone();
        let sh = enabled.active().unwrap();
        assert_eq!(sh.tables[0].range.a1(), "A1:C4");
        assert_eq!(sh.value(CellRef::parse("A4").unwrap()).display(), "Total");
        assert_eq!(sh.value(CellRef::parse("C4").unwrap()).display(), "8");
        assert_eq!(sh.value(CellRef::parse("D4").unwrap()).display(), "outside");
        s.execute("table.totalRow", json!({"table":"T","on":true})).unwrap();
        assert_eq!(s.doc().unwrap().wb, enabled);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.doc().unwrap().wb, enabled);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.doc().unwrap().wb, before);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(s.doc().unwrap().wb, enabled);
        s.execute("table.totalRow", json!({"table":"T","on":false})).unwrap();
        let disabled = s.doc().unwrap().wb.clone();
        assert_eq!(disabled.active().unwrap().tables[0].range.a1(), "A1:C3");
        assert!(disabled.active().unwrap().value(CellRef::parse("C4").unwrap()).is_empty());
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.doc().unwrap().wb, enabled);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(s.doc().unwrap().wb, disabled);
    }
}
