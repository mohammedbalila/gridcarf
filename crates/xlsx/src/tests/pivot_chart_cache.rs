//! PivotChart cache fallback through complete XLSX read, write and reopen.

use std::fmt::Write as _;
use std::io::{Cursor, Read};

use gridcraft_model::CompatibilityEffect;

use crate::xml::{El, parse};
use crate::{read_xlsx, write_xlsx};

const REL_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const SHEET_NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const CHART_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const ANCHOR_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
const DRAW_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const CORNER: &str = "'Pivot Data'!$A$3";

fn fixture(charts: &[String]) -> Vec<u8> {
    fixture_with_rows(
        charts,
        r#"<row r="3"><c r="A3" t="inlineStr"><is><t>Corner</t></is></c></row>
           <row r="4"><c r="A4" t="inlineStr"><is><t>East</t></is></c><c r="B4"><v>1.5</v></c></row>
           <row r="5"><c r="A5" t="inlineStr"><is><t>West</t></is></c><c r="B5"><v>2.5</v></c></row>"#,
    )
}

fn fixture_with_rows(charts: &[String], rows: &str) -> Vec<u8> {
    let book = format!(
        r#"<workbook xmlns="{SHEET_NS}" xmlns:r="{REL_NS}"><sheets><sheet name="Dashboard" sheetId="1" r:id="rId1"/><sheet name="Pivot Data" sheetId="2" r:id="rId2"/></sheets></workbook>"#
    );
    let book_rels = format!(
        r#"<Relationships xmlns="{PKG_NS}"><Relationship Id="rId1" Type="{REL_NS}/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="{REL_NS}/worksheet" Target="worksheets/sheet2.xml"/></Relationships>"#
    );
    let dashboard = format!(r#"<worksheet xmlns="{SHEET_NS}" xmlns:r="{REL_NS}"><sheetData/><drawing r:id="rId1"/></worksheet>"#);
    let dashboard_rels = format!(
        r#"<Relationships xmlns="{PKG_NS}"><Relationship Id="rId1" Type="{REL_NS}/drawing" Target="../drawings/drawing1.xml"/></Relationships>"#
    );
    let data = format!(r#"<worksheet xmlns="{SHEET_NS}"><sheetData>{rows}</sheetData></worksheet>"#);
    let mut drawing = format!(r#"<xdr:wsDr xmlns:xdr="{ANCHOR_NS}" xmlns:a="{DRAW_NS}" xmlns:c="{CHART_NS}" xmlns:r="{REL_NS}">"#);
    let mut drawing_rels = format!(r#"<Relationships xmlns="{PKG_NS}">"#);
    let mut chart_paths = Vec::new();
    for n in 1..=charts.len() {
        drawing.push_str(&format!(
            r#"<xdr:oneCellAnchor>
              <xdr:from><xdr:col>{n}</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
              <xdr:ext cx="5000000" cy="3000000"/>
              <xdr:graphicFrame><a:graphic><a:graphicData uri="{CHART_NS}"><c:chart r:id="rId{n}"/></a:graphicData></a:graphic></xdr:graphicFrame>
              <xdr:clientData/>
            </xdr:oneCellAnchor>"#
        ));
        drawing_rels.push_str(&format!(r#"<Relationship Id="rId{n}" Type="{REL_NS}/chart" Target="../charts/chart{n}.xml"/>"#));
        chart_paths.push(format!("xl/charts/chart{n}.xml"));
    }
    drawing.push_str("</xdr:wsDr>");
    drawing_rels.push_str("</Relationships>");
    let mut parts = vec![
        ("[Content_Types].xml", super::CT.as_bytes()),
        ("_rels/.rels", super::ROOT_RELS.as_bytes()),
        ("xl/workbook.xml", book.as_bytes()),
        ("xl/_rels/workbook.xml.rels", book_rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", dashboard.as_bytes()),
        ("xl/worksheets/_rels/sheet1.xml.rels", dashboard_rels.as_bytes()),
        ("xl/worksheets/sheet2.xml", data.as_bytes()),
        ("xl/drawings/drawing1.xml", drawing.as_bytes()),
        ("xl/drawings/_rels/drawing1.xml.rels", drawing_rels.as_bytes()),
    ];
    parts.extend(chart_paths.iter().zip(charts).map(|(name, xml)| (name.as_str(), xml.as_bytes())));
    super::make_zip(&parts)
}

fn chart(pivot: bool, series: &str) -> String {
    let link = if pivot { "<c:pivotSource><c:name>[Original.xlsx]Pivot Data!SalesPivot</c:name></c:pivotSource>" } else { "" };
    format!(
        r#"<c:chartSpace xmlns:c="{CHART_NS}">{link}<c:chart><c:plotArea><c:barChart><c:barDir val="col"/>{series}</c:barChart></c:plotArea></c:chart></c:chartSpace>"#
    )
}

fn series(index: usize, cats: &str, vals: &str) -> String {
    format!(r#"<c:ser><c:idx val="{index}"/><c:order val="{index}"/><c:cat>{cats}</c:cat><c:val>{vals}</c:val></c:ser>"#)
}

fn reference(kind: &str, formula: &str, count: &str, points: &str) -> String {
    format!(r#"<c:{kind}Ref><c:f>{formula}</c:f><c:{kind}Cache><c:ptCount val="{count}"/>{points}</c:{kind}Cache></c:{kind}Ref>"#)
}

fn chart_part(bytes: &[u8], n: usize) -> El {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut xml = Vec::new();
    zip.by_name(&format!("xl/charts/chart{n}.xml")).unwrap().read_to_end(&mut xml).unwrap();
    parse(&xml).unwrap()
}

fn assert_saved_cache(bytes: &[u8]) {
    let root = chart_part(bytes, 1);
    let source = &root.path(&["pivotSource", "name"]).unwrap().text;
    assert_eq!(source.split_once(']').unwrap().1, "Pivot Data!SalesPivot");
    let bar = root.path(&["chart", "plotArea", "barChart"]).unwrap();
    let mut saved = bar.kids("ser");
    let fallback = saved.next().unwrap();
    let val = fallback.path(&["val", "numLit"]).expect("placeholder numbers should save as indexed literals");
    assert_eq!(val.child_val("ptCount"), Some("4"));
    let numbers: Vec<(u32, f64)> = val.kids("pt").map(|p| (p.attr_u32("idx").unwrap(), p.child("v").unwrap().text.parse().unwrap())).collect();
    assert_eq!(numbers, [(0, 22.625), (2, -0.073_108_180_993_128_48)]);
    let cat = fallback.path(&["cat", "strLit"]).expect("placeholder labels should save as literals");
    assert_eq!(cat.child_val("ptCount"), Some("4"));
    let labels: Vec<(u32, &str)> = cat.kids("pt").map(|p| (p.attr_u32("idx").unwrap(), p.child("v").unwrap().text.as_str())).collect();
    assert_eq!(labels, [(0, "East, Central"), (1, "Quoted \"West\"; branch"), (2, "North & South"), (3, "")]);
    let live = saved.next().unwrap();
    assert_eq!(live.path(&["cat", "strRef", "f"]).unwrap().text, "'Pivot Data'!$A$4:$A$5");
    assert_eq!(live.path(&["val", "numRef", "f"]).unwrap().text, "'Pivot Data'!$B$4:$B$5");
    let complex = saved.next().unwrap();
    assert_eq!(complex.path(&["val", "numRef", "f"]).unwrap().text, "OFFSET($A$3,1,1,2,1)");
    let stale_range = saved.next().unwrap().path(&["val", "numLit"]).expect("changed values should keep the exact cache");
    let precise: f64 = stale_range.kids("pt").nth(1).unwrap().child("v").unwrap().text.parse().unwrap();
    assert_eq!(precise, 2.500_000_000_000_000_4);
    let ordinary = chart_part(bytes, 2);
    let ordinary = ordinary.path(&["chart", "plotArea", "barChart", "ser"]).unwrap();
    assert_eq!(ordinary.path(&["cat", "strRef", "f"]).unwrap().text, CORNER);
    assert_eq!(ordinary.path(&["val", "numRef", "f"]).unwrap().text, CORNER);
}

#[test]
fn pivot_chart_keeps_cached_points_for_a_placeholder_across_two_saves() {
    let cats = r#"<c:pt idx="2"><c:v>North &amp; South</c:v></c:pt><c:pt idx="0"><c:v>East, Central</c:v></c:pt><c:pt idx="1"><c:v>Quoted &quot;West&quot;; branch</c:v></c:pt>"#;
    let vals = r#"<c:pt idx="2"><c:v>-7.3108180993128481E-2</c:v></c:pt><c:pt idx="0"><c:v>22.625</c:v></c:pt><c:pt idx="1"><c:v>#N/A</c:v></c:pt>"#;
    let stale = series(0, &reference("str", CORNER, "4", cats), &reference("num", CORNER, "4", vals));
    let live = series(
        1,
        &reference("str", "'Pivot Data'!$A$4:$A$5", "2", r#"<c:pt idx="0"><c:v>East</c:v></c:pt><c:pt idx="1"><c:v>West</c:v></c:pt>"#),
        &reference("num", "'Pivot Data'!$B$4:$B$5", "2", r#"<c:pt idx="0"><c:v>1.5</c:v></c:pt><c:pt idx="1"><c:v>2.5</c:v></c:pt>"#),
    );
    let complex = series(2, &reference("str", "CategoryName", "4", cats), &reference("num", "OFFSET($A$3,1,1,2,1)", "4", vals));
    let stale_range = series(
        3,
        &reference("str", CORNER, "4", cats),
        &reference("num", "'Pivot Data'!$B$4:$B$5", "2", r#"<c:pt idx="0"><c:v>1.5</c:v></c:pt><c:pt idx="1"><c:v>2.5000000000000004</c:v></c:pt>"#),
    );
    let input = fixture(&[chart(true, &format!("{stale}{live}{complex}{stale_range}")), chart(false, &stale)]);
    let (wb, report) = read_xlsx(&input).unwrap();
    let static_issues: Vec<_> = report.issues.iter().filter(|i| i.effect == CompatibilityEffect::Static).collect();
    assert_eq!(static_issues.len(), 1);
    assert_eq!(static_issues[0].feature, "PivotChart series");
    assert!(static_issues[0].description.contains("Refresh"));
    assert_eq!(wb.sheets[0].charts[0].pivot.as_deref(), Some("Pivot Data!SalesPivot"));
    let saved = write_xlsx(&wb).unwrap();
    assert_saved_cache(&saved);
    let (reopened, report) = read_xlsx(&saved).unwrap();
    assert!(!report.issues.iter().any(|i| i.effect == CompatibilityEffect::Static));
    assert_saved_cache(&write_xlsx(&reopened).unwrap());
}

#[test]
fn oversized_pivot_chart_cache_keeps_reference_and_reports_loss() {
    let label = "x".repeat(gridcraft_formula::MAX_FORMULA_LEN);
    let large_text = format!(r#"<c:pt idx="0"><c:v>{label}</c:v></c:pt>"#);
    let huge_index = r#"<c:pt idx="4294967295"><c:v>42</c:v></c:pt>"#;
    let series = series(0, &reference("str", CORNER, "1", &large_text), &reference("num", CORNER, "4294967295", huge_index));
    let (wb, report) = read_xlsx(&fixture(&[chart(true, &series)])).unwrap();
    let saved = &wb.sheets[0].charts[0].series[0];
    assert_eq!(saved.categories.as_deref(), Some(CORNER));
    assert_eq!(saved.values, CORNER);
    assert!(report.warnings.iter().any(|w| w.contains("PivotChart") && w.contains("cache")), "{:?}", report.warnings);
}

fn long_live_data() -> (String, String) {
    let mut rows = String::new();
    let mut points = String::new();
    for i in 0..3000 {
        let row = i + 1;
        write!(rows, r#"<row r="{row}"><c r="B{row}"><v>12.5</v></c></row>"#).unwrap();
        write!(points, r#"<c:pt idx="{i}"><c:v>12.5</c:v></c:pt>"#).unwrap();
    }
    (rows, points)
}

#[test]
fn long_live_cache_and_dense_literal_keep_their_values_without_a_loss_issue() {
    let (rows, points) = long_live_data();
    let live = "'Pivot Data'!$B$1:$B$3000";
    let reference = series(0, "", &reference("num", live, "3000", &points));
    let dense = series(1, "", &format!(r#"<c:numLit><c:ptCount val="3000"/>{points}</c:numLit>"#));
    let (wb, report) = read_xlsx(&fixture_with_rows(&[chart(true, &format!("{reference}{dense}"))], &rows)).unwrap();
    let series = &wb.sheets[0].charts[0].series;
    assert_eq!(series[0].values, live);
    assert_eq!(series[1].values, format!("{{{}12.5}}", "12.5,".repeat(2999)));
    assert!(!report.warnings.iter().any(|w| w.contains("PivotChart")), "{:?}", report.warnings);
    assert!(!report.issues.iter().any(|i| i.feature.contains("PivotChart")), "{:?}", report.issues);
    let saved = chart_part(&write_xlsx(&wb).unwrap(), 1);
    let bar = saved.path(&["chart", "plotArea", "barChart"]).unwrap();
    assert_eq!(bar.kids("ser").next().unwrap().path(&["val", "numRef", "numCache"]).unwrap().child_val("ptCount"), Some("3000"));
    assert_eq!(bar.kids("ser").nth(1).unwrap().path(&["val", "numLit"]).unwrap().kids("pt").count(), 3000);
}

#[test]
fn long_same_length_cache_with_a_stale_point_reports_loss() {
    let (rows, points) = long_live_data();
    let stale = points.replacen(r#"<c:pt idx="1500"><c:v>12.5"#, r#"<c:pt idx="1500"><c:v>12.6"#, 1);
    let live = "'Pivot Data'!$B$1:$B$3000";
    let cache = reference("num", live, "3000", &stale);
    let (wb, report) = read_xlsx(&fixture_with_rows(&[chart(true, &series(0, "", &cache))], &rows)).unwrap();
    assert_eq!(wb.sheets[0].charts[0].series[0].values, live);
    assert!(report.issues.iter().any(|i| i.effect == CompatibilityEffect::Omitted && i.description.contains("PivotChart")), "{:?}", report.issues,);
}

#[test]
fn sparse_category_literals_retain_slots_and_oversized_literals_keep_legacy_data() {
    let cats = r#"<c:strLit><c:ptCount val="4"/><c:pt idx="2"><c:v>West</c:v></c:pt><c:pt idx="0"><c:v>East</c:v></c:pt></c:strLit>"#;
    let vals = r#"<c:numLit><c:ptCount val="9000"/><c:pt idx="8999"><c:v>42</c:v></c:pt></c:numLit>"#;
    let (wb, report) = read_xlsx(&fixture(&[chart(true, &series(0, cats, vals))])).unwrap();
    let s = &wb.sheets[0].charts[0].series[0];
    assert_eq!(s.categories.as_deref(), Some(r#"{"East","","West",""}"#));
    assert_eq!(s.values, "{42}");
    assert!(report.warnings.iter().any(|w| w.contains("PivotChart") && w.contains("cache")), "{:?}", report.warnings);
}

#[test]
fn malformed_points_warn_and_leave_direct_references_in_place() {
    let bad_points = [
        r#"<c:pt idx="0"><c:v>10</c:v></c:pt><c:pt idx="1"><c:v>20</c:v></c:pt><c:pt idx="0"><c:v>30</c:v></c:pt>"#,
        r#"<c:pt idx="0"><c:v>not a number</c:v></c:pt>"#,
    ];
    for points in bad_points {
        let vals = reference("num", CORNER, "3", points);
        let (wb, report) = read_xlsx(&fixture(&[chart(true, &series(0, "", &vals))])).unwrap();
        assert_eq!(wb.sheets[0].charts[0].series[0].values, CORNER);
        assert!(report.warnings.iter().any(|w| w.contains("PivotChart") && w.contains("cache")), "{:?}", report.warnings);
    }

    let complex = "OFFSET($A$3,1,1,2,1)";
    let vals = reference("num", complex, "3", bad_points[1]);
    let (wb, report) = read_xlsx(&fixture(&[chart(true, &series(0, "", &vals))])).unwrap();
    assert_eq!(wb.sheets[0].charts[0].series[0].values, complex);
    assert!(!report.warnings.iter().any(|w| w.contains("PivotChart")), "{:?}", report.warnings);
}
