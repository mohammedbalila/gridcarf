//! Pass-through of unmodelled package content (custom XML, external links, connections, query
//! tables, printer settings, form controls, OLE objects). Packages are built in code.

mod recovery;

use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

use gridcraft_model::Sheet;

use super::make_zip;
use crate::package::{normalize, resolve, split_dir};
use crate::xml;
use crate::{read_xlsx, write_xlsx};

const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const PRINTER: &[u8] = &[0, 1, 2, 3, 250, 251, 252, 253, 9, 9, 9];
const OLE: &[u8] = b"\xD0\xCF\x11\xE0 not really a compound file";
const EMF: &[u8] = b"\x01\x00\x00\x00 preview";
const CUSTOM_XML: &str = r#"<?xml version="1.0"?><root xmlns="urn:example:data"><v>42</v></root>"#;
const CUSTOM_PROPS: &str = r#"<?xml version="1.0"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties"/>"#;

fn rels(items: &[(&str, &str, &str)]) -> String {
    let mut s = String::from(r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    for (id, ty, target) in items {
        let ty = if ty.starts_with("http") { ty.to_string() } else { format!("{NS_R}/{ty}") };
        let ext = if target.contains("://") { r#" TargetMode="External""# } else { "" };
        s.push_str(&format!(r#"<Relationship Id="{id}" Type="{ty}" Target="{target}"{ext}/>"#));
    }
    s.push_str("</Relationships>");
    s
}

const CONTENT_TYPES: &str = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="bin" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.printerSettings"/><Default Extension="emf" ContentType="image/x-emf"/><Default Extension="vml" ContentType="application/vnd.openxmlformats-officedocument.vmlDrawing"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/embeddings/oleObject1.bin" ContentType="application/vnd.openxmlformats-officedocument.oleObject"/><Override PartName="/customXml/itemProps1.xml" ContentType="application/vnd.openxmlformats-officedocument.customXmlProperties+xml"/><Override PartName="/xl/externalLinks/externalLink1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.externalLink+xml"/><Override PartName="/xl/connections.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.connections+xml"/><Override PartName="/xl/queryTables/queryTable1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.queryTable+xml"/><Override PartName="/xl/ctrlProps/ctrlProp1.xml" ContentType="application/vnd.ms-excel.controlproperties+xml"/><Override PartName="/docProps/custom.xml" ContentType="application/vnd.openxmlformats-officedocument.custom-properties+xml"/><Override PartName="/xl/comments1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml"/></Types>"#;

const EXTERNAL_LINK: &str = r#"<?xml version="1.0"?><externalLink xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><externalBook r:id="rId1"><sheetNames><sheetName val="Sheet1"/></sheetNames><sheetDataSet><sheetData sheetId="0"><row r="1"><cell r="A1"><v>7</v></cell></row></sheetData></sheetDataSet></externalBook></externalLink>"#;
const CONNECTIONS: &str = r#"<?xml version="1.0"?><connections xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><connection id="1" name="Query" type="5" refreshedVersion="6"><dbPr connection="Provider=Example" command="SELECT 1"/></connection></connections>"#;
const QUERY_TABLE: &str = r#"<?xml version="1.0"?><queryTable xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" name="ExternalData_1" connectionId="1" autoFormatId="16" applyNumberFormats="0" applyBorderFormats="0" applyFontFormats="0" applyPatternFormats="0" applyAlignmentFormats="0" applyWidthHeightFormats="0"/>"#;
const CTRL_PROP: &str = r#"<?xml version="1.0"?><formControlPr xmlns="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" objectType="CheckBox" fmlaLink="$C$1" lockText="1"/>"#;
const COMMENTS: &str = r#"<?xml version="1.0"?><comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><authors><author>A</author></authors><commentList><comment ref="A1" authorId="0"><text><t>note</t></text></comment></commentList></comments>"#;
const VML: &str = r##"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:x="urn:schemas-microsoft-com:office:excel"><o:shapelayout v:ext="edit"><o:idmap v:ext="edit" data="1"/></o:shapelayout><v:shapetype id="_x0000_t75" coordsize="21600,21600" o:spt="75"/><v:shape id="_x0000_s1025" type="#_x0000_t75" style="position:absolute"><v:imagedata o:relid="rId1" o:title=""/><x:ClientData ObjectType="Pict"><x:Anchor>1, 0, 1, 0, 3, 0, 5, 0</x:Anchor><x:CF>Pict</x:CF></x:ClientData></v:shape><v:shape id="_x0000_s1026" type="#_x0000_t201" style="position:absolute"><x:ClientData ObjectType="Checkbox"><x:Anchor>4, 0, 1, 0, 6, 0, 2, 0</x:Anchor><x:FmlaLink>$C$1</x:FmlaLink></x:ClientData></v:shape><v:shape id="_x0000_s1027" type="#_x0000_t202" style="position:absolute"><v:textbox><div>old note<br></div></v:textbox><x:ClientData ObjectType="Note"><x:Row>0</x:Row><x:Column>0</x:Column></x:ClientData></v:shape></xml>"##;

/// A workbook with every kind of kept content: package root custom properties, custom XML, an
/// external link (with formulas using it), a connection, and on sheet "Data" a query table,
/// printer settings, an OLE object (in `mc:AlternateContent`), a form control, a note and the VML
/// drawing all of them share.
fn rich_package() -> Vec<u8> {
    rich_package_with(CONTENT_TYPES, false)
}

/// [`rich_package`] with other content types; `twin` makes sheet "Other" a copy of "Data" that
/// shares its VML drawing and embedded parts.
fn rich_package_with(content_types: &str, twin: bool) -> Vec<u8> {
    let workbook = format!(
        r#"<?xml version="1.0"?><workbook xmlns="{NS_MAIN}" xmlns:r="{NS_R}"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/><sheet name="Other" sheetId="2" r:id="rId2"/></sheets><externalReferences><externalReference r:id="rId3"/></externalReferences><definedNames><definedName name="ExternalData_1" localSheetId="0" hidden="1">Data!$E$1:$E$2</definedName></definedNames></workbook>"#
    );
    let sheet1 = format!(
        r#"<?xml version="1.0"?><worksheet xmlns="{NS_MAIN}" xmlns:r="{NS_R}" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" mc:Ignorable="x14"><sheetData><row r="1"><c r="A1"><f>[1]Sheet1!$A$1</f><v>7</v></c><c r="B1"><f>SUM([1]Sheet1!A1:A3)*2</f><v>14</v></c></row></sheetData><pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/><pageSetup orientation="landscape" r:id="rId1"/><legacyDrawing r:id="rId2"/><mc:AlternateContent><mc:Choice Requires="x14"><oleObjects><mc:AlternateContent><mc:Choice Requires="x14"><oleObject progId="Package" shapeId="1025" r:id="rId3"><objectPr defaultSize="0" r:id="rId4"><anchor moveWithCells="1"><from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></from><to><xdr:col>3</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>5</xdr:row><xdr:rowOff>0</xdr:rowOff></to></anchor></objectPr></oleObject></mc:Choice><mc:Fallback><oleObject progId="Package" shapeId="1025" r:id="rId3"/></mc:Fallback></mc:AlternateContent></oleObjects></mc:Choice></mc:AlternateContent><controls><control shapeId="1026" r:id="rId5" name="Check Box 1"/></controls></worksheet>"#
    );
    let sheet2 = if twin {
        sheet1.clone()
    } else {
        format!(r#"<?xml version="1.0"?><worksheet xmlns="{NS_MAIN}"><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData></worksheet>"#)
    };
    let root_rels = rels(&[("rId1", "officeDocument", "xl/workbook.xml"), ("rId2", "custom-properties", "docProps/custom.xml")]);
    let wb_rels = rels(&[
        ("rId1", "worksheet", "worksheets/sheet1.xml"),
        ("rId2", "worksheet", "worksheets/sheet2.xml"),
        ("rId3", "externalLink", "externalLinks/externalLink1.xml"),
        ("rId4", "customXml", "../customXml/item1.xml"),
        ("rId5", "connections", "connections.xml"),
    ]);
    let sheet1_rels = rels(&[
        ("rId1", "printerSettings", "../printerSettings/printerSettings1.bin"),
        ("rId2", "vmlDrawing", "../drawings/vmlDrawing1.vml"),
        ("rId3", "oleObject", "../embeddings/oleObject1.bin"),
        ("rId4", "image", "../media/image1.emf"),
        ("rId5", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/ctrlProp", "../ctrlProps/ctrlProp1.xml"),
        ("rId6", "queryTable", "../queryTables/queryTable1.xml"),
        ("rId7", "comments", "../comments1.xml"),
    ]);
    let custom_rels = rels(&[("rId1", "customXmlProps", "itemProps1.xml")]);
    let link_rels = rels(&[("rId1", "externalLinkPath", "file:///C:/never/opened/Book2.xlsx")]);
    let vml_rels = rels(&[("rId1", "image", "../media/image1.emf")]);
    let item_props = r#"<?xml version="1.0"?><ds:datastoreItem ds:itemID="{11111111-2222-3333-4444-555555555555}" xmlns:ds="http://schemas.openxmlformats.org/officeDocument/2006/customXml"/>"#;
    let mut files: Vec<(&str, &[u8])> = vec![
        ("[Content_Types].xml", content_types.as_bytes()),
        ("_rels/.rels", root_rels.as_bytes()),
        ("docProps/custom.xml", CUSTOM_PROPS.as_bytes()),
        ("xl/workbook.xml", workbook.as_bytes()),
        ("xl/_rels/workbook.xml.rels", wb_rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet1.as_bytes()),
        ("xl/worksheets/sheet2.xml", sheet2.as_bytes()),
        ("xl/worksheets/_rels/sheet1.xml.rels", sheet1_rels.as_bytes()),
        ("xl/printerSettings/printerSettings1.bin", PRINTER),
        ("xl/drawings/vmlDrawing1.vml", VML.as_bytes()),
        ("xl/drawings/_rels/vmlDrawing1.vml.rels", vml_rels.as_bytes()),
        ("xl/embeddings/oleObject1.bin", OLE),
        ("xl/media/image1.emf", EMF),
        ("xl/ctrlProps/ctrlProp1.xml", CTRL_PROP.as_bytes()),
        ("xl/queryTables/queryTable1.xml", QUERY_TABLE.as_bytes()),
        ("xl/comments1.xml", COMMENTS.as_bytes()),
        ("customXml/item1.xml", CUSTOM_XML.as_bytes()),
        ("customXml/_rels/item1.xml.rels", custom_rels.as_bytes()),
        ("customXml/itemProps1.xml", item_props.as_bytes()),
        ("xl/externalLinks/externalLink1.xml", EXTERNAL_LINK.as_bytes()),
        ("xl/externalLinks/_rels/externalLink1.xml.rels", link_rels.as_bytes()),
        ("xl/connections.xml", CONNECTIONS.as_bytes()),
    ];
    if twin {
        files.push(("xl/worksheets/_rels/sheet2.xml.rels", sheet1_rels.as_bytes()));
    }
    make_zip(&files)
}

/// An unpacked package: parts by name, content types, resolved relationships.
struct Unpacked {
    parts: HashMap<String, Vec<u8>>,
}

/// (id, type, resolved target, external)
type RelList = Vec<(String, String, String, bool)>;

impl Unpacked {
    fn new(bytes: &[u8]) -> Unpacked {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut parts = HashMap::new();
        for i in 0..z.len() {
            let mut f = z.by_index(i).unwrap();
            let mut v = vec![];
            f.read_to_end(&mut v).unwrap();
            parts.insert(f.name().to_string(), v);
        }
        Unpacked { parts }
    }
    fn text(&self, name: &str) -> String {
        String::from_utf8(self.parts.get(name).unwrap_or_else(|| panic!("no part {name}")).clone()).unwrap()
    }
    fn rels(&self, part: &str) -> RelList {
        let (dir, file) = split_dir(part);
        let name = if dir.is_empty() { format!("_rels/{file}.rels") } else { format!("{dir}/_rels/{file}.rels") };
        let Some(b) = self.parts.get(&name) else { return vec![] };
        let root = xml::parse(b).unwrap();
        root.kids("Relationship")
            .map(|r| {
                let ext = r.attr("TargetMode") == Some("External");
                let t = r.attr("Target").unwrap();
                (r.attr("Id").unwrap().into(), r.attr("Type").unwrap().into(), if ext { t.into() } else { resolve(dir, t) }, ext)
            })
            .collect()
    }
    fn target(&self, part: &str, id: &str) -> (String, String) {
        let r = self.rels(part).into_iter().find(|r| r.0 == id).unwrap_or_else(|| panic!("{part}: no relationship {id}"));
        (r.1, r.2)
    }
    /// Every part has a content type, every internal relationship target exists, rel ids are
    /// unique per part.
    fn assert_valid(&self) {
        let types = xml::parse(self.parts.get("[Content_Types].xml").unwrap()).unwrap();
        let defaults: Vec<String> = types.kids("Default").map(|d| d.attr("Extension").unwrap().to_ascii_lowercase()).collect();
        let overrides: Vec<String> = types.kids("Override").map(|d| normalize(d.attr("PartName").unwrap())).collect();
        let mut seen_override = std::collections::HashSet::new();
        for o in &overrides {
            assert!(seen_override.insert(o.clone()), "duplicate override {o}");
            assert!(self.parts.keys().any(|p| normalize(p) == *o), "override for a missing part {o}");
        }
        for name in self.parts.keys().filter(|n| *n != "[Content_Types].xml") {
            let ext = name.rsplit('.').next().unwrap().to_ascii_lowercase();
            assert!(overrides.contains(&normalize(name)) || defaults.contains(&ext), "{name} has no content type");
            if name.ends_with(".rels") {
                let source = name.replace("_rels/", "").trim_end_matches(".rels").to_string();
                let rels = self.rels(&source);
                let mut ids = std::collections::HashSet::new();
                for (id, _, target, ext) in rels {
                    assert!(ids.insert(id.clone()), "{name}: duplicate id {id}");
                    if !ext {
                        assert!(self.parts.keys().any(|p| normalize(p) == normalize(&target)), "{name}: {id} → missing {target}");
                    }
                }
            }
        }
    }
    /// The part holding exactly `data`.
    fn find(&self, data: &[u8]) -> Vec<&String> {
        self.parts.iter().filter(|(_, v)| v.as_slice() == data).map(|(k, _)| k).collect()
    }
}

/// The value of attribute `attr` of the first `<tag …>` in `x` (`r:id` of `<pageSetup`).
fn attr_of(x: &str, tag: &str, attr: &str) -> String {
    let i = x.find(&format!("<{tag} ")).unwrap_or_else(|| panic!("no <{tag}> in {x}"));
    let head = &x[i..i + x[i..].find('>').unwrap()];
    let j = head.find(&format!(" {attr}=\"")).unwrap_or_else(|| panic!("no {attr} in {head}")) + attr.len() + 3;
    head[j..j + head[j..].find('"').unwrap()].to_string()
}

fn sheet_part(u: &Unpacked, sheet: &str) -> String {
    let wb = u.text("xl/workbook.xml");
    let i = wb.find(&format!("<sheet name=\"{sheet}\"")).unwrap();
    let id = attr_of(&wb[i..], "sheet", "r:id");
    u.target("xl/workbook.xml", &id).1
}

#[test]
fn unmodelled_parts_survive_a_round_trip() {
    let (wb, rep) = read_xlsx(&rich_package()).unwrap();
    assert!(wb.preserved.is_some(), "{:?}", rep.warnings);
    let data = wb.sheet(0).unwrap();
    assert!(data.preserved.is_some(), "{:?}", rep.warnings);
    assert!(!rep.warnings.iter().any(|w| w.contains("dropped")), "{:?}", rep.warnings);
    assert_eq!(data.comments.len(), 1, "the note is modelled");

    let out = write_xlsx(&wb).unwrap();
    let u = Unpacked::new(&out);
    u.assert_valid();
    assert_kept_workbook_parts(&u);
    let sheet = sheet_part(&u, "Data");
    let sx = u.text(&sheet);
    assert_kept_sheet_parts(&u, &sheet, &sx);
    assert_round_trip_kept_content(&wb, &out);
}

#[test]
fn native_slicer_and_timeline_relationships_report_known_omissions() {
    use gridcraft_core::{CellRef, Value};
    use gridcraft_model::{CompatibilityEffect, Editability};

    let workbook = format!(
        r#"<workbook xmlns="{NS_MAIN}" xmlns:r="{NS_R}"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets><extLst><ext uri="urn:example:extension"/></extLst></workbook>"#
    );
    let sheet = format!(r#"<worksheet xmlns="{NS_MAIN}"><sheetData><row r="1"><c r="A1"><v>42</v></c></row></sheetData></worksheet>"#);
    let workbook_rels = rels(&[
        ("rId1", "worksheet", "worksheets/sheet1.xml"),
        ("rId2", "http://schemas.microsoft.com/office/2007/relationships/slicerCache", "slicerCaches/slicerCache1.xml"),
        ("rId3", "http://schemas.microsoft.com/office/2011/relationships/timelineCache", "timelineCaches/timelineCache1.xml"),
    ]);
    let sheet_rels = rels(&[
        ("rId1", "http://schemas.microsoft.com/office/2007/relationships/slicer", "../slicers/slicer1.xml"),
        ("rId2", "http://schemas.microsoft.com/office/2011/relationships/timeline", "../timelines/timeline1.xml"),
        ("rId3", "https://example.org/relationships/slicer", "../lookalike.xml"),
        ("rId4", "http://schemas.microsoft.com/office/2011/relationships/timeline", "https://example.org/timeline.xml"),
        ("rId5", "http://schemas.microsoft.com/office/2007/relationships/slicer", "../slicers/missing.xml"),
    ]);
    let bytes = make_zip(&[
        ("[Content_Types].xml", super::CT.as_bytes()),
        ("_rels/.rels", super::ROOT_RELS.as_bytes()),
        ("xl/workbook.xml", workbook.as_bytes()),
        ("xl/_rels/workbook.xml.rels", workbook_rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
        ("xl/worksheets/_rels/sheet1.xml.rels", sheet_rels.as_bytes()),
        ("xl/slicers/slicer1.xml", br#"<slicers xmlns="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"/>"#),
        ("xl/slicerCaches/slicerCache1.xml", br#"<slicerCacheDefinition xmlns="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"/>"#),
        ("xl/timelines/timeline1.xml", br#"<timelines xmlns="http://schemas.microsoft.com/office/spreadsheetml/2010/11/main"/>"#),
        (
            "xl/timelineCaches/timelineCache1.xml",
            br#"<timelineCacheDefinition xmlns="http://schemas.microsoft.com/office/spreadsheetml/2010/11/main"/>"#,
        ),
        ("xl/lookalike.xml", br#"<slicers xmlns="urn:example:vendor"/>"#),
    ]);
    let (wb, report) = read_xlsx(&bytes).unwrap();
    let value = |wb: &gridcraft_model::Workbook| wb.sheet(0).unwrap().value(CellRef::new(0, 0));
    assert_eq!(value(&wb), Value::Number(42.0));
    let omitted = [
        ("native slicer cache", "xl/workbook.xml", "xl/slicerCaches/slicerCache1.xml"),
        ("native timeline cache", "xl/workbook.xml", "xl/timelineCaches/timelineCache1.xml"),
        ("native slicer", "xl/worksheets/sheet1.xml", "xl/slicers/slicer1.xml"),
        ("native timeline", "xl/worksheets/sheet1.xml", "xl/timelines/timeline1.xml"),
    ];
    for (feature, owner, target) in omitted {
        let location = format!("{owner}: {target}");
        let issue = report.issues.iter().find(|i| i.location.as_deref() == Some(location.as_str())).unwrap();
        assert_eq!(issue.code, "feature.omitted", "{issue:?}");
        assert_eq!(issue.feature, feature);
        assert_eq!(issue.effect, CompatibilityEffect::Omitted);
        assert_eq!(issue.editability, Editability::Unavailable);
        assert!(issue.description.contains(feature) && issue.description.contains("omitted on save"), "{issue:?}");
    }
    for (owner, target) in [
        ("xl/worksheets/sheet1.xml", "xl/lookalike.xml"),
        ("xl/worksheets/sheet1.xml", "https://example.org/timeline.xml"),
        ("xl/worksheets/sheet1.xml", "xl/slicers/missing.xml"),
        ("xl/workbook.xml", "urn:example:extension"),
    ] {
        let location = format!("{owner}: {target}");
        let issue = report.issues.iter().find(|i| i.location.as_deref() == Some(location.as_str())).unwrap();
        assert_eq!(issue.code, "package.unknown", "{issue:?}");
        assert_eq!(issue.effect, CompatibilityEffect::Unknown);
        assert_eq!(issue.editability, Editability::Unknown);
    }

    let saved = write_xlsx(&wb).unwrap();
    let package = Unpacked::new(&saved);
    package.assert_valid();
    for (_, _, target) in omitted {
        assert!(!package.parts.contains_key(target), "{target} unexpectedly survived save");
    }
    let (reopened, report) = read_xlsx(&saved).unwrap();
    assert_eq!(value(&reopened), Value::Number(42.0));
    assert!(!report.issues.iter().any(|i| i.effect == CompatibilityEffect::Omitted), "{:?}", report.issues);
}

fn assert_kept_workbook_parts(u: &Unpacked) {
    // Every kept part is in the output byte for byte.
    for d in [
        PRINTER,
        OLE,
        EMF,
        CUSTOM_XML.as_bytes(),
        CUSTOM_PROPS.as_bytes(),
        EXTERNAL_LINK.as_bytes(),
        CONNECTIONS.as_bytes(),
        QUERY_TABLE.as_bytes(),
        CTRL_PROP.as_bytes(),
    ] {
        assert_eq!(u.find(d).len(), 1, "kept part missing or duplicated: {}", String::from_utf8_lossy(d));
    }
    // Package root: custom properties.
    assert!(u.rels("").iter().any(|r| r.1.ends_with("/custom-properties") && u.parts.get(&r.2).is_some_and(|b| b == CUSTOM_PROPS.as_bytes())));
    // Workbook: external link in <externalReferences>, its path kept as an external target.
    let wbx = u.text("xl/workbook.xml");
    let link_id = attr_of(&wbx, "externalReference", "r:id");
    let (ty, link) = u.target("xl/workbook.xml", &link_id);
    assert!(ty.ends_with("/externalLink"));
    assert_eq!(u.parts.get(&link).unwrap(), EXTERNAL_LINK.as_bytes());
    let path = u.rels(&link);
    assert_eq!(path.len(), 1);
    assert_eq!((path[0].0.as_str(), path[0].2.as_str(), path[0].3), ("rId1", "file:///C:/never/opened/Book2.xlsx", true));
    assert!(wbx.find("</sheets>").unwrap() < wbx.find("<externalReferences").unwrap());
    assert!(wbx.find("<externalReferences").unwrap() < wbx.find("<definedNames").unwrap());
    // The custom XML item keeps its properties part.
    let item = u.find(CUSTOM_XML.as_bytes())[0].clone();
    assert!(u.rels(&item).iter().any(|r| r.1.ends_with("/customXmlProps")));
    // Formulas using the link are written as they were.
    let sheet = sheet_part(u, "Data");
    let sx = u.text(&sheet);
    assert!(sx.contains("<f>[1]Sheet1!$A$1</f>"), "{sx}");
    assert!(sx.contains("<f>SUM([1]Sheet1!A1:A3)*2</f>"), "{sx}");
}

fn assert_kept_sheet_parts(u: &Unpacked, sheet: &str, sx: &str) {
    // Printer settings (#324).
    let ps = attr_of(sx, "pageSetup", "r:id");
    let (ty, target) = u.target(sheet, &ps);
    assert!(ty.ends_with("/printerSettings"));
    assert_eq!(u.parts.get(&target).unwrap(), PRINTER);
    // The OLE object and its preview image, re-linked.
    let ole = attr_of(sx, "oleObject", "r:id");
    assert_eq!(u.parts.get(&u.target(sheet, &ole).1).unwrap(), OLE);
    let preview = attr_of(sx, "objectPr", "r:id");
    assert_eq!(u.parts.get(&u.target(sheet, &preview).1).unwrap(), EMF);
    // Namespace declarations travel with the fragment.
    assert_eq!(attr_of(sx, "mc:AlternateContent", "xmlns:mc"), "http://schemas.openxmlformats.org/markup-compatibility/2006");
    assert_eq!(attr_of(sx, "mc:AlternateContent", "xmlns:x14"), "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main");
    // The form control.
    let ctrl = attr_of(sx, "control", "r:id");
    assert_eq!(u.parts.get(&u.target(sheet, &ctrl).1).unwrap(), CTRL_PROP.as_bytes());
    // Schema order: pageSetup < legacyDrawing < oleObjects < controls.
    let pos = |t: &str| sx.find(t).unwrap_or_else(|| panic!("no {t}"));
    assert!(pos("<pageSetup") < pos("<legacyDrawing") && pos("<legacyDrawing") < pos("<oleObjects") && pos("<oleObjects") < pos("<controls"));
    // Query table: a sheet relationship only.
    assert!(u.rels(sheet).iter().any(|r| r.1.ends_with("/queryTable") && u.parts.get(&r.2).is_some_and(|b| b == QUERY_TABLE.as_bytes())));
    // One VML drawing: the kept control shapes plus GridCraft's note, in a fresh shape block.
    let vml_id = attr_of(sx, "legacyDrawing", "r:id");
    let vml_part = u.target(sheet, &vml_id).1;
    let vml = u.text(&vml_part);
    assert!(vml.contains("_x0000_s1025") && vml.contains("_x0000_s1026"));
    assert!(!vml.contains("old note") && !vml.contains("_x0000_s1027"), "the source's note shape is replaced");
    assert_eq!(vml.matches("ObjectType=\"Note\"").count(), 1);
    assert!(vml.contains("_x0000_s2049") && vml.contains("data=\"1,2\""), "{vml}");
    let img = u.rels(&vml_part);
    assert_eq!(img.len(), 1);
    assert_eq!(u.parts.get(&img[0].2).unwrap(), EMF);
}

fn assert_round_trip_kept_content(wb: &gridcraft_model::Workbook, out: &[u8]) {
    // Reading the output keeps the same content again.
    let (wb2, rep2) = read_xlsx(out).unwrap();
    assert!(!rep2.warnings.iter().any(|w| w.contains("dropped")), "{:?}", rep2.warnings);
    let a = wb.sheet(0).unwrap().preserved.as_deref().unwrap();
    let b = wb2.sheet(0).unwrap().preserved.as_deref().unwrap();
    let mut da: Vec<&[u8]> = a.parts.iter().map(|p| &*p.data).collect();
    let mut db: Vec<&[u8]> = b.parts.iter().map(|p| &*p.data).collect();
    da.sort();
    db.sort();
    assert_eq!(da, db);
    assert_eq!(a.elements.len(), b.elements.len());
    assert_eq!(wb.preserved.as_deref().unwrap().parts.len(), wb2.preserved.as_deref().unwrap().parts.len());
}

#[test]
fn edited_workbooks_keep_consistent_relationships() {
    let (mut wb, _) = read_xlsx(&rich_package()).unwrap();
    // Insert a sheet before, rename the sheet with controls, drop its note, edit a cell.
    wb.sheets.insert(0, Arc::new(Sheet::new("New")));
    let s = wb.sheet_mut(1).unwrap();
    s.name = "Renamed".into();
    s.comments_mut().clear();
    s.set_value(gridcraft_core::CellRef::new(5, 5), gridcraft_core::Value::Number(3.0));
    let u = Unpacked::new(&write_xlsx(&wb).unwrap());
    u.assert_valid();
    let sheet = sheet_part(&u, "Renamed");
    assert_eq!(sheet, "xl/worksheets/sheet2.xml");
    let sx = u.text(&sheet);
    assert_eq!(u.parts.get(&u.target(&sheet, &attr_of(&sx, "pageSetup", "r:id")).1).unwrap(), PRINTER);
    assert_eq!(u.parts.get(&u.target(&sheet, &attr_of(&sx, "control", "r:id")).1).unwrap(), CTRL_PROP.as_bytes());
    // Without notes the kept VML is written on its own, still without the old note.
    let vml = u.text(&u.target(&sheet, &attr_of(&sx, "legacyDrawing", "r:id")).1);
    assert!(vml.contains("_x0000_s1026") && !vml.contains("ObjectType=\"Note\""));
    assert!(!u.text("xl/worksheets/sheet1.xml").contains("pageSetup orientation=\"landscape\" r:id"));

    // Deleting the sheet drops its content; workbook-level content stays.
    wb.sheets.remove(1);
    wb.names.clear();
    let u = Unpacked::new(&write_xlsx(&wb).unwrap());
    u.assert_valid();
    for d in [PRINTER, OLE, EMF, QUERY_TABLE.as_bytes(), CTRL_PROP.as_bytes()] {
        assert!(u.find(d).is_empty());
    }
    for d in [CUSTOM_XML.as_bytes(), EXTERNAL_LINK.as_bytes(), CONNECTIONS.as_bytes(), CUSTOM_PROPS.as_bytes()] {
        assert_eq!(u.find(d).len(), 1);
    }
}

/// #324: an openpyxl-style workbook whose sheet carries printer settings.
#[test]
fn printer_settings_and_page_setup_id_survive_save() {
    let ct = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="bin" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.printerSettings"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/></Types>"#;
    let bin: Vec<u8> = (0..=255u8).cycle().take(1024).collect();
    let pkg = super::minimal(
        r#"<sheetData><row r="1"><c r="A1"><v>42.1</v></c></row></sheetData><pageMargins left="0.75" right="0.75" top="1" bottom="1" header="0.5" footer="0.5"/><pageSetup orientation="landscape" r:id="rIdPs1"/>"#,
        &[("xl/worksheets/_rels/sheet1.xml.rels", &rels(&[("rIdPs1", "printerSettings", "../printerSettings/printerSettings1.bin")]))],
        "",
        "",
    );
    // Swap in content types that know `.bin`, and add the binary part.
    let mut u = Unpacked::new(&pkg);
    u.parts.insert("[Content_Types].xml".into(), ct.as_bytes().to_vec());
    u.parts.insert("xl/printerSettings/printerSettings1.bin".into(), bin.clone());
    let list: Vec<(&str, &[u8])> = u.parts.iter().map(|(k, v)| (k.as_str(), v.as_slice())).collect();
    let (wb, rep) = read_xlsx(&make_zip(&list)).unwrap();
    assert!(rep.warnings.iter().all(|w| !w.contains("printer")), "{:?}", rep.warnings);
    let out = Unpacked::new(&write_xlsx(&wb).unwrap());
    out.assert_valid();
    let sx = out.text("xl/worksheets/sheet1.xml");
    assert!(sx.contains("orientation=\"landscape\""));
    let (ty, target) = out.target("xl/worksheets/sheet1.xml", &attr_of(&sx, "pageSetup", "r:id"));
    assert!(ty.ends_with("/printerSettings"));
    assert_eq!(target, "xl/printerSettings/printerSettings1.bin");
    assert_eq!(out.parts.get(&target).unwrap(), &bin);
}

#[test]
fn damaged_kept_content_is_dropped_without_failing_the_read() {
    let ct = CONTENT_TYPES.replace("</Types>", r#"<Override PartName="/customXml/item2.xml" ContentType="application/xml"/></Types>"#);
    let workbook = format!(
        r#"<?xml version="1.0"?><workbook xmlns="{NS_MAIN}" xmlns:r="{NS_R}"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets><externalReferences><externalReference r:id="rId3"/><externalReference r:id="rId99"/></externalReferences></workbook>"#
    );
    let sheet = format!(
        r#"<?xml version="1.0"?><worksheet xmlns="{NS_MAIN}" xmlns:r="{NS_R}"><sheetData/><pageSetup r:id="rId1"/><controls><control shapeId="1" r:id="rId404" name="Broken"/></controls></worksheet>"#
    );
    let wb_rels = rels(&[
        ("rId1", "worksheet", "worksheets/sheet1.xml"),
        ("rId3", "externalLink", "externalLinks/externalLink1.xml"),
        // A missing part.
        ("rId4", "customXml", "../customXml/missing.xml"),
        // A cycle: item1 → itemProps1 → item1.
        ("rId5", "customXml", "../customXml/item1.xml"),
        // Kept content that points into a part GridCraft rewrites.
        ("rId6", "customXml", "../customXml/item2.xml"),
        // Damaged rels behind a kept part.
        ("rId7", "connections", "connections.xml"),
    ]);
    let cyclic = rels(&[("rId1", "customXmlProps", "itemProps1.xml")]);
    let back = rels(&[("rId1", "customXml", "item1.xml")]);
    let into_sheet = rels(&[("rId1", "worksheet", "../xl/worksheets/sheet1.xml")]);
    let sheet_rels = rels(&[("rId1", "printerSettings", "../printerSettings/missing.bin")]);
    let bytes = make_zip(&[
        ("[Content_Types].xml", ct.as_bytes()),
        ("_rels/.rels", super::ROOT_RELS.as_bytes()),
        ("xl/workbook.xml", workbook.as_bytes()),
        ("xl/_rels/workbook.xml.rels", wb_rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
        ("xl/worksheets/_rels/sheet1.xml.rels", sheet_rels.as_bytes()),
        ("xl/externalLinks/externalLink1.xml", EXTERNAL_LINK.as_bytes()),
        ("customXml/item1.xml", CUSTOM_XML.as_bytes()),
        ("customXml/_rels/item1.xml.rels", cyclic.as_bytes()),
        ("customXml/itemProps1.xml", b"<p/>"),
        ("customXml/_rels/itemProps1.xml.rels", back.as_bytes()),
        ("customXml/item2.xml", b"<q/>"),
        ("customXml/_rels/item2.xml.rels", into_sheet.as_bytes()),
        ("xl/connections.xml", CONNECTIONS.as_bytes()),
        ("xl/_rels/connections.xml.rels", b"<Relationships"),
    ]);
    let (wb, rep) = read_xlsx(&bytes).unwrap();
    let w = rep.warnings.join("\n");
    assert!(w.contains("customXml/missing.xml is missing"), "{w}");
    assert!(w.contains("refers to worksheet"), "{w}");
    assert!(w.contains("links to external workbooks were dropped"), "a link without a part drops all links: {w}");
    assert!(w.contains("printer settings were dropped"), "{w}");
    assert!(w.contains("dropped: the sheet has no VML drawing"), "{w}");
    let p = wb.preserved.as_deref().unwrap();
    // Only the cyclic custom XML pair is consistent enough to keep.
    let mut names: Vec<&str> = p.parts.iter().map(|q| q.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["customXml/item1.xml", "customXml/itemProps1.xml"]);
    assert!(p.elements.is_empty());
    assert!(wb.sheet(0).unwrap().preserved.is_none());
    let u = Unpacked::new(&write_xlsx(&wb).unwrap());
    u.assert_valid();
    assert!(!u.text("xl/workbook.xml").contains("externalReferences"));
}

#[test]
fn oversized_kept_parts_are_dropped() {
    let big = vec![0u8; crate::passthrough::MAX_KEPT_PART as usize + 1];
    let ct = CONTENT_TYPES;
    let wb_rels = rels(&[("rId1", "worksheet", "worksheets/sheet1.xml"), ("rId2", "customXml", "../customXml/item1.xml")]);
    let workbook = format!(
        r#"<?xml version="1.0"?><workbook xmlns="{NS_MAIN}" xmlns:r="{NS_R}"><sheets><sheet name="A" sheetId="1" r:id="rId1"/></sheets></workbook>"#
    );
    let sheet = format!(r#"<?xml version="1.0"?><worksheet xmlns="{NS_MAIN}"><sheetData/></worksheet>"#);
    let bytes = make_zip(&[
        ("[Content_Types].xml", ct.as_bytes()),
        ("_rels/.rels", super::ROOT_RELS.as_bytes()),
        ("xl/workbook.xml", workbook.as_bytes()),
        ("xl/_rels/workbook.xml.rels", wb_rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
        ("customXml/item1.xml", &big),
    ]);
    drop(big);
    let (wb, rep) = read_xlsx(&bytes).unwrap();
    assert!(wb.preserved.is_none());
    assert!(rep.warnings.iter().any(|w| w.contains("customXml") && w.contains("dropped")), "{:?}", rep.warnings);
}

#[test]
fn kept_content_types_are_escaped_on_save() {
    // A content type holding `"` and `<` (legal once unescaped by the XML reader) must not break
    // out of the attribute in the saved [Content_Types].xml.
    let hostile = r#"application/x-test&quot; PartName=&quot;/x&quot;/&gt;&lt;Default Extension=&quot;evil"#;
    let ct = CONTENT_TYPES.replace("application/vnd.openxmlformats-officedocument.customXmlProperties+xml", hostile);
    let (wb, rep) = read_xlsx(&rich_package_with(&ct, false)).unwrap();
    assert!(wb.preserved.is_some(), "{:?}", rep.warnings);
    let u = Unpacked::new(&write_xlsx(&wb).unwrap());
    let types = xml::parse(u.text("[Content_Types].xml").as_bytes()).unwrap();
    assert!(!types.kids("Default").any(|d| d.attr("Extension") == Some("evil")));
    let item = types.kids("Override").find(|o| o.attr("PartName") == Some("/customXml/itemProps1.xml")).unwrap();
    assert_eq!(item.attr("ContentType"), Some(r#"application/x-test" PartName="/x"/><Default Extension="evil"#));
    // And the saved file reads back.
    read_xlsx(&write_xlsx(&wb).unwrap()).unwrap();
}

#[test]
fn kept_vml_drawings_are_charged_to_the_kept_total() {
    // How much one sheet's kept content costs, with the default allowance.
    let one = {
        let (wb, rep) = read_xlsx(&rich_package()).unwrap();
        assert!(!rep.warnings.iter().any(|w| w.contains("dropped")), "{:?}", rep.warnings);
        wb.preserved.as_ref().map_or(0, |p| p.byte_len()) + wb.sheets.iter().filter_map(|s| s.preserved.as_ref()).map(|p| p.byte_len()).sum::<usize>()
    };
    // Two sheets share one VML drawing: each would keep its own copy. With room for one sheet's
    // content only, the second sheet's VML (and the controls drawn from it) is dropped.
    crate::passthrough::KEPT_TOTAL_FOR_TEST.with(|c| c.set(one + VML.len() / 2));
    let read = read_xlsx(&rich_package_with(CONTENT_TYPES, true));
    crate::passthrough::KEPT_TOTAL_FOR_TEST.with(|c| c.set(crate::passthrough::MAX_KEPT_TOTAL));
    let (wb, rep) = read.unwrap();
    let kept = wb.preserved.as_ref().map_or(0, |p| p.byte_len())
        + wb.sheets.iter().filter_map(|s| s.preserved.as_ref()).map(|p| p.byte_len()).sum::<usize>();
    assert!(kept <= one + VML.len() / 2, "kept {kept} bytes");
    assert!(wb.sheet(0).unwrap().preserved.as_ref().is_some_and(|p| p.vml.is_some()));
    assert!(wb.sheet(1).unwrap().preserved.as_ref().is_none_or(|p| p.vml.is_none()));
    assert!(rep.warnings.iter().any(|w| w.contains("\"Other\"") && w.contains("too large")), "{:?}", rep.warnings);
    // Both sheets keep their drawings when there is room.
    let (wb, _) = read_xlsx(&rich_package_with(CONTENT_TYPES, true)).unwrap();
    assert!(wb.sheets.iter().all(|s| s.preserved.as_ref().is_some_and(|p| p.vml.is_some())));
}

const XML_MAPS: &str = r#"<?xml version="1.0"?><MapInfo xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" SelectionNamespaces=""><Schema ID="Schema1"><xsd:schema xmlns:xsd="http://www.w3.org/2001/XMLSchema"><xsd:element name="DataList"><xsd:complexType><xsd:sequence><xsd:element name="ItemList"><xsd:complexType><xsd:sequence><xsd:element name="TableID" type="xsd:integer"/><xsd:element name="Item" maxOccurs="unbounded"><xsd:complexType><xsd:sequence><xsd:element name="No" type="xsd:string"/><xsd:element name="Blocked" type="xsd:boolean"/></xsd:sequence></xsd:complexType></xsd:element></xsd:sequence></xsd:complexType></xsd:element></xsd:sequence></xsd:complexType></xsd:element></xsd:schema></Schema><Map ID="1" Name="DataList_Map" RootElement="DataList" SchemaID="Schema1" ShowImportExportValidationErrors="0" AutoFit="1" Append="0" PreserveSortAFLayout="1" PreserveFormat="1"><DataBinding FileBinding="1" ConnectionID="1" DataBindingLoadMode="1"/></Map></MapInfo>"#;
const SINGLE_CELLS: &str = r#"<?xml version="1.0"?><singleXmlCells xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><singleXmlCell id="3" r="C1" connectionId="1"><xmlCellPr id="1" uniqueName="TableID"><xmlPr mapId="1" xpath="/DataList/ItemList/TableID" xmlDataType="integer"/></xmlCellPr></singleXmlCell></singleXmlCells>"#;

/// Isolate the single-cell part's schema handler from the accompanying opaque workbook graph.
/// A real package's unknown map/connection graph must still veto structural changes (below).
fn single_cell_schema_fixture() -> gridcraft_model::Workbook {
    use gridcraft_model::{Preserved, PreservedPart, PreservedRel, Workbook};
    let mut wb = Workbook::new();
    wb.sheet_mut(0).unwrap().preserved = Some(Arc::new(Preserved {
        rels: vec![PreservedRel {
            id: "mapped".into(),
            rel_type: format!("{NS_R}/tableSingleCells"),
            target: "xl/tables/cells.xml".into(),
            external: false,
        }],
        parts: vec![PreservedPart {
            name: "xl/tables/cells.xml".into(),
            content_type: "application/vnd.openxmlformats-officedocument.spreadsheetml.tableSingleCells+xml".into(),
            data: Arc::from(SINGLE_CELLS.as_bytes()),
            rels: vec![],
        }],
        ..Default::default()
    }));
    wb
}

#[test]
fn single_cell_mapping_adjusts_only_its_cell_attribute() {
    use gridcraft_formula::adjust::{Axis, Edit};
    let before = single_cell_schema_fixture();
    let after = crate::prepare_preserved_edit(&before, &before, &[("Sheet1".into(), Edit::Insert { axis: Axis::Rows, at: 0, count: 2 })], &[])
        .expect("same-owner mapping can adjust")
        .expect("mapped reference changes");
    let p = after.sheet(0).unwrap().preserved.as_ref().unwrap();
    assert_eq!(p.parts[0].data.as_ref(), SINGLE_CELLS.replace("r=\"C1\"", "r=\"C3\"").as_bytes());
    assert_eq!(before.sheet(0).unwrap().preserved.as_ref().unwrap().parts[0].data.as_ref(), SINGLE_CELLS.as_bytes());
}

#[test]
fn single_cell_mapping_refuses_deleted_or_cross_sheet_target() {
    use gridcraft_formula::adjust::{Axis, Edit};
    let mut before = single_cell_schema_fixture();
    before.sheets.push(Arc::new(Sheet::new("Other")));
    for edit in [
        Edit::Delete { axis: Axis::Rows, at: 0, count: 1 },
        Edit::MoveToSheet { from: gridcraft_core::RangeRef::parse("C1").unwrap(), to_sheet: "Other".into(), to_row: 2, to_col: 2 },
    ] {
        assert!(crate::prepare_preserved_edit(&before, &before, &[("Sheet1".into(), edit)], &[]).is_err());
    }
}

#[test]
fn preserved_reference_relationship_identity_is_bounded() {
    use gridcraft_formula::adjust::{Axis, Edit};
    let mut before = single_cell_schema_fixture();
    Arc::make_mut(before.sheet_mut(0).unwrap().preserved.as_mut().unwrap()).rels[0].id = "x".repeat(1025);
    assert!(crate::prepare_preserved_edit(&before, &before, &[("Sheet1".into(), Edit::Insert { axis: Axis::Rows, at: 0, count: 1 })], &[]).is_err());
}

#[test]
fn unknown_workbook_map_graph_still_refuses_structural_edit() {
    use gridcraft_formula::adjust::{Axis, Edit};
    let (before, _) = read_xlsx(&xml_map_package()).unwrap();
    assert!(crate::prepare_preserved_edit(&before, &before, &[("Item".into(), Edit::Insert { axis: Axis::Rows, at: 0, count: 1 })], &[]).is_err());
}
const XML_CONNECTION: &str = r#"<?xml version="1.0"?><connections xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><connection id="1" name="DataList" type="4" refreshedVersion="0" background="1"><webPr xml="1" sourceData="1" url="C:\temp\datalist.xml" htmlTables="1"/></connection></connections>"#;

/// A workbook bound to an XML map, shaped like a Business Central configuration package: the map,
/// an XML table whose columns carry XPaths, a single mapped cell and the XML connection.
fn xml_map_package() -> Vec<u8> {
    let types = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/xmlMaps.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.xmlMaps+xml"/><Override PartName="/xl/connections.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.connections+xml"/><Override PartName="/xl/tables/table1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/><Override PartName="/xl/tables/tableSingleCells1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.tableSingleCells+xml"/></Types>"#;
    let workbook = format!(
        r#"<?xml version="1.0"?><workbook xmlns="{NS_MAIN}" xmlns:r="{NS_R}"><sheets><sheet name="Item" sheetId="1" r:id="rId1"/></sheets></workbook>"#
    );
    let sheet = format!(
        r#"<?xml version="1.0"?><worksheet xmlns="{NS_MAIN}" xmlns:r="{NS_R}"><sheetData><row r="1"><c r="C1"><v>27</v></c></row><row r="3"><c r="A3" t="inlineStr"><is><t>No.</t></is></c><c r="B3" t="inlineStr"><is><t>Blocked</t></is></c></row><row r="4"><c r="A4" t="inlineStr"><is><t>1000</t></is></c><c r="B4" t="b"><v>0</v></c></row></sheetData><tableParts count="1"><tablePart r:id="rId2"/></tableParts></worksheet>"#
    );
    let table = format!(
        r#"<?xml version="1.0"?><table xmlns="{NS_MAIN}" id="1" name="Table1" displayName="Table1" ref="A3:B4" tableType="xml" connectionId="1" totalsRowShown="0"><autoFilter ref="A3:B4"/><tableColumns count="2"><tableColumn id="1" uniqueName="No" name="No."><xmlColumnPr mapId="1" xpath="/DataList/ItemList/Item/No" xmlDataType="string"/></tableColumn><tableColumn id="2" uniqueName="Blocked" name="Blocked"><xmlColumnPr mapId="1" xpath="/DataList/ItemList/Item/Blocked" xmlDataType="boolean"/></tableColumn></tableColumns><tableStyleInfo name="TableStyleMedium2" showRowStripes="1"/></table>"#
    );
    let root_rels = rels(&[("rId1", "officeDocument", "xl/workbook.xml")]);
    let wb_rels =
        rels(&[("rId1", "worksheet", "worksheets/sheet1.xml"), ("rId2", "xmlMaps", "xmlMaps.xml"), ("rId3", "connections", "connections.xml")]);
    let sheet_rels = rels(&[("rId1", "tableSingleCells", "../tables/tableSingleCells1.xml"), ("rId2", "table", "../tables/table1.xml")]);
    make_zip(&[
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", root_rels.as_bytes()),
        ("xl/workbook.xml", workbook.as_bytes()),
        ("xl/_rels/workbook.xml.rels", wb_rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
        ("xl/worksheets/_rels/sheet1.xml.rels", sheet_rels.as_bytes()),
        ("xl/tables/table1.xml", table.as_bytes()),
        ("xl/tables/tableSingleCells1.xml", SINGLE_CELLS.as_bytes()),
        ("xl/connections.xml", XML_CONNECTION.as_bytes()),
        ("xl/xmlMaps.xml", XML_MAPS.as_bytes()),
    ])
}

#[test]
fn xml_maps_and_their_bindings_survive_a_round_trip() {
    let (wb, rep) = read_xlsx(&xml_map_package()).unwrap();
    assert!(!rep.warnings.iter().any(|w| w.contains("dropped")), "{:?}", rep.warnings);
    let out = write_xlsx(&wb).unwrap();
    let u = Unpacked::new(&out);
    u.assert_valid();
    // The map keeps its name (its ids are what the bindings point at) and its relationship.
    assert_eq!(u.parts.get("xl/xmlMaps.xml").map(|b| String::from_utf8_lossy(b)).as_deref(), Some(XML_MAPS));
    assert!(u.rels("xl/workbook.xml").iter().any(|r| r.1.ends_with("/xmlMaps") && r.2 == "xl/xmlMaps.xml"));
    let types = u.text("[Content_Types].xml");
    assert!(
        types.contains(r#"PartName="/xl/xmlMaps.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.xmlMaps+xml""#),
        "{types}"
    );
    // The single mapped cell hangs off the sheet.
    let sheet = sheet_part(&u, "Item");
    assert!(u.rels(&sheet).iter().any(|r| r.1.ends_with("/tableSingleCells") && u.parts.get(&r.2).is_some_and(|b| b == SINGLE_CELLS.as_bytes())));
    // The table stays an XML table, its columns keep their XPaths.
    let (_, table) = u.rels(&sheet).into_iter().map(|r| (r.1, r.2)).find(|(ty, _)| ty.ends_with("/table")).unwrap();
    let t = xml::parse(u.text(&table).as_bytes()).unwrap();
    assert_eq!((t.attr("tableType"), t.attr("connectionId")), (Some("xml"), Some("1")));
    let cols: Vec<_> = t.child("tableColumns").unwrap().kids("tableColumn").collect();
    let mapped: Vec<_> = cols
        .iter()
        .map(|c| {
            let x = c.child("xmlColumnPr").unwrap();
            (c.attr("uniqueName"), x.attr("mapId"), x.attr("xpath"), x.attr("xmlDataType"))
        })
        .collect();
    assert_eq!(
        mapped,
        [
            (Some("No"), Some("1"), Some("/DataList/ItemList/Item/No"), Some("string")),
            (Some("Blocked"), Some("1"), Some("/DataList/ItemList/Item/Blocked"), Some("boolean")),
        ]
    );
}

/// Where a link with `target`, in a workbook read from `from`, points once saved into `to`.
#[cfg(unix)]
fn rebased(target: &str, from: &str, to: &str) -> String {
    let link = |rel_type: &str| gridcraft_model::PreservedRel {
        id: "rId1".into(),
        rel_type: format!("{NS_R}/{rel_type}"),
        target: target.into(),
        external: true,
    };
    let part = gridcraft_model::PreservedPart {
        name: "xl/externalLinks/externalLink1.xml".into(),
        content_type: String::new(),
        data: Arc::from(&b""[..]),
        rels: vec![link("externalLinkPath"), link("hyperlink")],
    };
    let mut wb = gridcraft_model::Workbook::new();
    wb.preserved = Some(Arc::new(gridcraft_model::Preserved { parts: vec![part], ..Default::default() }));
    wb.source_dir = Some(from.into());
    let Some(out) = crate::rebase_links(&wb, std::path::Path::new(to)) else { return target.into() };
    let rels = &out.preserved.as_ref().unwrap().parts[0].rels;
    assert_eq!(rels[1].target, target, "only paths of linked workbooks change");
    rels[0].target.clone()
}

#[test]
#[cfg(unix)]
fn saving_into_another_folder_rebases_relative_links_to_workbooks() {
    assert_eq!(rebased("Source.xlsx", "/links", "/links/move2"), "../Source.xlsx");
    assert_eq!(rebased("Source.xlsx", "/links", "/other/deeper"), "../../links/Source.xlsx");
    assert_eq!(rebased("../Source.xlsx", "/links/move2", "/links"), "Source.xlsx");
    assert_eq!(rebased("sub\\Source.xlsx", "/links", "/links/sub"), "Source.xlsx");
    assert_eq!(rebased("./sub/../Source.xlsx", "/links", "/links/sub/"), "../Source.xlsx");
    // Folder names from the file system are written as a URI path; the target's own spelling stays.
    assert_eq!(rebased("My%20Book.xlsx", "/links/q2 data", "/links"), "q2%20data/My%20Book.xlsx");
    assert_eq!(rebased("Résumé.xlsx", "/a/Été", "/a"), "Été/Résumé.xlsx");
    // The same folder, absolute paths, URLs and paths above the root are kept as they are.
    assert_eq!(rebased("Source.xlsx", "/links/", "/links/x/.."), "Source.xlsx");
    for target in ["/links/Source.xlsx", "file:///C:/links/Source.xlsx", "C:\\links\\Source.xlsx", "https://example.com/Source.xlsx", "../../x.xlsx"]
    {
        assert_eq!(rebased(target, "/links", "/links/move2"), target);
    }
    // Without the folder it was read from, nothing to rebase.
    let (mut wb, _) = read_xlsx(&rich_package()).unwrap();
    assert!(crate::rebase_links(&wb, std::path::Path::new("/elsewhere")).is_none());
    wb.source_dir = Some("relative/dir".into());
    assert!(crate::rebase_links(&wb, std::path::Path::new("/elsewhere")).is_none());
}

// ------------------------------------------------------------------ chart sheets (#727)

const NS_C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const CT_PREFIX: &str = "application/vnd.openxmlformats-officedocument";

fn chart_xml(formula: &str) -> String {
    format!(
        r#"<?xml version="1.0"?><c:chartSpace xmlns:c="{NS_C}"><c:chart><c:plotArea><c:barChart><c:ser><c:val><c:numRef><c:f>{formula}</c:f></c:numRef></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#
    )
}

fn chartsheet_xml(note: &str) -> String {
    format!(
        r#"<?xml version="1.0"?><chartsheet xmlns="{NS_MAIN}" xmlns:r="{NS_R}"><sheetViews><sheetView workbookViewId="0"/></sheetViews><pageSetup r:id="rId2"/><drawing r:id="rId1"/><!-- {note} --></chartsheet>"#
    )
}

fn drawing_xml(note: &str) -> String {
    format!(
        r#"<?xml version="1.0"?><xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing"><!-- {note} --></xdr:wsDr>"#
    )
}

fn worksheet_xml(value: &str) -> String {
    format!(
        r#"<?xml version="1.0"?><worksheet xmlns="{NS_MAIN}"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>{value}</t></is></c></row></sheetData></worksheet>"#
    )
}

/// Sheets in file order: Data, Chart1 (a chart sheet on Data), More, Spare, Last (a hidden chart
/// sheet on More, last in the list). "More" has a print area; the active tab is "More".
fn chartsheet_package() -> Vec<u8> {
    let workbook = format!(
        r#"<?xml version="1.0"?><workbook xmlns="{NS_MAIN}" xmlns:r="{NS_R}"><bookViews><workbookView activeTab="2"/></bookViews><sheets><sheet name="Data" sheetId="1" r:id="rId1"/><sheet name="Chart1" sheetId="2" r:id="rId2"/><sheet name="More" sheetId="3" r:id="rId3"/><sheet name="Spare" sheetId="4" r:id="rId4"/><sheet name="Last" sheetId="9" state="hidden" r:id="rId5"/></sheets><definedNames><definedName name="_xlnm.Print_Area" localSheetId="2">More!$A$1:$B$2</definedName></definedNames></workbook>"#
    );
    let content_types = format!(
        r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="bin" ContentType="{CT_PREFIX}.spreadsheetml.printerSettings"/><Override PartName="/xl/workbook.xml" ContentType="{CT_PREFIX}.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="{CT_PREFIX}.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet2.xml" ContentType="{CT_PREFIX}.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet3.xml" ContentType="{CT_PREFIX}.spreadsheetml.worksheet+xml"/><Override PartName="/xl/chartsheets/sheet1.xml" ContentType="{CT_PREFIX}.spreadsheetml.chartsheet+xml"/><Override PartName="/xl/chartsheets/sheet2.xml" ContentType="{CT_PREFIX}.spreadsheetml.chartsheet+xml"/><Override PartName="/xl/drawings/drawing1.xml" ContentType="{CT_PREFIX}.drawing+xml"/><Override PartName="/xl/drawings/drawing2.xml" ContentType="{CT_PREFIX}.drawing+xml"/><Override PartName="/xl/charts/chart1.xml" ContentType="{CT_PREFIX}.drawingml.chart+xml"/><Override PartName="/xl/charts/chart2.xml" ContentType="{CT_PREFIX}.drawingml.chart+xml"/></Types>"#
    );
    let (cs1, cs2) = (chartsheet_xml("first"), chartsheet_xml("last"));
    let (d1, d2) = (drawing_xml("first"), drawing_xml("last"));
    let (c1, c2) = (chart_xml("Data!$A$1:$A$3"), chart_xml("'More'!$A$1"));
    let (s1, s2, s3) = (worksheet_xml("data"), worksheet_xml("more"), worksheet_xml("spare"));
    make_zip(&[
        ("[Content_Types].xml", content_types.as_bytes()),
        ("_rels/.rels", rels(&[("rId1", "officeDocument", "xl/workbook.xml")]).as_bytes()),
        ("xl/workbook.xml", workbook.as_bytes()),
        (
            "xl/_rels/workbook.xml.rels",
            rels(&[
                ("rId1", "worksheet", "worksheets/sheet1.xml"),
                ("rId2", "chartsheet", "chartsheets/sheet1.xml"),
                ("rId3", "worksheet", "worksheets/sheet2.xml"),
                ("rId4", "worksheet", "worksheets/sheet3.xml"),
                ("rId5", "chartsheet", "chartsheets/sheet2.xml"),
            ])
            .as_bytes(),
        ),
        ("xl/worksheets/sheet1.xml", s1.as_bytes()),
        ("xl/worksheets/sheet2.xml", s2.as_bytes()),
        ("xl/worksheets/sheet3.xml", s3.as_bytes()),
        ("xl/chartsheets/sheet1.xml", cs1.as_bytes()),
        ("xl/chartsheets/sheet2.xml", cs2.as_bytes()),
        (
            "xl/chartsheets/_rels/sheet1.xml.rels",
            rels(&[("rId1", "drawing", "../drawings/drawing1.xml"), ("rId2", "printerSettings", "../printerSettings/printerSettings1.bin")])
                .as_bytes(),
        ),
        (
            "xl/chartsheets/_rels/sheet2.xml.rels",
            rels(&[("rId1", "drawing", "../drawings/drawing2.xml"), ("rId2", "printerSettings", "../printerSettings/printerSettings1.bin")])
                .as_bytes(),
        ),
        ("xl/printerSettings/printerSettings1.bin", PRINTER),
        ("xl/drawings/drawing1.xml", d1.as_bytes()),
        ("xl/drawings/drawing2.xml", d2.as_bytes()),
        ("xl/drawings/_rels/drawing1.xml.rels", rels(&[("rId1", "chart", "../charts/chart1.xml")]).as_bytes()),
        ("xl/drawings/_rels/drawing2.xml.rels", rels(&[("rId1", "chart", "../charts/chart2.xml")]).as_bytes()),
        ("xl/charts/chart1.xml", c1.as_bytes()),
        ("xl/charts/chart2.xml", c2.as_bytes()),
    ])
}

/// The sheets of a written workbook: (name, state) in file order.
fn written_sheets(u: &Unpacked) -> Vec<(String, String)> {
    let wb = xml::parse(u.parts.get("xl/workbook.xml").unwrap()).unwrap();
    wb.child("sheets").unwrap().kids("sheet").map(|s| (s.attr("name").unwrap().to_string(), s.attr("state").unwrap_or("").to_string())).collect()
}

/// The part a written chart sheet is in, found through the workbook relationship; its drawing and
/// that drawing's chart, followed through relationships.
fn chart_sheet_chain(u: &Unpacked, name: &str) -> (String, String, String) {
    let wb = xml::parse(u.parts.get("xl/workbook.xml").unwrap()).unwrap();
    let id = wb.child("sheets").unwrap().kids("sheet").find(|s| s.attr("name") == Some(name)).unwrap().attr("id").unwrap().to_string();
    let (ty, part) = u.target("xl/workbook.xml", &id);
    assert!(ty.ends_with("/chartsheet"), "{ty}");
    let drawing = u.rels(&part).into_iter().find(|r| r.1.ends_with("/drawing")).unwrap().2;
    let chart = u.rels(&drawing).into_iter().find(|r| r.1.ends_with("/chart")).unwrap().2;
    (part, drawing, chart)
}

#[test]
fn chart_sheets_survive_a_round_trip_in_place() {
    let (wb, rep) = read_xlsx(&chartsheet_package()).unwrap();
    assert!(!rep.warnings.iter().any(|w| w.contains("skipped") || w.contains("dropped")), "{:?}", rep.warnings);
    let names: Vec<&str> = wb.sheets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Data", "More", "Spare"], "chart sheets are not worksheets");

    let out = write_xlsx(&wb).unwrap();
    let u = Unpacked::new(&out);
    u.assert_valid();
    // Same order, same visibility, the way Excel shows the tabs.
    assert_eq!(
        written_sheets(&u),
        [("Data", ""), ("Chart1", ""), ("More", ""), ("Spare", ""), ("Last", "hidden")].map(|(a, b)| (a.to_string(), b.to_string())),
    );
    // Sheet ids are unique.
    let wbx = u.text("xl/workbook.xml");
    let mut ids: Vec<String> = wbx.match_indices("<sheet ").map(|(i, _)| attr_of(&wbx[i..], "sheet", "sheetId")).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 5, "{wbx}");
    // Positions in the file, not in the model, for the active tab and the print area.
    assert_eq!(attr_of(&wbx, "workbookView", "activeTab"), "2", "More is the third tab");
    assert_eq!(attr_of(&wbx, "definedName", "localSheetId"), "2", "{wbx}");

    // Every part of both chart sheets is in the output, byte for byte, behind the same chain.
    for (name, cs, drawing, chart) in [("Chart1", "first", "first", "Data!$A$1:$A$3"), ("Last", "last", "last", "'More'!$A$1")] {
        let (cs_part, d_part, c_part) = chart_sheet_chain(&u, name);
        assert_eq!(u.text(&cs_part), chartsheet_xml(cs), "{name}");
        assert_eq!(u.text(&d_part), drawing_xml(drawing), "{name}");
        assert_eq!(u.text(&c_part), chart_xml(chart), "{name}");
    }
    // The first chart sheet's printer settings follow its `<pageSetup r:id>`.
    let (cs_part, _, _) = chart_sheet_chain(&u, "Chart1");
    let ps = u.target(&cs_part, &attr_of(&u.text(&cs_part), "pageSetup", "r:id"));
    assert!(ps.0.ends_with("/printerSettings"));
    assert_eq!(u.parts.get(&ps.1).unwrap(), PRINTER);

    // Reading the output again finds the same two chart sheets, and a second save is stable.
    let (wb2, rep2) = read_xlsx(&out).unwrap();
    assert!(!rep2.warnings.iter().any(|w| w.contains("skipped") || w.contains("dropped")), "{:?}", rep2.warnings);
    let again = Unpacked::new(&write_xlsx(&wb2).unwrap());
    again.assert_valid();
    assert_eq!(written_sheets(&again), written_sheets(&u));
}

#[test]
fn a_chart_sheet_whose_data_sheet_was_renamed_is_dropped_with_its_parts() {
    let (mut wb, _) = read_xlsx(&chartsheet_package()).unwrap();
    wb.sheet_mut(0).unwrap().name = "Renamed".into();
    let u = Unpacked::new(&write_xlsx(&wb).unwrap());
    u.assert_valid();
    // "Chart1" plots Data, which is gone under that name; "Last" plots More, which is not.
    assert_eq!(written_sheets(&u).iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["Renamed", "More", "Spare", "Last"]);
    assert!(u.find(chart_xml("Data!$A$1:$A$3").as_bytes()).is_empty());
    assert!(u.find(drawing_xml("first").as_bytes()).is_empty());
    // Last shares the repaired fixture's printer dependency with the dropped Chart1.
    assert_eq!(u.find(PRINTER).len(), 1);
    let (last, _, _) = chart_sheet_chain(&u, "Last");
    let setup_id = attr_of(&u.text(&last), "pageSetup", "r:id");
    let (kind, printer) = u.target(&last, &setup_id);
    assert!(kind.ends_with("/printerSettings"));
    assert_eq!(u.parts[&printer], PRINTER);
    chart_sheet_chain(&u, "Last");
    let wbx = u.text("xl/workbook.xml");
    assert_eq!(attr_of(&wbx, "workbookView", "activeTab"), "1");
    assert_eq!(attr_of(&wbx, "definedName", "localSheetId"), "1");
}

#[test]
fn a_chart_sheet_whose_name_a_worksheet_took_is_dropped() {
    let (mut wb, _) = read_xlsx(&chartsheet_package()).unwrap();
    wb.sheet_mut(2).unwrap().name = "LAST".into();
    let u = Unpacked::new(&write_xlsx(&wb).unwrap());
    u.assert_valid();
    assert_eq!(written_sheets(&u).iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["Data", "Chart1", "More", "LAST"]);
    assert!(u.find(chart_xml("'More'!$A$1").as_bytes()).is_empty());
    chart_sheet_chain(&u, "Chart1");
}

#[test]
fn formulas_name_the_sheets_a_chart_plots() {
    use crate::passthrough::sheet_names_in;
    assert_eq!(sheet_names_in("Data!$A$1:$A$3"), ["Data"]);
    assert_eq!(sheet_names_in("'My ''Sheet'!$A$1,Data!$B$1"), ["My 'Sheet", "Data"]);
    assert_eq!(sheet_names_in("SUM(Data!A1:A3,'Other sheet'!B2)"), ["Data", "Other sheet"]);
    assert!(sheet_names_in("[1]Ext!A1").is_empty() && sheet_names_in("[1]Ext!A1:A3").is_empty());
    assert!(sheet_names_in("Name").is_empty() && sheet_names_in("").is_empty());
}

#[test]
fn file_version_and_calc_features_survive_a_save() {
    // Excel trusts the saved cell values only when workbook.xml carries both <fileVersion> and the
    // calcFeatures extension; without them it recalculates everything on open (#799).
    let calc_features = r#"<ext uri="{B58B0392-4F1F-4190-BB64-5DF3571DCE5F}" xmlns:xcalcf="http://schemas.microsoft.com/office/spreadsheetml/2018/calcfeatures"><xcalcf:calcFeatures><xcalcf:feature name="microsoft.com:RD"/><xcalcf:feature name="microsoft.com:Single"/><xcalcf:feature name="microsoft.com:FV"/><xcalcf:feature name="microsoft.com:CNMTM"/><xcalcf:feature name="microsoft.com:LET_WF"/><xcalcf:feature name="microsoft.com:ARRAYTEXT_WF"/></xcalcf:calcFeatures></ext>"#;
    let workbook = format!(
        r#"<?xml version="1.0"?><workbook xmlns="{NS_MAIN}" xmlns:r="{NS_R}"><fileVersion appName="xl" lastEdited="7" lowestEdited="7" rupBuild="11008"/><workbookPr/><sheets><sheet name="A" sheetId="1" r:id="rId1"/></sheets><calcPr calcId="191029"/><extLst><ext uri="{{140A7094-0E35-4892-8432-C4D2E57EDEB5}}" xmlns:x15="http://schemas.microsoft.com/office/spreadsheetml/2010/11/main"><x15:workbookPr chartTrackingRefBase="1"/></ext>{calc_features}</extLst></workbook>"#
    );
    let sheet = format!(
        r#"<?xml version="1.0"?><worksheet xmlns="{NS_MAIN}"><sheetData><row r="1"><c r="A1"><v>1</v></c><c r="B1"><f>A1*2</f><v>2</v></c></row></sheetData></worksheet>"#
    );
    let bytes = make_zip(&[
        ("[Content_Types].xml", super::CT.as_bytes()),
        ("_rels/.rels", super::ROOT_RELS.as_bytes()),
        ("xl/workbook.xml", workbook.as_bytes()),
        ("xl/_rels/workbook.xml.rels", rels(&[("rId1", "worksheet", "worksheets/sheet1.xml")]).as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
    ]);
    let (wb, _) = read_xlsx(&bytes).unwrap();
    // Twice: what was kept on the first save is read back and kept again.
    let (again, _) = read_xlsx(&write_xlsx(&wb).unwrap()).unwrap();
    for saved in [write_xlsx(&wb).unwrap(), write_xlsx(&again).unwrap()] {
        let u = Unpacked::new(&saved);
        u.assert_valid();
        let text = u.text("xl/workbook.xml");
        let root = xml::parse(text.as_bytes()).unwrap();
        // Schema order: fileVersion is the first child of <workbook>, extLst the last.
        assert_eq!(root.children.first().map(|e| e.name.as_str()), Some("fileVersion"), "{text}");
        assert_eq!(root.children.last().map(|e| e.name.as_str()), Some("extLst"), "{text}");
        assert!(text.contains(r#"<fileVersion appName="xl" lastEdited="7" lowestEdited="7" rupBuild="11008"/>"#), "{text}");
        assert!(text.contains(&format!("<extLst>{calc_features}</extLst>")), "{text}");
    }

    // A workbook that had neither gets a <fileVersion> naming GridCraft (#1699), kept on the next
    // save, and no calcFeatures: Excel recalculates it on open.
    let new = write_xlsx(&gridcraft_model::Workbook::new()).unwrap();
    let (reread, _) = read_xlsx(&new).unwrap();
    for saved in [new, write_xlsx(&reread).unwrap()] {
        let u = Unpacked::new(&saved);
        u.assert_valid();
        let plain = u.text("xl/workbook.xml");
        let root = xml::parse(plain.as_bytes()).unwrap();
        assert_eq!(root.children.first().map(|e| e.name.as_str()), Some("fileVersion"), "{plain}");
        assert!(plain.contains(r#"<fileVersion appName="gridcraft"/>"#) && !plain.contains("extLst"), "{plain}");
    }
}

const LOGO: &[u8] = b"\x89PNG\r\n\x1a\n letterhead logo";
const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n footer signature";
const HF_VML: &str = r##"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:x="urn:schemas-microsoft-com:office:excel"><o:shapelayout v:ext="edit"><o:idmap v:ext="edit" data="1"/></o:shapelayout><v:shapetype id="_x0000_t75" coordsize="21600,21600" o:spt="75" o:preferrelative="t" path="m@4@5l@4@11@9@11@9@5xe" filled="f" stroked="f"/><v:shape id="LH" o:spid="_x0000_s1025" type="#_x0000_t75" style="position:absolute;margin-left:0;margin-top:0;width:120pt;height:40pt;z-index:1"><v:imagedata o:relid="rId1" o:title="logo"/><o:lock v:ext="edit" rotation="t"/></v:shape><v:shape id="CF" o:spid="_x0000_s1026" type="#_x0000_t75" style="position:absolute;margin-left:0;margin-top:0;width:80pt;height:20pt;z-index:2"><v:imagedata o:relid="rId2" o:title="signature"/><o:lock v:ext="edit" rotation="t"/></v:shape></xml>"##;

/// The VML drawing of one note and nothing else. [`VML`] also holds a check box that only the
/// drawing lists, which is kept with the drawing (and its source shape block).
const NOTE_VML: &str = r##"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:x="urn:schemas-microsoft-com:office:excel"><o:shapelayout v:ext="edit"><o:idmap v:ext="edit" data="1"/></o:shapelayout><v:shape id="_x0000_s1027" type="#_x0000_t202" style="position:absolute"><v:textbox><div>old note<br></div></v:textbox><x:ClientData ObjectType="Note"><x:Row>0</x:Row><x:Column>0</x:Column></x:ClientData></v:shape></xml>"##;

/// #1257: a letterhead in the header and a picture in the footer (`&G`), drawn from a VML part
/// that `<legacyDrawingHF>` names, next to a note whose VML drawing GridCraft rewrites.
fn header_picture_package() -> Vec<u8> {
    let types = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="vml" ContentType="application/vnd.openxmlformats-officedocument.vmlDrawing"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/comments1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml"/></Types>"#;
    let workbook = format!(
        r#"<?xml version="1.0"?><workbook xmlns="{NS_MAIN}" xmlns:r="{NS_R}"><sheets><sheet name="Invoice" sheetId="1" r:id="rId1"/></sheets></workbook>"#
    );
    let sheet = format!(
        r#"<?xml version="1.0"?><worksheet xmlns="{NS_MAIN}" xmlns:r="{NS_R}"><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData><pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/><headerFooter><oddHeader>&amp;L&amp;G&amp;CInvoice</oddHeader><oddFooter>&amp;C&amp;G</oddFooter></headerFooter><legacyDrawing r:id="rId2"/><legacyDrawingHF r:id="rId3"/></worksheet>"#
    );
    let root_rels = rels(&[("rId1", "officeDocument", "xl/workbook.xml")]);
    let wb_rels = rels(&[("rId1", "worksheet", "worksheets/sheet1.xml")]);
    let sheet_rels = rels(&[
        ("rId1", "comments", "../comments1.xml"),
        ("rId2", "vmlDrawing", "../drawings/vmlDrawing1.vml"),
        ("rId3", "vmlDrawing", "../drawings/vmlDrawing2.vml"),
    ]);
    let hf_rels = rels(&[("rId1", "image", "../media/image1.png"), ("rId2", "image", "../media/image2.png")]);
    make_zip(&[
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", root_rels.as_bytes()),
        ("xl/workbook.xml", workbook.as_bytes()),
        ("xl/_rels/workbook.xml.rels", wb_rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
        ("xl/worksheets/_rels/sheet1.xml.rels", sheet_rels.as_bytes()),
        ("xl/comments1.xml", COMMENTS.as_bytes()),
        ("xl/drawings/vmlDrawing1.vml", NOTE_VML.as_bytes()),
        ("xl/drawings/vmlDrawing2.vml", HF_VML.as_bytes()),
        ("xl/drawings/_rels/vmlDrawing2.vml.rels", hf_rels.as_bytes()),
        ("xl/media/image1.png", LOGO),
        ("xl/media/image2.png", SIGNATURE),
    ])
}

#[test]
fn header_and_footer_pictures_survive_a_save() {
    let (wb, rep) = read_xlsx(&header_picture_package()).unwrap();
    assert!(!rep.warnings.iter().any(|w| w.contains("dropped")), "{:?}", rep.warnings);
    assert_eq!(wb.sheet(0).unwrap().comments.len(), 1, "the note is modelled");
    // Twice: what was kept on the first save is read back and kept again.
    let (again, _) = read_xlsx(&write_xlsx(&wb).unwrap()).unwrap();
    for saved in [write_xlsx(&wb).unwrap(), write_xlsx(&again).unwrap()] {
        let u = Unpacked::new(&saved);
        u.assert_valid();
        let sheet = sheet_part(&u, "Invoice");
        let sx = u.text(&sheet);
        assert!(sx.contains("<oddHeader>&amp;L&amp;G&amp;CInvoice</oddHeader>"), "{sx}");
        let (ty, vml) = u.target(&sheet, &attr_of(&sx, "legacyDrawingHF", "r:id"));
        assert!(ty.ends_with("/vmlDrawing"), "{ty}");
        assert_eq!(u.text(&vml), HF_VML);
        let pictures: Vec<(String, Vec<u8>)> = u.rels(&vml).into_iter().map(|r| (r.0, u.parts.get(&r.2).unwrap().clone())).collect();
        assert_eq!(pictures, [("rId1".to_string(), LOGO.to_vec()), ("rId2".to_string(), SIGNATURE.to_vec())]);
        // The notes keep their own drawing; CT_Worksheet puts legacyDrawingHF right after it.
        let notes = u.target(&sheet, &attr_of(&sx, "legacyDrawing", "r:id")).1;
        assert_ne!(notes, vml);
        assert!(u.text(&notes).contains("ObjectType=\"Note\""));
        assert!(sx.find("<headerFooter").unwrap() < sx.find("<legacyDrawing ").unwrap());
        assert!(sx.find("<legacyDrawing ").unwrap() < sx.find("<legacyDrawingHF ").unwrap());
    }
}

/// The shape blocks a VML drawing claims (`<o:idmap data>`) and the blocks of its shape ids
/// (`_x0000_s1025` is in block 1).
fn vml_blocks(vml: &str) -> (Vec<u64>, Vec<u64>) {
    let i = vml.find("<o:idmap").unwrap_or_else(|| panic!("no idmap in {vml}"));
    let data = attr_of(&vml[i..], "o:idmap", "data");
    let idmap = data.split(',').map(|b| b.trim().parse().unwrap()).collect();
    let spids = vml
        .match_indices("_x0000_s")
        .map(|(j, m)| vml[j + m.len()..].chars().take_while(char::is_ascii_digit).collect::<String>().parse::<u64>().unwrap() / 1024)
        .collect();
    (idmap, spids)
}

#[test]
fn notes_take_shape_blocks_the_header_pictures_dont_use() {
    let (wb, _) = read_xlsx(&header_picture_package()).unwrap();
    let u = Unpacked::new(&write_xlsx(&wb).unwrap());
    let sheet = sheet_part(&u, "Invoice");
    let sx = u.text(&sheet);
    let (hf_map, hf_ids) = vml_blocks(&u.text(&u.target(&sheet, &attr_of(&sx, "legacyDrawingHF", "r:id")).1));
    let (notes_map, notes_ids) = vml_blocks(&u.text(&u.target(&sheet, &attr_of(&sx, "legacyDrawing", "r:id")).1));
    assert_eq!((hf_map.as_slice(), hf_ids.as_slice()), ([1].as_slice(), [1, 1].as_slice()), "the header drawing is kept as it was");
    assert!(!notes_ids.is_empty());
    for b in notes_map.iter().chain(&notes_ids) {
        assert!(!hf_map.contains(b) && !hf_ids.contains(b), "notes reuse shape block {b}: {notes_map:?} {notes_ids:?}");
    }
    assert!(notes_ids.iter().all(|b| notes_map.contains(b)), "note ids outside their drawing's blocks");
}

#[test]
fn modelled_form_controls_join_kept_ones_in_one_controls_element() {
    use gridcraft_model::{Anchor, ControlKind, FormControl};
    let (mut wb, _) = read_xlsx(&rich_package()).unwrap();
    assert!(wb.sheet(0).unwrap().controls.is_empty(), "a check box stays with pass-through");
    let anchor = Anchor { cell: gridcraft_core::CellRef::new(8, 1), width: 120.0, height: 20.0, ..Default::default() };
    let slider = FormControl { id: 900, kind: ControlKind::Slider, anchor, link: Some("$B$9".into()), value: 40, ..Default::default() };
    if let Some(sh) = wb.sheet_mut(0) {
        sh.controls.push(slider);
    }
    let out = write_xlsx(&wb).unwrap();
    let u = Unpacked::new(&out);
    u.assert_valid();
    let sheet = sheet_part(&u, "Data");
    let sx = u.text(&sheet);
    assert_eq!(sx.matches("<controls").count(), 1, "{sx}");
    // The kept check box and the slider, each with its own properties part.
    let ids: Vec<&str> = sx.split("<control shapeId=\"").skip(1).filter_map(|r| r.split('"').next()).collect();
    assert!(ids.contains(&"1026"), "{ids:?}");
    let ours = ids.iter().find(|i| **i != "1026").copied().unwrap_or_default();
    assert_eq!(u.find(CTRL_PROP.as_bytes()).len(), 1, "the kept properties part is written once");
    // One VML drawing holds the kept shapes, the note and the slider, in fresh blocks.
    let vml = u.text(&u.target(&sheet, &attr_of(&sx, "legacyDrawing", "r:id")).1);
    assert!(vml.contains("_x0000_s1025") && vml.contains("_x0000_s1026"), "{vml}");
    assert!(vml.contains(&format!("id=\"_x0000_s{ours}\"")) && vml.contains("ObjectType=\"Scroll\""), "{vml}");
    assert!(vml.contains("id=\"_x0000_t201\""), "the form control shape type is defined");
    assert_eq!(ours, "2050", "the note is 2049, the slider follows it");

    // Read back: the slider is modelled again and the check box is kept, once.
    let (wb2, rep) = read_xlsx(&out).unwrap();
    assert!(!rep.warnings.iter().any(|w| w.contains("dropped")), "{:?}", rep.warnings);
    let data = wb2.sheet(0).unwrap();
    assert_eq!(data.controls.len(), 1);
    assert_eq!((data.controls[0].kind, data.controls[0].value, data.controls[0].link.as_deref()), (ControlKind::Slider, 40, Some("$B$9")));
    let kept = data.preserved.as_deref().unwrap();
    let controls = kept.elements.iter().find(|e| e.slot == "controls").map(|e| e.xml.clone()).unwrap_or_default();
    assert!(controls.contains("Check Box 1") && !controls.contains(&format!("shapeId=\"{ours}\"")), "{controls}");
    let kept_vml = String::from_utf8_lossy(&kept.vml.as_ref().unwrap().data).into_owned();
    assert!(!kept_vml.contains("ObjectType=\"Scroll\""), "the modelled slider's shape is not kept");
    // A second save writes the same controls again, without duplicates.
    let again = Unpacked::new(&write_xlsx(&wb2).unwrap());
    let sx2 = again.text(&sheet_part(&again, "Data"));
    assert_eq!(sx2.matches("<control shapeId=").count(), sx.matches("<control shapeId=").count(), "{sx2}");
    again.assert_valid();
}

#[test]
fn a_sheet_of_modelled_controls_keeps_nothing_for_them() {
    use gridcraft_model::{ControlKind, FormControl};
    let mut wb = gridcraft_model::Workbook::default();
    if let Some(sh) = wb.sheet_mut(0) {
        sh.controls.push(FormControl { id: 1, kind: ControlKind::Button, text: "Go".into(), ..Default::default() });
        sh.controls.push(FormControl { id: 2, kind: ControlKind::Spin, ..Default::default() });
    }
    let (back, rep) = read_xlsx(&write_xlsx(&wb).unwrap()).unwrap();
    assert!(rep.warnings.is_empty(), "{:?}", rep.warnings);
    let sh = back.sheet(0).unwrap();
    assert_eq!(sh.controls.len(), 2);
    assert!(sh.preserved.is_none(), "{:?}", sh.preserved);
}

// Ordinary Save regressions inspect the actual package before import can hide writer loss.
type OrdinaryTag = (String, String, Vec<(String, String, String)>);
struct OrdinaryNode {
    tag: OrdinaryTag,
    parent: Option<usize>,
    text: String,
    span: std::ops::Range<usize>,
}
fn ordinary_nodes(text: &str) -> Vec<OrdinaryNode> {
    use quick_xml::{NsReader, events::Event, name::ResolveResult};
    let mut reader = NsReader::from_str(text);
    reader.resolver_mut().set_max_declarations_per_element(text.len());
    let namespace = |ns: ResolveResult<'_>| match ns {
        ResolveResult::Bound(n) => quick_xml::escape::unescape(std::str::from_utf8(n.as_ref()).unwrap()).unwrap().into_owned(),
        ResolveResult::Unbound => String::new(),
        ResolveResult::Unknown(p) => panic!("unbound prefix {p:?}"),
    };
    let mut nodes: Vec<OrdinaryNode> = Vec::new();
    let mut stack = Vec::new();
    loop {
        let start = usize::try_from(reader.buffer_position()).unwrap();
        let event = reader.read_event().unwrap();
        let open = matches!(event, Event::Start(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let (ns, local) = reader.resolver().resolve_element(e.name());
                let mut attrs = Vec::new();
                for a in e.attributes() {
                    let a = a.unwrap();
                    if a.key.as_ref() == b"xmlns" || a.key.as_ref().starts_with(b"xmlns:") {
                        continue;
                    }
                    let (ns, local) = reader.resolver().resolve_attribute(a.key);
                    attrs.push((
                        namespace(ns),
                        String::from_utf8(local.as_ref().to_vec()).unwrap(),
                        a.normalized_value(quick_xml::XmlVersion::Explicit1_0).unwrap().into_owned(),
                    ));
                }
                let tag = (namespace(ns), String::from_utf8(local.as_ref().to_vec()).unwrap(), attrs);
                let index = nodes.len();
                nodes.push(OrdinaryNode {
                    tag,
                    parent: stack.last().copied(),
                    text: String::new(),
                    span: start..usize::try_from(reader.buffer_position()).unwrap(),
                });
                if open {
                    stack.push(index);
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::Text(e) => {
                if let Some(&i) = stack.last() {
                    nodes[i].text.push_str(&e.decode().unwrap());
                }
            }
            Event::GeneralRef(e) => {
                if let Some(&i) = stack.last() {
                    nodes[i].text.push_str(&quick_xml::escape::unescape(&format!("&{};", e.decode().unwrap())).unwrap());
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    nodes
}
fn ordinary_tags(text: &str) -> Vec<OrdinaryTag> {
    ordinary_nodes(text).into_iter().map(|n| n.tag).collect()
}
fn ordinary_field<'a>(node: &'a OrdinaryNode, ns: &str, name: &str) -> Option<&'a str> {
    let fields: Vec<_> = node.tag.2.iter().filter(|(n, k, _)| n == ns && k == name).collect();
    assert!(fields.len() <= 1);
    fields.first().map(|(_, _, v)| v.as_str())
}
fn ordinary_child(nodes: &[OrdinaryNode], parent: usize, ns: &str, name: &str) -> usize {
    let matches: Vec<_> =
        nodes.iter().enumerate().filter(|(_, n)| n.parent == Some(parent) && n.tag.0 == ns && n.tag.1 == name).map(|(i, _)| i).collect();
    assert_eq!(matches.len(), 1, "{ns}:{name}");
    matches[0]
}
fn ordinary_output(out: &Unpacked, label: &str) {
    if let Ok(root) = std::env::var("GRIDCRAFT_D3_I1_OUTPUTS_DIR") {
        for (name, bytes) in &out.parts {
            let path = std::path::Path::new(&root).join(label).join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
    }
}

fn ordinary_slider(wb: &mut gridcraft_model::Workbook) {
    Arc::make_mut(&mut wb.sheets[0]).controls.push(gridcraft_model::FormControl {
        id: 900,
        kind: gridcraft_model::ControlKind::Slider,
        name: "Native slider".into(),
        value: 40,
        link: Some("$B$9".into()),
        anchor: gridcraft_model::Anchor { cell: gridcraft_core::CellRef::new(8, 1), width: 120.0, height: 20.0, ..Default::default() },
        ..Default::default()
    });
}
fn ordinary_kept(wb: &mut gridcraft_model::Workbook) -> &mut gridcraft_model::Preserved {
    Arc::make_mut(Arc::make_mut(&mut wb.sheets[0]).preserved.as_mut().unwrap())
}
fn ordinary_reload(wb: &gridcraft_model::Workbook) -> Unpacked {
    let bytes = write_xlsx(wb).unwrap();
    let (again, report) = read_xlsx(&bytes).unwrap();
    assert!(!report.warnings.iter().any(|w| w.contains("dropped")), "{:?}", report.warnings);
    assert_eq!(again.sheets[0].comments, wb.sheets[0].comments);
    assert_eq!(again.sheets[0].controls.len(), wb.sheets[0].controls.len());
    let ids: std::collections::HashSet<_> = again.sheets[0].controls.iter().map(|c| c.id).collect();
    assert_eq!(ids.len(), again.sheets[0].controls.len());
    for c in &wb.sheets[0].controls {
        let matches: Vec<_> = again.sheets[0].controls.iter().filter(|x| x.name == c.name).collect();
        assert_eq!(matches.len(), 1, "{}", c.name);
        let got = matches[0];
        assert_eq!((got.kind, got.value, &got.link, &got.anchor), (c.kind, c.value, &c.link, &c.anchor));
    }
    let out = Unpacked::new(&write_xlsx(&again).unwrap());
    out.assert_valid();
    for retained in [OLE, EMF, CTRL_PROP.as_bytes()] {
        assert_eq!(out.find(retained).len(), 1, "retained {:?}", String::from_utf8_lossy(retained));
    }
    out
}

#[test]
fn ordinary_save_writer_control_real_close_and_branch_selection() {
    let mc = "http://schemas.openxmlformats.org/markup-compatibility/2006";
    for (before, after) in [("", ""), ("<!--<mc:Choice>-->", "<!--</controls>-->"), ("<?p fake='<Choice>'?>", "<![CDATA[</controls>]]>")] {
        for branch in ["plain", "choice", "fallback-first", "no-choice-controls", "multiple-choice", "fallback-only", "fallback-before-empty-choice"]
        {
            let (mut wb, _) = read_xlsx(&rich_package()).unwrap();
            ordinary_slider(&mut wb);
            let e = ordinary_kept(&mut wb).elements.iter_mut().find(|e| e.slot == "controls").unwrap();
            let original = e.xml.replace("</controls>", &format!("{after}</controls>"));
            let fallback = "<mc:Fallback><controls><marker/></controls></mc:Fallback>";
            e.xml = match branch {
                "plain" => format!("{before}{original}"),
                "choice" => format!("<mc:AlternateContent xmlns:mc='{mc}'>{before}<mc:Choice>{original}</mc:Choice>{fallback}</mc:AlternateContent>"),
                "fallback-first" => format!("<mc:AlternateContent xmlns:mc='{mc}'>{fallback}<mc:Choice>{original}</mc:Choice></mc:AlternateContent>"),
                "fallback-only" => format!("<mc:AlternateContent xmlns:mc='{mc}'><mc:Fallback>{original}</mc:Fallback></mc:AlternateContent>"),
                "fallback-before-empty-choice" => {
                    format!("<mc:AlternateContent xmlns:mc='{mc}'><mc:Fallback>{original}</mc:Fallback><mc:Choice/></mc:AlternateContent>")
                }
                "no-choice-controls" => format!(
                    "<mc:AlternateContent xmlns:mc='{mc}'><mc:Choice><wrapper><controls/></wrapper></mc:Choice><mc:Fallback>{original}</mc:Fallback></mc:AlternateContent>"
                ),
                _ => format!(
                    "<mc:AlternateContent xmlns:mc='{mc}'><mc:Choice/><mc:Choice><controls/></mc:Choice><mc:Fallback>{original}</mc:Fallback></mc:AlternateContent>"
                ),
            };
            let out = Unpacked::new(&write_xlsx(&wb).unwrap());
            let root = xml::parse(&out.parts[&sheet_part(&out, "Data")]).unwrap();
            let selected = root.children.iter().find_map(crate::controls::controls_of).unwrap();
            assert_eq!(selected.children.len(), 2, "{branch}: retained plus native entry, outside fake markup");
            let second = ordinary_reload(&wb);
            assert!(second.text(&sheet_part(&second, "Data")).contains("Native slider"));
        }
    }
}

#[test]
fn ordinary_save_writer_vml_real_close() {
    for suffix in ["<!--</xml>-->", "<?p close='</xml>'?>", ""] {
        let (mut wb, _) = read_xlsx(&rich_package()).unwrap();
        let v = ordinary_kept(&mut wb).vml.as_mut().unwrap();
        v.data = Arc::from(format!("{}{suffix}", String::from_utf8_lossy(&v.data)).replacen("</xml>", "<!--harmless--></xml>", 1).into_bytes());
        let out = Unpacked::new(&write_xlsx(&wb).unwrap());
        let tags = ordinary_tags(&out.text("xl/drawings/vmlDrawing1.vml"));
        assert_eq!(tags.iter().filter(|(_, n, _)| n == "shape").count(), 3);
        assert_eq!(tags.iter().filter(|(_, n, a)| n == "ClientData" && a.iter().any(|(_, n, v)| n == "ObjectType" && v == "Note")).count(), 1);
        ordinary_reload(&wb);
    }
}

#[test]
fn ordinary_save_writer_quote_scope_and_xmlns_attribute() {
    for (decl, body, attr) in [
        ("xmlns:p='urn:inert'".to_string(), format!("<a xmlns:p='{NS_R}' p:id='rId5'/><b p:id='inert'/>"), "name='Check \" Box 1'"),
        (format!("xmlns:p='{NS_R}'"), "<a xmlns:p='urn:inert'><b p:id='inert'/></a><b p:id='rId5'/>".into(), "name='Check \" Box 1'"),
        ("xmlns:p='urn:inert'".to_string(), format!("<a xmlns:p='{NS_R}'><b p:id='rId5'/></a><b p:id='inert'/>"), "name='Check \" Box 1'"),
        (format!("xmlns:xmlnsfoo='{NS_R}'"), "<b xmlnsfoo:id='rId5'/>".into(), "name='Check \" Box 1'"),
    ] {
        let (mut wb, _) = read_xlsx(&rich_package()).unwrap();
        let e = ordinary_kept(&mut wb).elements.iter_mut().find(|e| e.slot == "controls").unwrap();
        e.xml = e
            .xml
            .replacen("<controls", &format!("<controls {decl}"), 1)
            .replace("name=\"Check Box 1\"", attr)
            .replace("</controls>", &format!("{body}<marker id='inert' value='&amp;&quot;&#34;'/></controls>"));
        let expected_marker = "<marker id=\"inert\" value=\"&amp;&quot;&#34;\"/>";
        let out = Unpacked::new(&write_xlsx(&wb).unwrap());
        let sheet = sheet_part(&out, "Data");
        let text = out.text(&sheet);
        let tags = ordinary_tags(&text);
        assert!(text.contains(expected_marker), "entity bytes and attribute order: {text}");
        assert!(tags.iter().any(|(_, n, a)| n == "control" && a.iter().any(|(_, n, v)| n == "name" && v == "Check \" Box 1")));
        for (_, _, attrs) in tags {
            for (ns, _, value) in attrs {
                if ns == NS_R {
                    assert!(out.rels(&sheet).iter().any(|r| r.0 == value));
                }
                if ns == "urn:inert" {
                    assert_eq!(value, "inert");
                }
            }
        }
        ordinary_reload(&wb);
        // Build an actual imported package too: importer must not prune an inert scoped id.
        let imported = Unpacked::new(&rich_package());
        let mut parts = imported.parts.clone();
        let sheet = "xl/worksheets/sheet1.xml";
        let changed = String::from_utf8(parts[sheet].clone())
            .unwrap()
            .replace("<controls>", &format!("<controls {decl}>"))
            .replace("</controls>", &format!("{body}</controls>"));
        parts.insert(sheet.into(), changed.into_bytes());
        let files: Vec<_> = parts.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
        let (loaded, rep) = read_xlsx(&make_zip(&files)).unwrap();
        assert!(!rep.warnings.iter().any(|w| w.contains("controls were dropped")), "{:?}", rep.warnings);
        assert!(loaded.sheets[0].preserved.as_ref().unwrap().elements.iter().any(|e| e.slot == "controls"));
    }
}

fn ordinary_control_targets(out: &Unpacked, sheet: &str) {
    for (_, _, attrs) in ordinary_tags(&out.text(sheet)).iter().filter(|(ns, n, _)| ns == NS_MAIN && n == "control") {
        let id = &attrs.iter().find(|(ns, n, _)| ns == NS_R && n == "id").unwrap().2;
        let (kind, target) = out.target(sheet, id);
        assert!(kind.ends_with("/ctrlProp"));
        let properties = xml::parse(&out.parts[&target]).unwrap();
        assert_eq!(properties.name, "formControlPr");
        let name = &attrs.iter().find(|(_, n, _)| n == "name").unwrap().2;
        if let Some((value, link)) = match name.as_str() {
            "Native slider" => Some(("40", "$B$9")),
            "Native second" => Some(("73", "$C$10")),
            _ => None,
        } {
            assert_eq!(
                (properties.attr("objectType"), properties.attr("val"), properties.attr("fmlaLink")),
                (Some("Scroll"), Some(value), Some(link))
            );
        } else if name == "Check Box 1" {
            assert_eq!(out.parts[&target], CTRL_PROP.as_bytes());
        }
    }
}

fn ordinary_control_namespace_output(out: &Unpacked, wb: &gridcraft_model::Workbook, default: &str, r: &str) {
    let sheet = sheet_part(out, "Data");
    let text = out.text(&sheet);
    let nodes = ordinary_nodes(&text);
    ordinary_control_targets(out, &sheet);
    let controls: Vec<_> = nodes.iter().filter(|n| n.tag.1 == "control").collect();
    assert_eq!(controls.len(), 1 + 2 * wb.sheets[0].controls.len());
    assert!(controls.iter().all(|n| n.tag.0 == NS_MAIN && ordinary_field(n, NS_R, "id").is_some()));
    let mut targets = std::collections::HashSet::new();
    for (i, c) in wb.sheets[0].controls.iter().enumerate() {
        let own: Vec<_> = controls.iter().filter(|n| ordinary_field(n, "", "name") == Some(c.name.as_str())).collect();
        assert_eq!(own.len(), 2, "one Choice and one Fallback for {}", c.name);
        let parents: Vec<_> = own.iter().map(|n| &nodes[n.parent.unwrap()].tag).collect();
        assert!(parents.iter().all(|n| n.0 == "http://schemas.openxmlformats.org/markup-compatibility/2006"));
        assert_eq!(parents.iter().map(|n| n.1.as_str()).collect::<Vec<_>>(), ["Choice", "Fallback"]);
        let ids: Vec<_> = own.iter().map(|n| ordinary_field(n, NS_R, "id").unwrap()).collect();
        assert_eq!(ids[0], ids[1]);
        assert!(targets.insert(out.target(&sheet, ids[0]).1));
        assert!(own.iter().all(|n| ordinary_field(n, "", "shapeId") == Some((2050 + i).to_string().as_str())));
    }
    assert_eq!(nodes.iter().filter(|n| n.tag.1 == "marker" && n.tag.0 == default).count(), 1);
    assert_eq!(text.matches("<marker/><!--child shadow--><child xmlns:r=\"urn:temporary\"/>").count(), 1);
    if r != NS_R {
        assert_eq!(nodes.iter().filter(|n| ordinary_field(n, r, "label") == Some("retained")).count(), 1);
        assert_eq!(text.matches(" r:label=\"retained\"").count(), 1);
    }
}

#[test]
fn ordinary_save_writer_control_namespace_isolation() {
    for placement in ["container", "choice-ancestor", "fallback-ancestor"] {
        for (case, (default, r)) in [("", NS_R), ("urn:foreign", "urn:inert"), (NS_MAIN, "urn:inert"), (NS_MAIN, NS_R)].into_iter().enumerate() {
            let (mut wb, _) = read_xlsx(&rich_package()).unwrap();
            ordinary_slider(&mut wb);
            ordinary_slider(&mut wb);
            let second = &mut Arc::make_mut(&mut wb.sheets[0]).controls[1];
            second.id = 901;
            second.name = "Native second".into();
            second.value = 73;
            second.link = Some("$C$10".into());
            second.anchor = gridcraft_model::Anchor { cell: gridcraft_core::CellRef::new(9, 2), width: 64.0, height: 40.0, ..Default::default() };
            let retained = String::from_utf8(ordinary_kept(&mut wb).vml.as_ref().unwrap().data.to_vec()).unwrap();
            let e = ordinary_kept(&mut wb).elements.iter_mut().find(|e| e.slot == "controls").unwrap();
            e.xml = e
                .xml
                .replace(&format!("xmlns:r=\"{NS_R}\""), &format!("xmlns:k='{NS_R}' xmlns:r='{r}'"))
                .replace("r:id=", "k:id=")
                .replacen("<controls", &format!("<s:controls xmlns:s='{NS_MAIN}' xmlns='{default}'"), 1)
                .replace("</controls>", "<marker/><!--child shadow--><child xmlns:r='urn:temporary'/></s:controls>")
                .replace("<control ", "<s:control ");
            if r != NS_R {
                e.xml = e.xml.replacen("<s:controls", "<s:controls r:label='retained'", 1);
            }
            if placement != "container" {
                e.xml = e.xml.replace(&format!(" xmlns:k='{NS_R}' xmlns:r='{r}'"), "").replace(&format!(" xmlns='{default}'"), "");
                let branch = if placement == "choice-ancestor" {
                    format!("<mc:Choice>{}</mc:Choice>", e.xml)
                } else {
                    format!("<mc:Choice/><mc:Fallback>{}</mc:Fallback>", e.xml)
                };
                e.xml = format!(
                    "<mc:AlternateContent xmlns:mc='http://schemas.openxmlformats.org/markup-compatibility/2006' xmlns:k='{NS_R}' xmlns:r='{r}' xmlns='{default}'>{branch}</mc:AlternateContent>"
                );
            }
            let first = Unpacked::new(&write_xlsx(&wb).unwrap());
            for (save, out) in [first, ordinary_reload(&wb)].iter().enumerate() {
                ordinary_output(out, &format!("controls-{placement}-{case}-{}", save + 1));
                ordinary_control_namespace_output(out, &wb, default, r);
                ordinary_vml_output(out, &wb, &retained, save + 1, "");
            }
        }
    }
}

fn ordinary_vml_context(mut text: String, wrong: &str) -> String {
    if wrong == "normalized" {
        return text.replace("com:vml", "com:vm&#108;").replace("office:office", "office:offic&#101;").replace("office:excel", "office:exce&#108;");
    }
    for (p, alias) in [("v", "w"), ("o", "f"), ("x", "z")] {
        if wrong.contains(p) || wrong == "inert" {
            text = text.replace(&format!("xmlns:{p}="), &format!("xmlns:{alias}=")).replace(&format!("{p}:"), &format!("{alias}:"));
            if wrong.ends_with("inert") {
                text = text.replacen("<xml", &format!("<xml xmlns:{p}='urn:inert-{p}'"), 1);
            }
        }
    }
    if wrong == "inert" {
        text = text.replace("</xml>", "<v:marker o:label='inert'/></xml>");
    } else if wrong.is_empty() {
        text = text.replace("</xml>", "<child xmlns:v='urn:temporary'/></xml>");
    }
    text
}
fn ordinary_vml_output(out: &Unpacked, wb: &gridcraft_model::Workbook, retained: &str, save: usize, wrong: &str) {
    const V: &str = "urn:schemas-microsoft-com:vml";
    const O: &str = "urn:schemas-microsoft-com:office:office";
    const X: &str = "urn:schemas-microsoft-com:office:excel";
    let sheet = sheet_part(out, "Data");
    let part = out.target(&sheet, &attr_of(&out.text(&sheet), "legacyDrawing", "r:id")).1;
    let text = out.text(&part);
    let nodes = ordinary_nodes(&text);
    let controls = &wb.sheets[0].controls;
    let raw = retained.strip_suffix("</xml>").unwrap();
    let head = if retained.contains("<o:idmap") { raw.replacen("data=\"1\"", &format!("data=\"1{}\"", ",2".repeat(save)), 1) } else { raw.into() };
    assert!(text.starts_with(&head), "retained bytes: only each existing idmap claim may change");
    let named = |kind: &str, id: &str| {
        let matches: Vec<_> = nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.tag.0 == V && n.tag.1 == kind && ordinary_field(n, "", "id") == Some(id))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(matches.len(), 1, "{kind} {id}");
        matches[0]
    };
    for (id, spt) in [("_x0000_t202", "202"), ("_x0000_t201", "201")].iter().take(1 + usize::from(!controls.is_empty())) {
        let i = named("shapetype", id);
        assert_eq!(ordinary_field(&nodes[i], O, "spt"), Some(*spt));
        assert!(!text[nodes[i].span.clone()].contains("xmlns:x="));
        if *spt == "201" {
            let lock = ordinary_child(&nodes, i, O, "lock");
            assert_eq!((ordinary_field(&nodes[lock], V, "ext"), ordinary_field(&nodes[lock], "", "shapetype")), (Some("edit"), Some("t")));
        }
    }
    let expected_ids: Vec<_> = [1025, 1026].into_iter().chain(2049..2050 + controls.len()).map(|n| format!("_x0000_s{n}")).collect();
    let shapes: Vec<_> = nodes.iter().filter(|n| n.tag.1 == "shape").collect();
    assert!(shapes.iter().all(|n| n.tag.0 == V));
    assert_eq!(shapes.iter().map(|n| ordinary_field(n, "", "id").unwrap()).collect::<Vec<_>>(), expected_ids);
    for position in 0..=controls.len() {
        let shape = named("shape", &format!("_x0000_s{}", 2049 + position));
        assert_eq!(ordinary_field(&nodes[shape], O, "insetmode"), Some("auto"));
        let (kind, ty, fields) = if position == 0 {
            ("Note", "#_x0000_t202", vec![("Row", "0".into()), ("Column", "0".into()), ("Anchor", "1, 15, 0, 2, 3, 15, 4, 16".into())])
        } else {
            let c = &controls[position - 1];
            let lock = ordinary_child(&nodes, shape, O, "lock");
            assert_eq!((ordinary_field(&nodes[lock], V, "ext"), ordinary_field(&nodes[lock], "", "rotation")), (Some("edit"), Some("t")));
            (
                "Scroll",
                "#_x0000_t201",
                vec![
                    ("FmlaLink", c.link.clone().unwrap()),
                    ("Val", c.value.to_string()),
                    ("Anchor", ["1, 0, 8, 0, 2, 56, 9, 0", "2, 0, 9, 0, 3, 0, 11, 0"][position - 1].into()),
                ],
            )
        };
        assert_eq!(ordinary_field(&nodes[shape], "", "type"), Some(ty));
        let data = ordinary_child(&nodes, shape, X, "ClientData");
        assert_eq!(ordinary_field(&nodes[data], "", "ObjectType"), Some(kind));
        for (name, value) in fields {
            let i = ordinary_child(&nodes, data, X, name);
            assert_eq!(nodes[i].text, value, "{kind}/{name}");
        }
    }
    let clients: Vec<_> = nodes.iter().filter(|n| n.tag.1 == "ClientData").collect();
    assert!(clients.iter().all(|n| n.tag.0 == X));
    let expected: Vec<_> = ["Pict", "Checkbox", "Note"].into_iter().chain(std::iter::repeat_n("Scroll", controls.len())).collect();
    assert_eq!(clients.iter().map(|n| ordinary_field(n, "", "ObjectType").unwrap()).collect::<Vec<_>>(), expected);
    if wrong == "inert" {
        let marker: Vec<_> = nodes.iter().filter(|n| n.tag.0 == "urn:inert-v" && n.tag.1 == "marker").collect();
        assert_eq!(marker.len(), 1);
        assert_eq!(ordinary_field(marker[0], "urn:inert-o", "label"), Some("inert"));
    }
    if wrong == "normalized" {
        assert_eq!([text.matches("xmlns:v=").count(), text.matches("xmlns:o=").count(), text.matches("xmlns:x=").count()], [1; 3]);
    }
}
fn ordinary_vml_namespace_case(wrong: &str, slider: bool) {
    let (mut wb, _) = read_xlsx(&rich_package()).unwrap();
    if slider {
        ordinary_slider(&mut wb);
    }
    let v = ordinary_kept(&mut wb).vml.as_mut().unwrap();
    let retained = ordinary_vml_context(String::from_utf8(v.data.to_vec()).unwrap(), wrong);
    v.data = Arc::from(retained.clone().into_bytes());
    let first = Unpacked::new(&write_xlsx(&wb).unwrap());
    for (save, out) in [first, ordinary_reload(&wb)].iter().enumerate() {
        ordinary_output(out, &format!("vml-{wrong}-{slider}-{}", save + 1));
        ordinary_vml_output(out, &wb, &retained, save + 1, wrong);
    }
    Arc::make_mut(&mut Arc::make_mut(&mut wb.sheets[0]).comments).clear();
    Arc::make_mut(&mut wb.sheets[0]).controls.clear();
    let lone = Unpacked::new(&write_xlsx(&wb).unwrap());
    let sheet = sheet_part(&lone, "Data");
    let part = lone.target(&sheet, &attr_of(&lone.text(&sheet), "legacyDrawing", "r:id")).1;
    assert_eq!(lone.parts[&part], retained.as_bytes());
}

#[test]
fn ordinary_save_writer_vml_namespace_isolation() {
    for slider in [false, true] {
        for wrong in ["vox", "v", "o", "x", "v-inert", "o-inert", "x-inert", "inert", "normalized", ""] {
            ordinary_vml_namespace_case(wrong, slider);
        }
    }
}

#[test]
fn ordinary_save_writer_selected_empty_refuses_invisible_control() {
    let (mut wb, _) = read_xlsx(&rich_package()).unwrap();
    ordinary_slider(&mut wb);
    let e = ordinary_kept(&mut wb).elements.iter_mut().find(|e| e.slot == "controls").unwrap();
    e.xml="<mc:AlternateContent xmlns:mc='http://schemas.openxmlformats.org/markup-compatibility/2006'><mc:Choice><controls/></mc:Choice><mc:Fallback><controls></controls></mc:Fallback></mc:AlternateContent>".into();
    match write_xlsx(&wb) {
        Ok(bytes) => {
            let out = Unpacked::new(&bytes);
            let root = xml::parse(&out.parts[&sheet_part(&out, "Data")]).unwrap();
            let selected = root.children.iter().find_map(crate::controls::controls_of).unwrap();
            assert!(selected.children.is_empty(), "old writer's real reader target remains empty");
            let (again, _) = read_xlsx(&bytes).unwrap();
            assert!(!again.sheets[0].controls.iter().any(|c| c.name == "Native slider"), "reader loses the sheet-only control name");
            panic!("old producer succeeded with ignored native controls; must refuse before publication");
        }
        Err(crate::IoError::Format(message)) => assert!(message.contains("controls")),
        Err(other) => panic!("wrong writer refusal: {other}"),
    }
}

#[test]
fn ordinary_save_writer_literal_quote_alone() {
    let (mut wb, _) = read_xlsx(&rich_package()).unwrap();
    let e = ordinary_kept(&mut wb).elements.iter_mut().find(|e| e.slot == "controls").unwrap();
    e.xml = e.xml.replace("name=\"Check Box 1\"", "name='Check \" Box 1'");
    let out = Unpacked::new(&write_xlsx(&wb).unwrap());
    let text = out.text(&sheet_part(&out, "Data"));
    assert!(ordinary_tags(&text).iter().any(|(_, n, a)| n == "control" && a.iter().any(|(_, n, v)| n == "name" && v == "Check \" Box 1")));
    ordinary_reload(&wb);
}
#[test]
fn ordinary_save_writer_xmlns_prefix_attribute_alone() {
    let (mut wb, _) = read_xlsx(&rich_package()).unwrap();
    let p = ordinary_kept(&mut wb);
    p.rels.iter_mut().find(|r| r.id == "rId5").unwrap().id = "old-control".into();
    let e = p.elements.iter_mut().find(|e| e.slot == "controls").unwrap();
    e.xml = e.xml.replace("xmlns:r=", "xmlns:xmlnsfoo=").replace("r:id=", "xmlnsfoo:id=").replace("rId5", "old-control");
    let out = Unpacked::new(&write_xlsx(&wb).unwrap());
    let sheet = sheet_part(&out, "Data");
    for (ns, n, a) in ordinary_tags(&out.text(&sheet)) {
        if ns == NS_MAIN && n == "control" {
            for (ns, _, v) in a {
                if ns == NS_R {
                    assert!(out.rels(&sheet).iter().any(|r| r.0 == v), "actual REL id {v} must be remapped");
                }
            }
        }
    }
    ordinary_reload(&wb);
}
#[test]
fn ordinary_save_writer_controls_comment_alone() {
    let (mut wb, _) = read_xlsx(&rich_package()).unwrap();
    ordinary_slider(&mut wb);
    let e = ordinary_kept(&mut wb).elements.iter_mut().find(|e| e.slot == "controls").unwrap();
    e.xml = e.xml.replace("</controls>", "<!--</controls>--></controls>");
    let out = Unpacked::new(&write_xlsx(&wb).unwrap());
    let root = xml::parse(&out.parts[&sheet_part(&out, "Data")]).unwrap();
    assert_eq!(
        root.children.iter().find_map(crate::controls::controls_of).unwrap().children.len(),
        2,
        "actual native entry must be outside the comment"
    );
}

#[test]
fn ordinary_save_writer_empty_absent_large_namespace_and_caption_controls() {
    use gridcraft_model::{Preserved, PreservedXml, Workbook};
    for fragment in [
        "<controls/>",
        "<controls></controls>",
        "<mc:AlternateContent xmlns:mc='http://schemas.openxmlformats.org/markup-compatibility/2006'><mc:Choice/></mc:AlternateContent>",
    ] {
        let mut wb = Workbook::new();
        ordinary_slider(&mut wb);
        Arc::make_mut(&mut wb.sheets[0]).preserved =
            Some(Arc::new(Preserved { elements: vec![PreservedXml { slot: "controls".into(), xml: fragment.into() }], ..Default::default() }));
        if fragment == "<controls/>" {
            assert!(matches!(write_xlsx(&wb), Err(crate::IoError::Format(_))));
        } else {
            let bytes = write_xlsx(&wb).unwrap();
            let (again, _) = read_xlsx(&bytes).unwrap();
            assert_eq!(again.sheets[0].controls[0].name, "Native slider");
        }
        Arc::make_mut(&mut wb.sheets[0]).controls.clear();
        let out = Unpacked::new(&write_xlsx(&wb).unwrap());
        assert!(out.text(&sheet_part(&out, &wb.sheets[0].name)).contains(fragment.replace('\'', "\"").as_str()));
    }
    let (mut wb, _) = read_xlsx(&rich_package()).unwrap();
    ordinary_slider(&mut wb);
    let declarations = (0..300).map(|i| format!(" xmlns:p{i}='urn:inert-{i}'")).collect::<String>();
    let e = ordinary_kept(&mut wb).elements.iter_mut().find(|e| e.slot == "controls").unwrap();
    e.xml = e.xml.replacen("<controls", &format!("<controls{declarations}"), 1);
    let out = Unpacked::new(&write_xlsx(&wb).unwrap());
    assert_eq!(ordinary_tags(&out.text(&sheet_part(&out, "Data"))).iter().filter(|(_, n, _)| n == "control").count(), 3);
    ordinary_reload(&wb);
    let v = ordinary_kept(&mut wb).vml.as_mut().unwrap();
    v.data = Arc::from(
        String::from_utf8_lossy(&v.data).replace("</v:shape>", "<v:textbox><div>legacy caption<br></div></v:textbox></v:shape>").into_bytes(),
    );
    let out = Unpacked::new(&write_xlsx(&wb).unwrap());
    let text = out.text("xl/drawings/vmlDrawing1.vml");
    assert!(text.contains("legacy caption<br>"));
    assert_eq!(text.matches("<v:shape ").count(), 4);
    ordinary_reload(&wb);
}
