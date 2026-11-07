//! Public-command checks against a clean calculation, with independent expected values.

use std::collections::BTreeSet;

use gridcraft_core::{CellError, CellRef, Value};
use gridcraft_engine::{Session, calc::Calc};
use serde_json::{Value as Json, json};

fn session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s.doc_mut().unwrap().calc.fixed_now = Some(46_333.5);
    s
}

fn value(s: &Session, cell: &str) -> Value {
    s.doc().unwrap().wb.active().unwrap().value(CellRef::parse(cell).unwrap())
}

fn assert_clean(s: &Session, action: &str) {
    let wb = &s.doc().unwrap().wb;
    let mut fresh = (**wb).clone();
    let mut calc = Calc::new();
    calc.fixed_now = Some(46_333.5);
    calc.recalc_all(&mut fresh);
    assert_eq!(wb.sheets.len(), fresh.sheets.len(), "{action}");
    for (actual, clean) in wb.sheets.iter().zip(&fresh.sheets) {
        let cells: BTreeSet<_> = actual
            .cells
            .iter()
            .map(|(c, _)| c)
            .chain(clean.cells.iter().map(|(c, _)| c))
            .chain(actual.spill.keys().copied())
            .chain(clean.spill.keys().copied())
            .collect();
        for c in cells {
            assert_eq!(actual.value(c), clean.value(c), "{action}: {}!{c}", actual.name);
            let formula = |sh: &gridcraft_engine::model::Sheet| sh.cell(c).and_then(|cell| cell.formula.clone());
            assert_eq!(formula(actual), formula(clean), "{action}: formula and evaluation mode at {}!{c}", actual.name);
        }
        assert_eq!(actual.spill, clean.spill, "{action}: spilled values in {}", actual.name);
        assert_eq!(actual.spill_ranges, clean.spill_ranges, "{action}: spill ownership in {}", actual.name);
    }
}

fn execute(s: &mut Session, command: &str, params: Json) {
    s.execute(command, params.clone()).unwrap();
    assert_clean(s, &format!("{command} {params}"));
}

fn set(s: &mut Session, cell: &str, input: &str) {
    execute(s, "cell.set", json!({"cell": cell, "input": input}));
}

#[test]
fn edits_structure_and_history_match_clean_calculation() {
    let mut s = session();
    for (cell, input) in [("A1", "2"), ("C1", "=SEQUENCE(A1)"), ("E1", "=SUM(C1#)"), ("F1", "=C2*2")] {
        set(&mut s, cell, input);
    }
    set(&mut s, "A1", "3");
    assert_eq!(value(&s, "E1"), Value::Number(6.0));
    set(&mut s, "A1", "1");
    assert_eq!(value(&s, "E1"), Value::Number(1.0));
    assert_eq!(value(&s, "F1"), Value::Number(0.0));
    set(&mut s, "A1", "3");
    set(&mut s, "C2", "block");
    assert_eq!(value(&s, "C1"), Value::Error(CellError::Spill));
    execute(&mut s, "edit.clearContents", json!({"range": "C2"}));
    assert_eq!(value(&s, "E1"), Value::Number(6.0));
    set(&mut s, "C2", "block again");
    execute(&mut s, "edit.undo", json!({}));
    execute(&mut s, "edit.redo", json!({}));
    execute(&mut s, "edit.clearContents", json!({"range": "C2"}));
    execute(&mut s, "home.insertRows", json!({"rows": "1:1"}));
    assert_eq!(value(&s, "E2"), Value::Number(6.0));
    execute(&mut s, "home.deleteRows", json!({"rows": "1:1"}));
    execute(&mut s, "sheet.rename", json!({"sheet": 0, "name": "Inputs"}));
    execute(&mut s, "sheet.move", json!({"sheet": 0, "to": 1, "copy": true}));
    assert_eq!(s.doc().unwrap().wb.sheets.len(), 2);
    execute(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap().wb.sheets.len(), 1);
    execute(&mut s, "edit.redo", json!({}));
    assert_eq!(s.doc().unwrap().wb.sheets.len(), 2);
}

#[test]
fn dynamic_targets_and_their_dependents_follow_edits() {
    let mut s = session();
    for (cell, input) in [("A1", "H3"), ("H3", "7"), ("H4", "11"), ("B1", "=INDIRECT(A1)"), ("C1", "=OFFSET(H1,2,0)"), ("D1", "=B1+C1")] {
        set(&mut s, cell, input);
    }
    set(&mut s, "H3", "13");
    assert_eq!(value(&s, "D1"), Value::Number(26.0));
    set(&mut s, "A1", "H4");
    assert_eq!(value(&s, "D1"), Value::Number(24.0));
    execute(&mut s, "edit.undo", json!({}));
    execute(&mut s, "edit.redo", json!({}));
}

#[test]
fn computed_database_criteria_track_shifted_external_cells() {
    let mut s = session();
    for (cell, input) in [
        ("G1", "Amount"),
        ("G2", "10"),
        ("G3", "20"),
        ("H2", "15"),
        ("H3", "25"),
        ("J1", "Condition"),
        ("J2", "=G2<H2"),
        ("M1", "=DSUM(G1:G3,1,J1:J2)"),
        ("N1", "=M1*2"),
    ] {
        set(&mut s, cell, input);
    }
    assert_eq!(value(&s, "M1"), Value::Number(30.0));
    // H3 is outside both the database and the criterion formula's recorded G2/H2 references.
    s.execute("cell.set", json!({"cell": "H3", "input": "5"})).unwrap();
    assert_eq!(value(&s, "M1"), Value::Number(10.0));
    assert_eq!(value(&s, "N1"), Value::Number(20.0));
    assert_clean(&s, "edit shifted external criterion precedent");
    execute(&mut s, "edit.undo", json!({}));
    assert_eq!(value(&s, "M1"), Value::Number(30.0));
    execute(&mut s, "edit.redo", json!({}));
    assert_eq!(value(&s, "M1"), Value::Number(10.0));
}

#[test]
fn subtotal_history_has_no_intermediate_commands() {
    let mut s = session();
    execute(&mut s, "range.setValues", json!({"range": "A1", "values": [["Region", "Amount"], ["East", 10], ["East", 20], ["West", 30]]}));
    let original = s.doc().unwrap().wb.clone();
    set(&mut s, "D1", "7");
    let before_subtotal = s.doc().unwrap().wb.clone();
    execute(&mut s, "data.subtotal", json!({"range": "A1:B4", "groupBy": "A", "columns": ["B"]}));
    let subtotal = s.doc().unwrap().wb.clone();
    execute(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap().wb, before_subtotal, "one undo restores the subtotal's entry state");
    execute(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap().wb, original, "second undo restores the independent preceding edit");
    execute(&mut s, "edit.redo", json!({}));
    assert_eq!(s.doc().unwrap().wb, before_subtotal);
    execute(&mut s, "edit.redo", json!({}));
    assert_eq!(s.doc().unwrap().wb, subtotal);
}

#[test]
fn every_database_function_follows_a_replaced_computed_criterion() {
    for (function, expected) in [
        ("DAVERAGE", Value::Number(10.0)),
        ("DCOUNT", Value::Number(1.0)),
        ("DCOUNTA", Value::Number(1.0)),
        ("DGET", Value::Number(10.0)),
        ("DMAX", Value::Number(10.0)),
        ("DMIN", Value::Number(10.0)),
        ("DPRODUCT", Value::Number(10.0)),
        ("DSTDEV", Value::Error(CellError::Div0)),
        ("DSTDEVP", Value::Number(0.0)),
        ("DSUM", Value::Number(10.0)),
        ("DVAR", Value::Error(CellError::Div0)),
        ("DVARP", Value::Number(0.0)),
    ] {
        let mut s = session();
        for (cell, input) in [("G1", "Amount"), ("G2", "10"), ("G3", "20"), ("H2", "15"), ("H3", "25"), ("J1", "Amount"), ("J2", ">0")] {
            set(&mut s, cell, input);
        }
        set(&mut s, "M1", &format!("={function}(G1:G3,1,J1:J2)"));
        // Replacing the criterion doesn't rebuild M1's graph node.
        set(&mut s, "J1", "Condition");
        set(&mut s, "J2", "=G2<H2");
        set(&mut s, "H3", "5");
        assert_eq!(value(&s, "M1"), expected, "{function}");
    }
}

#[test]
fn legacy_mode_survives_edit_structure_and_history_recalculation() {
    use gridcraft_engine::model::{Cell, Formula, FormulaEvalMode, Workbook};
    use gridcraft_engine::{DocState, Trust};
    let mut wb = Workbook::new();
    let sh = wb.sheet_mut(0).unwrap();
    for (cell, n) in [("A1", 1.0), ("A2", 4.0), ("A3", 9.0)] {
        sh.set_value(CellRef::parse(cell).unwrap(), Value::Number(n));
    }
    let mut formula = Formula::new("=SQRT(A1:A3)");
    formula.eval_mode = FormulaEvalMode::Legacy;
    sh.set_cell(CellRef::parse("B2").unwrap(), Cell { formula: Some(formula.into()), ..Cell::default() });
    let mut s = Session::new();
    s.add_document(DocState::new(wb, None, "legacy calculation".into(), Trust::FromFile));
    assert_clean(&s, "legacy fixture");
    assert_eq!(value(&s, "B2"), Value::Number(2.0));
    set(&mut s, "A2", "16");
    assert_eq!(value(&s, "B2"), Value::Number(4.0));
    for (command, params) in [
        ("home.insertRows", json!({"rows": "1:1"})),
        ("home.deleteRows", json!({"rows": "1:1"})),
        ("sheet.rename", json!({"sheet": 0, "name": "Legacy"})),
        ("sheet.move", json!({"sheet": 0, "to": 1, "copy": true})),
        ("edit.undo", json!({})),
        ("edit.redo", json!({})),
    ] {
        execute(&mut s, command, params);
        for sh in &s.doc().unwrap().wb.sheets {
            for (_, cell) in sh.cells.iter() {
                if let Some(formula) = &cell.formula {
                    assert_eq!(formula.eval_mode, FormulaEvalMode::Legacy, "{command}");
                }
            }
        }
    }
}
