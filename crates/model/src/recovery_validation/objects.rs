//! Stored object invariants only. No decoding, reference resolution, executable-policy walk,
//! equation rebuild, writer or layout. Actual screen/page projection remains an owner gate.
use std::collections::HashSet;

use super::features::validate_numeric_identities;
use super::{RecoveryModelError, RecoveryRepresentation, validate_cell_ref, validate_color};
use crate::{
    Anchor, AnchorMode, CellPicture, Chart, ChartDataTable, ChartGroup, ChartKind, Comment, ControlAction, ControlKind, FormControl, Hyperlink,
    Image, LegendPos, LineDash, MacroInput, NoteBox, NoteCorner, QuartileMethod, Series, Shape, ShapeFormat, ShapeKind, ShapeTextAlign,
    ShapeTextAnchor, Slicer, Sparkline, SparklineKind, Trendline, TrendlineKind, Workbook,
};

fn validate_anchor(anchor: &Anchor) -> Result<(), RecoveryModelError> {
    let Anchor { cell, dx, dy, width, height, mode } = anchor;
    validate_cell_ref(*cell)?;
    match mode {
        AnchorMode::MoveAndSize | AnchorMode::MoveOnly | AnchorMode::Absolute => {}
    }
    // Signed offsets stay exact. An unscaled dx+width is not the consumer's evaluation order.
    if !dx.is_finite() || !dy.is_finite() || !width.is_finite() || !height.is_finite() || *width < 0.0 || *height < 0.0 {
        return Err(RecoveryModelError::Geometry);
    }
    Ok(())
}

fn validate_chart_kind(kind: &ChartKind) {
    match kind {
        ChartKind::ColumnClustered
        | ChartKind::ColumnStacked
        | ChartKind::ColumnStacked100
        | ChartKind::BarClustered
        | ChartKind::BarStacked
        | ChartKind::BarStacked100
        | ChartKind::Line
        | ChartKind::LineMarkers
        | ChartKind::LineStacked
        | ChartKind::Pie
        | ChartKind::Doughnut
        | ChartKind::Area
        | ChartKind::AreaStacked
        | ChartKind::AreaStacked100
        | ChartKind::Scatter
        | ChartKind::ScatterLines
        | ChartKind::Bubble
        | ChartKind::Radar
        | ChartKind::Histogram
        | ChartKind::Waterfall
        | ChartKind::Funnel
        | ChartKind::Treemap
        | ChartKind::Sunburst
        | ChartKind::BoxWhisker
        | ChartKind::Stock
        | ChartKind::Combo => {}
    }
}

fn validate_trendline(trend: &Trendline) -> Result<(), RecoveryModelError> {
    let Trendline { kind, name: _, forward, backward, intercept, display_equation: _, display_r_squared: _, color, width, dash } = trend;
    match kind {
        TrendlineKind::Linear
        | TrendlineKind::Exponential
        | TrendlineKind::Logarithmic
        | TrendlineKind::Polynomial(_)
        | TrendlineKind::Power
        | TrendlineKind::MovingAverage(_) => {}
    }
    if !forward.is_finite() || !backward.is_finite() || intercept.is_some_and(|n| !n.is_finite()) {
        return Err(RecoveryModelError::Value);
    }
    if width.is_some_and(|n| !n.is_finite() || n < 0.0) {
        return Err(RecoveryModelError::Geometry);
    }
    if let Some(color) = color {
        validate_color(*color)?;
    }
    if let Some(dash) = dash {
        match dash {
            LineDash::Solid
            | LineDash::Dot
            | LineDash::Dash
            | LineDash::LgDash
            | LineDash::DashDot
            | LineDash::LgDashDot
            | LineDash::LgDashDotDot
            | LineDash::SysDash
            | LineDash::SysDot
            | LineDash::SysDashDot
            | LineDash::SysDashDotDot => {}
        }
    }
    Ok(())
}

fn validate_chart(chart: &Chart) -> Result<(), RecoveryModelError> {
    let Chart {
        id: _,
        kind,
        anchor,
        title: _,
        series,
        legend,
        data_labels: _,
        data_table,
        gridlines: _,
        style: _,
        hole_size: _,
        first_slice_angle: _,
        x_title: _,
        y_title: _,
        source: _,
        by_rows: _,
        pivot: _,
    } = chart;
    validate_anchor(anchor)?;
    validate_chart_kind(kind);
    match legend {
        LegendPos::None | LegendPos::Bottom | LegendPos::Top | LegendPos::Left | LegendPos::Right => {}
    }
    if let Some(table) = data_table {
        let ChartDataTable { show_keys: _, horizontal_border: _, vertical_border: _, outline: _ } = table;
    }
    for series in series {
        let Series { name: _, categories: _, values: _, bubble_sizes: _, color, secondary: _, kind, smooth: _, quartiles, trendlines } = series;
        if let Some(color) = color {
            validate_color(*color)?;
        }
        if let Some(kind) = kind {
            validate_chart_kind(kind);
        }
        match quartiles {
            QuartileMethod::Exclusive | QuartileMethod::Inclusive => {}
        }
        for trend in trendlines {
            validate_trendline(trend)?;
        }
    }
    Ok(())
}

fn validate_shape(shape: &Shape) -> Result<(), RecoveryModelError> {
    let Shape { id: _, kind, anchor, fill, line, text: _, format, math: _ } = shape;
    validate_anchor(anchor)?;
    match kind {
        ShapeKind::Rectangle
        | ShapeKind::RoundedRectangle
        | ShapeKind::Ellipse
        | ShapeKind::Triangle
        | ShapeKind::Line
        | ShapeKind::Arrow
        | ShapeKind::TextBox
        | ShapeKind::Icon
        | ShapeKind::Ink
        | ShapeKind::Equation => {}
    }
    validate_color(*fill)?;
    validate_color(*line)?;
    let ShapeFormat { no_line: _, font_size, bold: _, italic: _, font: _, text_color, text_outline, align, anchor, rtl: _ } = format;
    // Font size has a finite-only model domain, unlike an outline or anchor extent.
    if font_size.is_some_and(|n| !n.is_finite()) {
        return Err(RecoveryModelError::Style);
    }
    if let Some(color) = text_color {
        validate_color(*color)?;
    }
    if let Some((color, width)) = text_outline {
        validate_color(*color)?;
        if !width.is_finite() || *width < 0.0 {
            return Err(RecoveryModelError::Geometry);
        }
    }
    if let Some(align) = align {
        match align {
            ShapeTextAlign::Left | ShapeTextAlign::Center | ShapeTextAlign::Right => {}
        }
    }
    if let Some(anchor) = anchor {
        match anchor {
            ShapeTextAnchor::Top | ShapeTextAnchor::Middle | ShapeTextAnchor::Bottom => {}
        }
    }
    Ok(())
}

fn validate_control(control: &FormControl) -> Result<(), RecoveryModelError> {
    let FormControl { id: _, kind, anchor, name: _, text: _, link: _, value, min, max, step, page, horizontal: _, action, print: _, locked: _ } =
        control;
    validate_anchor(anchor)?;
    match kind {
        ControlKind::Button => {} // Dormant numbers are retained even when outside active domains.
        ControlKind::Slider | ControlKind::Spin => {
            // Scalar equivalent of check_numbers, without allocating document-bearing messages.
            let span = max.checked_sub(*min).ok_or(RecoveryModelError::Control)?;
            if *max > crate::CONTROL_MAX || *min > crate::CONTROL_MAX || span == 0 || *step == 0 || *step > span || *value < *min || *value > *max {
                return Err(RecoveryModelError::Control);
            }
            if *kind == ControlKind::Slider && (*page < *step || *page > crate::CONTROL_MAX) {
                return Err(RecoveryModelError::Control);
            }
        }
    }
    if let Some(action) = action {
        match action {
            ControlAction::Macro { name: _, input, output: _ } => match input {
                MacroInput::Selection | MacroInput::Fixed(_) => {}
            },
            // Engine budgets/trust/adoption precede any traversal, clone or serialization.
            ControlAction::Command { id: _, params: _ } => {}
        }
    }
    Ok(())
}

fn validate_comment(comment: &Comment) -> Result<(), RecoveryModelError> {
    let Comment { author: _, text: _, replies: _, threaded: _, resolved: _, visible: _, dates: _, note_box } = comment;
    if let Some(note_box) = note_box {
        let NoteBox { from, to, size_pt } = note_box;
        for corner in [from, to] {
            // Full signed offsets and raw pixels are retained. The writer saturates base+delta;
            // recovery neither invokes it nor projects relative corners to absolute coordinates.
            let NoteCorner { col: _, col_px: _, row: _, row_px: _ } = corner;
        }
        if size_pt.is_some_and(|(width, height)| !width.is_finite() || !height.is_finite() || width < 0.0 || height < 0.0) {
            return Err(RecoveryModelError::Geometry);
        }
    }
    Ok(())
}

/// Private B3 composition seam: objects only, preserving inert payloads and derived caches.
pub(super) fn validate_workbook_objects(workbook: &Workbook) -> Result<(), RecoveryModelError> {
    // One floating namespace across all sheets/kinds; tables/pivots are separate B2 namespaces.
    // The existing helper checks count/byte bounds and reserves fallibly before copying IDs.
    validate_numeric_identities(workbook.sheets.iter().flat_map(|sheet| {
        sheet
            .charts
            .iter()
            .map(|v| u64::from(v.id))
            .chain(sheet.images.iter().map(|v| u64::from(v.id)))
            .chain(sheet.shapes.iter().map(|v| u64::from(v.id)))
            .chain(sheet.slicers.iter().map(|v| u64::from(v.id)))
            .chain(sheet.controls.iter().map(|v| u64::from(v.id)))
    }))?;
    for sheet in &workbook.sheets {
        if !sheet.chart_groups.is_empty() {
            let mut ungrouped = HashSet::new();
            ungrouped.try_reserve(sheet.charts.len()).map_err(|_| RecoveryModelError::Capacity)?;
            ungrouped.extend(sheet.charts.iter().map(|chart| chart.id));
            for ChartGroup { name, chart_ids } in &sheet.chart_groups {
                if name.len() > ChartGroup::MAX_NAME_BYTES
                    || !(2..=ChartGroup::MAX_MEMBERS).contains(&chart_ids.len())
                    || !chart_ids.iter().all(|id| ungrouped.remove(id))
                {
                    return Err(RecoveryModelError::UnsupportedRepresentation(RecoveryRepresentation::AmbiguousIdentity));
                }
            }
        }
        for chart in &sheet.charts {
            validate_chart(chart)?;
        }
        for image in &sheet.images {
            let Image { id: _, anchor, data: _, mime: _, alt: _ } = image;
            validate_anchor(anchor)?;
        }
        for (cell, picture) in &sheet.cell_pictures {
            validate_cell_ref(*cell)?;
            let CellPicture { data: _, mime: _, alt: _ } = picture.as_ref();
        }
        for shape in &sheet.shapes {
            validate_shape(shape)?;
        }
        for sparkline in &sheet.sparklines {
            let Sparkline { cell, source: _, kind, color, markers: _ } = sparkline;
            validate_cell_ref(*cell)?;
            validate_color(*color)?;
            match kind {
                SparklineKind::Line | SparklineKind::Column | SparklineKind::WinLoss => {}
            }
        }
        for slicer in &sheet.slicers {
            let Slicer { id: _, table: _, column: _, caption: _, anchor, selected: _, columns: _ } = slicer;
            validate_anchor(anchor)?;
        }
        for control in &sheet.controls {
            validate_control(control)?;
        }
        for (cell, comment) in sheet.comments.iter() {
            validate_cell_ref(*cell)?;
            validate_comment(comment)?;
        }
        for (cell, hyperlink) in &sheet.hyperlinks {
            validate_cell_ref(*cell)?;
            let Hyperlink { target: _, tooltip: _, external: _ } = hyperlink;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "objects_tests.rs"]
mod tests;
