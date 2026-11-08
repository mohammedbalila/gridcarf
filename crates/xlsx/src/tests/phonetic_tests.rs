//! Japanese phonetic runs (`rPh`, furigana) of shared strings survive an open + save.
//!
//! Behaviour reference: ISO/IEC 29500-1 §18.4.6 `rPh` (`sb`/`eb` are zero-based offsets into the
//! base text, `sb < eb`, child `<t>`) and §18.4.7 `phoneticPr` (`fontId`, `type`, `alignment`).

use gridcraft_core::{CellRef, Value};
use gridcraft_model::{Cell, PhoneticProps, PhoneticRun, PhoneticString, StyleId, Workbook};

use super::minimal;
use crate::package::Package;
use crate::{read_xlsx, write_xlsx};

/// A string item with two `<rPh>` runs and a `<phoneticPr>`, and one with no phonetic data.
const SST: &str = r#"<?xml version="1.0"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="2" uniqueCount="2"><si><t>東京都</t><rPh sb="0" eb="2"><t>トウキョウ</t></rPh><rPh sb="2" eb="3"><t>ト &amp; &lt;x&gt;</t></rPh><phoneticPr fontId="1" type="noConversion" alignment="center"/></si><si><t>大阪</t></si></sst>"#;

const SHEET_BODY: &str = r#"<sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row></sheetData>"#;

const SST_RELS: &str = r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>"#;

fn at(a: &str) -> CellRef {
    CellRef::parse(a).unwrap()
}

/// A one-sheet package whose shared string table is `sst`.
fn package(sst: &str) -> Vec<u8> {
    minimal(SHEET_BODY, &[("xl/sharedStrings.xml", sst)], SST_RELS, "")
}

/// A part of a written package, as text.
fn part(bytes: &[u8], name: &str) -> String {
    let mut pkg = Package::open(bytes).unwrap();
    String::from_utf8(pkg.read(name).unwrap().unwrap()).unwrap()
}

fn text_cell(sheet: &mut gridcraft_model::Sheet, a: &str, text: &str) {
    sheet.cells.set(at(a), Cell { value: Value::text(text), formula: None, style: StyleId::DEFAULT, py: None });
}

#[test]
fn phonetic_runs_and_props_survive_open_and_save() {
    let (wb, report) = read_xlsx(&package(SST)).unwrap();
    assert!(!report.warnings.iter().any(|w| w.contains("phonetic")), "{:?}", report.warnings);
    let sheet = wb.sheet(0).unwrap();
    // A reading is a hint, not part of the text.
    assert_eq!(sheet.value(at("A1")), Value::text("東京都"));
    assert_eq!(sheet.value(at("B1")), Value::text("大阪"));
    assert_eq!(
        wb.phonetics,
        vec![PhoneticString {
            text: "東京都".into(),
            runs: vec![PhoneticRun { start: 0, end: 2, text: "トウキョウ".into() }, PhoneticRun { start: 2, end: 3, text: "ト & <x>".into() },],
            props: Some(PhoneticProps { font_id: 1, kind: Some("noConversion".into()), alignment: Some("center".into()) }),
        }]
    );

    let bytes = write_xlsx(&wb).unwrap();
    let sst = part(&bytes, "xl/sharedStrings.xml");
    assert!(
        sst.contains(
            r#"<si><t>東京都</t><rPh sb="0" eb="2"><t>トウキョウ</t></rPh><rPh sb="2" eb="3"><t>ト &amp; &lt;x&gt;</t></rPh><phoneticPr fontId="1" type="noConversion" alignment="center"/></si>"#
        ),
        "{sst}"
    );
    assert!(sst.contains("<si><t>大阪</t></si>"), "a string without phonetic data stays plain: {sst}");

    // A second open + save keeps the runs and reproduces the same part.
    let (again, report) = read_xlsx(&bytes).unwrap();
    assert_eq!(again.phonetics, wb.phonetics);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert_eq!(part(&write_xlsx(&again).unwrap(), "xl/sharedStrings.xml"), sst);
}

#[test]
fn a_plain_japanese_string_writes_the_plain_si() {
    let mut wb = Workbook::new();
    text_cell(wb.sheet_mut(0).unwrap(), "A1", "日本語");
    let bytes = write_xlsx(&wb).unwrap();
    let sst = part(&bytes, "xl/sharedStrings.xml");
    assert!(sst.contains("<si><t>日本語</t></si>"), "{sst}");
    assert!(!sst.contains("<rPh") && !sst.contains("<phoneticPr"), "{sst}");
}

#[test]
fn property_only_shared_string_survives_open_and_save() {
    // `phoneticPr` without any `rPh` is valid (a shared string item permits it on its own), so
    // open + save must not delete it.
    let sst = r#"<?xml version="1.0"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="1" uniqueCount="1"><si><t>東京</t><phoneticPr fontId="2" type="Hiragana" alignment="left"/></si></sst>"#;
    let body = r#"<sheetData><row r="1"><c r="A1" t="s"><v>0</v></c></row></sheetData>"#;
    let bytes = minimal(body, &[("xl/sharedStrings.xml", sst)], SST_RELS, "");
    let (wb, report) = read_xlsx(&bytes).unwrap();
    assert!(!report.warnings.iter().any(|w| w.contains("phonetic")), "{:?}", report.warnings);
    assert_eq!(
        wb.phonetics,
        vec![PhoneticString {
            text: "東京".into(),
            runs: vec![],
            props: Some(PhoneticProps { font_id: 2, kind: Some("Hiragana".into()), alignment: Some("left".into()) }),
        }]
    );
    let written = write_xlsx(&wb).unwrap();
    let out = part(&written, "xl/sharedStrings.xml");
    assert!(out.contains(r#"<si><t>東京</t><phoneticPr fontId="2" type="Hiragana" alignment="left"/></si>"#), "{out}");
    // A second open + save reproduces the same part.
    let (again, report) = read_xlsx(&written).unwrap();
    assert_eq!(again.phonetics, wb.phonetics);
    assert!(!report.warnings.iter().any(|w| w.contains("phonetic")), "{:?}", report.warnings);
    assert_eq!(part(&write_xlsx(&again).unwrap(), "xl/sharedStrings.xml"), out);
}

#[test]
fn hostile_phonetic_runs_lose_only_the_runs() {
    // Missing `eb`, missing `sb`, `sb >= eb`, offsets past the text, junk offsets, no offsets at
    // all, no `<t>`, an empty `<t>`, and junk `phoneticPr` attributes.
    let sst = r#"<?xml version="1.0"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="1" uniqueCount="1"><si junk="1"><t>課きく</t><rPh sb="1"><t>no eb</t></rPh><rPh eb="2"><t>no sb</t></rPh><rPh sb="3" eb="1"><t>reversed</t></rPh><rPh sb="0" eb="99"><t>clamped</t></rPh><rPh sb="junk" eb="1"><t>junk</t></rPh><rPh><t>no offsets</t></rPh><rPh sb="0" eb="1" junk="2"/><rPh sb="0" eb="1"><t/></rPh><phoneticPr fontId="junk" type="junk" alignment="junk"/></si></sst>"#;
    let (wb, report) = read_xlsx(&package(sst)).unwrap();
    // The base text is intact and only the one run that can be clamped is kept.
    assert_eq!(wb.sheet(0).unwrap().value(at("A1")), Value::text("課きく"));
    assert_eq!(
        wb.phonetics,
        vec![PhoneticString {
            text: "課きく".into(),
            runs: vec![PhoneticRun { start: 0, end: 3, text: "clamped".into() }],
            props: Some(PhoneticProps::default()),
        }]
    );
    assert!(report.warnings.iter().any(|w| w.contains("phonetic runs were dropped")), "{:?}", report.warnings);

    // What is written back is valid: offsets within the text, a fontId on every `<phoneticPr>`.
    let bytes = write_xlsx(&wb).unwrap();
    let out = part(&bytes, "xl/sharedStrings.xml");
    assert!(out.contains(r#"<si><t>課きく</t><rPh sb="0" eb="3"><t>clamped</t></rPh><phoneticPr fontId="0"/></si>"#), "{out}");
    assert_eq!(read_xlsx(&bytes).unwrap().0.phonetics, wb.phonetics);
}

#[test]
fn identical_text_with_different_readings_keeps_the_first_and_warns() {
    let sst = r#"<?xml version="1.0"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="2" uniqueCount="2"><si><t>東</t><rPh sb="0" eb="1"><t>ヒガシ</t></rPh></si><si><t>東</t><rPh sb="0" eb="1"><t>アズマ</t></rPh></si></sst>"#;
    let (wb, report) = read_xlsx(&package(sst)).unwrap();
    // The saved table holds each text once, so only one reading can be written: the first.
    assert_eq!(wb.phonetics.len(), 1);
    assert_eq!(wb.phonetics[0].runs.first().map(|r| r.text.as_str()), Some("ヒガシ"));
    assert!(report.warnings.iter().any(|w| w.contains("phonetic readings")), "{:?}", report.warnings);

    // The same text twice with the same reading is not a conflict.
    let same = sst.replace("アズマ", "ヒガシ");
    let (wb, report) = read_xlsx(&package(&same)).unwrap();
    assert_eq!(wb.phonetics.len(), 1);
    assert!(!report.warnings.iter().any(|w| w.contains("phonetic readings")), "{:?}", report.warnings);
}

#[test]
fn phonetic_data_never_used_by_a_cell_is_not_written() {
    let mut wb = Workbook::new();
    text_cell(wb.sheet_mut(0).unwrap(), "A1", "日本語");
    wb.phonetics =
        vec![PhoneticString { text: "別の文字列".into(), runs: vec![PhoneticRun { start: 0, end: 2, text: "ベツ".into() }], props: None }];
    let bytes = write_xlsx(&wb).unwrap();
    let sst = part(&bytes, "xl/sharedStrings.xml");
    assert!(sst.contains("<si><t>日本語</t></si>"), "{sst}");
    assert!(!sst.contains("<rPh"), "{sst}");
    let (back, report) = read_xlsx(&bytes).unwrap();
    assert!(back.phonetics.is_empty());
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn comments_and_inline_strings_still_ignore_phonetic_runs() {
    let body = r#"<sheetData><row r="1"><c r="A1" t="inlineStr"><is><r><t>inline</t></r><rPh sb="0" eb="6"><t>インライン</t></rPh><phoneticPr fontId="1"/></is></c></row></sheetData>"#;
    let sheet_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="../comments1.xml"/></Relationships>"#;
    let comments = r#"<comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><authors><author>Ann</author></authors><commentList><comment ref="A1" authorId="0"><text><r><rPr><b/></rPr><t>Ann:</t></r><r><t>メモ</t></r><rPh sb="0" eb="2"><t>メモ</t></rPh><phoneticPr fontId="1"/></text></comment></commentList></comments>"#;
    let bytes = minimal(body, &[("xl/worksheets/_rels/sheet1.xml.rels", sheet_rels), ("xl/comments1.xml", comments)], "", "");
    let (wb, _) = read_xlsx(&bytes).unwrap();
    let sheet = wb.sheet(0).unwrap();
    assert_eq!(sheet.value(at("A1")), Value::text("inline"));
    assert_eq!(sheet.comments.get(&at("A1")).map(|c| c.text.as_str()), Some("メモ"));
    // Neither path has a shared string table entry to key phonetic data by.
    assert!(wb.phonetics.is_empty());
}

#[test]
fn phonetic_known_loss_diagnostic_matches_xml_normalization_and_last_winner() {
    let run = |start, end, text: &str| PhoneticRun { start, end, text: text.into() };
    let normal = PhoneticString { text: "東😀".into(), runs: vec![run(1, 3, "reading")], props: None };
    let empty = PhoneticString { text: normal.text.clone(), runs: vec![], props: None };
    let mut other = normal.clone();
    other.runs[0].text = "last reading".into();
    let mut property = empty.clone();
    property.props = Some(PhoneticProps::default());
    for (annotations, loss, expected) in [
        (vec![], false, vec![]),
        (vec![empty.clone()], false, vec![]),
        (vec![normal.clone(), normal.clone()], false, vec![normal.clone()]),
        (vec![empty.clone(), normal.clone()], false, vec![normal.clone()]),
        (vec![normal.clone(), empty], true, vec![]),
        (vec![normal.clone(), other.clone()], true, vec![other]),
        (vec![property.clone(), normal.clone()], true, vec![normal.clone()]),
        (vec![normal.clone(), property.clone()], true, vec![property]),
    ] {
        let mut wb = Workbook::new();
        text_cell(wb.sheet_mut(0).unwrap(), "A1", "東😀");
        wb.phonetics = annotations;
        let before = wb.phonetics.clone();
        assert_eq!(crate::xlsx_known_phonetic_loss(&wb).unwrap(), loss);
        assert_eq!(wb.phonetics, before, "diagnosis is read-only");
        assert_eq!(read_xlsx(&write_xlsx(&wb).unwrap()).unwrap().0.phonetics, expected);
    }
    for (start, end, reading, expected) in [
        (0, 99, "clamped", vec![run(0, 3, "clamped")]),
        (1, 1, "empty range", vec![]),
        (3, 1, "reversed", vec![]),
        (9, 99, "outside", vec![]),
        (0, 1, "", vec![]),
    ] {
        let mut wb = Workbook::new();
        text_cell(wb.sheet_mut(0).unwrap(), "A1", "東😀");
        wb.phonetics = vec![PhoneticString { text: "東😀".into(), runs: vec![run(start, end, reading)], props: None }];
        assert!(crate::xlsx_known_phonetic_loss(&wb).unwrap());
        let bytes = write_xlsx(&wb).unwrap();
        let sst = part(&bytes, "xl/sharedStrings.xml");
        if expected.is_empty() {
            assert!(!sst.contains("<rPh"), "{sst}");
            assert!(read_xlsx(&bytes).unwrap().0.phonetics.is_empty());
        } else {
            assert!(sst.contains(r#"<rPh sb="0" eb="3"><t>clamped</t></rPh>"#), "{sst}");
            assert_eq!(read_xlsx(&bytes).unwrap().0.phonetics[0].runs, expected);
        }
    }
    for (kind, alignment, loss) in [
        ("halfwidthKatakana", "noControl", false),
        ("fullwidthKatakana", "left", false),
        ("Hiragana", "center", false),
        ("noConversion", "distributed", false),
        ("unknown", "center", true),
        ("Hiragana", "unknown", true),
        (" Hiragana", "left ", true),
    ] {
        let mut wb = Workbook::new();
        text_cell(wb.sheet_mut(0).unwrap(), "A1", "東😀");
        let props = PhoneticProps { font_id: 0, kind: Some(kind.into()), alignment: Some(alignment.into()) };
        wb.phonetics = vec![PhoneticString { props: Some(props.clone()), ..normal.clone() }];
        assert_eq!(crate::xlsx_known_phonetic_loss(&wb).unwrap(), loss);
        let bytes = write_xlsx(&wb).unwrap();
        let back = read_xlsx(&bytes).unwrap().0;
        if loss {
            assert_ne!(back.phonetics[0].props, Some(props));
            let sst = part(&bytes, "xl/sharedStrings.xml");
            assert!(!sst.contains("unknown") && !sst.contains(" Hiragana") && !sst.contains("left "));
        } else {
            assert_eq!(back.phonetics, wb.phonetics);
        }
    }
    let mut unused = Workbook::new();
    unused.phonetics = vec![normal];
    assert!(!crate::xlsx_known_phonetic_loss(&unused).unwrap(), "used-SST membership is explicitly outside this diagnostic");
    assert!(read_xlsx(&write_xlsx(&unused).unwrap()).unwrap().0.phonetics.is_empty());
}

#[test]
fn phonetic_diagnostic_index_budget_and_arithmetic_fail_closed() {
    use crate::write::{check_phonetic_index_bytes, known_phonetic_loss_with_budget};
    let width = std::mem::size_of::<usize>();
    let limit = 8 * 1024 * 1024;
    assert!(check_phonetic_index_bytes(limit / width, usize::MAX).is_ok());
    for (count, budget) in [(limit / width + 1, usize::MAX), (usize::MAX, usize::MAX), (1, width - 1)] {
        assert!(matches!(check_phonetic_index_bytes(count, budget), Err(crate::IoError::TooLarge(_))));
    }
    let normal = PhoneticString { text: "東".into(), runs: vec![PhoneticRun { start: 0, end: 1, text: "reading".into() }], props: None };
    assert!(!known_phonetic_loss_with_budget(&[normal.clone(), normal.clone()], 2 * width).unwrap());
    assert!(matches!(known_phonetic_loss_with_budget(&[normal], width - 1), Err(crate::IoError::TooLarge(_))));
    assert!(!known_phonetic_loss_with_budget(&[], 0).unwrap());
}

fn membership_book() -> Workbook {
    let mut wb = Workbook::new();
    text_cell(wb.sheet_mut(0).unwrap(), "A1", "東");
    wb.phonetics = vec![PhoneticString { text: "東".into(), runs: vec![PhoneticRun { start: 0, end: 1, text: "ヒガシ".into() }], props: None }];
    wb
}
fn membership_table(names: &[&str]) -> gridcraft_model::Table {
    use gridcraft_model::{Table, TableColumn, TotalsFn};
    Table {
        id: 1,
        name: "Data".into(),
        range: gridcraft_core::RangeRef::new(at("A1"), CellRef::new(1, names.len() as u32 - 1)),
        header_row: true,
        totals_row: false,
        columns: names
            .iter()
            .map(|s| TableColumn { name: (*s).into(), totals: TotalsFn::None, totals_label: None, formula: None, unique_name: None, xml: None })
            .collect(),
        style: "TableStyleLight1".into(),
        banded_rows: true,
        banded_cols: false,
        first_col: false,
        last_col: false,
        filter_button: true,
        filter: vec![],
        table_type: None,
        connection_id: None,
    }
}
fn membership_cache_cases(cases: &mut Vec<(&'static str, Workbook, bool, &'static str)>) {
    use gridcraft_core::RangeRef;
    use gridcraft_model::{Formula, PyCode, WhatIfTable};
    use std::sync::Arc;
    for name in ["formula", "dynamic", "legacy", "python", "formula_python", "what_if"] {
        let mut wb = membership_book();
        let sh = wb.sheet_mut(0).unwrap();
        let mut cell = Cell::value(Value::text("東"));
        if name != "python" && name != "what_if" {
            let mut f = Formula::new("\"東\"");
            f.dynamic = name == "dynamic";
            if name == "legacy" {
                f.array = Some(RangeRef::parse("A1:A2").unwrap());
            }
            cell.formula = Some(Arc::new(f));
        }
        if name == "python" || name == "formula_python" {
            cell.py = Some(Arc::new(PyCode::new("'東'")));
        }
        if name == "what_if" {
            sh.what_if_tables.push(WhatIfTable {
                anchor: at("A1"),
                range: RangeRef::parse("A1:A2").unwrap(),
                formula_text: String::new(),
                input1: Some(at("D1")),
                input2: None,
                two_variable: None,
                row_oriented: None,
                input1_deleted: None,
                input2_deleted: None,
                calculate: None,
            });
        }
        sh.cells.set(at("A1"), cell);
        cases.push((name, wb.clone(), false, "str"));
        wb.sheet_mut(0).unwrap().tables.push(membership_table(&["東"]));
        cases.push((
            match name {
                "formula" => "header_formula",
                "dynamic" => "header_dynamic",
                "legacy" => "header_legacy",
                "python" => "header_python",
                "formula_python" => "header_both",
                _ => "header_what_if",
            },
            wb,
            name != "what_if",
            if name == "what_if" { "str" } else { "s" },
        ));
    }
}

fn membership_cases() -> Vec<(&'static str, Workbook, bool, &'static str)> {
    use gridcraft_core::{Array, CellError, RangeRef};
    use gridcraft_model::{Formula, PyCode};
    use std::sync::Arc;
    let mut cases = vec![("literal", membership_book(), true, "s")];
    for (name, value) in [
        ("unused", Value::Number(1.0)),
        ("boolean", Value::Bool(true)),
        ("error", Value::Error(CellError::Calc)),
        ("nonleading", Array::row(vec![Value::Number(1.0), Value::text("東")]).into()),
        ("two_stage", Array::scalar(Array::scalar(Value::text("東")).into()).into()),
        ("too_deep", Array::scalar(Array::scalar(Array::scalar(Value::text("東")).into()).into()).into()),
        ("empty_array", Value::Array(Arc::new(Array { rows: 0, cols: 0, data: vec![] }))),
    ] {
        let mut wb = membership_book();
        wb.sheet_mut(0).unwrap().cells.set(at("A1"), Cell::value(value));
        cases.push((
            name,
            wb,
            name == "two_stage",
            if name == "two_stage" {
                "s"
            } else if name == "error" {
                "e"
            } else {
                ""
            },
        ));
    }
    membership_cache_cases(&mut cases);
    for (name, names, expected) in [
        ("header_override", vec!["Other"], false),
        ("header_generated", vec!["東"], true),
        ("blank", vec![" "], false),
        ("duplicate", vec!["x", "X"], false),
        ("unicode", vec!["ΟΣ", "ος", "İ", "i\u{307}"], false),
    ] {
        let mut wb = membership_book();
        wb.sheet_mut(0).unwrap().tables.push(membership_table(&names));
        cases.push((name, wb, expected, "s"));
    }
    for (name, header, totals, end) in [("inactive_header", false, false, 1), ("rejected_table", false, true, 0)] {
        let mut wb = membership_book();
        let mut t = membership_table(&["Other"]);
        t.header_row = header;
        t.totals_row = totals;
        t.range.end.row = end;
        wb.sheet_mut(0).unwrap().tables.push(t);
        cases.push((name, wb, true, "s"));
    }
    let mut wb = membership_book();
    wb.sheet_mut(0).unwrap().tables.extend([membership_table(&["東"]), membership_table(&["Other"])]);
    cases.push(("last_header", wb, false, "s"));
    for name in [
        "spill",
        "spill_dynamic",
        "spill_legacy",
        "spill_single",
        "spill_empty",
        "spill_python",
        "spill_blocked",
        "spill_formula",
        "spill_outside",
        "spill_anchor",
        "spill_header",
        "spill_overlap",
    ] {
        let mut wb = membership_book();
        let sh = wb.sheet_mut(0).unwrap();
        sh.cells.set(at("A1"), Cell::value(Value::Number(1.0)));
        sh.spill_ranges.insert(at("A1"), RangeRef::parse("A1:A3").unwrap());
        if name == "spill_single" {
            sh.spill_ranges.insert(at("A1"), RangeRef::cell(at("A2")));
        }
        if name == "spill_dynamic" || name == "spill_legacy" {
            let mut f = Formula::new("1");
            if name == "spill_legacy" {
                f.array = Some(RangeRef::parse("A1:A3").unwrap());
            }
            sh.cells.set(at("A1"), Cell { formula: Some(Arc::new(f)), value: Value::Number(1.0), ..Cell::default() });
        }
        sh.spill.insert(
            if name == "spill_outside" {
                at("A4")
            } else if name == "spill_anchor" {
                at("A1")
            } else {
                at("A2")
            },
            Value::text("東"),
        );
        if name == "spill_empty" || name == "spill_python" {
            let mut c = Cell::default();
            if name == "spill_python" {
                c.py = Some(Arc::new(PyCode::new("1")));
            }
            sh.cells.set(at("A2"), c);
        }
        if name == "spill_blocked" {
            text_cell(sh, "A2", "Other");
        }
        if name == "spill_formula" {
            sh.cells.set(at("A2"), Cell::formula(Formula::new("1")));
        }
        if name == "spill_header" {
            let mut t = membership_table(&["Other"]);
            t.range = RangeRef::parse("A2:A3").unwrap();
            sh.tables.push(t);
        }
        if name == "spill_overlap" {
            sh.spill_ranges.insert(at("B1"), RangeRef::parse("A1:B3").unwrap());
        }
        cases.push((name, wb, matches!(name, "spill" | "spill_dynamic" | "spill_legacy" | "spill_empty" | "spill_overlap"), ""));
    }
    // Original one-pixel green PNG generated from raw RGB bytes; no external asset.
    let picture = Arc::new(gridcraft_model::CellPicture {
        data: vec![
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0, 144, 119, 83, 222, 0, 0, 0, 12, 73,
            68, 65, 84, 120, 156, 99, 144, 89, 229, 1, 0, 1, 244, 1, 15, 24, 202, 173, 154, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
        ],
        mime: "image/png".into(),
        alt: "original synthetic picture".into(),
    });
    for name in ["picture", "picture_header", "picture_spill"] {
        let mut wb = membership_book();
        let sh = wb.sheet_mut(0).unwrap();
        sh.cell_pictures.insert(at("A1"), picture.clone());
        if name == "picture_header" {
            sh.tables.push(membership_table(&["東"]));
        }
        if name == "picture_spill" {
            sh.spill_ranges.insert(at("B1"), RangeRef::parse("A1:B2").unwrap());
            sh.spill.insert(at("A1"), Value::text("東"));
            sh.cells.set(at("A1"), Cell::default());
        }
        cases.push((name, wb, false, "e"));
    }
    for (name, text) in [("suffix_generated", "X2"), ("blank_generated", "Column1")] {
        let mut wb = membership_book();
        wb.phonetics[0].text = text.into();
        let names = if name == "suffix_generated" { vec!["x", "X"] } else { vec![" "] };
        wb.sheet_mut(0).unwrap().tables.push(membership_table(&names));
        cases.push((name, wb, true, "s"));
    }
    let mut wb = membership_book();
    wb.phonetics[0] = PhoneticString { text: "".into(), runs: vec![], props: Some(PhoneticProps::default()) };
    text_cell(wb.sheet_mut(0).unwrap(), "A1", "");
    cases.push(("empty_literal", wb, true, "s"));
    cases
}

#[test]
fn phonetic_sst_membership_current_writer_precedence() {
    use gridcraft_model::{Sheet, Visibility};
    // Literal expectations catch a raw-cell scan, cached-text scan, recursive scalarization,
    // wrong header/picture/what-if precedence, and incorrect spill eligibility.
    for (name, original, emitted, kind) in membership_cases() {
        for global in [false, true] {
            let mut wb = original.clone();
            if global {
                let mut sh = Sheet::new("Hidden match");
                sh.visibility = if name.len() % 2 == 0 { Visibility::Hidden } else { Visibility::VeryHidden };
                text_cell(&mut sh, "B3", &wb.phonetics[0].text);
                wb.sheets.push(sh.into());
            }
            let bytes = write_xlsx(&wb).unwrap();
            let sst = crate::xml::parse(part(&bytes, "xl/sharedStrings.xml").as_bytes()).unwrap();
            let texts: Vec<_> = sst.kids("si").map(crate::xml::rich_text).collect();
            if name == "unicode" && !global {
                assert_eq!(texts, ["ΟΣ", "ος2", "İ", "i\u{307}2"]);
            }
            assert_eq!(texts.contains(&wb.phonetics[0].text), emitted || global, "{name}/{global}: {texts:?}");
            if !kind.is_empty() {
                let xml = part(&bytes, "xl/worksheets/sheet1.xml");
                let root = crate::xml::parse(xml.as_bytes()).unwrap();
                let a1 = root.child("sheetData").unwrap().kids("row").flat_map(|r| r.kids("c")).find(|c| c.attr("r") == Some("A1")).unwrap();
                assert_eq!(a1.attr("t"), Some(kind), "{name}: {xml}");
            }
            assert_eq!(read_xlsx(&bytes).unwrap().0.phonetics, if emitted || global { wb.phonetics.clone() } else { vec![] }, "{name}/{global}");
            // Optional owned checkpoint capture: every decompressed member, no normalization.
            if let Some(dir) = std::env::var_os("GRIDCRAFT_D2_MEMBERS_DIR") {
                let root = std::path::PathBuf::from(dir).join(format!("{name}-{global}"));
                std::fs::create_dir(&root).unwrap();
                std::fs::write(root.join("original.zip"), &bytes).unwrap();
                let mut pkg = Package::open(&bytes).unwrap();
                for member in pkg.names() {
                    let path = root.join(&member);
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(path, pkg.read(&member).unwrap().unwrap()).unwrap();
                }
            }
        }
    }
}

#[test]
fn phonetic_shared_text_visitor_matches_real_sst_without_mutation() {
    use crate::sheet_write::{TextPlanningBudget, visit_shared_text};
    use std::collections::BTreeSet;
    for (name, wb, _, _) in membership_cases() {
        let before = wb.clone();
        let mut seen = BTreeSet::new();
        visit_shared_text(&wb, &mut TextPlanningBudget::default(), |t, _| {
            seen.insert(t.to_owned());
            Ok(false)
        })
        .unwrap();
        let bytes = write_xlsx(&wb).unwrap();
        let root = crate::xml::parse(part(&bytes, "xl/sharedStrings.xml").as_bytes()).unwrap();
        let real: BTreeSet<_> = root.kids("si").map(crate::xml::rich_text).collect();
        assert_eq!(seen, real, "{name}");
        assert_eq!(wb, before, "{name}: visitation must be read-only");
    }
}

#[test]
fn phonetic_membership_boundaries_fail_closed() {
    use crate::sheet_write::{TextPlanningBudget, visit_shared_text};
    use crate::write::{phonetic_bit_bytes, phonetic_loss_with_budget};
    assert_eq!(phonetic_bit_bytes(0).unwrap(), 0);
    assert_eq!(phonetic_bit_bytes(8).unwrap(), 1);
    assert_eq!(phonetic_bit_bytes(9).unwrap(), 2);
    assert_eq!(phonetic_bit_bytes(8 * 1024 * 1024 / 8).unwrap(), 131_072);
    assert_eq!(phonetic_bit_bytes(8 * 1024 * 1024 / 4).unwrap(), 262_144);
    assert!(matches!(phonetic_bit_bytes(usize::MAX), Err(crate::IoError::TooLarge(_))));
    let wb = membership_book();
    for (steps, text, ok) in [(2, 3, true), (1, 3, false), (2, 2, false)] {
        let result = phonetic_loss_with_budget(&wb, &mut TextPlanningBudget { bytes: 0, steps, text });
        if ok {
            assert!(!result.unwrap());
        } else {
            assert!(matches!(result, Err(crate::IoError::TooLarge(_))));
        }
    }
    let mut duplicate = membership_book();
    duplicate.phonetics.push(duplicate.phonetics[0].clone());
    assert!(matches!(
        phonetic_loss_with_budget(&duplicate, &mut TextPlanningBudget { bytes: 0, steps: 2, text: 5 }),
        Err(crate::IoError::TooLarge(_))
    ));
    assert!(!phonetic_loss_with_budget(&duplicate, &mut TextPlanningBudget { bytes: 0, steps: 2, text: 6 }).unwrap());
    let mut wb = membership_book();
    wb.sheet_mut(0).unwrap().tables.push(membership_table(&["x"]));
    // One x header: table/column Vecs, two rounded maps, base and lowercase buffers.
    // Pinned 64/32-bit logical peaks are hand-derived; work=8 visits, text=1+1+80+2.
    let peak = if usize::BITS == 64 { 401 } else { 261 };
    for (bytes, steps, text, ok) in [(peak, 8, 84, true), (peak - 1, 8, 84, false), (peak, 7, 84, false), (peak, 8, 83, false)] {
        let mut budget = TextPlanningBudget { bytes, steps, text };
        let result = visit_shared_text(&wb, &mut budget, |_, _| Ok(false));
        if ok {
            result.unwrap();
            assert_eq!(budget.bytes, peak);
        } else {
            assert!(matches!(result, Err(crate::IoError::TooLarge(_))));
        }
    }
    wb.sheets.push(wb.sheets[0].clone());
    visit_shared_text(&wb, &mut TextPlanningBudget { bytes: peak, steps: 16, text: 168 }, |_, _| Ok(false)).unwrap();
    for range in [
        gridcraft_core::RangeRef { start: CellRef::new(0, 0), end: CellRef::new(1, u32::MAX) },
        gridcraft_core::RangeRef { start: at("B1"), end: at("A2") },
    ] {
        let mut wb = membership_book();
        let mut t = membership_table(&["x"]);
        t.range = range;
        wb.sheet_mut(0).unwrap().tables.push(t);
        assert!(matches!(visit_shared_text(&wb, &mut TextPlanningBudget::default(), |_, _| Ok(false)), Err(crate::IoError::TooLarge(_))));
    }
    let mut wb = Workbook::new();
    for anchor in [at("A1"), at("B1")] {
        wb.sheet_mut(0).unwrap().spill_ranges.insert(anchor, gridcraft_core::RangeRef::parse("A1:A3").unwrap());
    }
    visit_shared_text(&wb, &mut TextPlanningBudget { bytes: 0, steps: 9, text: 0 }, |_, _| panic!("no sparse payload exists")).unwrap();
    assert!(matches!(
        visit_shared_text(&wb, &mut TextPlanningBudget { bytes: 0, steps: 8, text: 0 }, |_, _| Ok(false)),
        Err(crate::IoError::TooLarge(_))
    ));
    wb.phonetics = vec![PhoneticString { text: "unused".into(), runs: vec![], props: None }];
    assert!(!phonetic_loss_with_budget(&wb, &mut TextPlanningBudget { bytes: 0, steps: 0, text: 0 }).unwrap());
    wb.phonetics[0].runs = vec![PhoneticRun { start: 0, end: 99, text: "clamped".into() }];
    assert!(phonetic_loss_with_budget(&wb, &mut TextPlanningBudget { bytes: 0, steps: 0, text: 0 }).unwrap());
}

#[test]
fn phonetic_membership_matches_winners_and_exact_unicode() {
    for (name, wb, emitted, _) in membership_cases() {
        assert_eq!(crate::xlsx_phonetic_loss(&wb).unwrap(), !emitted, "{name}");
    }
    for overlap in [false, true] {
        let mut wb = Workbook::new();
        wb.phonetics = membership_book().phonetics;
        if overlap {
            wb.sheet_mut(0).unwrap().tables.push(membership_table(&["Other"]));
        }
        wb.sheet_mut(0).unwrap().tables.push(membership_table(&["東"]));
        assert!(!crate::xlsx_phonetic_loss(&wb).unwrap());
        assert_eq!(read_xlsx(&write_xlsx(&wb).unwrap()).unwrap().0.phonetics, wb.phonetics);
    }
    let mut wb = Workbook::new();
    for i in 0..9 {
        let text = format!("p{i}");
        text_cell(wb.sheet_mut(0).unwrap(), &format!("A{}", i + 1), &text);
        wb.phonetics.push(PhoneticString { text, runs: vec![], props: Some(PhoneticProps::default()) });
    }
    wb.phonetics.push(wb.phonetics[8].clone());
    wb.phonetics.push(PhoneticString { text: "empty wrapper".into(), runs: vec![], props: None });
    assert!(!crate::xlsx_phonetic_loss(&wb).unwrap());
    text_cell(wb.sheet_mut(0).unwrap(), "A9", "different");
    assert!(crate::xlsx_phonetic_loss(&wb).unwrap());
    for text in ["é", "E"] {
        let mut wb = membership_book();
        wb.phonetics[0].text = text.into();
        text_cell(wb.sheet_mut(0).unwrap(), "A1", if text == "é" { "e\u{301}" } else { "e" });
        assert!(crate::xlsx_phonetic_loss(&wb).unwrap(), "membership is exact, not folded/normalized");
    }
}

#[test]
fn phonetic_real_million_position_spill_prefix() {
    for inside in [false, true] {
        let mut wb = membership_book();
        let sh = wb.sheet_mut(0).unwrap();
        sh.cells.set(at("A1"), Cell::value(Value::Number(1.0)));
        sh.spill_ranges.insert(at("A1"), gridcraft_core::RangeRef::parse("A1:A1000001").unwrap());
        sh.spill.insert(at(if inside { "A1000000" } else { "A1000001" }), Value::text("東"));
        assert_eq!(sh.cells.len(), 1);
        assert_eq!(sh.spill.len(), 1);
        let bytes = write_xlsx(&wb).unwrap();
        assert_eq!(read_xlsx(&bytes).unwrap().0.phonetics, if inside { wb.phonetics.clone() } else { vec![] });
        assert_eq!(crate::xlsx_phonetic_loss(&wb).unwrap(), !inside);
        let mut budget = crate::sheet_write::TextPlanningBudget { bytes: 0, steps: 1_000_003, text: 3 };
        assert_eq!(crate::write::phonetic_loss_with_budget(&wb, &mut budget).unwrap(), !inside);
        assert!(matches!(
            crate::write::phonetic_loss_with_budget(&wb, &mut crate::sheet_write::TextPlanningBudget { bytes: 0, steps: 1_000_002, text: 3 }),
            Err(crate::IoError::TooLarge(_))
        ));
    }
}

#[test]
fn phonetic_planning_counts_skipped_candidates_and_capacity_requests() {
    use crate::sheet_write::{TextPlanningBudget, visit_shared_text};
    let mut wb = membership_book();
    wb.sheet_mut(0).unwrap().tables.push(membership_table(&["Total", "total3", "Total", "Total"]));
    for (steps, ok) in [(20, true), (19, false)] {
        let result = visit_shared_text(&wb, &mut TextPlanningBudget { steps, ..TextPlanningBudget::default() }, |_, _| Ok(false));
        if ok {
            result.unwrap();
        } else {
            assert!(matches!(result, Err(crate::IoError::TooLarge(_))));
        }
    }
    let mut wb = Workbook::new();
    let mut t = membership_table(&["ignored"]);
    t.header_row = false;
    t.totals_row = true;
    t.range.end.row = 0;
    wb.sheet_mut(0).unwrap().tables.push(t);
    visit_shared_text(
        &wb,
        &mut TextPlanningBudget { bytes: std::mem::size_of::<(&gridcraft_model::Table, Vec<String>)>(), steps: 2, text: 0 },
        |_, _| panic!("rejected table emits nothing"),
    )
    .unwrap();
    assert!(matches!(
        visit_shared_text(&wb, &mut TextPlanningBudget { steps: 1, ..TextPlanningBudget::default() }, |_, _| Ok(false)),
        Err(crate::IoError::TooLarge(_))
    ));
    for width in [0usize, 1, 3, 4, 7, 8, 10_000] {
        let mut used = std::collections::HashSet::<String>::new();
        let mut next = std::collections::HashMap::<String, u32>::new();
        let mut columns = Vec::<String>::new();
        used.try_reserve(width).unwrap();
        next.try_reserve(width).unwrap();
        columns.try_reserve_exact(width).unwrap();
        assert!(used.capacity() >= width && next.capacity() >= width && columns.capacity() >= width);
        assert!(used.capacity() <= width * 4 && next.capacity() <= width * 4);
        println!(
            "planning request width={width} used_capacity={} next_capacity={} column_capacity={} rounded_map_request={}",
            used.capacity(),
            next.capacity(),
            columns.capacity(),
            4 * width * (std::mem::size_of::<String>() + std::mem::size_of::<(String, u32)>() + 2) + 32
        );
    }
    for input in ["", "x", "ΟΣ", "İ", "i\u{307}"] {
        let lower = input.to_lowercase();
        assert!(lower.capacity() <= (24 * input.len()).max(8));
        println!(
            "lowercase input_bytes={} output_bytes={} actual_capacity={} conservative_precharge={}",
            input.len(),
            lower.len(),
            lower.capacity(),
            48 * input.len() + 32
        );
    }
}
