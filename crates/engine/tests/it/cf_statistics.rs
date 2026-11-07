//! Complete effective-cell statistics. Overlapping Applies To areas are treated as a union
//! (the reviewed public-spec interpretation); these synthetic fixtures are not an Excel oracle.

use gridcraft_engine::{
    Session,
    cf::CfCache,
    core::{CellError, CellRef, RangeRef, Value},
    model::{Cell, CfRule, CfValueKind, Color, CondFormat, Formula, Style, Workbook},
};
use serde_json::json;

fn cell(a1: &str) -> CellRef {
    CellRef::parse(a1).unwrap()
}

fn range(a1: &str) -> RangeRef {
    RangeRef::parse(a1).unwrap()
}

fn add_rule(wb: &mut Workbook, ranges: &[&str], rule: CfRule) {
    wb.sheet_mut(0).unwrap().cond_formats.push(CondFormat {
        ranges: ranges.iter().map(|a1| range(a1)).collect(),
        rule,
        priority: 1,
        stop_if_true: false,
    });
}

fn scale() -> CfRule {
    CfRule::ColorScale { stops: vec![(CfValueKind::Min, Color::rgb(0, 0, 0)), (CfValueKind::Max, Color::rgb(200, 200, 200))] }
}

#[test]
fn far_tail_maximum_is_included() {
    let mut wb = Workbook::new();
    let sh = wb.sheet_mut(0).unwrap();
    for (a1, n) in [("A1", 0.0), ("A2", 50.0), ("C1000000", 100.0)] {
        sh.set_value(cell(a1), Value::Number(n));
    }
    add_rule(&mut wb, &["A1:C1000000"], scale());
    let mut cache = CfCache::new();
    // The interior shade proves that the late maximum sets the threshold, not just that
    // the late cell's own colour happens to clamp to the upper endpoint.
    assert_eq!(cache.look(&wb, 0, cell("A2")).and_then(|l| l.scale_fill), Some([100, 100, 100]));
    assert_eq!(cache.look(&wb, 0, cell("C1000000")).and_then(|l| l.scale_fill), Some([200, 200, 200]));
}

#[test]
fn nonfinite_cached_numbers_never_produce_conditional_geometry() {
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut wb = Workbook::new();
        let sh = wb.sheet_mut(0).unwrap();
        sh.set_value(cell("A1"), Value::Number(1.0));
        // Cached model values can be constructed without Value::number's finite guard.
        sh.set_value(cell("A2"), Value::Number(invalid));
        add_rule(
            &mut wb,
            &["A1:A2"],
            CfRule::DataBar { min: CfValueKind::Min, max: CfValueKind::Max, color: Color::rgb(0, 0, 0), gradient: false, show_value: true },
        );
        assert!(CfCache::new().look(&wb, 0, cell("A2")).is_none(), "{invalid}");
    }
}

#[test]
fn duplicate_after_cutoff_is_counted() {
    let mut wb = Workbook::new();
    let sh = wb.sheet_mut(0).unwrap();
    for (a1, text) in [("A1", "repeat"), ("A2", "different"), ("C1000000", "REPEAT")] {
        sh.set_value(cell(a1), Value::from(text));
    }
    add_rule(&mut wb, &["A1:C1000000"], CfRule::Duplicate { style: Box::default() });
    let mut cache = CfCache::new();
    for (a1, expected) in [("A1", true), ("A2", false), ("C1000000", true)] {
        assert_eq!(cache.look(&wb, 0, cell(a1)).and_then(|l| l.style).is_some(), expected, "{a1}");
    }
}

#[test]
fn excessive_statistics_ranges_are_not_used_as_valid_thresholds() {
    let mut wb = Workbook::new();
    wb.sheet_mut(0).unwrap().set_value(cell("A1"), Value::Number(1.0));
    add_rule(&mut wb, &vec!["A1"; 4097], scale());
    assert!(CfCache::new().look(&wb, 0, cell("A1")).is_none());
}

#[test]
fn repeated_and_overlapping_ranges_do_not_make_a_value_duplicate() {
    for ranges in [["A1:A2", "A1:A2"], ["A1:A2", "A2:A3"]] {
        let mut wb = Workbook::new();
        let sh = wb.sheet_mut(0).unwrap();
        for (a1, text) in [("A1", "first"), ("A2", "only once"), ("A3", "third")] {
            sh.set_value(cell(a1), Value::from(text));
        }
        add_rule(&mut wb, &ranges, CfRule::Duplicate { style: Box::default() });
        assert!(CfCache::new().look(&wb, 0, cell("A2")).is_none(), "ranges {ranges:?}");
    }
}

#[test]
fn overlapping_ranges_do_not_weight_the_average() {
    let mut wb = Workbook::new();
    let sh = wb.sheet_mut(0).unwrap();
    for (a1, n) in [("A1", 0.0), ("A2", 5.0), ("A3", 6.0)] {
        sh.set_value(cell(a1), Value::Number(n));
    }
    // Union mean is 11/3, so 5 is above average. Repeating the 6 ten times gives 65/12,
    // incorrectly excluding 5. No coordinate should acquire weight from Applies To spelling.
    let mut ranges = vec!["A1:A3"];
    ranges.extend(["A3"; 10]);
    add_rule(&mut wb, &ranges, CfRule::AboveAverage { below: false, equal: false, std_dev: 0, style: Box::default() });
    assert!(CfCache::new().look(&wb, 0, cell("A2")).and_then(|l| l.style).is_some());
}

#[test]
fn numeric_statistics_ignore_text_booleans_errors_and_blanks() {
    let mut wb = Workbook::new();
    let sh = wb.sheet_mut(0).unwrap();
    for (row, value) in [
        Value::Number(0.0),
        Value::Number(10.0),
        Value::Number(20.0),
        Value::from("1000"),
        Value::Bool(true),
        Value::Error(CellError::NA),
        Value::Empty,
        Value::from(""),
        Value::from(" "),
    ]
    .into_iter()
    .enumerate()
    {
        sh.set_value(CellRef::new(row as u32, 0), value);
    }
    add_rule(&mut wb, &["A1:A9"], scale());
    let mut cache = CfCache::new();
    assert_eq!(cache.look(&wb, 0, cell("A2")).and_then(|l| l.scale_fill), Some([100, 100, 100]));
    for row in 3..9 {
        assert!(cache.look(&wb, 0, CellRef::new(row, 0)).is_none());
    }
}

#[test]
fn duplicate_statistics_preserve_nonempty_lowercase_display_comparison() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute(
        "range.setValues",
        json!({"range": "A1", "values": [[1], ["'1"], [true], ["'TRUE"], ["=NA()"], ["'#N/A"], ["=\"\""], ["=\"\""], [null], [" "], [" "], ["x"], ["ΟΣ"], ["ος"]]}),
    )
    .unwrap();
    s.execute("home.conditionalFormat", json!({"range":"A1:A14", "rule":{"type":"duplicate"}})).unwrap();
    let mut cache = CfCache::new();
    let wb = &s.doc().unwrap().wb;
    // Whole-string lowercasing handles final Greek sigma context; per-character folding does not.
    for (row, expected) in [true, true, true, true, true, true, true, true, false, true, true, false, true, true].into_iter().enumerate() {
        assert_eq!(cache.look(wb, 0, CellRef::new(row as u32, 0)).and_then(|l| l.style).is_some(), expected, "row {}", row + 1);
    }
}

#[test]
fn spill_children_contribute_when_the_anchor_is_outside_the_rule() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("cell.set", json!({"cell":"A1", "input":"=SEQUENCE(4)"})).unwrap();
    s.execute("home.conditionalFormat", json!({"range":"A2:A4", "rule":{"type":"top10", "rank":1}})).unwrap();
    let mut cache = CfCache::new();
    let wb = &s.doc().unwrap().wb;
    assert!(cache.look(wb, 0, cell("A2")).is_none());
    assert!(cache.look(wb, 0, cell("A4")).and_then(|l| l.style).is_some());
}

#[test]
fn stored_and_spilled_values_use_the_existing_effective_value_precedence() {
    let mut wb = Workbook::new();
    let mut style = Style::default();
    style.font.bold = true;
    let style = wb.styles.intern(style);
    let sh = wb.sheet_mut(0).unwrap();
    sh.set_value(cell("A1"), Value::Number(0.0));
    sh.set_cell(cell("A2"), Cell::styled(Value::Empty, style));
    sh.set_value(cell("A3"), Value::Number(20.0));
    sh.set_cell(cell("A4"), Cell::formula(Formula::new("=\"\"")));
    for (a1, value) in [("A2", 10.0), ("A3", 999.0), ("A4", 999.0)] {
        sh.spill.insert(cell(a1), Value::Number(value));
    }
    sh.spill_ranges.insert(cell("A1"), range("A1:A4"));
    add_rule(&mut wb, &["A1:A4"], scale());
    let mut cache = CfCache::new();
    assert_eq!(cache.look(&wb, 0, cell("A2")).and_then(|l| l.scale_fill), Some([100, 100, 100]));
    assert_eq!(cache.look(&wb, 0, cell("A3")).and_then(|l| l.scale_fill), Some([200, 200, 200]));
    assert!(cache.look(&wb, 0, cell("A4")).is_none());
}
