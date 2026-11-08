//! Feature/region leaves only; no evaluation, source resolution, cache building or writer work.
use super::structure::{validate_ascii_identities, validate_disjoint_rectangles};
use super::{RecoveryModelError, RecoveryRepresentation, validate_cell_ref, validate_color, validate_range, validate_style};
use crate::{
    AutoFilter, CfRule, CfValueKind, CondFormat, FilterCriterion, PivotCalcField, PivotField, PivotFilter, PivotTable, PivotValue, Table,
    TableColumn, Validation, WhatIfTable, Workbook, XmlColumnPr,
};

// Copy only bounded numeric keys; every namespace invokes this independently. No owned strings.
pub(super) fn validate_numeric_identities(entries: impl Clone + Iterator<Item = u64>) -> Result<(), RecoveryModelError> {
    let count = entries.clone().try_fold(0usize, |n, _| n.checked_add(1).ok_or(RecoveryModelError::Capacity))?;
    if count.checked_mul(std::mem::size_of::<u64>()).is_none_or(|bytes| bytes > isize::MAX as usize) {
        return Err(RecoveryModelError::Capacity);
    }
    let mut keys = Vec::new();
    keys.try_reserve_exact(count).map_err(|_| RecoveryModelError::Capacity)?;
    keys.extend(entries);
    keys.sort_unstable();
    if keys.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(RecoveryModelError::UnsupportedRepresentation(RecoveryRepresentation::AmbiguousIdentity));
    }
    Ok(())
}

fn validate_filters(criteria: &[(u32, FilterCriterion)], width: u32) -> Result<(), RecoveryModelError> {
    for (offset, criterion) in criteria {
        if *offset >= width {
            return Err(RecoveryModelError::Filter);
        }
        match criterion {
            FilterCriterion::FillColor(color) | FilterCriterion::FontColor(color) => validate_color(*color)?,
            FilterCriterion::Values { values: _, blanks: _ }
            | FilterCriterion::Custom { a: _, b: _, and: _ }
            | FilterCriterion::Top10 { bottom: _, percent: _, count: _ }
            | FilterCriterion::AboveAverage(_)
            | FilterCriterion::Icon { set: _, id: _ } => {} // Bounded integers and inert imported strings stay exact.
        }
    }
    validate_numeric_identities(criteria.iter().map(|(offset, _)| u64::from(*offset)))
}

fn validate_table(table: &Table) -> Result<(), RecoveryModelError> {
    let Table {
        id: _,
        name: _,
        range,
        // A one-row header+totals table is safe retained metadata; data_range may be None.
        header_row: _,
        totals_row: _,
        columns,
        style: _,
        banded_rows: _,
        banded_cols: _,
        first_col: _,
        last_col: _,
        filter_button: _,
        filter,
        table_type: _,
        connection_id: _,
    } = table;
    validate_range(*range)?; // Must precede width arithmetic and any range helper.
    let width = range.end.col.checked_sub(range.start.col).and_then(|n| n.checked_add(1)).ok_or(RecoveryModelError::Range)?;
    if usize::try_from(width).map_err(|_| RecoveryModelError::Capacity)? != columns.len() {
        return Err(RecoveryModelError::Table);
    }
    for column in columns {
        let TableColumn { name: _, totals, totals_label: _, formula: _, unique_name: _, xml } = column;
        match totals {
            crate::TotalsFn::None
            | crate::TotalsFn::Average
            | crate::TotalsFn::Count
            | crate::TotalsFn::CountNums
            | crate::TotalsFn::Max
            | crate::TotalsFn::Min
            | crate::TotalsFn::Sum
            | crate::TotalsFn::StdDev
            | crate::TotalsFn::Var
            | crate::TotalsFn::Custom => {}
        }
        if let Some(xml) = xml {
            let XmlColumnPr { map_id: _, xpath: _, data_type: _, denormalized: _ } = xml;
        }
    }
    validate_ascii_identities(columns.iter().map(|column| (None, column.name.as_str())))?;
    validate_filters(filter, width)
}

fn validate_what_if(table: &WhatIfTable) -> Result<(), RecoveryModelError> {
    let WhatIfTable {
        anchor,
        range,
        input1,
        input2,
        formula_text: _,
        two_variable: _,
        row_oriented: _,
        input1_deleted: _,
        input2_deleted: _,
        calculate: _,
    } = table;
    validate_cell_ref(*anchor)?;
    validate_range(*range)?;
    for input in [input1, input2].into_iter().flatten() {
        validate_cell_ref(*input)?;
    }
    if *anchor != range.start {
        return Err(RecoveryModelError::WhatIf);
    }
    Ok(())
}

fn validate_cf_value(value: &CfValueKind) -> Result<(), RecoveryModelError> {
    match value {
        CfValueKind::Number(n) | CfValueKind::Percent(n) | CfValueKind::Percentile(n) => {
            if !n.is_finite() {
                return Err(RecoveryModelError::Value);
            }
        }
        CfValueKind::Min | CfValueKind::Max | CfValueKind::Formula(_) => {}
    }
    Ok(())
}

fn validate_operator(op: &crate::CfOperator) {
    match op {
        crate::CfOperator::Between
        | crate::CfOperator::NotBetween
        | crate::CfOperator::Equal
        | crate::CfOperator::NotEqual
        | crate::CfOperator::Greater
        | crate::CfOperator::Less
        | crate::CfOperator::GreaterOrEqual
        | crate::CfOperator::LessOrEqual => {}
    }
}

fn validate_cf(format: &CondFormat) -> Result<(), RecoveryModelError> {
    let CondFormat { ranges, rule, priority: _, stop_if_true: _ } = format;
    for range in ranges {
        validate_range(*range)?;
    }
    match rule {
        CfRule::CellIs { op, a: _, b: _, style } => {
            validate_operator(op);
            validate_style(style)?;
        }
        CfRule::Expression { formula: _, style }
        | CfRule::ContainsText { text: _, style }
        | CfRule::NotContainsText { text: _, style }
        | CfRule::BeginsWith { text: _, style }
        | CfRule::EndsWith { text: _, style }
        | CfRule::Blanks { style }
        | CfRule::NoBlanks { style }
        | CfRule::Errors { style }
        | CfRule::NoErrors { style }
        | CfRule::Duplicate { style }
        | CfRule::Unique { style }
        | CfRule::Top10 { bottom: _, percent: _, rank: _, style }
        | CfRule::AboveAverage { below: _, equal: _, std_dev: _, style }
        | CfRule::TimePeriod { period: _, style } => validate_style(style)?,
        CfRule::ColorScale { stops } => {
            for (value, color) in stops {
                validate_cf_value(value)?;
                validate_color(*color)?;
            }
        }
        CfRule::DataBar { min, max, color, gradient: _, show_value: _ } => {
            validate_cf_value(min)?;
            validate_cf_value(max)?;
            validate_color(*color)?;
        }
        CfRule::IconSet { set: _, thresholds, reverse: _, show_value: _ } => {
            for value in thresholds {
                validate_cf_value(value)?;
            }
        }
    }
    Ok(())
}

fn validate_validation(validation: &Validation) -> Result<(), RecoveryModelError> {
    let Validation {
        ranges,
        kind,
        op,
        f1: _,
        f2: _,
        allow_blank: _,
        in_cell_dropdown: _,
        input_title: _,
        input_message: _,
        show_input: _,
        error_title: _,
        error_message: _,
        error_style,
        show_error: _,
    } = validation;
    for range in ranges {
        validate_range(*range)?;
    }
    match kind {
        crate::ValidationKind::Any
        | crate::ValidationKind::Whole
        | crate::ValidationKind::Decimal
        | crate::ValidationKind::List
        | crate::ValidationKind::Date
        | crate::ValidationKind::Time
        | crate::ValidationKind::TextLength
        | crate::ValidationKind::Custom => {}
    }
    validate_operator(op);
    match error_style {
        crate::ErrorStyle::Stop | crate::ErrorStyle::Warning | crate::ErrorStyle::Information => {}
    }
    Ok(())
}

fn validate_pivot_field(field: &PivotField) {
    let PivotField { source_col: _, sort, date_group, collapsed_items: _ } = field;
    match sort {
        crate::PivotSort::Asc | crate::PivotSort::Desc | crate::PivotSort::None => {}
    }
    match date_group {
        crate::PivotDateGroup::None
        | crate::PivotDateGroup::Years
        | crate::PivotDateGroup::Quarters
        | crate::PivotDateGroup::Months
        | crate::PivotDateGroup::Days => {}
    }
}

fn validate_pivot(pivot: &PivotTable) -> Result<(), RecoveryModelError> {
    let PivotTable {
        id: _,
        name: _,
        source: _,
        anchor,
        rows,
        columns,
        values,
        filters,
        layout,
        enable_show_details: _,
        grand_totals_rows: _,
        grand_totals_cols: _,
        subtotals: _,
        subtotals_top: _,
        style: _,
        show_headers: _,
        last_range,
        calculated_fields,
        values_on_rows,
        values_position,
    } = pivot;
    validate_cell_ref(*anchor)?;
    if let Some(range) = last_range {
        validate_range(*range)?;
    }
    if *values_on_rows && values_position.is_some_and(|position| position > rows.len()) {
        return Err(RecoveryModelError::Pivot);
    }
    match layout {
        crate::PivotLayout::Compact | crate::PivotLayout::Outline | crate::PivotLayout::Tabular => {}
    }
    for field in rows.iter().chain(columns) {
        validate_pivot_field(field);
    }
    for value in values {
        let PivotValue { source_col: _, func, name: _, show_as, number_format: _ } = value;
        match func {
            crate::PivotFunc::Sum
            | crate::PivotFunc::Count
            | crate::PivotFunc::Average
            | crate::PivotFunc::Max
            | crate::PivotFunc::Min
            | crate::PivotFunc::Product
            | crate::PivotFunc::CountNumbers
            | crate::PivotFunc::StdDev
            | crate::PivotFunc::StdDevP
            | crate::PivotFunc::Var
            | crate::PivotFunc::VarP => {}
        }
        match show_as {
            crate::PivotShowAs::Normal
            | crate::PivotShowAs::PercentOfGrandTotal
            | crate::PivotShowAs::PercentOfColumnTotal
            | crate::PivotShowAs::PercentOfRowTotal
            | crate::PivotShowAs::RunningTotal
            | crate::PivotShowAs::Rank => {}
        }
    }
    for filter in filters {
        let PivotFilter { source_col: _, selected: _ } = filter;
    }
    for field in calculated_fields {
        let PivotCalcField { name: _, formula: _ } = field;
    }
    // CalcSet::new resolves trimmed ASCII names; preserve original spelling and formulas.
    validate_ascii_identities(calculated_fields.iter().map(|field| (None, field.name.trim())))
}

/// B2 feature/region scope only. Does not validate the structural envelope or other families.
pub(super) fn validate_workbook_features(workbook: &Workbook) -> Result<(), RecoveryModelError> {
    // Table ids are inert: no numeric lookup; import/writer allocate their own ids.
    validate_ascii_identities(workbook.sheets.iter().flat_map(|sheet| sheet.tables.iter().map(|table| (None, table.name.as_str()))))?;
    validate_ascii_identities(workbook.sheets.iter().flat_map(|sheet| sheet.pivots.iter().map(|pivot| (None, pivot.name.as_str()))))?;
    validate_numeric_identities(workbook.sheets.iter().flat_map(|sheet| sheet.pivots.iter().map(|pivot| u64::from(pivot.id))))?;
    for sheet in &workbook.sheets {
        for table in &sheet.tables {
            validate_table(table)?;
        }
        // Table and merge ownership are independent; never combine the two sweeps.
        validate_disjoint_rectangles(sheet.tables.iter().map(|table| table.range))?;
        if let Some(AutoFilter { range, criteria }) = &sheet.autofilter {
            validate_range(*range)?;
            let width = range.end.col.checked_sub(range.start.col).and_then(|n| n.checked_add(1)).ok_or(RecoveryModelError::Range)?;
            validate_filters(criteria, width)?;
        }
        for table in &sheet.what_if_tables {
            validate_what_if(table)?;
        }
        // Coordinates are checked before encoding the exact (row,col) anchor as a u64 key.
        validate_numeric_identities(sheet.what_if_tables.iter().map(|table| (u64::from(table.anchor.row) << 32) | u64::from(table.anchor.col)))?;
        for format in &sheet.cond_formats {
            validate_cf(format)?;
        }
        for validation in &sheet.validations {
            validate_validation(validation)?;
        }
        for pivot in &sheet.pivots {
            validate_pivot(pivot)?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "features_tests.rs"]
mod tests;
