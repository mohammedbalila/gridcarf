#![allow(clippy::unwrap_used)]
use std::io::Write as _;
use std::sync::Arc;

use gridcraft_engine::model::Formula;

use super::*;

#[test]
fn discovery_reports_a_directory_read_error() {
    let path = std::env::temp_dir().join(format!("gridcraft-check-not-a-directory-{}", std::process::id()));
    std::fs::write(&path, "not a directory").unwrap();
    let mut discovery = Discovery::default();
    let mut seen = std::collections::HashSet::new();
    assert!(walk(&path, 0, &mut discovery, &mut seen));
    assert!(discovery.files.is_empty());
    assert_eq!(discovery.failures.len(), 1);
    assert_eq!(discovery.failures[0].path, path.to_string_lossy());
    assert!(discovery.failures[0].error.as_ref().unwrap().contains("cannot read directory"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn discovery_reports_omissions_only_after_the_file_limit() {
    let mut discovery = Discovery { files: vec![PathBuf::from("book.xlsx"); MAX_FILES - 1], ..Default::default() };
    assert!(discovery.add_file(PathBuf::from("last.xlsx")));
    assert!(discovery.failures.is_empty());
    assert!(!discovery.add_file(PathBuf::from("omitted.xlsx")));
    assert_eq!(discovery.files.len(), MAX_FILES);
    assert_eq!(discovery.failures[0].path, "omitted.xlsx");
    assert!(discovery.failures[0].error.as_ref().unwrap().contains("file limit"));
}

fn at(a1: &str) -> CellRef {
    CellRef::parse(a1).unwrap()
}

/// A workbook whose cells are `(A1, input, cached value)`: an input starting with `=` is a
/// formula with that cached value, anything else a constant.
fn book(cells: &[(&str, &str, Value)]) -> Workbook {
    let mut wb = Workbook::new();
    let sh = wb.sheet_mut(0).unwrap();
    for (a1, input, cached) in cells {
        let cell = if input.starts_with('=') {
            Cell { value: cached.clone(), formula: Some(Arc::new(Formula::new(input))), ..Default::default() }
        } else {
            Cell::value(cached.clone())
        };
        sh.set_cell(at(a1), cell);
    }
    wb
}

fn check(wb: &Workbook, opts: &Options) -> FileReport {
    let bytes = gridcraft_engine::io::save_bytes(wb, "t.xlsx").unwrap();
    check_bytes("t.xlsx", &bytes, opts)
}

fn n(x: f64) -> Value {
    Value::number(x)
}

#[test]
fn matching_cached_values_pass() {
    let wb = book(&[("A1", "2", n(2.0)), ("B1", "=A1*3", n(6.0)), ("C1", "=A1&\"x\"", Value::text("2x")), ("D1", "=A1>1", Value::Bool(true))]);
    let r = check(&wb, &Options::default());
    assert_eq!(r.error, None);
    assert_eq!((r.calc.formula_cells, r.calc.checked, r.calc.matched, r.calc.mismatched), (3, 3, 3, 0));
    assert!(r.is_clean(), "{r:?}");
}

#[test]
fn wrong_cached_value_is_a_mismatch_and_ranks_its_function() {
    // Excel "said" SUM gives 7; GridCraft computes 6. C1 reads B1 and is consistent with
    // Excel's 7, so it mismatches too, but only as a downstream effect.
    let wb = book(&[("A1", "2", n(2.0)), ("A2", "4", n(4.0)), ("B1", "=SUM(A1:A2)", n(7.0)), ("C1", "=B1+1", n(8.0))]);
    let r = check(&wb, &Options::default());
    assert_eq!((r.calc.checked, r.calc.mismatched), (2, 2));
    let first = &r.calc.mismatches[0];
    assert_eq!(first.cell, "Sheet1!B1");
    assert_eq!(first.formula, "=SUM(A1:A2)");
    assert_eq!((first.expected.clone(), first.actual.clone(), first.kind, first.primary), (n(7.0), n(6.0), "numeric", true));
    assert!(!r.calc.mismatches[1].primary);
    assert_eq!(r.calc.functions["SUM"], FnStat { checked: 1, mismatched: 1, primary: 1 });
    assert_eq!(r.calc.functions["(no function)"], FnStat { checked: 1, mismatched: 1, primary: 0 });
    assert!(!r.is_clean());
}

#[test]
fn errors_compare_by_kind() {
    let wb = book(&[("A1", "=1/0", Value::Error(CellError::Div0)), ("A2", "=1/0", Value::Error(CellError::NA)), ("A3", "=1/0", n(0.0))]);
    let r = check(&wb, &Options::default());
    assert_eq!((r.calc.matched, r.calc.mismatched), (1, 2));
    assert!(r.calc.mismatches.iter().all(|m| m.kind == "error"));
    assert_eq!(r.calc.signatures.get("#N/A → #DIV/0!"), Some(&1));
    assert_eq!(r.calc.signatures.get("number → #DIV/0!"), Some(&1));
    assert!(values_match(&Value::Error(CellError::NA), &Value::Error(CellError::NA), 0.0));
    assert!(!values_match(&Value::Error(CellError::NA), &Value::Error(CellError::Value), 0.0));
}

#[test]
fn numbers_compare_within_tolerance() {
    assert!(values_match(&n(3.0000000001), &n(3.0), 1e-9));
    assert!(!values_match(&n(3.0000000001), &n(3.0), 1e-12));
    // Absolute near zero.
    assert!(values_match(&n(1e-12), &n(0.0), 1e-9));
    assert!(values_match(&n(1e12 + 1.0), &n(1e12), 1e-9));
    assert!(!values_match(&n(0.0), &Value::Empty, 1.0));
    assert!(!values_match(&Value::text("1"), &n(1.0), 1.0));
    assert!(values_match(&Value::text("a"), &Value::text("a"), 0.0));
    assert!(!values_match(&Value::text("a"), &Value::text("A"), 0.0));
    assert!(!values_match(&Value::Bool(true), &n(1.0), 1.0));
    let wb = book(&[("A1", "=1+2", n(3.0000000001))]);
    assert_eq!(check(&wb, &Options::default()).calc.matched, 1);
    assert_eq!(check(&wb, &Options { tolerance: 1e-12, ..Default::default() }).calc.mismatched, 1);
}

/// Business Central writes TRUE and FALSE as `<c><f>TRUE()</f><v>1</v></c>`, without `t="b"`:
/// the cache holds the right value under a number type, so it matches and is counted apart.
#[test]
fn booleans_cached_as_numbers_match_and_are_counted_apart() {
    let wb = book(&[
        ("A1", "=TRUE()", n(1.0)),
        ("B1", "=FALSE()", n(0.0)),
        ("C1", "=TRUE()", Value::Bool(true)),
        ("D1", "=1=1", n(0.0)),
        ("E1", "=TRUE()+0", n(1.0)),
    ]);
    let r = check(&wb, &Options::default());
    assert_eq!((r.calc.checked, r.calc.matched, r.calc.mismatched), (5, 4, 1));
    // A cached 0 for TRUE is still a mismatch, and a number result is compared as a number.
    assert_eq!(r.calc.mismatches[0].cell, "Sheet1!D1");
    assert_eq!(r.calc.signatures.get("number → bool"), Some(&1));
    assert_eq!(r.calc.bool_as_number, 2);
    let v = report_json(std::slice::from_ref(&r), &Options::default());
    assert_eq!((v["files"][0]["calc"]["boolAsNumber"].clone(), v["totals"]["boolAsNumber"].clone()), (json!(2), json!(2)));
    assert!(bool_cached_as_number(&n(1.0), &Value::Bool(true)));
    assert!(!bool_cached_as_number(&n(1.0), &Value::Bool(false)));
    assert!(!bool_cached_as_number(&n(2.0), &Value::Bool(true)));
    assert!(!bool_cached_as_number(&Value::Bool(true), &n(1.0)));
}

#[test]
fn missing_cached_values_and_volatile_formulas_are_skipped() {
    let wb =
        book(&[("A1", "=1+1", Value::Empty), ("B1", "=NOW()", n(1.0)), ("B2", "=B1+1", n(2.0)), ("B3", "=CELL(\"filename\")", Value::text("x"))]);
    let r = check(&wb, &Options::default());
    assert_eq!((r.calc.formula_cells, r.calc.checked, r.calc.no_oracle, r.calc.volatile), (4, 0, 1, 3));
}

#[test]
fn full_calc_on_load_means_no_oracle() {
    // A formula without a cached value makes GridCraft's writer flag the file
    // `fullCalcOnLoad`, like libraries that store 0 for every formula: B1's wrong "cached"
    // value must not count as a mismatch then.
    let wb = book(&[("A1", "=1+1", Value::Empty), ("B1", "=2+2", n(0.0))]);
    let bytes = gridcraft_engine::io::save_bytes(&wb, "t.xlsx").unwrap();
    assert!(full_calc_on_load(&bytes));
    let r = check_bytes("t.xlsx", &bytes, &Options::default());
    assert!(r.calc.stale_cache);
    assert_eq!((r.calc.checked, r.calc.no_oracle, r.calc.mismatched), (0, 2, 0));
    let fresh = gridcraft_engine::io::save_bytes(&book(&[("B1", "=2+2", n(0.0))]), "t.xlsx").unwrap();
    assert!(!full_calc_on_load(&fresh));
    assert!(!full_calc_on_load(b"not a zip"));
}

#[test]
fn volatile_names_propagate() {
    let mut wb = book(&[("A1", "=Stamp+1", n(1.0))]);
    wb.names.push(gridcraft_engine::model::DefinedName {
        name: "Stamp".into(),
        scope: None,
        formula: "TODAY()".into(),
        comment: String::new(),
        hidden: false,
    });
    assert!(volatile_names(&wb).contains("STAMP"));
    assert_eq!(check(&wb, &Options::default()).calc.volatile, 1);
}

#[test]
fn volatile_names_propagate_through_a_long_chain_in_one_walk() {
    // Chain0 uses Chain1, Chain1 uses Chain2, ... and only the last is volatile: listed in this order, a rescan
    // until nothing changes marked one name per pass (quadratic). Every name in the chain is
    // volatile; a name off the chain is not.
    const LEN: usize = 5_000;
    let mut wb = book(&[("A1", "=1", n(1.0))]);
    let name =
        |name: String, formula: String| gridcraft_engine::model::DefinedName { name, scope: None, formula, comment: String::new(), hidden: false };
    for i in 0..LEN {
        let formula = if i + 1 == LEN { "NOW()".to_string() } else { format!("Chain{}+1", i + 1) };
        wb.names.push(name(format!("Chain{i}"), formula));
    }
    wb.names.push(name("Plain".into(), "Chain0*0+Other".into()));
    wb.names.push(name("Other".into(), "1".into()));
    let set = volatile_names(&wb);
    assert!((0..LEN).all(|i| set.contains(&format!("CHAIN{i}"))));
    assert!(set.contains("PLAIN"));
    assert!(!set.contains("OTHER"));
    assert_eq!(set.len(), LEN + 1);
}

#[test]
fn round_trip_of_a_simple_book_is_clean() {
    let mut wb = book(&[("A1", "2", n(2.0)), ("B1", "=A1*3", n(6.0)), ("A2", "x", Value::text("x"))]);
    wb.sheet_mut(0).unwrap().merges.push(gridcraft_engine::core::RangeRef::parse("C1:D2").unwrap());
    let source = gridcraft_engine::io::save_bytes(&wb, "t.xlsx").unwrap();
    let rt = round_trip(&wb, &source, &Options::default());
    assert_eq!(rt.error, None);
    assert!(rt.is_clean(), "{rt:?}");
    assert!(rt.categories.iter().any(|c| c.name == "package"));
}

#[test]
fn diff_detects_lost_features() {
    let mut a = book(&[("A1", "2", n(2.0)), ("B1", "=A1*3", n(6.0))]);
    a.sheet_mut(0).unwrap().merges.push(gridcraft_engine::core::RangeRef::parse("C1:D2").unwrap());
    let mut b = a.clone();
    let sh = b.sheet_mut(0).unwrap();
    sh.merges.clear();
    sh.set_cell(at("A1"), Cell::value(n(3.0)));
    sh.set_cell(at("B1"), Cell::value(n(6.0)));
    b.sheets.push(Arc::new(Sheet::new("Extra")));
    let cats = diff_workbooks(&a, &b, &Options { max_report: 1, ..Default::default() });
    let get = |name: &str| cats.iter().find(|c| c.name == name).unwrap();
    assert_eq!(get("values").diffs, 1);
    assert_eq!(get("values").examples, ["Sheet1!A1: 2 → 3"]);
    assert_eq!(get("formulas").examples, ["Sheet1!B1: =A1*3 → "]);
    assert_eq!(get("merges").examples, ["Sheet1: lost C1:D2"]);
    assert_eq!(get("sheets").diffs, 1);
    assert_eq!(get("tables").diffs, 0);
}

#[test]
fn diff_detects_changed_cond_format_rule() {
    use gridcraft_engine::model::{CfRule, CfValueKind, Color, CondFormat};
    let bar = |gradient: bool, priority: u32| CondFormat {
        ranges: vec![gridcraft_engine::core::RangeRef::parse("A1").unwrap()],
        rule: CfRule::DataBar { min: CfValueKind::Min, max: CfValueKind::Max, color: Color::Rgb(0x638EC6), gradient, show_value: true },
        priority,
        stop_if_true: false,
    };
    let with = |cf: CondFormat| {
        let mut wb = book(&[("A1", "2", n(2.0))]);
        wb.sheet_mut(0).unwrap().cond_formats.push(cf);
        wb
    };
    let diffs = |a: &Workbook, b: &Workbook| diff_workbooks(a, b, &Options::default()).into_iter().find(|c| c.name == "condFormats").unwrap().diffs;
    assert_eq!(diffs(&with(bar(false, 1)), &with(bar(true, 1))), 1);
    assert_eq!(diffs(&with(bar(true, 1)), &with(bar(true, 1))), 0);
    assert_eq!(diffs(&with(bar(true, 1)), &with(bar(true, 5))), 0);
}

fn zip_of(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, body) in parts {
        z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(body.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

#[test]
fn package_validation_finds_what_excel_would_repair() {
    let bytes = zip_of(&[
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="r"/><Override PartName="/xl/workbook.xml" ContentType="w"/><Override PartName="/xl/gone.xml" ContentType="g"/></Types>"#,
        ),
        ("_rels/.rels", r#"<Relationships><Relationship Id="rId1" Type="x/officeDocument" Target="xl/workbook.xml"/></Relationships>"#),
        ("xl/workbook.xml", "<workbook/>"),
        ("xl/styles.xml", "<styleSheet/>"),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<Relationships><Relationship Id="rId1" Type="x/styles" Target="styles.xml"/><Relationship Id="rId2" Type="x/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="x/hyperlink" Target="https://example.com" TargetMode="External"/><Relationship Id="rId3" Type="x/theme" Target="/xl/../xl/styles.xml"/></Relationships>"#,
        ),
    ]);
    let issues = validate_package(&bytes).unwrap();
    assert_eq!(
        issues,
        [
            "part xl/styles.xml has no content type",
            "content type override for missing part /xl/gone.xml",
            "xl/_rels/workbook.xml.rels: rId2 (worksheet) targets missing part xl/worksheets/sheet1.xml",
            "xl/_rels/workbook.xml.rels: duplicate relationship id rId2",
        ]
    );
    assert!(validate_package(b"not a zip").is_err());
    let own = gridcraft_engine::io::save_bytes(&book(&[("A1", "=1", n(1.0))]), "t.xlsx").unwrap();
    assert_eq!(validate_package(&own).unwrap(), Vec::<String>::new());
}

const WB_RELS: &str = "xl/_rels/workbook.xml.rels";
const SHEET_RELS: &str = "xl/worksheets/_rels/sheet1.xml.rels";
const MADE_UP: &str = r#"<Relationship Id="rId90" Type="http://example.com/relationships/madeUp" Target="madeUp.xml"/>"#;
/// Not empty: an empty part isn't a loss.
const MADE_UP_PART: (&str, &str) = ("xl/madeUp.xml", "<madeUp><item/></madeUp>");

/// `bytes` with relationships added to rels parts (created when missing) and `parts` added.
fn with_rels(bytes: &[u8], rels: &[(&str, &str)], parts: &[(&str, &str)]) -> Vec<u8> {
    let mut src = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut pending: Vec<(&str, &str)> = rels.to_vec();
    for i in 0..src.len() {
        let mut f = src.by_index(i).unwrap();
        let name = f.name().to_string();
        let mut body = String::new();
        f.read_to_string(&mut body).unwrap();
        if let Some(at) = pending.iter().position(|(r, _)| *r == name) {
            body = body.replace("</Relationships>", &format!("{}</Relationships>", pending.remove(at).1));
        }
        out.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        out.write_all(body.as_bytes()).unwrap();
    }
    let created = pending.iter().map(|(r, extra)| {
        (*r, format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{extra}</Relationships>"#))
    });
    for (name, body) in created.collect::<Vec<_>>().iter().map(|(n, b)| (*n, b.as_str())).chain(parts.iter().copied()) {
        out.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        out.write_all(body.as_bytes()).unwrap();
    }
    out.finish().unwrap().into_inner()
}

/// The `package` examples of checking `source`.
fn package_of(source: &[u8]) -> Vec<String> {
    let r = check_bytes("t.xlsx", source, &Options::default());
    r.round_trip.categories.iter().find(|c| c.name == "package").unwrap().examples.clone()
}

fn plain() -> Vec<u8> {
    gridcraft_engine::io::save_bytes(&book(&[("A1", "1", n(1.0))]), "t.xlsx").unwrap()
}

#[test]
fn package_reports_relationship_kinds_the_output_lost() {
    // A kind GridCraft doesn't know is dropped on save; a calculation chain is never written
    // (Excel rebuilds it), so its absence isn't a loss.
    let calc_chain =
        r#"<Relationship Id="rId91" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain" Target="calcChain.xml"/>"#;
    let source = with_rels(&plain(), &[(WB_RELS, &format!("{MADE_UP}{calc_chain}"))], &[MADE_UP_PART, ("xl/calcChain.xml", "<calcChain/>")]);
    assert_eq!(package_of(&source), ["lost relationship madeUp"]);
    assert!(check_bytes("t.xlsx", &plain(), &Options::default()).round_trip.is_clean());
}

// In the three tests below the made-up kind stays lost, so the comparison is known to run.

#[test]
fn an_internal_link_written_as_a_relationship_is_not_a_loss() {
    // openpyxl writes `#Sheet1!B2` as an external relationship; GridCraft writes `location`.
    let link = r##"<Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="#Sheet1!B2" TargetMode="External"/>"##;
    let source = with_rels(&plain(), &[(WB_RELS, MADE_UP), (SHEET_RELS, link)], &[MADE_UP_PART]);
    assert_eq!(package_of(&source), ["lost relationship madeUp"]);
}

#[test]
fn comments_without_a_comment_are_not_a_loss() {
    let rels = r#"<Relationship Id="rId6" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="../comments1.xml"/><Relationship Id="rId7" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/vmlDrawing" Target="../drawings/vmlDrawing1.vml"/>"#;
    let comments = r#"<comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><authors><author>A</author></authors><commentList/></comments>"#;
    let vml = r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office"><o:shapelayout><o:idmap data="1"/></o:shapelayout><v:shapetype id="_x0000_t202" path="m,l,21600r21600,l21600,xe"><v:path gradientshapeok="t"/></v:shapetype></xml>"#;
    let source = with_rels(
        &plain(),
        &[(WB_RELS, MADE_UP), (SHEET_RELS, rels)],
        &[MADE_UP_PART, ("xl/comments1.xml", comments), ("xl/drawings/vmlDrawing1.vml", vml)],
    );
    assert_eq!(package_of(&source), ["lost relationship madeUp"]);
}

#[test]
fn empty_drawings_and_person_lists_are_not_a_loss() {
    let drawing = r#"<Relationship Id="rId8" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/drawing1.xml"/>"#;
    let person = r#"<Relationship Id="rId92" Type="http://schemas.microsoft.com/office/2017/10/relationships/person" Target="persons/person.xml"/>"#;
    let source = with_rels(
        &plain(),
        &[(WB_RELS, &format!("{MADE_UP}{person}")), (SHEET_RELS, drawing)],
        &[
            MADE_UP_PART,
            ("xl/drawings/drawing1.xml", r#"<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing"/>"#),
            ("xl/persons/person.xml", r#"<personList xmlns="http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments"/>"#),
        ],
    );
    assert_eq!(package_of(&source), ["lost relationship madeUp"]);
}

#[test]
fn json_lists_warning_messages() {
    let with = FileReport { path: "a.ods".into(), warnings: vec!["ODS data-only import".into()], ..Default::default() };
    let without = FileReport { path: "b.xlsx".into(), ..Default::default() };
    let v = report_json(&[with, without], &Options::default());
    assert_eq!(v["files"][0]["warnings"], json!(["ODS data-only import"]));
    assert_eq!(v["files"][1]["warnings"], json!([]));
}

#[test]
fn unopenable_files_are_reported() {
    let r = check_bytes("broken.xlsx", b"PK\x03\x04 not really", &Options::default());
    assert!(r.error.is_some());
    assert!(!r.is_clean());
}

#[test]
fn json_schema_keys() {
    let wb = book(&[("A1", "=1+1", n(3.0))]);
    let reports = vec![check(&wb, &Options::default()), check_bytes("bad.xlsx", b"", &Options::default())];
    let v = report_json(&reports, &Options::default());
    let keys = |v: &Json| v.as_object().unwrap().keys().cloned().collect::<BTreeSet<String>>();
    let set = |k: &[&str]| k.iter().map(|s| s.to_string()).collect::<BTreeSet<String>>();
    assert_eq!(keys(&v), set(&["schema", "tolerance", "files", "totals", "functions", "signatures"]));
    assert_eq!(v["schema"], SCHEMA);
    let f = &v["files"][0];
    assert_eq!(keys(f), set(&["path", "opened", "error", "warnings", "calc", "roundTrip"]));
    assert_eq!(
        keys(&f["calc"]),
        set(&[
            "formulaCells",
            "checked",
            "matched",
            "mismatched",
            "noOracle",
            "staleCache",
            "volatile",
            "boolAsNumber",
            "mismatches",
            "functions",
            "signatures"
        ])
    );
    assert_eq!(f["calc"]["signatures"], json!({"number → number": 1}));
    assert_eq!(keys(&f["calc"]["mismatches"][0]), set(&["cell", "formula", "expected", "actual", "kind", "functions", "primary"]));
    assert_eq!(f["calc"]["mismatches"][0]["expected"], json!(3.0));
    assert_eq!(keys(&f["roundTrip"]), set(&["error", "differing", "categories", "sourcePackage"]));
    assert_eq!(
        keys(&f["roundTrip"]["categories"]),
        set(&[
            "sheets",
            "values",
            "formulas",
            "numFmts",
            "merges",
            "names",
            "tables",
            "condFormats",
            "validations",
            "charts",
            "comments",
            "hyperlinks",
            "pivots",
            "package",
            "slicers",
            "controls",
            "pictures",
            "shapes",
            "styles",
            "printSettings",
            "formulaModes"
        ])
    );
    assert_eq!(keys(&f["roundTrip"]["categories"]["values"]), set(&["equal", "diffs", "examples"]));
    assert_eq!(v["files"][1]["opened"], json!(false));
    assert_eq!(
        keys(&v["totals"]),
        set(&[
            "files",
            "opened",
            "failedToOpen",
            "formulaCells",
            "checked",
            "matched",
            "mismatched",
            "noOracle",
            "volatile",
            "boolAsNumber",
            "roundTripFailed",
            "roundTripDiffering",
            "categoriesDiffering"
        ])
    );
    assert_eq!(keys(&v["functions"][0]), set(&["name", "checked", "mismatched", "primary"]));
}

/// Build the audited slicer input through public commands before serialization.
fn slicer_book() -> Workbook {
    let mut session = gridcraft_engine::Session::new();
    session.new_workbook();
    session.execute("range.setValues", json!({"range":"A1", "values":[["Region","Sales"],["East",10],["West",20]]})).unwrap();
    session.execute("insert.table", json!({"range":"A1:B3","name":"SalesTable","header":true})).unwrap();
    session.execute("insert.slicer", json!({"table":"SalesTable","column":"Region","at":"D1"})).unwrap();
    (*session.doc().unwrap().wb).clone()
}

fn feature_book() -> Workbook {
    use gridcraft_engine::model::*;
    let mut wb = slicer_book();
    let sheet = wb.sheet_mut(0).unwrap();
    sheet.comments.insert(
        at("A1"),
        Comment {
            author: "Author".into(),
            text: "Original".into(),
            replies: vec![],
            threaded: false,
            resolved: false,
            visible: false,
            dates: vec![],
            note_box: None,
        },
    );
    sheet.hyperlinks.insert(at("A2"), Hyperlink { target: "https://example.com/first".into(), tooltip: Some("Details".into()), external: true });
    sheet.validations.push(Validation {
        ranges: vec![gridcraft_engine::core::RangeRef::parse("B2:B3").unwrap()],
        f1: "1".into(),
        ..Default::default()
    });
    sheet.pivots.push(PivotTable { id: 47, source: "SalesTable".into(), ..Default::default() });
    sheet.charts.push(Chart {
        id: 91,
        kind: ChartKind::Line,
        anchor: Anchor::default(),
        title: Some("Sales".into()),
        series: vec![Series { values: "Sheet1!$B$2:$B$3".into(), ..Default::default() }],
        legend: LegendPos::Bottom,
        data_labels: false,
        data_table: None,
        gridlines: true,
        style: 2,
        hole_size: 75,
        first_slice_angle: 0,
        x_title: None,
        y_title: None,
        source: None,
        by_rows: false,
        pivot: None,
    });
    sheet.controls.push(FormControl { id: 80, text: "Run".into(), ..Default::default() });
    sheet.images.push(Image { id: 62, anchor: Anchor::default(), data: Arc::new(vec![1, 2, 3]), mime: "image/png".into(), alt: "Picture".into() });
    sheet.cell_pictures.insert(at("C2"), Arc::new(CellPicture { data: vec![4, 5, 6], mime: "image/png".into(), alt: "Cell picture".into() }));
    sheet.shapes.push(Shape {
        id: 19,
        kind: ShapeKind::Rectangle,
        anchor: Anchor::default(),
        fill: Color::Auto,
        line: Color::Auto,
        text: "Label".into(),
        format: ShapeFormat::default(),
        math: EquationCache::default(),
    });
    wb
}

type SheetChange = (&'static str, fn(&mut Sheet));

fn category_diffs(a: &Workbook, b: &Workbook, name: &str) -> usize {
    diff_workbooks(a, b, &Options::default()).iter().find(|c| c.name == name).map(|c| c.diffs).unwrap_or(0)
}

#[test]
fn check_detects_slicer_loss() {
    // Start with JSON, before the lossy XLSX writer has a chance to remove the slicer.
    let wb = slicer_book();
    let bytes = gridcraft_engine::io::save_bytes(&wb, "slicer.json").unwrap();
    let report = check_bytes("slicer.json", &bytes, &Options::default());
    let json = report_json(&[report], &Options::default());
    assert!(json["files"][0]["roundTrip"]["categories"]["slicers"]["diffs"].as_u64().unwrap_or(0) > 0, "{json}");
    let path = std::env::temp_dir().join(format!("gridcraft-slicer-check-{}.json", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    let exit = main(&[path.to_string_lossy().into_owned(), "--json".into()]);
    std::fs::remove_file(path).unwrap();
    assert_eq!(exit, ExitCode::FAILURE);
}

#[test]
fn check_detects_payload_changes() {
    use gridcraft_engine::model::PivotLayout;
    let a = feature_book();
    let changes: &[SheetChange] = &[
        ("comments", |s| s.comments.get_mut(&at("A1")).unwrap().text = "Changed".into()),
        ("hyperlinks", |s| s.hyperlinks.get_mut(&at("A2")).unwrap().target = "https://example.com/second".into()),
        ("charts", |s| s.charts[0].title = Some("Changed".into())),
        ("charts", |s| s.charts[0].x_title = Some("New axis".into())),
        ("charts", |s| s.charts[0].series[0].secondary = true),
        ("validations", |s| s.validations[0].f1 = "9".into()),
        ("pivots", |s| s.pivots[0].layout = PivotLayout::Tabular),
        ("tables", |s| s.tables[0].banded_rows = !s.tables[0].banded_rows),
    ];
    for (category, change) in changes {
        let mut b = a.clone();
        change(b.sheet_mut(0).unwrap());
        assert!(category_diffs(&a, &b, category) > 0, "missed {category} payload change");
    }
}

#[test]
fn check_detects_drawing_payload_changes_and_complete_losses() {
    let a = feature_book();
    let changes: &[SheetChange] = &[
        ("slicers", |s| s.slicers[0].selected = Some(vec!["East".into()])),
        ("controls", |s| s.controls[0].text = "Changed".into()),
        ("pictures", |s| s.images[0].data = Arc::new(vec![9])),
        ("pictures", |s| Arc::make_mut(s.cell_pictures.get_mut(&at("C2")).unwrap()).alt = "Changed".into()),
        ("shapes", |s| s.shapes[0].text = "Changed".into()),
        ("printSettings", |s| s.print.header = "Confidential".into()),
        ("slicers", |s| s.slicers.clear()),
        ("controls", |s| s.controls.clear()),
        ("pictures", |s| s.images.clear()),
        ("pictures", |s| s.cell_pictures.clear()),
        ("shapes", |s| s.shapes.clear()),
    ];
    for (category, change) in changes {
        let mut b = a.clone();
        change(b.sheet_mut(0).unwrap());
        assert!(category_diffs(&a, &b, category) > 0, "missed {category} change/loss");
    }
}

#[test]
fn check_detects_full_style_changes_and_ignores_interned_ids() {
    use gridcraft_engine::model::{LineInfo, Style, StyleId};
    let mut a = book(&[("A1", "1", n(1.0))]);
    let mut bold = Style::default();
    bold.font.bold = true;
    let aid = a.styles.intern(bold.clone());
    a.sheet_mut(0).unwrap().cells.get_mut(at("A1")).unwrap().style = aid;
    a.sheet_mut(0).unwrap().rows.insert(2, LineInfo { style: Some(aid), ..Default::default() });
    let mut b = a.clone();
    b.sheet_mut(0).unwrap().cells.get_mut(at("A1")).unwrap().style = StyleId::DEFAULT;
    assert!(category_diffs(&a, &b, "styles") > 0);
    b = a.clone();
    b.sheet_mut(0).unwrap().rows.get_mut(&2).unwrap().style = None;
    assert!(category_diffs(&a, &b, "styles") > 0);
    // Same style through a different interning order is equal.
    let mut b = a.clone();
    b.styles = Default::default();
    let mut italic = Style::default();
    italic.font.italic = true;
    b.styles.intern(italic);
    let bid = b.styles.intern(bold);
    b.sheet_mut(0).unwrap().cells.get_mut(at("A1")).unwrap().style = bid;
    b.sheet_mut(0).unwrap().rows.get_mut(&2).unwrap().style = Some(bid);
    assert_eq!(category_diffs(&a, &b, "styles"), 0);
}

#[test]
fn check_ignores_generated_object_ids() {
    let a = feature_book();
    let mut b = a.clone();
    let s = b.sheet_mut(0).unwrap();
    s.tables[0].id += 100;
    s.charts[0].id += 100;
    s.pivots[0].id += 100;
    s.slicers[0].id += 100;
    s.controls[0].id += 100;
    s.images[0].id += 100;
    s.shapes[0].id += 100;
    assert!(diff_workbooks(&a, &b, &Options::default()).iter().all(|c| c.diffs == 0));
}

#[test]
fn check_compares_formula_modes_without_scalar_normalization_noise() {
    use gridcraft_engine::model::FormulaEvalMode;
    let scalar = book(&[("A1", "=1+2", n(3.0)), ("B1", "=SUM(A1:A2)", n(3.0))]);
    assert!(round_trip(&scalar, b"", &Options::default()).is_clean());
    let mut a = book(&[("A1", "1", n(1.0)), ("A2", "2", n(2.0)), ("B1", "=SUM(A1:A2*2)", n(6.0))]);
    let mut b = a.clone();
    Arc::make_mut(b.sheet_mut(0).unwrap().cells.get_mut(at("B1")).unwrap().formula.as_mut().unwrap()).eval_mode = FormulaEvalMode::Legacy;
    assert!(category_diffs(&a, &b, "formulaModes") > 0);
    assert!(round_trip(&a, b"", &Options::default()).is_clean());
    // A changed CSE extent is not only a change to formula text.
    Arc::make_mut(a.sheet_mut(0).unwrap().cells.get_mut(at("B1")).unwrap().formula.as_mut().unwrap()).array =
        Some(gridcraft_engine::core::RangeRef::parse("B1:B2").unwrap());
    assert!(category_diffs(&a, &b, "formulaModes") > 0);
}

#[test]
fn check_preserves_zero_oracle_visibility() {
    let wb = book(&[("A1", "=1+1", Value::Empty), ("B1", "=2+2", Value::Empty)]);
    let r = check(&wb, &Options::default());
    let j = report_json(&[r], &Options::default());
    assert_eq!(j["files"][0]["calc"]["checked"], 0);
    assert_eq!(j["files"][0]["calc"]["matched"], 0);
    assert_eq!(j["files"][0]["calc"]["noOracle"], 2);
    assert_eq!(j["totals"]["checked"], 0);
    assert_eq!(j["totals"]["matched"], 0);
    assert_eq!(j["totals"]["noOracle"], 2);
}

#[test]
fn check_reports_source_package_coverage_separately() {
    let wb = book(&[]);
    let opts = Options::default();
    let json_bytes = gridcraft_engine::io::save_bytes(&wb, "t.json").unwrap();
    let r = check_bytes("t.json", &json_bytes, &opts);
    let j = report_json(&[r], &opts);
    assert_eq!(j["files"][0]["roundTrip"]["sourcePackage"]["status"], "notApplicable");
    let bytes = plain();
    let r = check_bytes("t.xlsx", &bytes, &opts);
    let j = report_json(&[r], &opts);
    assert_eq!(j["files"][0]["roundTrip"]["sourcePackage"]["status"], "compared");
    assert!(j["files"][0]["roundTrip"]["sourcePackage"]["relationshipKinds"].as_u64().unwrap() > 0);
    let malformed = zip_of(&[(WB_RELS, "<Relationships><broken></Relationships>")]);
    let rt = round_trip(&wb, &malformed, &opts);
    assert!(!rt.is_clean(), "unreadable source coverage must not silently pass");
    assert!(rt.categories.iter().any(|c| c.name == "package" && c.diffs > 0));
}

#[test]
fn check_detects_named_style_payload_loss() {
    use gridcraft_engine::model::Style;
    let mut a = book(&[]);
    let mut style = Style::default();
    style.font.bold = true;
    a.cell_styles.push(("Emphasis".into(), style));
    let b = book(&[]);
    assert!(category_diffs(&a, &b, "styles") > 0);
}

#[test]
fn check_dynamic_spill_retains_array_mode_and_shape() {
    use gridcraft_engine::model::FormulaEvalMode;
    let mut a = book(&[("A1", "1", n(1.0)), ("A2", "2", n(2.0)), ("B1", "=A1:A2*2", Value::Empty)]);
    Calc::new().recalc_all(&mut a);
    assert_eq!(a.sheet(0).unwrap().spill_ranges[&at("B1")].a1(), "B1:B2");
    assert!(round_trip(&a, b"", &Options::default()).is_clean());
    let bytes = gridcraft_engine::io::save_bytes(&a, "spill.xlsx").unwrap();
    let (mut b, _) = gridcraft_engine::io::open_bytes("spill.xlsx", &bytes).unwrap();
    Calc::new().recalc_all(&mut b);
    let s = b.sheet(0).unwrap();
    assert_eq!(s.cell(at("B1")).unwrap().formula.as_ref().unwrap().eval_mode, FormulaEvalMode::Array);
    assert_eq!(s.spill_ranges[&at("B1")].a1(), "B1:B2");
    assert_eq!(s.spill[&at("B2")], n(4.0));
    assert_eq!(category_diffs(&a, &b, "formulaModes"), 0);
    Arc::make_mut(b.sheet_mut(0).unwrap().cells.get_mut(at("B1")).unwrap().formula.as_mut().unwrap()).eval_mode = FormulaEvalMode::Legacy;
    assert!(category_diffs(&a, &b, "formulaModes") > 0);
}

#[test]
fn check_control_round_trip_ignores_generated_default_name() {
    use gridcraft_engine::model::{Anchor, FormControl};
    let mut wb = book(&[]);
    wb.sheet_mut(0).unwrap().controls.push(FormControl {
        id: 80,
        anchor: Anchor { cell: at("D1"), width: 100.0, height: 30.0, ..Default::default() },
        text: "Run".into(),
        ..Default::default()
    });
    let rt = round_trip(&wb, b"", &Options::default());
    assert!(rt.is_clean(), "{rt:?}");
}

#[test]
fn check_detects_theme_changes_behind_shared_style_indices() {
    let a = book(&[]);
    let mut b = a.clone();
    b.theme.colors[4] = 0x123456;
    assert!(category_diffs(&a, &b, "styles") > 0);
}

#[test]
fn check_detects_default_row_override_lost_at_styled_column_intersections() {
    use gridcraft_engine::model::{LineInfo, Style, StyleId};
    // Both a nearby blank intersection and the grid's far edge must remain sparse.
    for address in ["A2", "XFD1048576"] {
        let cell = at(address);
        let mut a = book(&[]);
        let mut bold = Style::default();
        bold.font.bold = true;
        let bold_id = a.styles.intern(bold);
        let sheet = a.sheet_mut(0).unwrap();
        sheet.cols.insert(cell.col, LineInfo { style: Some(bold_id), ..Default::default() });
        sheet.rows.insert(cell.row, LineInfo { style: Some(StyleId::DEFAULT), ..Default::default() });
        assert!(sheet.cells.is_empty());
        assert_eq!(sheet.style_id(cell), StyleId::DEFAULT);
        let mut b = a.clone();
        b.sheet_mut(0).unwrap().rows.get_mut(&cell.row).unwrap().style = None;
        assert_eq!(b.sheet(0).unwrap().style_id(cell), bold_id);
        assert!(category_diffs(&a, &b, "styles") > 0, "lost row override at {address}");
    }
}

#[test]
fn check_detects_empty_cell_override_lost_to_row_or_column_inheritance() {
    use gridcraft_engine::model::{LineInfo, Style, StyleId};
    for from_row in [false, true] {
        let cell = at("A2");
        let mut a = book(&[("A2", "1", n(1.0))]);
        let mut bold = Style::default();
        bold.font.bold = true;
        let bold_id = a.styles.intern(bold);
        let sheet = a.sheet_mut(0).unwrap();
        let lines = if from_row { &mut sheet.rows } else { &mut sheet.cols };
        let line = if from_row { cell.row } else { cell.col };
        lines.insert(line, LineInfo { style: Some(bold_id), ..Default::default() });
        // Clearing content through a mutable cell retains its explicit default style record.
        sheet.cells.get_mut(cell).unwrap().clear_content();
        assert!(sheet.cell(cell).is_some());
        assert_eq!(sheet.style_id(cell), StyleId::DEFAULT);
        let mut b = a.clone();
        assert!(b.sheet_mut(0).unwrap().remove_cell(cell).is_some());
        assert_eq!(b.sheet(0).unwrap().style_id(cell), bold_id);
        assert_eq!(category_diffs(&a, &b, "values"), 0);
        assert!(category_diffs(&a, &b, "styles") > 0, "lost cell override, row inheritance={from_row}");
    }
}

#[test]
fn check_ignores_default_line_styles_without_changed_inheritance() {
    use gridcraft_engine::model::{LineInfo, StyleId};
    let a = book(&[]);
    let mut b = a.clone();
    let sheet = b.sheet_mut(0).unwrap();
    sheet.cols.insert(0, LineInfo { style: Some(StyleId::DEFAULT), ..Default::default() });
    sheet.rows.insert(1, LineInfo { style: Some(StyleId::DEFAULT), ..Default::default() });
    assert_eq!(category_diffs(&a, &b, "styles"), 0);
}
