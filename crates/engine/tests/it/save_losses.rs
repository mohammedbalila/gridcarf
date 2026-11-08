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

fn html_hidden_data(hidden_row: bool, picture: bool) -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("cell.set", json!({"cell":"A1", "input":"visible marker"})).unwrap();
    s.execute("cell.set", json!({"cell":"B2", "input":"hidden marker"})).unwrap();
    if hidden_row {
        s.execute("home.hideRows", json!({"rows":"2:2"})).unwrap();
    } else {
        s.execute("home.hideColumns", json!({"cols":"B:B"})).unwrap();
    }
    if picture {
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1, 1).write_to(&mut png, image::ImageFormat::Png).unwrap();
        s.execute("insert.picture", json!({"base64":io::base64_encode(png.get_ref()), "at":"D1"})).unwrap();
    }
    s
}

#[test]
fn html_hidden_data_requires_acknowledgement_before_any_output() {
    for hidden_row in [true, false] {
        let mut s = html_hidden_data(hidden_row, !hidden_row);
        let path = target(&s, "html");
        std::fs::write(&path, b"original destination").unwrap();
        let a = analyze(&mut s, &path);
        assert!(a["issues"].as_array().unwrap().iter().any(|i| i["feature"] == "hidden cell data" && i["code"] == "feature.omitted"), "{a}");
        for command in ["file.save", "file.exportHtml", "file.saveBytes"] {
            let mut p = json!({"path":path, "format":"html"});
            assert!(s.execute_headless(command, p.clone()).is_err(), "{command} wrote hidden data without a decision");
            assert_eq!(std::fs::read(&path).unwrap(), b"original destination");
            p["expectedDocument"] = a["document"].clone();
            p["expectedRevision"] = a["revision"].clone();
            p["confirmedLosses"] = json!(["feature.omitted"]);
            let result = s.execute_headless(command, p).unwrap();
            let bytes = if command == "file.saveBytes" {
                io::base64_decode(result["base64"].as_str().unwrap()).unwrap()
            } else {
                std::fs::read(&path).unwrap()
            };
            let html = String::from_utf8(bytes).unwrap();
            assert!(html.contains("visible marker"));
            assert!(!html.contains("hidden marker"));
            std::fs::write(&path, b"original destination").unwrap();
        }
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn html_keeps_hidden_column_data_without_fixed_layout() {
    let mut s = html_hidden_data(false, false);
    let a = analyze(&mut s, "copy.html");
    assert!(a["issues"].as_array().unwrap().is_empty(), "{a}");
    let result = s.execute_headless("file.saveBytes", json!({"format":"html"})).unwrap();
    let bytes = io::base64_decode(result["base64"].as_str().unwrap()).unwrap();
    assert!(String::from_utf8(bytes).unwrap().contains("hidden marker"));
    s.execute("cell.set", json!({"cell":"B2", "input":""})).unwrap();
    s.execute("home.hideRows", json!({"rows":"2:2"})).unwrap();
    assert!(analyze(&mut s, "copy.html")["issues"].as_array().unwrap().is_empty(), "empty hidden rows contain no omitted data");
}

#[path = "../support/owned_directory.rs"]
mod phonetic_directory;
use gridcraft_engine::model::{PhoneticProps, PhoneticRun, PhoneticString};

fn phonetic(reading: &str) -> PhoneticString {
    PhoneticString { text: "東".into(), runs: vec![PhoneticRun { start: 0, end: 1, text: reading.into() }], props: None }
}
fn phonetic_session(annotations: Vec<PhoneticString>) -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("cell.set", json!({"cell":"A1", "input":"東"})).unwrap();
    Arc::make_mut(&mut s.doc_mut().unwrap().wb).phonetics = annotations;
    s
}
fn phonetic_dir(s: &Session) -> phonetic_directory::Directory {
    phonetic_directory::Directory::acquire(
        (0..).map(|n| std::env::temp_dir().join(format!("gridcraft-phonetic-{}-{}-{n}", std::process::id(), s.doc().unwrap().uid))),
    )
    .unwrap()
}
fn annotation_issue(a: &Value) -> &Value {
    let issues = a["issues"].as_array().unwrap();
    assert_eq!(issues.len(), 1, "{a}");
    let issue = &issues[0];
    assert_eq!(issue["feature"], "phonetic annotations");
    assert_eq!(issue["code"], "feature.omitted");
    assert_eq!(issue["location"], Value::Null);
    assert_eq!(issue["effect"], "omitted");
    assert_eq!(issue["editability"], "preservedOnly");
    for private in ["東", "ヒガシ", "アズマ", "private property"] {
        assert!(!a.to_string().contains(private), "{a}");
    }
    issue
}
fn saved_phonetics(s: &mut Session, format: &str, a: &Value) -> Vec<PhoneticString> {
    let mut p = if a["issues"].as_array().unwrap().is_empty() { json!({}) } else { confirm(a, &format!("copy.{format}")) };
    p["format"] = json!(format);
    let out = s.execute_headless("file.saveBytes", p).unwrap();
    let bytes = io::base64_decode(out["base64"].as_str().unwrap()).unwrap();
    io::open_bytes(&format!("copy.{format}"), &bytes).unwrap().0.phonetics
}

#[test]
fn phonetic_flat_saves_require_a_decision_and_really_omit_metadata() {
    let mut description = None;
    for ext in ["csv", "tsv", "html"] {
        for property_only in [false, true] {
            let mut p = phonetic("ヒガシ");
            if property_only {
                p.runs.clear();
                p.props = Some(PhoneticProps::default());
            }
            let mut s = phonetic_session(vec![p]);
            let dir = phonetic_dir(&s);
            let path = dir.join(format!("copy.{ext}")).to_str().unwrap().to_owned();
            std::fs::write(&path, b"owned sentinel").unwrap();
            let a = analyze(&mut s, &path);
            let issue = annotation_issue(&a);
            if let Some(expected) = &description {
                assert_eq!(&issue["description"], expected);
            } else {
                description = Some(issue["description"].clone());
            }
            for (command, params) in [("file.save", json!({"path":path})), ("file.saveBytes", json!({"format":ext}))] {
                let err = s.execute_headless(command, params).unwrap_err();
                let failure: Value = serde_json::from_str(&err.to_string()).unwrap();
                assert_eq!(failure["error"], "save.losses");
                annotation_issue(&failure["analysis"]);
                assert_eq!(std::fs::read(&path).unwrap(), b"owned sentinel");
                assert!(s.doc().unwrap().is_dirty());
                assert!(s.ui_requests.is_empty());
            }
            s.execute_headless("file.save", confirm(&a, &path)).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            let text = String::from_utf8(bytes.clone()).unwrap();
            assert!(text.contains("東"));
            assert!(!text.contains("ヒガシ") && !text.contains("<ruby"));
            if ext != "html" {
                assert!(io::open_bytes(&path, &bytes).unwrap().0.phonetics.is_empty());
            }
        }
    }
    let mut s = phonetic_session(vec![PhoneticString { text: "東".into(), runs: vec![], props: None }]);
    for ext in ["csv", "tsv", "html"] {
        assert!(analyze(&mut s, &format!("copy.{ext}"))["issues"].as_array().unwrap().is_empty());
        s.execute_headless("file.saveBytes", json!({"format":ext})).unwrap();
    }
}

#[test]
fn phonetic_xlsx_known_losses_refuse_and_acknowledgement_matches_real_codec() {
    let normal = phonetic("ヒガシ");
    let mut cases = Vec::new();
    for (start, end, reading) in [(0, 99, "ヒガシ"), (0, 0, "ヒガシ"), (1, 0, "ヒガシ"), (9, 10, "ヒガシ"), (0, 1, "")] {
        let mut p = normal.clone();
        p.runs = vec![PhoneticRun { start, end, text: reading.into() }];
        cases.push(vec![p]);
    }
    for (kind, alignment) in [(Some("private property"), None), (None, Some("private property")), (Some(" Hiragana"), Some("center "))] {
        let mut p = normal.clone();
        p.props = Some(PhoneticProps { font_id: 0, kind: kind.map(str::to_owned), alignment: alignment.map(str::to_owned) });
        cases.push(vec![p]);
    }
    cases.push(vec![normal.clone(), phonetic("アズマ")]);
    cases.push(vec![normal.clone(), PhoneticString { text: "東".into(), runs: vec![], props: None }]);
    for annotations in cases {
        let mut s = phonetic_session(annotations);
        let dir = phonetic_dir(&s);
        let path = dir.join("copy.xlsx").to_str().unwrap().to_owned();
        std::fs::write(&path, b"owned sentinel").unwrap();
        let a = analyze(&mut s, &path);
        annotation_issue(&a);
        assert!(s.execute_headless("file.save", json!({"path":path})).is_err());
        assert!(s.execute_headless("file.saveBytes", json!({"format":"xlsx"})).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"owned sentinel");
        assert!(s.doc().unwrap().is_dirty());
        assert!(s.ui_requests.is_empty());
    }
    let mut clamped = normal.clone();
    clamped.runs[0].end = 99;
    clamped.props = Some(PhoneticProps { font_id: 0, kind: Some("private property".into()), alignment: Some("center".into()) });
    let mut s = phonetic_session(vec![phonetic("アズマ"), clamped]);
    let a = analyze(&mut s, "copy.xlsx");
    let mut expected = normal;
    expected.props = Some(PhoneticProps { font_id: 0, kind: None, alignment: Some("center".into()) });
    assert_eq!(saved_phonetics(&mut s, "xlsx", &a), vec![expected]);
}

#[test]
fn phonetic_supported_xlsx_and_exact_json_are_not_blocked() {
    let normal = phonetic("ヒガシ");
    let empty = PhoneticString { text: "東".into(), runs: vec![], props: None };
    let mut props = empty.clone();
    props.props = Some(PhoneticProps::default());
    let mut supported = normal.clone();
    supported.props = Some(PhoneticProps { font_id: 0, kind: Some("Hiragana".into()), alignment: Some("left".into()) });
    let supplementary =
        PhoneticString { text: "東😀".into(), runs: vec![PhoneticRun { start: 1, end: 3, text: "ヒガシ".into() }], props: None };
    for (annotations, expected) in [
        (vec![], vec![]),
        (vec![empty.clone()], vec![]),
        (vec![normal.clone()], vec![normal.clone()]),
        (vec![props.clone()], vec![props]),
        (vec![supported.clone()], vec![supported]),
        (vec![normal.clone(), normal.clone()], vec![normal.clone()]),
        (vec![empty, normal.clone()], vec![normal.clone()]),
        (vec![supplementary.clone()], vec![supplementary]),
    ] {
        let mut s = phonetic_session(annotations);
        s.execute("cell.set", json!({"cell":"B1", "input":"東😀"})).unwrap();
        let a = analyze(&mut s, "copy.xlsx");
        assert!(a["issues"].as_array().unwrap().is_empty(), "{a}");
        assert_eq!(saved_phonetics(&mut s, "xlsx", &a), expected);
    }
    let mut unusual = normal.clone();
    unusual.runs = vec![PhoneticRun { start: u32::MAX, end: 0, text: "".into() }];
    unusual.props = Some(PhoneticProps { font_id: 91, kind: Some("private property".into()), alignment: Some(" odd ".into()) });
    let exact = vec![normal.clone(), unusual.clone(), normal, unusual];
    let mut s = phonetic_session(exact.clone());
    let a = analyze(&mut s, "copy.json");
    assert!(a["issues"].as_array().unwrap().is_empty(), "{a}");
    assert_eq!(saved_phonetics(&mut s, "json", &a), exact);
}

#[test]
fn phonetic_decisions_remain_document_revision_and_code_scoped() {
    let mut s = phonetic_session(vec![phonetic("ヒガシ"); 200]);
    let dir = phonetic_dir(&s);
    let path = dir.join("copy.csv").to_str().unwrap().to_owned();
    std::fs::write(&path, b"owned sentinel").unwrap();
    let a = analyze(&mut s, &path);
    annotation_issue(&a);
    let json_analysis = analyze(&mut s, "copy.json");
    assert!(json_analysis["issues"].as_array().unwrap().is_empty());
    let mut wrong_document = confirm(&a, &path);
    wrong_document["expectedDocument"] = json!(a["document"].as_u64().unwrap() + 1);
    let mut missing_codes = confirm(&a, &path);
    missing_codes["confirmedLosses"] = json!([]);
    for p in [wrong_document, missing_codes, confirm(&json_analysis, &path)] {
        assert!(s.execute_headless("file.save", p).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"owned sentinel");
    }
    s.execute("cell.set", json!({"cell":"B1", "input":"revision change"})).unwrap();
    assert!(s.execute_headless("file.save", confirm(&a, &path)).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"owned sentinel");
    assert!(s.doc().unwrap().is_dirty());
    let fresh = analyze(&mut s, &path);
    assert_eq!(annotation_issue(&a), annotation_issue(&fresh));
    s.execute_headless("file.save", confirm(&fresh, &path)).unwrap();
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains("東") && !text.contains("ヒガシ"));
}
