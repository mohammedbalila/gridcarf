//! A synthetic local PivotTable graph with native slicer and timeline state.
//! Extension structure follows MS-XLSX sections 2.3.2, 2.3.5 and 2.6.

use std::fmt::Write as _;
use std::sync::Arc;

use gridcraft_core::{CellRef, Value};
use gridcraft_model::{Cell, Formula};

use crate::package::Package;
use crate::xml::El;
use crate::{read_xlsx, write_xlsx};

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PACKAGE_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const X15: &str = "http://schemas.microsoft.com/office/spreadsheetml/2010/11/main";
const SLICER_REL: &str = "http://schemas.microsoft.com/office/2007/relationships";
const TIMELINE_REL: &str = "http://schemas.microsoft.com/office/2011/relationships";
const SLICER_ANCHOR: &str = "http://schemas.microsoft.com/office/drawing/2010/slicer";
const TIMELINE_ANCHOR: &str = "http://schemas.microsoft.com/office/drawing/2012/timeslicer";
const DRAWING: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
const GRAPHICS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const SLICER_CACHE_MOUNT: &str = "{BBE1A952-AA13-448e-AADC-164F8A28A991}";
const TIMELINE_CACHE_MOUNT: &str = "{D0CA8CA8-9F24-4464-BF8E-62219DCF47F9}";
const SLICER_MOUNT: &str = "{A8765BA9-456A-4dab-B4F3-ACF838C121DE}";
const TIMELINE_MOUNT: &str = "{7E03D99C-DC04-49d9-9315-930204A7B6E9}";
const EXT_CACHE: &str = "{725AE2AE-9491-48be-B2B4-4EB974FC3084}";
const WORKBOOK: &str = "xl/workbook.xml";
const REPORT: &str = "xl/worksheets/sheet1.xml";

fn rels(links: &[(&str, &str, &str, &str)]) -> String {
    let mut xml = format!(r#"<Relationships xmlns="{PACKAGE_REL}">"#);
    for (id, ns, kind, target) in links {
        write!(xml, r#"<Relationship Id="{id}" Type="{ns}/{kind}" Target="{target}"/>"#).unwrap();
    }
    xml.push_str("</Relationships>");
    xml
}

fn pivot(name: &str, range: &str) -> String {
    format!(
        r#"<pivotTableDefinition xmlns="{MAIN}" name="{name}" cacheId="2" dataCaption="Results">
            <location ref="{range}" firstHeaderRow="1" firstDataRow="1" firstDataCol="1"/>
            <pivotFields count="3"><pivotField axis="axisRow" showAll="0"><items count="2"><item x="0"/><item x="1"/></items></pivotField><pivotField showAll="0"/><pivotField dataField="1" showAll="0"/></pivotFields>
            <rowFields count="1"><field x="0"/></rowFields>
            <dataFields count="1"><dataField name="Total Units" fld="2" subtotal="sum" numFmtId="201"/></dataFields>
            <formats count="1"><format dxfId="0"><pivotArea type="normal" dataOnly="1"/></format></formats>
            <pivotTableStyleInfo name="NativeReportStyle" showRowHeaders="1"/>
            <filters count="1"><filter fld="0" type="captionEqual" id="1" stringValue1="Mint"><autoFilter ref="A1"><filterColumn colId="0"><customFilters><customFilter operator="equal" val="Mint"/></customFilters></filterColumn></autoFilter></filter></filters>
        </pivotTableDefinition>"#
    )
}

fn control_anchor(id: u32, col: u32, ns: &str, kind: &str, name: &str) -> String {
    let to_col = col + 2;
    format!(
        r#"<xdr:twoCellAnchor>
            <xdr:from><xdr:col>{col}</xdr:col><xdr:colOff>20</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>30</xdr:rowOff></xdr:from>
            <xdr:to><xdr:col>{to_col}</xdr:col><xdr:colOff>40</xdr:colOff><xdr:row>6</xdr:row><xdr:rowOff>50</xdr:rowOff></xdr:to>
            <mc:AlternateContent xmlns:mc="{MC}"><mc:Choice xmlns:control="{ns}" Requires="control">
                <xdr:graphicFrame macro=""><xdr:nvGraphicFramePr><xdr:cNvPr id="{id}" name="{name}"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr>
                    <xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm>
                    <a:graphic><a:graphicData uri="{ns}"><control:{kind} name="{name}"/></a:graphicData></a:graphic>
                </xdr:graphicFrame>
            </mc:Choice><mc:Fallback/></mc:AlternateContent>
            <xdr:clientData/>
        </xdr:twoCellAnchor>"#
    )
}

enum Fixture {
    Complete,
    MissingTimelineRelationship,
    ExtraRootRelationship,
    InvalidPivotFilter,
}

fn fixture(shape: Fixture) -> Vec<u8> {
    let workbook = format!(
        r#"<workbook xmlns="{MAIN}" xmlns:r="{REL}" xmlns:x14="{X14}" xmlns:x15="{X15}">
            <sheets><sheet name="Report" sheetId="7" r:id="rId8"/><sheet name="Data" sheetId="11" r:id="rId3"/></sheets>
            <pivotCaches><pivotCache cacheId="2" r:id="rId13"/></pivotCaches>
            <extLst><ext uri="{SLICER_CACHE_MOUNT}"><x14:slicerCaches><x14:slicerCache r:id="rId34"/></x14:slicerCaches></ext>
                <ext uri="{TIMELINE_CACHE_MOUNT}"><x15:timelineCacheRefs><x15:timelineCacheRef r:id="rId51"/></x15:timelineCacheRefs></ext></extLst>
        </workbook>"#
    );
    let workbook_rels = rels(&[
        ("rId8", REL, "worksheet", "worksheets/sheet1.xml"),
        ("rId3", REL, "worksheet", "worksheets/sheet2.xml"),
        ("rId7", REL, "styles", "styles.xml"),
        ("rId13", REL, "pivotCacheDefinition", "pivotCache/pivotCacheDefinition1.xml"),
        ("rId34", SLICER_REL, "slicerCache", "slicerCaches/slicerCache1.xml"),
        ("rId51", TIMELINE_REL, "timelineCache", "timelineCaches/timelineCache1.xml"),
    ]);
    let report = format!(
        r#"<worksheet xmlns="{MAIN}" xmlns:r="{REL}" xmlns:x14="{X14}" xmlns:x15="{X15}">
            <sheetData><row r="4"><c r="B4"><v>19</v></c><c r="E4"><v>19</v></c></row></sheetData>
            <drawing r:id="rId8"/>
            <extLst><ext uri="{SLICER_MOUNT}"><x14:slicerList><x14:slicer r:id="rId39"/></x14:slicerList></ext>
                <ext uri="{TIMELINE_MOUNT}"><x15:timelineRefs><x15:timelineRef r:id="rId59"/></x15:timelineRefs></ext></extLst>
        </worksheet>"#
    );
    let mut report_links = vec![
        ("rId8", REL, "drawing", "../drawings/drawing1.xml"),
        ("rId15", REL, "pivotTable", "../pivotTables/pivotTable1.xml"),
        ("rId18", REL, "pivotTable", "../pivotTables/pivotTable2.xml"),
        ("rId39", SLICER_REL, "slicer", "../slicers/slicer1.xml"),
    ];
    if !matches!(shape, Fixture::MissingTimelineRelationship) {
        report_links.push(("rId59", TIMELINE_REL, "timeline", "../timelines/timeline1.xml"));
    }
    let report_rels = rels(&report_links);
    let data = format!(
        r#"<worksheet xmlns="{MAIN}" xmlns:r="{REL}"><sheetData>
            <row r="1"><c r="A1" t="inlineStr"><is><t>Blend</t></is></c><c r="B1" t="inlineStr"><is><t>Day</t></is></c><c r="C1" t="inlineStr"><is><t>Units</t></is></c></row>
            <row r="2"><c r="A2" t="inlineStr"><is><t>Mint</t></is></c><c r="B2" s="1"><v>45658</v></c><c r="C2"><v>19</v></c></row>
            <row r="3"><c r="A3" t="inlineStr"><is><t>Oolong</t></is></c><c r="B3" s="1"><v>45659</v></c><c r="C3"><v>23</v></c></row>
        </sheetData><tableParts count="1"><tablePart r:id="rId3"/></tableParts></worksheet>"#
    );
    let data_rels = rels(&[("rId3", REL, "table", "../tables/table1.xml")]);
    let table = format!(
        r#"<table xmlns="{MAIN}" id="1" name="TeaOrders" displayName="TeaOrders" ref="A1:C3" totalsRowShown="0"><autoFilter ref="A1:C3"/><tableColumns count="3"><tableColumn id="1" name="Blend"/><tableColumn id="2" name="Day"/><tableColumn id="3" name="Units"/></tableColumns><tableStyleInfo name="TableStyleMedium2" showRowStripes="1"/></table>"#
    );
    let cache = format!(
        r#"<pivotCacheDefinition xmlns="{MAIN}" xmlns:r="{REL}" xmlns:x14="{X14}" r:id="rId5" recordCount="2" createdVersion="6" refreshedVersion="6" minRefreshableVersion="3">
            <cacheSource type="worksheet"><worksheetSource name="TeaOrders"/></cacheSource>
            <cacheFields count="3"><cacheField name="Blend"><sharedItems count="2"><s v="Mint"/><s v="Oolong"/></sharedItems></cacheField>
                <cacheField name="Day" numFmtId="14"><sharedItems containsDate="1" containsNonDate="0" containsSemiMixedTypes="0" containsString="0" count="2"><d v="2025-01-01T00:00:00"/><d v="2025-01-02T00:00:00"/></sharedItems></cacheField>
                <cacheField name="Units" numFmtId="201"><sharedItems containsNumber="1" containsString="0"/></cacheField></cacheFields>
            <extLst><ext uri="{EXT_CACHE}"><x14:pivotCacheDefinition pivotCacheId="1"/></ext></extLst>
        </pivotCacheDefinition>"#
    );
    let records = format!(
        r#"<pivotCacheRecords xmlns="{MAIN}" count="2"><r><x v="0"/><x v="0"/><n v="19"/></r><r><x v="1"/><x v="1"/><n v="23"/></r></pivotCacheRecords>"#
    );
    let cache_rels = rels(&[("rId5", REL, "pivotCacheRecords", "pivotCacheRecords1.xml")]);
    let pivot_rels = rels(&[("rId6", REL, "pivotCacheDefinition", "../pivotCache/pivotCacheDefinition1.xml")]);
    let mut pivots = [pivot("TeaTotals", "A3:B6"), pivot("TeaComparison", "D3:E6")];
    if matches!(shape, Fixture::InvalidPivotFilter) {
        pivots[0] = pivots[0].replace("<filter fld=\"0\"", "<filter fld=\"9\"");
    }
    let connections = r#"<pivotTables><pivotTable tabId="7" name="TeaTotals"/><pivotTable tabId="7" name="TeaComparison"/></pivotTables>"#;
    let slicer = format!(
        r#"<slicers xmlns="{X14}"><slicer name="BlendControl" cache="BlendCache" caption="Tea blends" style="SlicerStyleLight1"/></slicers>"#
    );
    let slicer_cache = format!(
        r#"<slicerCacheDefinition xmlns="{X14}" name="BlendCache" sourceName="Blend">{connections}
            <data><tabular pivotCacheId="1"><items count="2"><i x="0" s="1" nd="0"/><i x="1" s="0" nd="0"/></items></tabular></data>
        </slicerCacheDefinition>"#
    );
    let timeline = format!(
        r#"<timelines xmlns="{X15}"><timeline name="DayControl" cache="DayCache" caption="Order dates" style="TimeSlicerStyleLight1"/></timelines>"#
    );
    let timeline_cache = format!(
        r#"<timelineCacheDefinition xmlns="{X15}" name="DayCache" sourceName="Day">
            <state pivotCacheId="1" minimalRefreshVersion="6" lastRefreshVersion="6" filterType="dateBetween" singleRangeFilterState="1">
                <selection startDate="2025-01-01T00:00:00" endDate="2025-01-01T00:00:00"/>
                <bounds startDate="2025-01-01T00:00:00" endDate="2025-01-02T00:00:00"/>
            </state>{connections}
        </timelineCacheDefinition>"#
    );
    let drawing = format!(
        r#"<xdr:wsDr xmlns:xdr="{DRAWING}" xmlns:a="{GRAPHICS}">{}{}</xdr:wsDr>"#,
        control_anchor(14, 6, SLICER_ANCHOR, "slicer", "BlendControl"),
        control_anchor(22, 10, TIMELINE_ANCHOR, "timeslicer", "DayControl"),
    );
    let styles = format!(
        r#"<styleSheet xmlns="{MAIN}">
            <numFmts count="1"><numFmt numFmtId="201" formatCode="0.0&quot; units&quot;"/></numFmts>
            <fonts count="1"><font/></fonts><fills count="1"><fill><patternFill patternType="none"/></fill></fills><borders count="1"><border/></borders>
            <cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
            <cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="14" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/></cellXfs>
            <dxfs count="1"><dxf><font><b/></font></dxf></dxfs>
            <tableStyles count="1"><tableStyle name="NativeReportStyle" pivot="1" table="0" count="1"><tableStyleElement type="wholeTable" dxfId="0"/></tableStyle></tableStyles>
        </styleSheet>"#
    );
    let parts = [
        (WORKBOOK, "sheet.main", &workbook),
        (REPORT, "worksheet", &report),
        ("xl/worksheets/sheet2.xml", "worksheet", &data),
        ("xl/styles.xml", "styles", &styles),
        ("xl/tables/table1.xml", "table", &table),
        ("xl/pivotCache/pivotCacheDefinition1.xml", "pivotCacheDefinition", &cache),
        ("xl/pivotCache/pivotCacheRecords1.xml", "pivotCacheRecords", &records),
        ("xl/pivotTables/pivotTable1.xml", "pivotTable", &pivots[0]),
        ("xl/pivotTables/pivotTable2.xml", "pivotTable", &pivots[1]),
        ("xl/drawings/drawing1.xml", "drawing", &drawing),
        ("xl/slicers/slicer1.xml", "slicer", &slicer),
        ("xl/slicerCaches/slicerCache1.xml", "slicerCache", &slicer_cache),
        ("xl/timelines/timeline1.xml", "timeline", &timeline),
        ("xl/timelineCaches/timelineCache1.xml", "timelineCache", &timeline_cache),
    ];
    let mut types = super::CT.trim_end_matches("</Types>").to_string();
    for (part, kind, _) in parts {
        let content_type = match kind {
            "drawing" => "application/vnd.openxmlformats-officedocument.drawing+xml".into(),
            "slicer" | "slicerCache" | "timeline" | "timelineCache" => format!("application/vnd.ms-excel.{kind}+xml"),
            _ => format!("application/vnd.openxmlformats-officedocument.spreadsheetml.{kind}+xml"),
        };
        write!(types, r#"<Override PartName="/{part}" ContentType="{content_type}"/>"#).unwrap();
    }
    types.push_str("</Types>");
    let mut root_links = vec![("rId1", REL, "officeDocument", WORKBOOK)];
    if matches!(shape, Fixture::ExtraRootRelationship) {
        root_links.push(("rId2", SLICER_REL, "slicer", "xl/slicers/slicer1.xml"));
    }
    let root_rels = rels(&root_links);
    let mut package = vec![
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", root_rels.as_bytes()),
        ("xl/_rels/workbook.xml.rels", workbook_rels.as_bytes()),
        ("xl/worksheets/_rels/sheet1.xml.rels", report_rels.as_bytes()),
        ("xl/worksheets/_rels/sheet2.xml.rels", data_rels.as_bytes()),
        ("xl/pivotCache/_rels/pivotCacheDefinition1.xml.rels", cache_rels.as_bytes()),
        ("xl/pivotTables/_rels/pivotTable1.xml.rels", pivot_rels.as_bytes()),
        ("xl/pivotTables/_rels/pivotTable2.xml.rels", pivot_rels.as_bytes()),
    ];
    package.extend(parts.iter().map(|(part, _, text)| (*part, text.as_bytes())));
    super::make_zip(&package)
}

fn part(package: &mut Package<'_>, name: &str) -> El {
    package.read_xml(name).unwrap().unwrap_or_else(|| panic!("missing {name}"))
}

fn mounted(package: &mut Package<'_>, owner: &str, id: &str, kind: &str) -> String {
    let rel = package.rels(owner).unwrap().into_iter().find(|rel| rel.id == id).unwrap_or_else(|| panic!("{owner} missing {id}"));
    assert_eq!(rel.kind, kind);
    assert!(!rel.external);
    assert!(package.has(&rel.target), "{owner} targets missing {}", rel.target);
    rel.target
}

fn by_kind(package: &mut Package<'_>, owner: &str, kind: &str) -> Vec<String> {
    package.rels(owner).unwrap().into_iter().filter(|rel| rel.kind == kind).map(|rel| mounted(package, owner, &rel.id, kind)).collect()
}

fn control_target(package: &mut Package<'_>, owner: &str, tree: &El, uri: &str, group: &str, tag: &str, kind: &str) -> String {
    let extension = tree.child("extLst").into_iter().flat_map(|list| list.kids("ext")).find(|ext| ext.attr("uri") == Some(uri));
    let id = extension
        .and_then(|ext| ext.path(&[group, tag]))
        .and_then(|control| control.attr("id"))
        .unwrap_or_else(|| panic!("{owner} missing native {kind} extension mount"));
    mounted(package, owner, id, kind)
}

fn assert_native_graph(bytes: &[u8]) {
    let mut package = Package::open(bytes).unwrap();
    let workbook = part(&mut package, WORKBOOK);
    let slicer_cache = control_target(&mut package, WORKBOOK, &workbook, SLICER_CACHE_MOUNT, "slicerCaches", "slicerCache", "slicerCache");
    let timeline_cache =
        control_target(&mut package, WORKBOOK, &workbook, TIMELINE_CACHE_MOUNT, "timelineCacheRefs", "timelineCacheRef", "timelineCache");
    let sheet = workbook.child("sheets").unwrap().kids("sheet").find(|sheet| sheet.attr("name") == Some("Report")).unwrap();
    assert_eq!(sheet.attr("sheetId"), Some("7"));
    let report_part = mounted(&mut package, WORKBOOK, sheet.attr("id").unwrap(), "worksheet");
    let report = part(&mut package, &report_part);
    let slicer = control_target(&mut package, &report_part, &report, SLICER_MOUNT, "slicerList", "slicer", "slicer");
    let timeline = control_target(&mut package, &report_part, &report, TIMELINE_MOUNT, "timelineRefs", "timelineRef", "timeline");
    let slicer = part(&mut package, &slicer);
    let timeline = part(&mut package, &timeline);
    let slicer_cache = part(&mut package, &slicer_cache);
    let timeline_cache = part(&mut package, &timeline_cache);
    for (control, view, name, cache, cache_name) in
        [(slicer, "slicer", "BlendControl", &slicer_cache, "BlendCache"), (timeline, "timeline", "DayControl", &timeline_cache, "DayCache")]
    {
        let control = control.child(view).unwrap();
        assert_eq!(control.attr("name"), Some(name));
        assert_eq!(control.attr("cache"), Some(cache_name));
        assert_eq!(cache.attr("name"), Some(cache_name));
        let pivots: Vec<_> = cache.child("pivotTables").unwrap().kids("pivotTable").map(|p| (p.attr("tabId"), p.attr("name"))).collect();
        assert_eq!(pivots, [(Some("7"), Some("TeaTotals")), (Some("7"), Some("TeaComparison"))]);
    }
    let slicer_state = slicer_cache.path(&["data", "tabular"]).unwrap();
    let timeline_state = timeline_cache.child("state").unwrap();
    assert_eq!(slicer_state.attr("pivotCacheId"), Some("1"));
    assert_eq!(timeline_state.attr("pivotCacheId"), Some("1"));
    let selection: Vec<_> = slicer_state.child("items").unwrap().kids("i").map(|item| (item.attr("x"), item.attr("s"))).collect();
    assert_eq!(selection, [(Some("0"), Some("1")), (Some("1"), Some("0"))]);
    assert_eq!(timeline_state.child("selection").unwrap().attr("endDate"), Some("2025-01-01T00:00:00"));

    let cache = workbook.path(&["pivotCaches", "pivotCache"]).unwrap();
    assert_eq!(cache.attr("cacheId"), Some("2"));
    assert_eq!(workbook.child("pivotCaches").unwrap().kids("pivotCache").count(), 1);
    let cache_path = mounted(&mut package, WORKBOOK, cache.attr("id").unwrap(), "pivotCacheDefinition");
    let cache = part(&mut package, &cache_path);
    assert_eq!(cache.path(&["cacheSource", "worksheetSource"]).unwrap().attr("name"), Some("TeaOrders"));
    assert_eq!(cache.path(&["extLst", "ext", "pivotCacheDefinition"]).unwrap().attr("pivotCacheId"), Some("1"));
    let records_path = mounted(&mut package, &cache_path, cache.attr("id").unwrap(), "pivotCacheRecords");
    let records = part(&mut package, &records_path);
    assert_eq!(records.kids("r").count(), 2);
    assert_eq!(records.kids("r").nth(1).unwrap().child("n").unwrap().attr("v"), Some("23"));

    let paths = by_kind(&mut package, &report_part, "pivotTable");
    assert_eq!(paths.len(), 2);
    let styles_path = by_kind(&mut package, WORKBOOK, "styles").remove(0);
    let styles = part(&mut package, &styles_path);
    let formats = styles.child("numFmts").unwrap();
    let dxfs = styles.child("dxfs").unwrap();
    let style = styles.child("tableStyles").unwrap().kids("tableStyle").find(|style| style.attr("name") == Some("NativeReportStyle")).unwrap();
    let mut names = Vec::new();
    for path in paths {
        let pivot = part(&mut package, &path);
        names.push(pivot.attr("name").unwrap().to_string());
        assert_eq!(pivot.attr("cacheId"), Some("2"));
        assert_eq!(by_kind(&mut package, &path, "pivotCacheDefinition"), [cache_path.as_str()]);
        assert_eq!(pivot.path(&["filters", "filter"]).unwrap().attr("stringValue1"), Some("Mint"));
        assert_eq!(pivot.child("pivotTableStyleInfo").unwrap().attr("name"), Some("NativeReportStyle"));
        let dxf_id = pivot.path(&["formats", "format"]).unwrap().attr_u32("dxfId").unwrap();
        assert_eq!(style.child("tableStyleElement").unwrap().attr_u32("dxfId"), Some(dxf_id));
        assert!(dxfs.kids("dxf").nth(dxf_id as usize).unwrap().path(&["font", "b"]).is_some());
        let format_id = pivot.path(&["dataFields", "dataField"]).unwrap().attr("numFmtId").unwrap();
        assert!(formats.kids("numFmt").any(|f| f.attr("numFmtId") == Some(format_id) && f.attr("formatCode") == Some("0.0\" units\"")));
    }
    names.sort();
    assert_eq!(names, ["TeaComparison", "TeaTotals"]);

    let drawing_path = mounted(&mut package, &report_part, report.child("drawing").unwrap().attr("id").unwrap(), "drawing");
    let drawing = part(&mut package, &drawing_path);
    for (kind, name, col) in [("slicer", "BlendControl", "6"), ("timeslicer", "DayControl", "10")] {
        let anchor = drawing
            .kids("twoCellAnchor")
            .find(|anchor| {
                anchor.path(&["AlternateContent", "Choice", "graphicFrame", "graphic", "graphicData", kind]).and_then(|c| c.attr("name"))
                    == Some(name)
            })
            .unwrap_or_else(|| panic!("missing {kind} drawing anchor"));
        assert_eq!(anchor.path(&["from", "col"]).unwrap().text, col);
    }
}

#[test]
fn native_pivot_slicer_and_timeline_graph_survives_an_unrelated_edit() {
    let input = fixture(Fixture::Complete);
    assert_native_graph(&input);
    let (mut workbook, imported) = read_xlsx(&input).unwrap();
    let spare = CellRef::parse("Z20").unwrap();
    workbook
        .sheet_mut(0)
        .unwrap()
        .cells
        .set(spare, Cell { value: Value::Number(42.0), formula: Some(Arc::new(Formula::new("40+2"))), ..Cell::default() });
    let saved = write_xlsx(&workbook).unwrap();
    assert_native_graph(&saved);
    assert!(imported.issues.is_empty(), "{:?}", imported.issues);
    let (reopened, reopened_report) = read_xlsx(&saved).unwrap();
    assert!(reopened_report.issues.is_empty(), "{:?}", reopened_report.issues);
    let report = reopened.sheet(0).unwrap();
    let cell = report.cell(spare).unwrap();
    assert_eq!(cell.value, Value::Number(42.0));
    assert_eq!(cell.formula.as_ref().unwrap().text, "40+2");
    assert_eq!(report.pivots.len(), 2);
    assert_eq!(reopened.sheet(1).unwrap().tables[0].name, "TeaOrders");
    assert_native_graph(&write_xlsx(&reopened).unwrap());
}

#[test]
fn native_analysis_refuses_changes_inside_the_source() {
    use gridcraft_core::RangeRef;
    use gridcraft_formula::adjust::{Axis, Edit};

    let (original, _) = read_xlsx(&fixture(Fixture::Complete)).unwrap();
    let source_cell = CellRef::parse("C2").unwrap();
    let mut candidate = original.clone();
    candidate.sheet_mut(1).unwrap().set_value(source_cell, Value::Number(31.0));
    assert!(crate::prepare_preserved_edit(&original, &candidate, &[], &[]).is_err());
    let error = write_xlsx(&candidate).unwrap_err();
    assert!(error.to_string().contains("native PivotTable"), "{error}");
    assert_eq!(original.sheet(1).unwrap().value(source_cell), Value::Number(19.0));

    let mut candidate = original.clone();
    candidate.sheet_mut(1).unwrap().tables[0].range = RangeRef::parse("A1:C4").unwrap();
    let edit = Edit::Insert { axis: Axis::Rows, at: 1, count: 1 };
    assert!(crate::prepare_preserved_edit(&original, &candidate, &[("Data".into(), edit)], &[]).is_err());
    assert!(write_xlsx(&candidate).is_err());
    assert_native_graph(&write_xlsx(&original).unwrap());
}

#[test]
fn native_analysis_refuses_merges_crossing_source_or_report_boundaries() {
    use gridcraft_core::RangeRef;

    let (original, _) = read_xlsx(&fixture(Fixture::Complete)).unwrap();
    for (sheet, range) in [(0, "B6:C6"), (1, "C3:D3")] {
        let mut candidate = original.clone();
        candidate.sheet_mut(sheet).unwrap().merges.push(RangeRef::parse(range).unwrap());
        assert!(crate::prepare_preserved_edit(&original, &candidate, &[], &[]).is_err(), "{range}");
        let error = write_xlsx(&candidate).unwrap_err();
        assert!(error.to_string().contains("native PivotTable"), "{error}");
    }

    let mut candidate = original.clone();
    let unrelated = RangeRef::parse("Z20:AA20").unwrap();
    candidate.sheet_mut(0).unwrap().merges.push(unrelated);
    assert!(crate::prepare_preserved_edit(&original, &candidate, &[], &[]).is_ok());
    let saved = write_xlsx(&candidate).unwrap();
    assert_native_graph(&saved);
    let (reopened, _) = read_xlsx(&saved).unwrap();
    assert_eq!(reopened.sheet(0).unwrap().merges, [unrelated]);
}

#[test]
fn native_analysis_missing_relationship_keeps_loss_diagnostics() {
    let (workbook, report) = read_xlsx(&fixture(Fixture::MissingTimelineRelationship)).unwrap();
    assert!(report.issues.iter().any(|i| i.feature == "native slicer" && i.code == "feature.omitted"), "{:?}", report.issues);
    let saved = write_xlsx(&workbook).unwrap();
    let mut package = Package::open(&saved).unwrap();
    assert!(by_kind(&mut package, WORKBOOK, "slicerCache").is_empty());
    assert!(by_kind(&mut package, REPORT, "slicer").is_empty());
    let (reopened, _) = read_xlsx(&saved).unwrap();
    assert_eq!(reopened.sheet(1).unwrap().value(CellRef::parse("C2").unwrap()), Value::Number(19.0));
}

#[test]
fn native_analysis_remains_outside_recovery_validation() {
    let (workbook, report) = read_xlsx(&fixture(Fixture::Complete)).unwrap();
    assert!(report.issues.is_empty(), "{:?}", report.issues);
    assert_eq!(crate::validate_recovery_preserved(&workbook), Err(crate::RecoveryPreservedError::UnsupportedNative));
}

#[test]
fn native_analysis_does_not_hide_an_invalid_filter_index() {
    let (workbook, report) = read_xlsx(&fixture(Fixture::InvalidPivotFilter)).unwrap();
    assert!(workbook.preserved.as_deref().is_none_or(|owner| owner.native_analysis.is_none()));
    assert!(report.issues.iter().any(|issue| issue.feature == "native slicer" && issue.code == "feature.omitted"));
    assert!(report.warnings.iter().any(|warning| warning.contains("PivotTable filter refers to a missing field")), "{:?}", report.warnings);
}

#[test]
fn native_analysis_keeps_the_diagnostic_for_an_unwritten_root_relationship() {
    let (workbook, report) = read_xlsx(&fixture(Fixture::ExtraRootRelationship)).unwrap();
    assert!(
        report.issues.iter().any(|issue| issue.code == "feature.omitted"
            && issue.feature == "native slicer"
            && issue.location.as_deref() == Some("package: xl/slicers/slicer1.xml")),
        "{:?}",
        report.issues
    );
    let saved = write_xlsx(&workbook).unwrap();
    assert_native_graph(&saved);
    let mut package = Package::open(&saved).unwrap();
    assert!(by_kind(&mut package, "", "slicer").is_empty());
}

#[test]
fn native_analysis_does_not_reuse_a_pivot_dxf_for_a_generated_rule() {
    use gridcraft_core::RangeRef;
    use gridcraft_model::{CfRule, CondFormat, Style};

    let (mut workbook, _) = read_xlsx(&fixture(Fixture::Complete)).unwrap();
    let mut style = Style::default();
    style.font.italic = true;
    workbook.sheet_mut(0).unwrap().cond_formats.push(CondFormat {
        ranges: vec![RangeRef::parse("Z20").unwrap()],
        rule: CfRule::Expression { formula: "Z20>0".into(), style: Box::new(style) },
        priority: 1,
        stop_if_true: false,
    });

    for _ in 0..2 {
        let saved = write_xlsx(&workbook).unwrap();
        assert_native_graph(&saved);
        let mut package = Package::open(&saved).unwrap();
        let sheet = part(&mut package, REPORT);
        let rule = sheet.path(&["conditionalFormatting", "cfRule"]).unwrap();
        assert_eq!(rule.attr_u32("dxfId"), Some(1));
        let styles = part(&mut package, "xl/styles.xml");
        let dxfs = styles.child("dxfs").unwrap();
        assert_eq!(dxfs.kids("dxf").count(), 2, "the generated DXF accumulated or displaced the native one");
        let generated = dxfs.kids("dxf").nth(1).unwrap();
        assert!(generated.path(&["font", "i"]).is_some());
        assert!(generated.path(&["font", "b"]).is_none());
        let (reopened, report) = read_xlsx(&saved).unwrap();
        assert!(report.issues.is_empty(), "{:?}", report.issues);
        let CfRule::Expression { style, .. } = &reopened.sheet(0).unwrap().cond_formats[0].rule else {
            panic!("generated conditional format did not reopen");
        };
        assert!(style.font.italic && !style.font.bold);
        workbook = reopened;
    }
}
