//! Native groups containing only ordinary, axis-aligned chart frames.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use gridcraft_core::{CellRef, MAX_COLS, MAX_ROWS};
use gridcraft_model::{Anchor, Chart, ChartGroup, LineIndex, Sheet};

use crate::tables::EMU_PER_PX;
use crate::xml::{El, esc_attr};
use crate::{IoError, read::Ctx};

use super::{DrawingSource, Obj, anchor_open, has_drawing_links, read_chart_frame, write_chart_frame};

const CHART_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";

pub(super) struct Grid {
    cols: LineIndex,
    rows: LineIndex,
}

impl Grid {
    fn new(sheet: &Sheet) -> Self {
        Self {
            cols: LineIndex::new(&sheet.cols, sheet.default_col_width, MAX_COLS),
            rows: LineIndex::new(&sheet.rows, sheet.default_row_height, MAX_ROWS),
        }
    }

    fn rect(&self, anchor: &Anchor) -> Option<Rect> {
        if !anchor.cell.is_valid() {
            return None;
        }
        let rect = Rect {
            x: self.cols.start(anchor.cell.col) + f64::from(anchor.dx),
            y: self.rows.start(anchor.cell.row) + f64::from(anchor.dy),
            w: f64::from(anchor.width),
            h: f64::from(anchor.height),
        };
        rect.valid().then_some(rect)
    }

    fn anchor(&self, rect: Rect, parent: &Anchor) -> Option<Anchor> {
        if !rect.valid() || rect.x + rect.w > self.cols.start(MAX_COLS) || rect.y + rect.h > self.rows.start(MAX_ROWS) {
            return None;
        }
        let col = self.cols.at(rect.x);
        let row = self.rows.at(rect.y);
        let anchor = Anchor {
            cell: CellRef::new(row, col),
            dx: (rect.x - self.cols.start(col)).max(0.0) as f32,
            dy: (rect.y - self.rows.start(row)).max(0.0) as f32,
            width: rect.w as f32,
            height: rect.h as f32,
            mode: parent.mode,
        };
        [anchor.dx, anchor.dy, anchor.width, anchor.height].iter().all(|x| x.is_finite()).then_some(anchor)
    }
}

#[derive(Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn valid(self) -> bool {
        [self.x, self.y, self.w, self.h].iter().all(|x| x.is_finite()) && self.x >= 0.0 && self.y >= 0.0 && self.w > 0.0 && self.h > 0.0
    }

    fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self { x, y, w: (self.x + self.w).max(other.x + other.w) - x, h: (self.y + self.h).max(other.y + other.h) - y }
    }
}

fn axis_aligned(transform: &El) -> bool {
    transform.attr("rot").is_none_or(|v| v.trim().parse::<i32>() == Ok(0))
        && ["flipH", "flipV"].iter().all(|key| transform.attr(key).is_none() || transform.attr_bool(key) == Some(false))
}

fn transform_rect(transform: &El, offset: &str, extent: &str) -> Option<Rect> {
    let offset = transform.child(offset)?;
    let extent = transform.child(extent)?;
    Some(Rect {
        x: offset.attr_i64("x")? as f64,
        y: offset.attr_i64("y")? as f64,
        w: extent.attr_i64("cx")? as f64,
        h: extent.attr_i64("cy")? as f64,
    })
}

fn chart_frames<'a>(group: &'a El, parent: &Anchor, grid: &Grid) -> Option<Vec<(&'a El, Anchor)>> {
    let props = group.child("grpSpPr").filter(|p| p.children.len() == 1)?;
    let transform = props.child("xfrm").filter(|t| axis_aligned(t))?;
    let children = transform_rect(transform, "chOff", "chExt")?;
    let outer = grid.rect(parent)?;
    if children.w <= 0.0 || children.h <= 0.0 {
        return None;
    }
    let sx = outer.w / children.w;
    let sy = outer.h / children.h;
    let mut frames = Vec::new();
    for obj in &group.children {
        match obj.name.as_str() {
            "nvGrpSpPr" | "grpSpPr" => continue,
            "graphicFrame" if frames.len() < ChartGroup::MAX_MEMBERS => {}
            _ => return None,
        }
        let graphic = obj.path(&["graphic", "graphicData"])?;
        if graphic.attr("uri") != Some(CHART_NS) || graphic.child("chart").and_then(|c| c.attr("id")).is_none() {
            return None;
        }
        let t = obj.child("xfrm").filter(|t| axis_aligned(t))?;
        let r = transform_rect(t, "off", "ext")?;
        let target = Rect { x: outer.x + (r.x - children.x) * sx, y: outer.y + (r.y - children.y) * sy, w: r.w * sx, h: r.h * sy };
        frames.push((obj, grid.anchor(target, parent)?));
    }
    (frames.len() >= 2).then_some(frames)
}

pub(super) fn read_group(
    cx: &mut Ctx<'_>,
    source: &DrawingSource<'_>,
    n: usize,
    group: &El,
    anchor: Anchor,
    sheet: &mut Sheet,
    grid: &mut Option<Grid>,
) -> Result<bool, IoError> {
    let part = source.part;
    if !group.children.iter().any(|child| child.name == "graphicFrame") {
        cx.warn(format!("grouped shapes are not supported and were skipped ({part}, object {n})"));
        return Ok(false);
    }
    let name = group.path(&["nvGrpSpPr", "cNvPr"]).and_then(|e| e.attr("name")).unwrap_or("Chart Group");
    let mut has_hyperlink = has_drawing_links(group);
    let frames = (name.len() <= ChartGroup::MAX_NAME_BYTES)
        .then(|| grid.get_or_insert_with(|| Grid::new(sheet)))
        .and_then(|grid| chart_frames(group, &anchor, grid));
    let Some(frames) = frames else {
        cx.warn(format!("grouped charts with unsupported members or transforms were skipped ({part}, object {n})"));
        return Ok(has_hyperlink);
    };
    let mut chart_ids = Vec::with_capacity(frames.len());
    let count = frames.len();
    for (frame, child_anchor) in frames {
        has_hyperlink |= has_drawing_links(frame);
        if let Some(chart) = read_chart_frame(cx, source, n, frame, child_anchor)? {
            chart_ids.push(chart.id);
            sheet.charts.push(chart);
        }
    }
    if chart_ids.len() == count {
        sheet.chart_groups.push(ChartGroup { name: name.into(), chart_ids });
    } else {
        cx.warn(format!("grouped charts could not all be read; grouping is not kept on save ({part}, object {n})"));
    }
    Ok(has_hyperlink)
}

struct Member {
    index: usize,
    rect: Rect,
}

pub(super) struct GroupOut<'a> {
    ordinal: usize,
    name: &'a str,
    anchor: Anchor,
    rect: Rect,
    members: Vec<Member>,
}

pub(crate) fn has_write_loss(sheet: &Sheet) -> bool {
    let (groups, _) = groups_for_write(sheet, sheet.charts.iter().enumerate());
    groups.len() != sheet.chart_groups.len()
}

pub(super) fn groups_for_write<'a>(
    sheet: &'a Sheet,
    charts: impl Iterator<Item = (usize, &'a Chart)>,
) -> (HashMap<usize, GroupOut<'a>>, HashSet<usize>) {
    if sheet.chart_groups.is_empty() {
        return (HashMap::new(), HashSet::new());
    }
    let grid = Grid::new(sheet);
    let charts: HashMap<_, _> =
        charts.filter(|(_, chart)| !crate::chartex::is_chartex(chart.kind)).map(|(index, chart)| (chart.id, (index, &chart.anchor))).collect();
    let mut groups = HashMap::new();
    let mut grouped = HashSet::new();
    for (i, group) in sheet.chart_groups.iter().enumerate() {
        if !(2..=ChartGroup::MAX_MEMBERS).contains(&group.chart_ids.len()) || group.name.len() > ChartGroup::MAX_NAME_BYTES {
            continue;
        }
        let mut unique = HashSet::new();
        let members: Vec<_> = group
            .chart_ids
            .iter()
            .filter_map(|id| {
                let (index, anchor) = *charts.get(id)?;
                let rect = grid.rect(anchor)?;
                (!grouped.contains(&index) && unique.insert(index)).then_some((Member { index, rect }, anchor))
            })
            .collect();
        if members.len() != group.chart_ids.len() {
            continue;
        }
        let Some((_, first_anchor)) = members.first() else { continue };
        if members.iter().any(|(_, anchor)| anchor.mode != first_anchor.mode) {
            continue;
        }
        let Some(rect) = members.iter().map(|(member, _)| member.rect).reduce(Rect::union) else { continue };
        let Some(anchor) = grid.anchor(rect, first_anchor) else { continue };
        let Some(first) = members.iter().map(|(member, _)| member.index).min() else { continue };
        grouped.extend(unique);
        groups.insert(
            first,
            GroupOut { ordinal: i, name: &group.name, anchor, rect, members: members.into_iter().map(|(member, _)| member).collect() },
        );
    }
    (groups, grouped)
}

pub(super) fn write_group(s: &mut String, sheet: &Sheet, group: &GroupOut<'_>, objs: &[Obj<'_>], first_id: usize) {
    let (open, close) = anchor_open(sheet, &group.anchor);
    s.push_str(&open);
    let emu = |v: f64| (v * EMU_PER_PX).round() as i64;
    let id = objs.len() + group.ordinal + first_id;
    let x = emu(group.rect.x);
    let y = emu(group.rect.y);
    let w = emu(group.rect.w).max(1);
    let h = emu(group.rect.h).max(1);
    let _ = write!(
        s,
        "<xdr:grpSp><xdr:nvGrpSpPr><xdr:cNvPr id=\"{id}\" name=\"{}\"/><xdr:cNvGrpSpPr/></xdr:nvGrpSpPr><xdr:grpSpPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{w}\" cy=\"{h}\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"{w}\" cy=\"{h}\"/></a:xfrm></xdr:grpSpPr>",
        esc_attr(group.name),
    );
    for member in &group.members {
        let Some(Obj::Chart(_, rid)) = objs.get(member.index) else { continue };
        let Rect { x, y, w, h } = member.rect;
        let transform = format!(
            "<xdr:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></xdr:xfrm>",
            emu(x - group.rect.x),
            emu(y - group.rect.y),
            emu(w).max(1),
            emu(h).max(1),
        );
        write_chart_frame(s, member.index + first_id, rid, &transform);
    }
    s.push_str("</xdr:grpSp><xdr:clientData/>");
    s.push_str(close);
}
