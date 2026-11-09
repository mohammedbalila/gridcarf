//! Grouped charts through the public XLSX import and save paths.

use std::io::{Cursor, Read};

use gridcraft_model::AnchorMode;

use crate::tests::minimal;
use crate::xml::{El, parse};
use crate::{read_xlsx, write_xlsx};

const REL_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const CHART_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const DRAW_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const ANCHOR_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";

fn fixture(group_flags: &str) -> Vec<u8> {
    let frame = |id: u32, x: i32| {
        format!(
            r#"<xdr:graphicFrame><xdr:nvGraphicFramePr><xdr:cNvPr id="{id}" name="Chart {id}"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr>
            <xdr:xfrm><a:off x="{x}" y="800000"/><a:ext cx="2000000" cy="1000000"/></xdr:xfrm>
            <a:graphic><a:graphicData uri="{CHART_NS}"><c:chart r:id="rId{id}"/></a:graphicData></a:graphic></xdr:graphicFrame>"#
        )
    };
    let drawing = format!(
        r#"<xdr:wsDr xmlns:xdr="{ANCHOR_NS}" xmlns:a="{DRAW_NS}" xmlns:c="{CHART_NS}" xmlns:r="{REL_NS}">
        <xdr:twoCellAnchor editAs="oneCell">
          <xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>0</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
          <xdr:to><xdr:col>5</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>10</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>
          <xdr:grpSp><xdr:nvGrpSpPr><xdr:cNvPr id="3" name="Revenue charts"/><xdr:cNvGrpSpPr/></xdr:nvGrpSpPr>
          <xdr:grpSpPr><a:xfrm {group_flags}><a:off x="0" y="0"/><a:ext cx="4000000" cy="2000000"/>
          <a:chOff x="400000" y="800000"/><a:chExt cx="4000000" cy="1000000"/></a:xfrm></xdr:grpSpPr>{}{}
          </xdr:grpSp><xdr:clientData/></xdr:twoCellAnchor></xdr:wsDr>"#,
        frame(1, 2_400_000),
        frame(2, 400_000),
    );
    let draw_rels = format!(
        r#"<Relationships xmlns="{PKG_NS}"><Relationship Id="rId1" Type="{REL_NS}/chart" Target="../charts/chart1.xml"/><Relationship Id="rId2" Type="{REL_NS}/chart" Target="../charts/chart2.xml"/></Relationships>"#
    );
    let sheet_rels = format!(
        r#"<Relationships xmlns="{PKG_NS}"><Relationship Id="rId1" Type="{REL_NS}/drawing" Target="../drawings/drawing1.xml"/></Relationships>"#
    );
    let chart = |pivot: &str| {
        format!(
            r#"<c:chartSpace xmlns:c="{CHART_NS}"><c:pivotSource><c:name>[Source.xlsx]Data!{pivot}</c:name></c:pivotSource>
            <c:chart><c:plotArea><c:barChart><c:barDir val="col"/><c:ser><c:idx val="0"/><c:order val="0"/>
            <c:val><c:numRef><c:f>Data!$A$1</c:f><c:numCache><c:ptCount val="2"/><c:pt idx="0"><c:v>12</c:v></c:pt><c:pt idx="1"><c:v>17</c:v></c:pt></c:numCache></c:numRef></c:val>
            </c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#
        )
    };
    minimal(
        r#"<sheetData/><drawing r:id="rId1"/>"#,
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", &sheet_rels),
            ("xl/drawings/drawing1.xml", &drawing),
            ("xl/drawings/_rels/drawing1.xml.rels", &draw_rels),
            ("xl/charts/chart1.xml", &chart("RightPivot")),
            ("xl/charts/chart2.xml", &chart("LeftPivot")),
        ],
        "",
        "",
    )
}

fn part(archive: &mut zip::ZipArchive<Cursor<&[u8]>>, path: &str) -> El {
    let mut bytes = Vec::new();
    archive.by_name(path).unwrap().read_to_end(&mut bytes).unwrap();
    parse(&bytes).unwrap()
}

#[test]
fn chart_group_survives_two_saves_with_positions_and_links() {
    let mut bytes = fixture("");
    for _ in 0..2 {
        let (wb, report) = read_xlsx(&bytes).unwrap();
        let sheet = wb.sheet(0).unwrap();
        assert_eq!(sheet.charts.len(), 2, "both charts must appear in the worksheet model");
        assert!(!report.warnings.iter().any(|w| w.contains("grouped shapes") || w.contains("grouped charts")));
        let width = sheet.col_left(5);
        let height = sheet.row_top(10);
        for (chart, x) in sheet.charts.iter().zip([width / 2.0, 0.0]) {
            let actual_x = sheet.col_left(chart.anchor.cell.col) + f64::from(chart.anchor.dx);
            let actual_y = sheet.row_top(chart.anchor.cell.row) + f64::from(chart.anchor.dy);
            assert!((actual_x - x).abs() < 0.01 && actual_y.abs() < 0.01);
            assert!((f64::from(chart.anchor.width) - width / 2.0).abs() < 0.01);
            assert!((f64::from(chart.anchor.height) - height).abs() < 0.01);
            assert_eq!(chart.anchor.mode, AnchorMode::MoveOnly);
        }

        bytes = write_xlsx(&wb).unwrap();
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
        let drawing = part(&mut zip, "xl/drawings/drawing1.xml");
        let anchors: Vec<_> = drawing.kids("twoCellAnchor").collect();
        assert_eq!(anchors.len(), 1);
        let group = anchors[0].child("grpSp").unwrap();
        assert_eq!(group.path(&["nvGrpSpPr", "cNvPr"]).unwrap().attr("name"), Some("Revenue charts"));
        let rels = part(&mut zip, "xl/drawings/_rels/drawing1.xml.rels");
        let frames: Vec<_> = group.kids("graphicFrame").collect();
        assert_eq!(frames.len(), 2);
        for (frame, expected) in frames.into_iter().zip(["Data!RightPivot", "Data!LeftPivot"]) {
            let id = frame.path(&["graphic", "graphicData", "chart"]).unwrap().attr("id").unwrap();
            let target = rels.kids("Relationship").find(|rel| rel.attr("Id") == Some(id)).unwrap().attr("Target").unwrap();
            let chart = part(&mut zip, &format!("xl/{}", target.strip_prefix("../").unwrap()));
            assert_eq!(chart.path(&["pivotSource", "name"]).unwrap().text.split_once(']').unwrap().1, expected);
            let vals = chart.path(&["chart", "plotArea", "barChart", "ser", "val", "numLit"]).unwrap();
            assert_eq!(vals.kids("pt").map(|p| p.child("v").unwrap().text.as_str()).collect::<Vec<_>>(), ["12", "17"]);
        }
    }
}

#[test]
fn rotated_chart_groups_still_report_loss() {
    let (wb, report) = read_xlsx(&fixture(r#"rot="60000""#)).unwrap();
    assert!(wb.sheet(0).unwrap().charts.is_empty());
    assert!(report.warnings.iter().any(|w| w.contains("grouped shapes") || w.contains("grouped charts")));
}

#[test]
fn chart_group_xlsx_save_handles_out_of_range_line_metadata() {
    let (mut wb, _) = read_xlsx(&fixture("")).unwrap();
    let sheet = wb.sheet_mut(0).unwrap();
    // GridCraft JSON import can provide sparse line metadata without XLSX's index clipping.
    sheet.rows.insert(u32::MAX, Default::default());
    sheet.cols.insert(u32::MAX, Default::default());
    let bytes = write_xlsx(&wb).unwrap();
    let (reopened, _) = read_xlsx(&bytes).unwrap();
    assert_eq!(reopened.sheet(0).unwrap().chart_groups.len(), 1);
}

#[test]
fn chart_group_ids_do_not_collide_with_retained_drawing_ids() {
    let (wb, _) = read_xlsx(&fixture("")).unwrap();
    let sheet = wb.sheet(0).unwrap();
    let objects = sheet.charts.iter().enumerate().map(|(i, chart)| crate::drawing::Obj::Chart(chart, format!("rId{}", i + 1))).collect::<Vec<_>>();
    let anchors = [2, 4].map(|id| {
        format!(
            r#"<xdr:twoCellAnchor xmlns:xdr="{ANCHOR_NS}"><xdr:sp><xdr:nvSpPr><xdr:cNvPr id="{id}" name="Control {id}"/></xdr:nvSpPr></xdr:sp><xdr:clientData/></xdr:twoCellAnchor>"#
        )
    });
    let first_id = crate::passthrough::first_drawing_id(&anchors, objects.len() + sheet.chart_groups.len()).unwrap();
    let drawing = crate::drawing::write_drawing(sheet, &wb.theme, &objects, first_id, &anchors);
    let drawing = parse(drawing.as_bytes()).unwrap();
    let group = drawing.child("twoCellAnchor").unwrap().child("grpSp").unwrap();
    assert_eq!(group.path(&["nvGrpSpPr", "cNvPr"]).unwrap().attr("id"), Some("7"));
    let member_ids =
        group.kids("graphicFrame").map(|frame| frame.path(&["nvGraphicFramePr", "cNvPr"]).unwrap().attr("id").unwrap()).collect::<Vec<_>>();
    assert_eq!(member_ids, ["5", "6"]);
}
