use super::*;
use crate::Calc;
use gridcraft_core::{CellRef, Value};
use gridcraft_model::{Cell, Formula, Workbook};
use std::sync::Arc;
fn put(wb: &mut Workbook, row: u32, text: &str) {
    wb.sheet_mut(0)
        .unwrap()
        .cells
        .set(CellRef::new(row, 1), Cell { formula: Some(Arc::new(Formula::new(text))), value: Value::Number(-7.), ..Cell::default() });
}
#[test]
fn real_node_limit_is_inclusive_and_does_not_evaluate() {
    let mut wb = Workbook::new();
    for row in 0..8192 {
        put(&mut wb, row, "=A1");
    }
    let calc = Calc::from_recovery(&wb, 0).unwrap();
    assert_eq!(calc.graph.len(), 8192);
    assert_eq!(calc.last_recalc_cells, 0);
    assert_eq!(wb.sheet(0).unwrap().value(CellRef::new(0, 1)), Value::Number(-7.));
    put(&mut wb, 8192, "=1");
    assert!(matches!(Calc::from_recovery(&wb, 0), Err(RecoveryCalcError::Limit("nodes"))));
}

#[test]
fn global_limits_cover_multiple_sheets_and_duplicate_index_attempts() {
    let mut wb = Workbook::new();
    put(&mut wb, 0, "=A1+A1");
    wb.sheets.push(Arc::new(gridcraft_model::Sheet::new("other")));
    wb.sheet_mut(1).unwrap().cells.set(CellRef::default(), Cell { formula: Some(Arc::new(Formula::new("A1"))), ..Cell::default() });
    let mut b = Budget::new();
    b.limits.schedules = 3;
    assert!(matches!(build(&wb, &mut b), Err(RecoveryCalcError::Limit("schedules"))));
    assert_eq!(b.schedules, 3);
    let mut b = Budget::new();
    b.limits.indexes = 2;
    assert!(matches!(build(&wb, &mut b), Err(RecoveryCalcError::Limit("indexes"))));
    assert_eq!(b.indexes, 2);
    let mut b = Budget::new();
    b.limits.areas = 5;
    assert!(matches!(build(&wb, &mut b), Err(RecoveryCalcError::Limit("areas"))));
    assert_eq!(b.areas, 5);
    let mut b = Budget::new();
    b.limits.nodes = 1;
    assert!(matches!(build(&wb, &mut b), Err(RecoveryCalcError::Limit("nodes"))));
    assert_eq!(b.nodes, 1);
    let mut b = Budget::new();
    let calc = build(&wb, &mut b).unwrap();
    assert_eq!((calc.graph.len(), b.indexes, b.areas), (2, 3, 6));
    assert_eq!(calc.graph.nodes[&(0, CellRef::new(0, 1))].areas.len(), 2);
}
#[test]
fn malformed_none_is_not_reparsed_and_python_is_nondynamic() {
    let mut wb = Workbook::new();
    put(&mut wb, 0, "=)");
    wb.sheet_mut(0).unwrap().cells.set(
        CellRef::new(1, 1),
        Cell {
            py: Some(Arc::new(gridcraft_model::PyCode::new("raise RuntimeError('must not execute')"))),
            value: Value::Number(42.),
            ..Cell::default()
        },
    );
    gridcraft_model::validate_recovery_model(&wb).unwrap();
    let calc = Calc::from_recovery(&wb, 0).unwrap();
    assert_eq!(calc.graph.len(), 1);
    assert!(calc.graph.nodes[&(0, CellRef::new(1, 1))].areas.is_empty());
    assert!(!calc.graph.nodes[&(0, CellRef::new(1, 1))].dynamic);
    // An external unvalidated caller cannot use expr=None to cause hidden reparsing either.
    let f = Arc::make_mut(wb.sheet_mut(0).unwrap().cells.get_mut(CellRef::new(0, 1)).unwrap().formula.as_mut().unwrap());
    f.text = "A1".into();
    f.expr = None;
    assert_eq!(Calc::from_recovery(&wb, 0).unwrap().graph.len(), 1);
    let mut ordinary = Calc::new();
    ordinary.rebuild(&wb);
    assert_eq!(ordinary.graph.len(), 2);
}
#[test]
fn circular_representatives_preserve_order_and_exclude_cached_nonloops() {
    use gridcraft_core::CellError;
    let mut wb = Workbook::new();
    for (row, text) in [(0, "=B2"), (1, "=B1"), (2, "=B1+B1"), (3, "=B4"), (4, "=INDIRECT(\"B5\")")] {
        put(&mut wb, row, text);
        wb.sheet_mut(0).unwrap().cells.get_mut(CellRef::new(row, 1)).unwrap().value = Value::Error(CellError::Circ);
    }
    let calc = Calc::from_recovery(&wb, 0).unwrap();
    assert_eq!(calc.circular, [(0, CellRef::new(0, 1)), (0, CellRef::new(3, 1))]);
    let mut ordinary = Calc::new();
    ordinary.rebuild(&wb);
    assert_eq!(calc.circular, ordinary.circular);
    wb.calc.iterative = true;
    assert!(Calc::from_recovery(&wb, 0).unwrap().circular.is_empty());
}
#[test]
fn initial_work_only_consumes_the_fixed_ceiling() {
    let wb = Workbook { sheets: vec![], ..Workbook::new() };
    // Even an empty graph spends three owned sort calls after the initial debit.
    assert!(Calc::from_recovery(&wb, 16777213).is_ok());
    assert!(matches!(Calc::from_recovery(&wb, 16777214), Err(RecoveryCalcError::Limit("work"))));
    assert!(matches!(Calc::from_recovery(&wb, 16777217), Err(RecoveryCalcError::Limit("work"))));
    assert!(matches!(Calc::from_recovery(&wb, usize::MAX), Err(RecoveryCalcError::Limit("work"))));
    let wb = Workbook::new();
    assert!(matches!(Calc::from_recovery(&wb, 16777216), Err(RecoveryCalcError::Limit("work"))));
}
#[test]
fn requested_storage_accounts_old_new_overlap_and_failure_at_every_request() {
    let mut b = Budget::new();
    let mut v = b.vector::<u64>(1).unwrap();
    b.push(&mut v, 1).unwrap();
    assert_eq!(b.storage, 8);
    b.limits.storage = 23;
    assert_eq!(b.push(&mut v, 2), Err(RecoveryCalcError::Limit("storage")));
    assert_eq!((v.len(), v.capacity(), b.storage), (1, 1, 8));
    b.limits.storage = 24;
    b.push(&mut v, 2).unwrap();
    assert_eq!((v.capacity(), b.storage), (2, 24));
    assert!(b.vector::<u64>(usize::MAX).is_err());
    let mut wb = Workbook::new();
    put(&mut wb, 0, "=B1+SUM(A1:BM2)");
    wb.sheet_mut(0).unwrap().cells.get_mut(CellRef::new(0, 1)).unwrap().value = Value::Error(gridcraft_core::CellError::Circ);
    let mut b = Budget::new();
    build(&wb, &mut b).unwrap();
    let requests = b.requests;
    let bytes = b.storage;
    assert!(requests > 20);
    for request in 1..=requests {
        let mut b = Budget::new();
        b.refuse_request = Some(request);
        assert!(matches!(build(&wb, &mut b), Err(RecoveryCalcError::Allocation)), "request {request}");
        assert_eq!(b.requests, request);
    }
    let mut b = Budget::new();
    b.limits.storage = bytes;
    assert!(build(&wb, &mut b).is_ok());
    assert_eq!(b.storage, bytes);
    let mut b = Budget::new();
    b.limits.storage = bytes - 1;
    assert!(matches!(build(&wb, &mut b), Err(RecoveryCalcError::Limit("storage"))));
    println!("actual build requests={requests} cumulative_requested_bytes={bytes}");
}
#[test]
fn real_maximum_simple_graph_requested_storage_is_recorded() {
    let mut wb = Workbook::new();
    for row in 0..8192 {
        put(&mut wb, row, &format!("=A{}", row + 1));
    }
    let mut b = Budget::new();
    let calc = build(&wb, &mut b).unwrap();
    assert_eq!((b.nodes, b.areas, b.indexes), (8192, 16384, 8192));
    assert!(b.storage < 67108864);
    assert_eq!(calc.graph.len(), 8192);
    println!(
        "N8192 actual requests={} cumulative_requested_bytes={} work={} node_map_capacity={}",
        b.requests,
        b.storage,
        b.work,
        calc.graph.nodes.capacity()
    );
}
#[test]
fn real_raw_edge_limit_is_inclusive_and_rows_reserve_only_unique_edges() {
    let mut wb = Workbook::new();
    for row in 0..256 {
        put(&mut wb, row, "=SUM(A1:IV256)");
        wb.sheet_mut(0).unwrap().cells.get_mut(CellRef::new(row, 1)).unwrap().value = Value::Error(gridcraft_core::CellError::Circ);
    }
    let mut b = Budget::new();
    let calc = build(&wb, &mut b).unwrap();
    assert_eq!((b.offers, b.adjacency_capacity), (65536, 65536));
    assert_eq!(calc.circular, [(0, CellRef::new(0, 1))]);
    println!(
        "E65536 actual requests={} cumulative_requested_bytes={} work={} adjacency_capacity={}",
        b.requests, b.storage, b.work, b.adjacency_capacity
    );
    put(&mut wb, 256, "=SUM(A1:IV256)"); // non-Circ targets still spend raw E
    let mut b = Budget::new();
    assert!(matches!(build(&wb, &mut b), Err(RecoveryCalcError::Limit("offers"))));
    assert_eq!(b.offers, 65536);
}

#[test]
fn real_index_limit_and_dynamic_invocation_depth_are_inclusive() {
    let mut wb = Workbook::new();
    for row in 0..256 {
        put(&mut wb, row, "=SUM(A1:BL2)");
    }
    let mut b = Budget::new();
    build(&wb, &mut b).unwrap();
    assert_eq!(b.indexes, 16384);
    put(&mut wb, 256, "=SUM(A1:BL2)");
    assert!(matches!(Calc::from_recovery(&wb, 0), Err(RecoveryCalcError::Limit("indexes"))));
    let mut wb = Workbook::new();
    let mut text = "A1".to_string();
    for _ in 0..7 {
        text = format!("A1:INDEX({text},1)");
    }
    put(&mut wb, 0, &text);
    assert!(Calc::from_recovery(&wb, 0).is_ok());
    put(&mut wb, 0, &format!("A1:INDEX({text},1)"));
    assert!(matches!(Calc::from_recovery(&wb, 0), Err(RecoveryCalcError::Limit("depth"))));
}
#[test]
fn mixed_formula_python_remains_model_refused_and_hash_requests_check_overflow() {
    let mut wb = Workbook::new();
    put(&mut wb, 0, "A1");
    wb.sheet_mut(0).unwrap().cells.get_mut(CellRef::new(0, 1)).unwrap().py = Some(Arc::new(gridcraft_model::PyCode::new("pass")));
    assert!(gridcraft_model::validate_recovery_model(&wb).is_err());
    let mut b = Budget::new();
    let mut map = super::super::HashMap::<usize, usize>::default();
    assert_eq!(b.hash(&mut map, usize::MAX), Err(RecoveryCalcError::Limit("storage")));
    assert_eq!(b.requests, 0);
    assert_eq!(map.capacity(), 0);
}

#[test]
fn independent_flags_and_sumif_resized_preorder_are_preserved() {
    use crate::Area;
    use gridcraft_core::RangeRef;
    let mut wb = Workbook::new();
    for (row, text, dynamic) in [
        (0, "=SUM(A1)", false),
        (1, "=NOW()", true),
        (2, "=_xludf.SUM(A1)", true),
        (3, "=DSUM(A1:B3,1,D1:D2)", true),
        (4, "=missingDefinedName", true),
        (5, "=Table1[Column]", true),
    ] {
        put(&mut wb, row, text);
        let calc = Calc::from_recovery(&wb, 0).unwrap();
        assert_eq!(calc.graph.nodes[&(0, CellRef::new(row, 1))].dynamic, dynamic, "{text}");
    }
    put(&mut wb, 6, "=SUMIF(A1:B3,1,D2)");
    let calc = Calc::from_recovery(&wb, 0).unwrap();
    assert_eq!(
        calc.graph.nodes[&(0, CellRef::new(6, 1))].areas,
        [
            Area { sheet: 0, range: RangeRef::new(CellRef::new(1, 3), CellRef::new(3, 4)) },
            Area { sheet: 0, range: RangeRef::new(CellRef::new(0, 0), CellRef::new(2, 1)) },
            Area { sheet: 0, range: RangeRef::cell(CellRef::new(1, 3)) },
        ]
    );
}
