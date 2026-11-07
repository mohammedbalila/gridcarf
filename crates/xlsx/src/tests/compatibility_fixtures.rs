//! Original, bounded inputs for the compatibility baseline; no Excel-authored binaries.

use std::io::Write;

use gridcraft_core::{CellRef, Value};

use super::minimal;
use crate::read_xlsx;

fn emit(name: &str, bytes: &[u8]) {
    if let Some(dir) = std::env::var_os("GRIDCRAFT_COMPAT_FIXTURE_DIR") {
        let path = std::path::PathBuf::from(dir).join(name);
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
        file.write_all(bytes).unwrap();
        println!("fixture: {} ({} bytes)", path.display(), bytes.len());
    }
}

#[test]
fn synthetic_baseline_inputs_are_bounded() {
    // Ordinary legacy formula, deliberately lacking cm / array metadata. The cached 2 is
    // derived from documented implicit intersection at row 2, not observed in Excel.
    let legacy = minimal(
        r#"<sheetData><row r="1"><c r="A1"><v>1</v></c></row><row r="2"><c r="A2"><v>4</v></c><c r="B2"><f>SQRT(A1:A3)</f><v>2</v></c></row><row r="3"><c r="A3"><v>9</v></c></row></sheetData>"#,
        &[],
        "",
        "",
    );
    let (wb, _) = read_xlsx(&legacy).unwrap();
    let b2 = wb.sheet(0).unwrap().cell(CellRef::new(1, 1)).unwrap();
    assert_eq!(b2.value, Value::Number(2.0));
    let mut package = crate::package::Package::open(&legacy).unwrap();
    let sheet = package.read_xml("xl/worksheets/sheet1.xml").unwrap().unwrap();
    let b2 = sheet.child("sheetData").unwrap().kids("row").flat_map(|r| r.kids("c")).find(|c| c.attr("r") == Some("B2")).unwrap();
    assert_eq!(b2.attr("cm"), None);
    assert_eq!(b2.child("f").unwrap().attr("t"), None);
    assert_eq!(b2.child("f").unwrap().text, "SQRT(A1:A3)");
    emit("legacy-intersection.xlsx", &legacy);

    let whatif = minimal(
        r#"<sheetData><row r="1"><c r="A1"><v>1</v></c><c r="B1"><f>A1*2</f><v>2</v></c></row><row r="2"><c r="A2"><v>2</v></c><c r="B2"><f t="dataTable" ref="B2:B3" r1="A1"/><v>4</v></c></row><row r="3"><c r="A3"><v>3</v></c><c r="B3"><v>6</v></c></row></sheetData>"#,
        &[],
        "",
        "",
    );
    // Validate the input record directly. Importing it first would erase the very
    // unsupported content the later save-policy repair needs to inspect.
    let mut package = crate::package::Package::open(&whatif).unwrap();
    let sheet = package.read_xml("xl/worksheets/sheet1.xml").unwrap().unwrap();
    let table = sheet.child("sheetData").unwrap().kids("row").flat_map(|r| r.kids("c")).find(|c| c.attr("r") == Some("B2")).unwrap();
    let formula = table.child("f").unwrap();
    assert_eq!((formula.attr("t"), formula.attr("ref"), formula.attr("r1")), (Some("dataTable"), Some("B2:B3"), Some("A1")));
    emit("whatif.xlsx", &whatif);

    let text = "x".repeat(32_767);
    let comments: String = (1..=64).map(|row| format!(r#"<comment ref="A{row}" authorId="0"><text><t>{text}</t></text></comment>"#)).collect();
    let comments = format!(
        r#"<comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><authors><author>Audit</author></authors><commentList>{comments}</commentList></comments>"#
    );
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="../comments1.xml"/></Relationships>"#;
    let notes = minimal("<sheetData/>", &[("xl/comments1.xml", &comments), ("xl/worksheets/_rels/sheet1.xml.rels", rels)], "", "");
    let (wb, _) = read_xlsx(&notes).unwrap();
    let notes_read = &wb.sheet(0).unwrap().comments;
    assert_eq!(notes_read.len(), 64);
    assert!(notes_read.values().all(|note| note.text.len() == 32_767 && note.text == text));
    assert_eq!(notes_read.values().map(|note| note.text.len()).sum::<usize>(), 2_097_088);
    emit("notes.xlsx", &notes);
    emit("plain.xlsx", &minimal("<sheetData/>", &[], "", ""));
}
