//! Synthetic native OOXML data-table preservation; no Excel-authored oracle.
use super::minimal;
use crate::{read_xlsx, write_xlsx};
use gridcraft_core::{CellRef, Value};

fn fixture(attrs: &str) -> Vec<u8> {
    minimal(
        &format!(
            r#"<sheetData><row r="2"><c r="C2"><f t="dataTable" ref="C2:D3" {attrs}/><v>12</v></c><c r="D2"><v>14</v></c></row><row r="3"><c r="C3"><v>18</v></c><c r="D3"><v>21</v></c></row></sheetData>"#
        ),
        &[],
        "",
        "",
    )
}

fn assert_native(bytes: &[u8], expected: &[(&str, &str)]) {
    let mut package = crate::package::Package::open(bytes).unwrap();
    let xml = package.read_xml("xl/worksheets/sheet1.xml").unwrap().unwrap();
    let cell = xml.child("sheetData").unwrap().child("row").unwrap().child("c").unwrap();
    let formula = cell.child("f").expect("native what-if definition must survive save");
    for (key, value) in expected {
        assert_eq!(formula.attr(key), Some(*value), "attribute {key}");
    }
    let (back, _) = read_xlsx(bytes).unwrap();
    for (at, value) in [("C2", 12.0), ("D2", 14.0), ("C3", 18.0), ("D3", 21.0)] {
        let sh = back.sheet(0).unwrap();
        let cell = CellRef::parse(at).unwrap();
        assert_eq!(sh.value(cell), Value::Number(value));
        assert!(sh.cell(cell).unwrap().formula.is_none(), "TABLE must not enter ordinary calculation");
    }
}

#[test]
fn definition_roundtrip() {
    for attrs in [r#"r1="A1""#, r#"r1="A1" dtr="1" dt2D="0" ca="1" del1="0""#, r#"r1="A1" r2="A2" dt2D="1" dtr="0" ca="0" del1="1" del2="1""#] {
        let input = fixture(attrs);
        let mut package = crate::package::Package::open(&input).unwrap();
        let xml = package.read_xml("xl/worksheets/sheet1.xml").unwrap().unwrap();
        let original = xml.child("sheetData").unwrap().child("row").unwrap().child("c").unwrap().child("f").unwrap();
        let expected: Vec<_> = ["t", "ref", "r1", "r2", "dt2D", "dtr", "ca", "del1", "del2"]
            .into_iter()
            .filter_map(|key| original.attr(key).map(|v| (key, v)))
            .collect();
        let (wb, _) = read_xlsx(&input).unwrap();
        assert_native(&write_xlsx(&wb).unwrap(), &expected);
    }
}

#[test]
fn unrelated_edit_keeps_definition() {
    let (mut wb, _) = read_xlsx(&fixture(r#"r1="A1" dtr="1""#)).unwrap();
    wb.sheet_mut(0).unwrap().set_value(CellRef::parse("Z20").unwrap(), Value::Number(99.0));
    assert_native(&write_xlsx(&wb).unwrap(), &[("t", "dataTable"), ("ref", "C2:D3"), ("r1", "A1"), ("dtr", "1")]);
}

#[test]
fn optional_flags_and_static_issue_are_preserved() {
    let (wb, report) = read_xlsx(&fixture(r#"r1="$A$1" dtr="true" ca="false""#)).unwrap();
    let table = &wb.sheet(0).unwrap().what_if_tables[0];
    assert_eq!(table.input1, CellRef::parse("A1"));
    assert_eq!(table.row_oriented, Some(true));
    assert_eq!(table.calculate, Some(false));
    assert_eq!(table.two_variable, None);
    let issues: Vec<_> = report.issues.iter().filter(|i| i.feature == "what-if data table").collect();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].effect, gridcraft_model::CompatibilityEffect::Static);
    assert_eq!(issues[0].editability, gridcraft_model::Editability::PreservedOnly);
    assert!(issues[0].description.contains("outside precedent edits"));
    let bytes = write_xlsx(&wb).unwrap();
    let (back, _) = read_xlsx(&bytes).unwrap();
    assert_eq!(back.sheet(0).unwrap().what_if_tables, wb.sheet(0).unwrap().what_if_tables);
}

#[test]
fn empty_master_and_non_numeric_cached_values_survive() {
    let bytes = minimal(
        r#"<sheetData><row r="2"><c r="B2"><f t="dataTable" ref="B2:E2" r1="A10" dtr="1"/></c><c r="C2" t="b"><v>1</v></c><c r="D2" t="e"><v>#DIV/0!</v></c><c r="E2" t="str"><v>cached</v></c></row></sheetData>"#,
        &[],
        "",
        "",
    );
    let (wb, _) = read_xlsx(&bytes).unwrap();
    let bytes = write_xlsx(&wb).unwrap();
    let (back, _) = read_xlsx(&bytes).unwrap();
    assert_eq!(back.sheet(0).unwrap().what_if_tables.len(), 1);
    for at in ["B2", "C2", "D2", "E2"] {
        let at = CellRef::parse(at).unwrap();
        assert_eq!(wb.sheet(0).unwrap().value(at), back.sheet(0).unwrap().value(at));
    }
}

#[test]
fn malformed_definition_reports_omission_without_losing_cache() {
    for attrs in [r#"ref="invalid" r1="A1""#, r#"ref="B2" r1="invalid""#, r#"ref="B2" ca="invalid""#] {
        let bytes = minimal(&format!(r#"<sheetData><row r="2"><c r="B2"><f t="dataTable" {attrs}/><v>42</v></c></row></sheetData>"#), &[], "", "");
        let (wb, report) = read_xlsx(&bytes).unwrap();
        assert!(wb.sheet(0).unwrap().what_if_tables.is_empty());
        assert_eq!(wb.sheet(0).unwrap().value(CellRef::parse("B2").unwrap()), Value::Number(42.0));
        assert!(report.issues.iter().any(|i| i.effect == gridcraft_model::CompatibilityEffect::Omitted));
    }
}

#[test]
fn nonempty_native_text_is_inert_and_roundtrips_escaped_unicode() {
    let bytes = minimal(
        r#"<sheetData><row r="2"><c r="B2"><f t="dataTable" ref="B2:B3" r1="A1"> TABLE(A1,)&amp;&lt;"☃" </f><v>42</v></c></row></sheetData>"#,
        &[],
        "",
        "",
    );
    let (wb, report) = read_xlsx(&bytes).unwrap();
    assert_eq!(wb.sheet(0).unwrap().what_if_tables.len(), 1, "nonempty native text is not an invalid definition");
    assert!(!report.issues.iter().any(|i| i.feature == "what-if data table" && i.effect == gridcraft_model::CompatibilityEffect::Omitted));
    assert!(wb.sheet(0).unwrap().cell(CellRef::parse("B2").unwrap()).unwrap().formula.is_none());
    let output = write_xlsx(&wb).unwrap();
    let mut package = crate::package::Package::open(&output).unwrap();
    let xml = package.read_xml("xl/worksheets/sheet1.xml").unwrap().unwrap();
    let f = xml.child("sheetData").unwrap().child("row").unwrap().child("c").unwrap().child("f").unwrap();
    assert_eq!(f.text, " TABLE(A1,)&<\"☃\" ");
    let (back, _) = read_xlsx(&output).unwrap();
    assert_eq!(back.sheet(0).unwrap().what_if_tables, wb.sheet(0).unwrap().what_if_tables);
    assert_eq!(back.sheet(0).unwrap().value(CellRef::parse("B2").unwrap()), Value::Number(42.0));
}
