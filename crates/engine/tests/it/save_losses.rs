//! Loss decisions belong to one document revision and precede any output write.
use std::io::{Cursor, Write};
use std::sync::Arc;

use gridcraft_engine::{
    Session, io,
    model::{Preserved, PreservedXml, Workbook},
};
use serde_json::{Value, json};

fn slicer() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range":"A1", "values":[["Region"],["East"],["West"]]})).unwrap();
    s.execute("insert.table", json!({"range":"A1:A3", "name":"Sales", "header":true})).unwrap();
    s.execute("insert.slicer", json!({"table":"Sales", "column":"Region", "at":"D1"})).unwrap();
    s
}
fn target(s: &Session, ext: &str) -> String {
    std::env::temp_dir().join(format!("gridcraft-save-loss-{}-{}.{}", std::process::id(), s.doc().unwrap().uid, ext)).to_string_lossy().into_owned()
}
fn analyze(s: &mut Session, path: &str) -> Value {
    s.execute_headless("file.analyzeSave", json!({"path":path})).unwrap()
}
fn confirm(a: &Value, path: &str) -> Value {
    json!({"path":path, "expectedRevision":a["revision"], "expectedDocument":a["document"],
        "confirmedLosses":a["issues"].as_array().unwrap().iter().map(|i|i["code"].clone()).collect::<Vec<_>>()})
}
fn what_if() -> Vec<u8> {
    let bytes = io::save_bytes(&Workbook::new(), "input.xlsx").unwrap();
    let mut src = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut dst = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..src.len() {
        let mut f = src.by_index(i).unwrap();
        dst.start_file(f.name(), zip::write::SimpleFileOptions::default()).unwrap();
        if f.name() == "xl/worksheets/sheet1.xml" {
            dst.write_all(br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><f t="dataTable" ref="A1:A2" r1="B1"/><v>4</v></c></row></sheetData></worksheet>"#).unwrap();
        } else {
            std::io::copy(&mut f, &mut dst).unwrap();
        }
    }
    dst.finish().unwrap().into_inner()
}
#[test]
fn save_refuses_unacknowledged_loss() {
    let mut s = slicer();
    let path = target(&s, "xlsx");
    std::fs::write(&path, b"original destination").unwrap();
    let result = s.execute_headless("file.save", json!({"path":path}));
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(result.is_err(), "unacknowledged slicer loss was saved: {result:?}");
    assert_eq!(bytes, b"original destination");
    assert!(s.doc().unwrap().is_dirty());
}
#[test]
fn loss_confirmation_is_revision_scoped() {
    let mut s = slicer();
    let path = target(&s, "xlsx");
    let a = analyze(&mut s, &path);
    s.execute("insert.slicer", json!({"table":"Sales", "column":"Region", "at":"F1"})).unwrap();
    assert!(s.execute_headless("file.save", confirm(&a, &path)).is_err());
    assert!(!std::path::Path::new(&path).exists());
    let a = analyze(&mut s, &path);
    s.execute_headless("file.save", confirm(&a, &path)).unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(!s.doc().unwrap().is_dirty());
}
#[test]
fn open_loss_remains_visible() {
    let mut s = Session::new();
    s.execute_headless("file.open", json!({"name":"what-if.xlsx", "base64":io::base64_encode(&what_if())})).unwrap();
    let a = analyze(&mut s, "copy.xlsx");
    assert!(a["issues"].as_array().unwrap().iter().any(|i| i["code"] == "feature.static" && i["feature"] == "what-if data table"), "{a}");
    s.execute("cell.set", json!({"cell":"C1", "input":"1"})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(a["issues"], analyze(&mut s, "copy.xlsx")["issues"]);
}
#[test]
fn headless_save_never_opens_dialog() {
    let mut s = slicer();
    let r = s.execute_headless("file.saveBytes", json!({"format":"xlsx"}));
    assert!(r.is_err(), "headless encoded a lossy workbook");
    let failure: Value = serde_json::from_str(&r.unwrap_err().to_string()).unwrap();
    assert_eq!(failure["error"], "save.losses");
    assert_eq!(failure["analysis"]["issues"][0]["code"], "feature.omitted");
    assert!(s.ui_requests.is_empty());
}
#[test]
fn confirmation_cannot_authorize_another_document() {
    let mut s = slicer();
    let path = target(&s, "xlsx");
    let a = analyze(&mut s, &path);
    let other = slicer();
    assert_eq!(other.doc().unwrap().revision, s.doc().unwrap().revision);
    s.add_document(other.doc().unwrap().clone());
    assert!(s.execute_headless("file.save", confirm(&a, &path)).is_err());
    assert!(!std::path::Path::new(&path).exists());
}
#[test]
fn json_reports_opaque_omissions_but_keeps_vba_and_model_slicers() {
    let mut s = slicer();
    let wb = Arc::make_mut(&mut s.doc_mut().unwrap().wb);
    wb.vba_project = Some(Arc::new(vec![1, 2, 3]));
    wb.preserved =
        Some(Arc::new(Preserved { elements: vec![PreservedXml { slot: "fileVersion".into(), xml: "<fileVersion/>".into() }], ..Default::default() }));
    let a = analyze(&mut s, "copy.json");
    let issues = a["issues"].as_array().unwrap();
    assert!(!issues.iter().any(|i| i["feature"] == "VBA project"));
    let bytes = io::save_bytes(&s.doc().unwrap().wb, "copy.json").unwrap();
    let (reopened, _) = io::open_bytes("copy.json", &bytes).unwrap();
    assert_eq!(reopened.vba_project, s.doc().unwrap().wb.vba_project);
    assert!(issues.iter().any(|i| i["feature"] == "preserved package content"));
    assert!(!issues.iter().any(|i| i["feature"] == "slicer"));
}
#[test]
fn recalculation_and_undo_invalidate_confirmation() {
    let mut s = slicer();
    let a = analyze(&mut s, "copy.xlsx");
    s.execute("formulas.calculateNow", json!({})).unwrap();
    assert_ne!(a["revision"], analyze(&mut s, "copy.xlsx")["revision"]);
    s.execute("edit.undo", json!({})).unwrap();
    assert_ne!(a["revision"], analyze(&mut s, "copy.xlsx")["revision"]);
}

#[test]
fn unsupported_package_relationships_and_extensions_are_unknown() {
    let bytes = io::save_bytes(&Workbook::new(), "input.xlsx").unwrap();
    let mut src = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut dst = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..src.len() {
        let mut f = src.by_index(i).unwrap();
        dst.start_file(f.name(), zip::write::SimpleFileOptions::default()).unwrap();
        let name = f.name().to_string();
        if name == "xl/_rels/workbook.xml.rels" || name == "xl/workbook.xml" {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut f, &mut text).unwrap();
            text = if name.ends_with(".rels") {
                text.replace(
                    "</Relationships>",
                    r#"<Relationship Id="unknown" Type="urn:synthetic/unknownFeature" Target="unknown.xml"/></Relationships>"#,
                )
            } else {
                text.replace("</workbook>", r#"<extLst><ext uri="urn:synthetic"><mystery/></ext></extLst></workbook>"#)
            };
            dst.write_all(text.as_bytes()).unwrap();
        } else {
            std::io::copy(&mut f, &mut dst).unwrap();
        }
    }
    let mut s = Session::new();
    s.execute_headless("file.open", json!({"name":"unknown.xlsx", "base64":io::base64_encode(&dst.finish().unwrap().into_inner())})).unwrap();
    let a = analyze(&mut s, "copy.xlsx");
    let issues = a["issues"].as_array().unwrap();
    assert!(issues.iter().any(|i| i["code"] == "package.unknown" && i["feature"] == "unknownFeature"), "{a}");
    assert!(issues.iter().any(|i| i["code"] == "package.unknown" && i["feature"] == "workbook extension"), "{a}");
}
#[test]
fn exports_require_acknowledgement_and_vba_is_format_specific() {
    let mut s = slicer();
    Arc::make_mut(&mut s.doc_mut().unwrap().wb).vba_project = Some(Arc::new(vec![7, 8, 9]));
    let plain = analyze(&mut s, "copy.xlsx");
    assert!(plain["issues"].as_array().unwrap().iter().any(|i| i["feature"] == "VBA project"));
    let macro_book = analyze(&mut s, "copy.xlsm");
    assert!(!macro_book["issues"].as_array().unwrap().iter().any(|i| i["feature"] == "VBA project"));
    for id in ["file.exportCsv", "file.exportHtml"] {
        let path = target(&s, if id.ends_with("Csv") { "csv" } else { "html" });
        assert!(s.execute_headless(id, json!({"path":path})).is_err());
        assert!(!std::path::Path::new(&path).exists());
        let a = analyze(&mut s, &path);
        s.execute_headless(id, confirm(&a, &path)).unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(s.doc().unwrap().is_dirty());
    }
}

#[test]
fn encrypted_import_requires_explicit_unprotected_save_decision() {
    let plain = io::save_bytes(&Workbook::new(), "input.xlsx").unwrap();
    let encrypted = gridcraft_xlsx::testing::encrypt_workbook(&plain, "synthetic password");
    let mut s = Session::new();
    s.execute_headless("file.open", json!({"name":"encrypted.xlsx", "base64":io::base64_encode(&encrypted), "password":"synthetic password"}))
        .unwrap();
    let path = target(&s, "xlsx");
    std::fs::write(&path, &encrypted).unwrap();
    let a = analyze(&mut s, &path);
    assert!(a["issues"].as_array().unwrap().iter().any(|i| i["code"] == "encryption.removed"));
    assert!(!a.to_string().contains("synthetic password"));
    assert!(s.execute_headless("file.save", json!({"path":path})).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), encrypted);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn selected_sheet_export_analyzes_the_sheet_actually_written() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("home.insertSheet", json!({"name":"Calculations"})).unwrap();
    s.execute("cell.set", json!({"cell":"A1", "input":"=1+2"})).unwrap();
    s.execute("sheet.activate", json!({"sheet":0})).unwrap();
    let path = target(&s, "csv");
    let failure = s.execute_headless("file.exportCsv", json!({"path":path, "sheet":"Calculations"})).unwrap_err();
    let error: Value = serde_json::from_str(&failure.to_string()).unwrap();
    assert!(error["analysis"]["issues"].as_array().unwrap().iter().any(|i| i["code"] == "feature.static" && i["location"] == "Calculations"));
    assert!(!std::path::Path::new(&path).exists());
}

#[test]
fn safe_notice_limit_does_not_hide_a_later_known_loss() {
    let mut report = gridcraft_xlsx::ReadReport::default();
    for i in 0..1000 {
        report.notice(format!("safe notice {i}"));
    }
    report.warn("what-if data tables are not supported; kept their values");
    assert!(report.issues.iter().any(|i| i.code == "feature.static"));
}

#[test]
fn issue_limit_retains_an_explicit_incomplete_inventory_risk() {
    let mut report = gridcraft_xlsx::ReadReport::default();
    for i in 0..1005 {
        report.warn(format!("unsupported feature {i} was dropped"));
    }
    assert_eq!(report.issues.len(), 1001);
    assert!(report.issues.iter().any(|i| i.code == "package.unknown" && i.feature == "additional import issues"));
    let mut s = Session::new();
    s.new_workbook();
    s.doc_mut().unwrap().retain_import_issues(report.issues);
    let a = analyze(&mut s, "copy.xlsx");
    let mut p = confirm(&a, "copy.xlsx");
    p["format"] = json!("xlsx");
    p["confirmedLosses"] = json!(["feature.omitted"]);
    let error = s.execute_headless("file.saveBytes", p).unwrap_err().to_string();
    assert!(error.contains("save.losses"));
    assert!(error.contains("package.unknown"));
}

#[test]
fn html_reports_hyperlink_and_conditional_format_omissions() {
    for feature in ["hyperlinks", "conditional formatting"] {
        let mut s = Session::new();
        s.new_workbook();
        if feature == "hyperlinks" {
            s.execute("insert.link", json!({"cell":"A1", "target":"https://example.com", "text":"Example"})).unwrap();
        } else {
            s.execute(
                "home.conditionalFormat",
                json!({"range":"A1:A3", "rule":{"type":"cellIs", "operator":"greaterThan", "value":"1", "preset":"lightRedFill"}}),
            )
            .unwrap();
        }
        let a = analyze(&mut s, "copy.html");
        assert!(a["issues"].as_array().unwrap().iter().any(|i| i["feature"] == feature && i["code"] == "feature.omitted"), "{a}");
        assert!(s.execute_headless("file.saveBytes", json!({"format":"html"})).is_err());
    }
}
