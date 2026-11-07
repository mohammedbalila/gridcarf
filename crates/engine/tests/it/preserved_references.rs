//! Preserved reference safety through real XLSX import and Session commands.
//! Fixtures are synthetic public-schema XML, not Excel-authored files.

use gridcraft_core::{CellRef, Value};
use gridcraft_engine::{DocState, Session, Trust, io};
use gridcraft_model::{Preserved, Workbook};
use serde_json::{Value as Json, json};
use std::io::{Cursor, Write};
use std::sync::Arc;

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const LINK: &str = "Data!$C$10";
const PROP: &str = r#"<?xml version="1.0"?><formControlPr xmlns="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" objectType="CheckBox" fmlaLink="Data!$C$10" lockText="1"/>"#;
const VML: &str = r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:x="urn:schemas-microsoft-com:office:excel"><v:shape id="_x0000_s1025" style="position:absolute;margin-left:0pt;margin-top:0pt;width:48pt;height:15pt"><x:ClientData ObjectType="Checkbox"><x:MoveWithCells/><x:SizeWithCells/><x:Anchor>0, 0, 0, 0, 1, 0, 1, 0</x:Anchor><x:FmlaLink>Data!$C$10</x:FmlaLink></x:ClientData></v:shape></xml>"#;
const CONTROL: &str = r#"<controls><control shapeId="1025" r:id="rIdControl" name="Checkbox"><controlPr linkedCell="Data!$C$10"><anchor moveWithCells="1" sizeWithCells="1"><from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>0</xdr:row><xdr:rowOff>0</xdr:rowOff></from><to><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></to></anchor></controlPr></control></controls>"#;
const UNKNOWN: &str = r#"<data xmlns="urn:gridcraft:test:unknown"><reference>Data!C10</reference></data>"#;
const QUERY: &str = r#"<queryTable xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" name="ExternalData_1" connectionId="1"/>"#;

#[derive(Default)]
struct Fixture<'a> {
    prop: Option<&'a str>,
    vml: Option<&'a str>,
    control: Option<&'a str>,
    unknown: bool,
    query: bool,
    blank_linked_cell: bool,
    owner_cells: &'a str,
    shared_owner: bool,
    owner_namespace: Option<&'a str>,
}

fn relationships(items: &[(&str, &str, &str)]) -> String {
    let mut xml = String::from(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    for (id, kind, target) in items {
        xml.push_str(&format!(r#"<Relationship Id="{id}" Type="{REL}/{kind}" Target="{target}"/>"#));
    }
    xml.push_str("</Relationships>");
    xml
}

fn fixture(options: Fixture<'_>) -> Session {
    let mut types = String::from(
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="vml" ContentType="application/vnd.openxmlformats-officedocument.vmlDrawing"/>"#,
    );
    for (part, kind) in [
        ("xl/workbook.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"),
        ("xl/worksheets/sheet1.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"),
        ("xl/worksheets/sheet2.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"),
        ("xl/worksheets/sheet3.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"),
        ("xl/ctrlProps/control.xml", "application/vnd.ms-excel.controlproperties+xml"),
    ] {
        types.push_str(&format!(r#"<Override PartName="/{part}" ContentType="{kind}"/>"#));
    }
    let mut wb_rels = vec![
        ("rId1", "worksheet", "worksheets/sheet1.xml"),
        ("rId2", "worksheet", "worksheets/sheet2.xml"),
        ("rId3", "worksheet", "worksheets/sheet3.xml"),
    ];
    if options.unknown {
        wb_rels.push(("rIdUnknown", "customXml", "../customXml/item1.xml"));
    }
    if options.query {
        wb_rels.push(("rIdConnection", "connections", "connections.xml"));
        for (part, kind) in [("queryTables/query.xml", "queryTable"), ("connections.xml", "connections")] {
            types.push_str(&format!(
                r#"<Override PartName="/xl/{part}" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.{kind}+xml"/>"#
            ));
        }
    }
    types.push_str("</Types>");
    let workbook = format!(
        r#"<workbook xmlns="{MAIN}" xmlns:r="{REL}"><sheets><sheet name="Controls" sheetId="1" r:id="rId1"/><sheet name="Data" sheetId="2" r:id="rId2"/><sheet name="Target" sheetId="3" r:id="rId3"/></sheets></workbook>"#
    );
    let controls = format!(
        r#"<worksheet xmlns="{MAIN}" xmlns:r="{REL}" xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing"><sheetData>{}</sheetData><legacyDrawing r:id="rIdVml"/>{}</worksheet>"#,
        options.owner_cells,
        options.control.unwrap_or(CONTROL)
    );
    let controls = if let Some(namespace) = options.owner_namespace {
        controls
            .replacen(&format!(r#"<worksheet xmlns="{MAIN}""#), &format!(r#"<s:worksheet xmlns:s="{MAIN}" xmlns="{namespace}""#), 1)
            .replace("<sheetData>", "<s:sheetData>")
            .replace("</sheetData>", "</s:sheetData>")
            .replace("<legacyDrawing ", "<s:legacyDrawing ")
            .replace("</worksheet>", "</s:worksheet>")
    } else {
        controls
    };
    let data = if options.blank_linked_cell { "" } else { r#"<row r="10"><c r="C10"><v>42</v></c></row>"# };
    let data = format!(r#"<worksheet xmlns="{MAIN}"><sheetData>{data}</sheetData></worksheet>"#);
    let target = if options.shared_owner { controls.clone() } else { format!(r#"<worksheet xmlns="{MAIN}"><sheetData/></worksheet>"#) };
    let mut files = vec![
        ("[Content_Types].xml", types),
        ("_rels/.rels", relationships(&[("rId1", "officeDocument", "xl/workbook.xml")])),
        ("xl/workbook.xml", workbook),
        ("xl/_rels/workbook.xml.rels", relationships(&wb_rels)),
        ("xl/worksheets/sheet1.xml", controls),
        ("xl/worksheets/sheet2.xml", data),
        ("xl/worksheets/sheet3.xml", target),
        (
            "xl/worksheets/_rels/sheet1.xml.rels",
            relationships(&[("rIdControl", "ctrlProp", "../ctrlProps/control.xml"), ("rIdVml", "vmlDrawing", "../drawings/control.vml")]),
        ),
        ("xl/ctrlProps/control.xml", options.prop.unwrap_or(PROP).into()),
        ("xl/drawings/control.vml", options.vml.unwrap_or(VML).into()),
    ];
    if options.shared_owner {
        files.push((
            "xl/worksheets/_rels/sheet3.xml.rels",
            relationships(&[("rIdControl", "ctrlProp", "../ctrlProps/control.xml"), ("rIdVml", "vmlDrawing", "../drawings/control.vml")]),
        ));
    }
    if options.unknown {
        files.push(("customXml/item1.xml", UNKNOWN.into()));
    }
    if options.query {
        files.push(("xl/worksheets/_rels/sheet2.xml.rels", relationships(&[("rIdQuery", "queryTable", "../queryTables/query.xml")])));
        files.push(("xl/queryTables/query.xml", QUERY.into()));
        files.push(("xl/connections.xml", format!(r#"<connections xmlns="{MAIN}"><connection id="1" name="Query" type="5" refreshedVersion="6"><dbPr connection="Provider=Synthetic" command="SELECT 1"/></connection></connections>"#)));
    }
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, body) in files {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    let bytes = zip.finish().unwrap().into_inner();
    let (wb, _) = io::open_bytes("preserved.xlsx", &bytes).unwrap();
    // Import must really retain these objects, rather than passing the test with an empty graph.
    let owner = wb.sheet(0).unwrap();
    assert!(owner.controls.is_empty());
    let kept = owner.preserved.as_ref().unwrap();
    assert_eq!(kept.parts.len(), 1);
    assert!(kept.vml.is_some());
    assert_eq!(kept.elements.iter().filter(|e| e.slot == "controls").count(), 1);
    if options.shared_owner {
        let shared = wb.sheet(2).unwrap().preserved.as_ref().unwrap();
        assert_eq!(shared.parts[0].name, kept.parts[0].name);
        assert!(shared.vml.is_some());
    }
    if options.unknown {
        assert!(wb.preserved.as_ref().unwrap().parts.iter().any(|p| p.data.as_ref() == UNKNOWN.as_bytes()));
    }
    if options.query {
        assert!(wb.sheet(1).unwrap().preserved.as_ref().unwrap().parts.iter().any(|p| p.data.as_ref() == QUERY.as_bytes()));
    }
    let mut s = Session::new();
    s.add_document(DocState::new(wb, None, "preserved".into(), Trust::FromFile));
    s
}

fn owner(wb: &Workbook) -> &Preserved {
    wb.sheet(wb.sheet_index("Controls").unwrap()).unwrap().preserved.as_deref().unwrap()
}

fn copies(wb: &Workbook) -> [String; 3] {
    let p = owner(wb);
    [
        String::from_utf8(p.parts.iter().find(|p| p.content_type == "application/vnd.ms-excel.controlproperties+xml").unwrap().data.to_vec())
            .unwrap(),
        String::from_utf8(p.vml.as_ref().unwrap().data.to_vec()).unwrap(),
        p.elements.iter().find(|p| p.slot == "controls").unwrap().xml.clone(),
    ]
}

fn activate(s: &mut Session, sheet: &str) {
    s.execute("sheet.activate", json!({"sheet":sheet})).unwrap();
}

fn assert_links(s: &Session, before: &[String; 3], expected: &str) {
    for (original, actual) in before.iter().zip(copies(&s.doc().unwrap().wb)) {
        assert_eq!(original.matches(LINK).count(), 1, "fixture has one link in each representation");
        assert_eq!(actual, original.replace(LINK, expected), "only the intended reference bytes may change");
    }
    let bytes = io::save_bytes(&s.doc().unwrap().wb, "out.xlsx").unwrap();
    let (back, _) = io::open_bytes("out.xlsx", &bytes).unwrap();
    for text in copies(&back) {
        assert!(text.contains(expected), "saved reference did not survive: {text}");
    }
}

fn assert_refused(s: &mut Session, command: &str, params: Json) {
    let d = s.doc_mut().unwrap();
    d.touched = Some(vec![]);
    d.edits = Some(vec![]);
    let before = d.clone();
    let error = s.execute(command, params).expect_err("unsafe preserved-reference operation must be refused");
    assert!(error.to_string().to_lowercase().contains("preserved"), "wrong refusal for {command}: {error}");
    let after = s.doc().unwrap();
    assert!(Arc::ptr_eq(&before.wb, &after.wb), "refusal published a candidate: {command}");
    assert_eq!(*after.wb, *before.wb);
    assert_eq!(after.selection, before.selection);
    assert_eq!(after.sheet_selections, before.sheet_selections);
    assert_eq!((after.undo.len(), after.redo.len(), after.revision), (before.undo.len(), before.redo.len(), before.revision));
    assert_eq!(after.touched, before.touched);
    assert_eq!(after.edits, before.edits);
}

#[test]
fn linked_cell_tracks_insert_delete() {
    let mut s = fixture(Fixture::default());
    let before = copies(&s.doc().unwrap().wb);
    let geometry = s.doc().unwrap().wb.sheet(0).unwrap().clone();
    activate(&mut s, "Data");
    s.execute("home.insertRows", json!({"rows":"5"})).unwrap();
    assert_links(&s, &before, "Data!$C$11");
    assert_eq!(s.doc().unwrap().wb.sheet(1).unwrap().value(CellRef::parse("C11").unwrap()), Value::Number(42.0));
    s.execute("home.deleteRows", json!({"rows":"5"})).unwrap();
    assert_links(&s, &before, LINK);
    s.execute("home.insertColumns", json!({"cols":"B"})).unwrap();
    assert_links(&s, &before, "Data!$D$10");
    s.execute("home.deleteColumns", json!({"cols":"B"})).unwrap();
    assert_links(&s, &before, LINK);
    let after = s.doc().unwrap().wb.sheet(0).unwrap();
    assert_eq!(after.rows, geometry.rows);
    assert_eq!(after.cols, geometry.cols);
    assert_eq!((after.default_row_height, after.default_col_width), (geometry.default_row_height, geometry.default_col_width));
}

#[test]
fn linked_cell_tracks_real_cut_paste_and_history() {
    let mut s = fixture(Fixture::default());
    let before = copies(&s.doc().unwrap().wb);
    activate(&mut s, "Data");
    s.execute("edit.cut", json!({"range":"C10"})).unwrap();
    s.execute("edit.pasteSpecial", json!({"at":"F20","what":"all"})).unwrap();
    assert_links(&s, &before, "Data!$F$20");
    s.execute("edit.undo", json!({})).unwrap();
    assert_links(&s, &before, LINK);
    s.execute("edit.redo", json!({})).unwrap();
    assert_links(&s, &before, "Data!$F$20");
}

#[test]
fn blank_linked_cell_tracks_cross_sheet_cut() {
    let mut s = fixture(Fixture { blank_linked_cell: true, ..Default::default() });
    let before = copies(&s.doc().unwrap().wb);
    activate(&mut s, "Data");
    s.execute("edit.cut", json!({"range":"C10"})).unwrap();
    s.execute("edit.pasteSpecial", json!({"sheet":"Target","at":"F20","what":"all"})).unwrap();
    assert_links(&s, &before, "Target!$F$20");
}

#[test]
fn copy_sheet_has_explicit_policy() {
    let mut s = fixture(Fixture::default());
    assert_refused(&mut s, "sheet.move", json!({"sheet":"Controls","to":3,"copy":true}));
}

#[test]
fn control_owner_coordinates_are_refused() {
    let mut s = fixture(Fixture::default());
    assert_refused(&mut s, "home.insertRows", json!({"rows":"1"}));
}

#[test]
fn move_into_control_owner_is_refused() {
    let mut s = fixture(Fixture::default());
    activate(&mut s, "Data");
    s.execute("edit.cut", json!({"range":"C10"})).unwrap();
    assert_refused(&mut s, "edit.pasteSpecial", json!({"sheet":"Controls","at":"Z20","what":"all"}));
}

#[test]
fn deleting_link_target_is_refused() {
    let mut s = fixture(Fixture::default());
    activate(&mut s, "Data");
    assert_refused(&mut s, "home.deleteRows", json!({"rows":"10"}));
}

#[test]
fn opaque_reference_policy_refuses_unknown() {
    let mut s = fixture(Fixture { unknown: true, ..Default::default() });
    activate(&mut s, "Data");
    assert_refused(&mut s, "home.insertRows", json!({"rows":"5"}));
}

#[test]
fn surviving_unknown_graph_refuses_sheet_rename() {
    let mut s = fixture(Fixture { unknown: true, ..Default::default() });
    assert_refused(&mut s, "sheet.rename", json!({"sheet":"Data","name":"Renamed"}));
}

#[test]
fn query_table_has_no_invented_range_handler() {
    let mut s = fixture(Fixture { query: true, ..Default::default() });
    activate(&mut s, "Data");
    assert_refused(&mut s, "home.insertRows", json!({"rows":"5"}));
}

#[test]
fn conflicting_link_copies_are_refused() {
    let control = CONTROL.replace(LINK, "Data!$D$10");
    let mut s = fixture(Fixture { control: Some(&control), ..Default::default() });
    activate(&mut s, "Data");
    assert_refused(&mut s, "home.insertRows", json!({"rows":"5"}));
}

#[test]
fn unhandled_formula_field_is_refused() {
    let prop = PROP.replace(" lockText", " fmlaRange=\"Data!$D$1:$D$3\" lockText");
    let mut s = fixture(Fixture { prop: Some(&prop), ..Default::default() });
    activate(&mut s, "Data");
    assert_refused(&mut s, "home.insertRows", json!({"rows":"5"}));
}

#[test]
fn pending_owner_autofit_is_refused_before_publication() {
    let mut s = fixture(Fixture { owner_cells: r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Caption</t></is></c></row>"#, ..Default::default() });
    // Whole-row formatting queues fit_rows; candidate row metrics have not changed yet.
    assert_refused(&mut s, "home.fontSize", json!({"range":"1:1","size":32}));
}

#[test]
fn namespace_spoof_in_control_properties_is_refused() {
    let prop = PROP.replace("http://schemas.microsoft.com/office/spreadsheetml/2009/9/main", "urn:unknown");
    let mut s = fixture(Fixture { prop: Some(&prop), ..Default::default() });
    activate(&mut s, "Data");
    assert_refused(&mut s, "home.insertRows", json!({"rows":"5"}));
}

#[test]
fn inherited_foreign_default_namespace_is_refused() {
    let mut s = fixture(Fixture { owner_namespace: Some("urn:foreign"), ..Default::default() });
    activate(&mut s, "Data");
    assert_refused(&mut s, "home.insertRows", json!({"rows":"5"}));
}

#[test]
fn inherited_empty_default_namespace_is_refused() {
    let mut s = fixture(Fixture { owner_namespace: Some(""), ..Default::default() });
    activate(&mut s, "Data");
    assert_refused(&mut s, "home.insertRows", json!({"rows":"5"}));
}

#[test]
fn explicit_fragment_namespace_override_is_respected() {
    let control = CONTROL.replace("<controls>", &format!("<controls xmlns = '{MAIN}'>"));
    let mut s = fixture(Fixture { owner_namespace: Some("urn:foreign"), control: Some(&control), ..Default::default() });
    let before = copies(&s.doc().unwrap().wb);
    activate(&mut s, "Data");
    s.execute("home.insertRows", json!({"rows":"5"})).unwrap();
    assert_links(&s, &before, "Data!$C$11");
}

#[test]
fn unknown_control_extension_is_refused() {
    let prop = PROP
        .replace("/>", r#"><extLst><ext uri="urn:unknown"><u:reference xmlns:u="urn:unknown">Data!C10</u:reference></ext></extLst></formControlPr>"#);
    let mut s = fixture(Fixture { prop: Some(&prop), ..Default::default() });
    activate(&mut s, "Data");
    assert_refused(&mut s, "home.insertRows", json!({"rows":"5"}));
}

#[test]
fn ambiguous_cross_owner_part_is_refused() {
    let mut shared = fixture(Fixture { shared_owner: true, ..Default::default() });
    activate(&mut shared, "Data");
    assert_refused(&mut shared, "home.insertRows", json!({"rows":"5"}));
}

#[test]
fn invalid_boolean_attribute_is_refused() {
    let prop = PROP.replace("lockText=\"1\"", "lockText=\"\"");
    let mut invalid = fixture(Fixture { prop: Some(&prop), ..Default::default() });
    activate(&mut invalid, "Data");
    assert_refused(&mut invalid, "home.insertRows", json!({"rows":"5"}));
}

#[test]
fn owner_grid_metric_change_is_refused() {
    let mut s = fixture(Fixture::default());
    assert_refused(&mut s, "home.rowHeight", json!({"rows":"1","height":40}));
}

#[test]
fn hidden_owner_grid_metrics_are_refused() {
    let mut s = fixture(Fixture::default());
    assert_refused(&mut s, "home.hideRows", json!({"rows":"1:1"}));
    assert_refused(&mut s, "home.hideColumns", json!({"cols":"A:A"}));
}

#[test]
fn duplicate_empty_marker_is_refused() {
    let control = CONTROL.replace("<from>", "<from/><from>");
    let mut s = fixture(Fixture { control: Some(&control), ..Default::default() });
    activate(&mut s, "Data");
    assert_refused(&mut s, "home.insertRows", json!({"rows":"5"}));
}

#[test]
fn nonzero_or_conflicting_geometry_is_refused() {
    for vml in [VML.replace("0, 0, 0, 0, 1, 0, 1, 0", "0, 1, 0, 0, 1, 0, 1, 0"), VML.replace("1, 0, 1, 0", "2, 0, 2, 0")] {
        let mut s = fixture(Fixture { vml: Some(&vml), ..Default::default() });
        activate(&mut s, "Data");
        assert_refused(&mut s, "home.insertRows", json!({"rows":"5"}));
    }
}

#[test]
fn quoted_sheet_rename_remains_adjustable_after_reorder() {
    let mut s = fixture(Fixture::default());
    s.execute("sheet.rename", json!({"sheet":"Data","name":"New Data"})).unwrap();
    s.execute("sheet.move", json!({"sheet":"Controls","to":3})).unwrap();
    activate(&mut s, "New Data");
    s.execute("home.insertRows", json!({"rows":"5"})).unwrap();
    let texts = copies(&s.doc().unwrap().wb);
    assert!(texts[0].contains("&apos;New Data&apos;!$C$11"));
    assert!(texts[1].contains("'New Data'!$C$11"));
    assert!(texts[2].contains("&apos;New Data&apos;!$C$11"));
}

#[test]
fn unsupported_link_text_encoding_is_refused_atomically() {
    let mut s = fixture(Fixture::default());
    assert_refused(&mut s, "sheet.rename", json!({"sheet":"Data","name":"A&B"}));
}

#[test]
fn owner_identity_survives_reorder_and_unrelated_sheet_removal() {
    let mut s = fixture(Fixture { owner_cells: r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Caption</t></is></c></row>"#, ..Default::default() });
    let before = copies(&s.doc().unwrap().wb);
    let uid = s.doc().unwrap().wb.sheet(0).unwrap().uid.get();
    s.execute("sheet.move", json!({"sheet":"Controls","to":3})).unwrap();
    s.execute("home.deleteSheet", json!({"sheet":"Target"})).unwrap();
    assert_eq!(s.doc().unwrap().wb.sheet_index("Controls"), Some(1));
    assert_eq!(s.doc().unwrap().wb.sheet(1).unwrap().uid.get(), uid);
    activate(&mut s, "Data");
    s.execute("home.insertRows", json!({"rows":"5"})).unwrap();
    assert_links(&s, &before, "Data!$C$11");
    activate(&mut s, "Controls");
    assert_refused(&mut s, "home.fontSize", json!({"range":"1:1","size":32}));
}

#[test]
fn unknown_graph_refuses_table_names_but_allows_table_style() {
    let mut s = fixture(Fixture::default());
    activate(&mut s, "Data");
    s.execute("insert.table", json!({"range":"F1:G3","header":true,"name":"Items"})).unwrap();
    // Combine an existing table with imported opaque workbook content, as after opening a
    // workbook containing both. Mutations below still use their real command owners.
    let unknown = fixture(Fixture { unknown: true, ..Default::default() });
    Arc::make_mut(&mut s.doc_mut().unwrap().wb).preserved = unknown.doc().unwrap().wb.preserved.clone();
    let kept = s.doc().unwrap().wb.preserved.clone();
    s.execute("table.style", json!({"table":"Items","style":"TableStyleMedium2"})).unwrap();
    assert_eq!(s.doc().unwrap().wb.preserved, kept);
    assert_refused(&mut s, "table.rename", json!({"table":"Items","name":"Renamed"}));
    assert_refused(&mut s, "cell.set", json!({"cell":"F1","input":"RenamedColumn"}));
}

#[test]
fn single_quoted_attributes_and_comments_preserve_unrelated_bytes() {
    let prop = PROP.replace('"', "'").replace("<formControlPr", "<!-- keep me --><formControlPr");
    let mut s = fixture(Fixture { prop: Some(&prop), ..Default::default() });
    let before = copies(&s.doc().unwrap().wb);
    activate(&mut s, "Data");
    s.execute("home.insertRows", json!({"rows":"5"})).unwrap();
    assert_links(&s, &before, "Data!$C$11");
}

#[test]
fn queued_autofit_below_all_markers_is_safe() {
    let mut s = fixture(Fixture::default());
    let before = copies(&s.doc().unwrap().wb);
    s.execute("cell.set", json!({"cell":"A20","input":"Caption"})).unwrap();
    s.execute("home.fontSize", json!({"range":"20:20","size":32})).unwrap();
    assert!(s.doc().unwrap().wb.sheet(0).unwrap().row_height(19) > 40.0);
    assert_eq!(copies(&s.doc().unwrap().wb), before);
}

#[test]
fn queued_autofit_of_custom_height_row_is_safe() {
    let mut s = fixture(Fixture {
        owner_cells: r#"<row r="1" ht="15" customHeight="1"><c r="A1" t="inlineStr"><is><t>Caption</t></is></c></row>"#,
        ..Default::default()
    });
    let before = s.doc().unwrap().wb.clone();
    assert!(before.sheet(0).unwrap().rows.get(&0).unwrap().custom);
    s.execute("home.fontSize", json!({"range":"1:1","size":32})).unwrap();
    assert_eq!(s.doc().unwrap().wb.sheet(0).unwrap().row_height(0), before.sheet(0).unwrap().row_height(0));
    assert_eq!(copies(&s.doc().unwrap().wb), copies(&before));
}

#[test]
fn unknown_control_on_custom_height_row_allows_safe_formatting() {
    let prop = PROP
        .replace("/>", r#"><extLst><ext uri="urn:unknown"><u:reference xmlns:u="urn:unknown">Data!C10</u:reference></ext></extLst></formControlPr>"#);
    let mut s = fixture(Fixture {
        prop: Some(&prop),
        owner_cells: r#"<row r="1" ht="15" customHeight="1"><c r="A1" t="inlineStr"><is><t>Caption</t></is></c></row>"#,
        ..Default::default()
    });
    let before = s.doc().unwrap().wb.clone();
    s.execute("home.fontSize", json!({"range":"1:1","size":32})).expect("custom rows cannot auto-fit, even for unknown control schemas");
    assert_eq!(s.doc().unwrap().wb.sheet(0).unwrap().row_height(0), before.sheet(0).unwrap().row_height(0));
    assert_eq!(copies(&s.doc().unwrap().wb), copies(&before));
    assert_refused(&mut s, "home.rowHeight", json!({"rows":"1","height":40}));
    activate(&mut s, "Data");
    assert_refused(&mut s, "home.insertRows", json!({"rows":"5"}));

    let mut noncustom = fixture(Fixture {
        prop: Some(&prop),
        owner_cells: r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Caption</t></is></c></row>"#,
        ..Default::default()
    });
    assert_refused(&mut noncustom, "home.fontSize", json!({"range":"1:1","size":32}));
}

#[test]
fn safe_unrelated_edits_keep_unknown_bytes() {
    let mut s = fixture(Fixture { unknown: true, query: true, ..Default::default() });
    let before = s.doc().unwrap().wb.clone();
    activate(&mut s, "Data");
    s.execute("cell.set", json!({"cell":"Z30","input":"99"})).unwrap();
    s.execute("home.bold", json!({"range":"Z30"})).unwrap();
    s.execute("sheet.hide", json!({"sheet":"Target"})).unwrap();
    assert_eq!(s.doc().unwrap().wb.preserved, before.preserved);
    assert_eq!(owner(&s.doc().unwrap().wb), owner(&before));
    assert_eq!(s.doc().unwrap().wb.sheet(1).unwrap().preserved, before.sheet(1).unwrap().preserved);
}

#[test]
fn owner_deletion_and_plain_sheet_copy_stay_available() {
    let mut s = fixture(Fixture::default());
    s.execute("sheet.move", json!({"sheet":"Data","to":3,"copy":true})).unwrap();
    assert_eq!(s.doc().unwrap().wb.sheets.len(), 4);
    s.execute("home.deleteSheet", json!({"sheet":"Controls"})).unwrap();
    assert!(s.doc().unwrap().wb.sheet_index("Controls").is_none());
    let bytes = io::save_bytes(&s.doc().unwrap().wb, "out.xlsx").unwrap();
    let (back, _) = io::open_bytes("out.xlsx", &bytes).unwrap();
    assert!(back.sheets.iter().all(|sh| sh.preserved.is_none()));
}
