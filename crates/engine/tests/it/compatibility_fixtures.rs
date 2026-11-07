//! Creates the slicer baseline through the public command seam, with opt-in file emission.

use std::io::Write;

use gridcraft_engine::{Session, io};
use serde_json::json;

fn emit(name: &str, bytes: &[u8]) {
    if let Some(dir) = std::env::var_os("GRIDCRAFT_COMPAT_FIXTURE_DIR") {
        let path = std::path::PathBuf::from(dir).join(name);
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
        file.write_all(bytes).unwrap();
        println!("fixture: {} ({} bytes)", path.display(), bytes.len());
    }
}

#[test]
fn synthetic_slicer_input_is_bounded() {
    let mut session = Session::new();
    session.new_workbook();
    session.execute("range.setValues", json!({"range": "A1", "values": [["Region", "Sales"], ["East", 10], ["West", 20]]})).unwrap();
    session.execute("insert.table", json!({"range": "A1:B3", "name": "SalesTable", "header": true})).unwrap();
    session.execute("insert.slicer", json!({"table": "SalesTable", "column": "Region", "at": "D1"})).unwrap();
    let workbook = &session.doc().unwrap().wb;
    let sheet = workbook.sheet(0).unwrap();
    assert_eq!(sheet.tables.len(), 1);
    assert_eq!(sheet.slicers.len(), 1);
    assert_eq!(sheet.slicers[0].column, "Region");
    emit("slicer-before.json", &io::save_bytes(workbook, "slicer-before.json").unwrap());
    emit("slicer.xlsx", &io::save_bytes(workbook, "slicer.xlsx").unwrap());
}
