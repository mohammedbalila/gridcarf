//! Private E0 layout ceilings and ordinary-rebuild baselines, not recovery admission tests.
use super::*;
use crate::Calc;
use gridcraft_core::{CellError, Value};
use gridcraft_model::{Cell, Formula, FormulaEvalMode, PyCode, Workbook};
use std::mem::{align_of, size_of};
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

fn layout<T>(name: &str, ceiling: usize) {
    println!("layout {name}: size={} align={} ceiling={ceiling}", size_of::<T>(), align_of::<T>());
    assert!(size_of::<T>() <= ceiling, "{name} exceeds the E0 requested-byte planning ceiling");
    assert!(align_of::<T>() <= 16, "{name} invalidates the reviewed slot alignment allowance");
}

#[test]
fn actual_private_layouts_fit_requested_byte_plan() {
    // These ceilings protect a numerical resource argument; they do not specify a Rust ABI.
    layout::<CellRef>("CellRef", 8);
    layout::<RangeRef>("RangeRef", 16);
    layout::<Key>("Key", 16);
    layout::<Area>("Area", 24);
    layout::<Node>("Node", 32);
    layout::<Graph>("Graph", 336);
    layout::<Calc>("Calc", 392);
    layout::<Vec<usize>>("Vec header", 24);
    layout::<HashMap<Key, Node>>("map header", 48);
    layout::<HashSet<Key>>("set header", 48);
    layout::<Intervals<Key>>("narrow interval header", 112);
    layout::<Intervals<(u32, u32, Key)>>("wide interval header", 112);
    layout::<(Key, Node)>("node slot/staging", 48);
    layout::<(Key, HashSet<Key>)>("cell map slot", 64);
    layout::<((usize, u32), Intervals<Key>)>("column map slot", 128);
    layout::<(usize, Intervals<(u32, u32, Key)>)>("wide map slot", 120);
    layout::<(Key, usize)>("circ index slot", 24);
    layout::<(u32, u32, Key)>("narrow interval item", 24);
    layout::<(u32, u32, (u32, u32, Key))>("wide interval item", 32);
    layout::<(Key, Key)>("grouped cell entry", 32);
    layout::<((usize, u32), u32, u32, Key)>("grouped column entry", 40);
    layout::<(usize, RangeRef, Key)>("grouped wide entry", 40);
    layout::<(&gridcraft_formula::Expr, usize)>("pending cursor", 16);
    layout::<(usize, usize)>("SCC frame", 16);
    layout::<bool>("dead/on-stack element", 1);
    layout::<BTreeSet<(u32, u32, Key)>>("empty pending header", 24);
    layout::<gridcraft_functions::FnSpec>("registry spec", 128);
    let registry = gridcraft_functions::all();
    let name_bytes: usize = registry.iter().map(|spec| spec.name.len()).sum();
    println!("registry entries={} name_bytes={name_bytes}", registry.len());
    assert!(registry.len() <= 1024 && name_bytes <= 32768, "revisit borrowed function-classification work bounds");
}

fn put(wb: &mut Workbook, at: CellRef, text: &str, value: Value) {
    let mut formula = Formula::new(text);
    formula.eval_mode = FormulaEvalMode::Legacy;
    formula.dynamic = true;
    wb.sheet_mut(0).unwrap().cells.set(at, Cell { formula: Some(Arc::new(formula)), value, ..Cell::default() });
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::disallowed_methods, reason = "host-only E0 characterization timing")]
#[test]
fn ordinary_simple_rebuild_baselines_preserve_caches_and_dependents() {
    for n in [64u32, 1024, 8192] {
        let input_started = Instant::now();
        let mut wb = Workbook::new();
        for row in 0..n {
            put(&mut wb, CellRef::new(row, 1), &format!("=A{}", row + 1), Value::Number(-123.5));
        }
        let input_elapsed = input_started.elapsed();
        let mut calc = Calc::new();
        let operation_started = Instant::now();
        calc.rebuild(&wb);
        let operation_elapsed = operation_started.elapsed();
        assert_eq!(calc.graph.len(), n as usize);
        assert!(calc.graph.dynamic.is_empty());
        assert!(calc.circular.is_empty());
        assert_eq!(calc.last_recalc_cells, 0);
        for row in 0..n {
            let key = (0, CellRef::new(row, 1));
            let node = calc.graph.nodes.get(&key).unwrap();
            assert_eq!(node.areas, [Area { sheet: 0, range: RangeRef::new(CellRef::new(row, 0), CellRef::new(row, 0)) }]);
            let cell = wb.sheet(0).unwrap().cells.get(key.1).unwrap();
            assert_eq!(cell.value, Value::Number(-123.5));
            assert_eq!(cell.formula.as_ref().unwrap().eval_mode, FormulaEvalMode::Legacy);
            assert!(cell.formula.as_ref().unwrap().dynamic);
            let mut deps = Vec::new();
            calc.graph.dependents(0, CellRef::new(row, 0), &mut deps);
            assert_eq!(deps, [key]);
        }
        println!(
            "ordinary simple n={n} input_us={} rebuild_us={} nodes_capacity={} cell_groups_capacity={}",
            input_elapsed.as_micros(),
            operation_elapsed.as_micros(),
            calc.graph.nodes.capacity(),
            calc.graph.cell_deps.capacity()
        );
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::disallowed_methods, reason = "host-only E0 characterization timing")]
#[test]
fn ordinary_interval_boundary_baselines_answer_inside_and_outside_queries() {
    for (width, end) in [(64u32, "BL"), (65, "BM")] {
        let input_started = Instant::now();
        let mut wb = Workbook::new();
        for row in 0..65 {
            put(&mut wb, CellRef::new(row, 100), &format!("=SUM(A{}:{end}{})", row + 1, row + 1), Value::Number(77.0));
        }
        let input_elapsed = input_started.elapsed();
        let mut calc = Calc::new();
        let operation_started = Instant::now();
        calc.rebuild(&wb);
        let operation_elapsed = operation_started.elapsed();
        assert_eq!(calc.graph.len(), 65);
        assert_eq!(calc.graph.col_deps.len(), if width == 64 { 64 } else { 0 });
        assert_eq!(calc.graph.wide_deps.len(), if width == 65 { 1 } else { 0 });
        let (mut items, mut trees, mut dead, mut pending) = (0, 0, 0, 0);
        for iv in calc.graph.col_deps.values() {
            items += iv.items.capacity();
            trees += iv.max_end.capacity();
            dead += iv.dead.capacity();
            pending += iv.pending.len();
        }
        for iv in calc.graph.wide_deps.values() {
            items += iv.items.capacity();
            trees += iv.max_end.capacity();
            dead += iv.dead.capacity();
            pending += iv.pending.len();
        }
        assert_eq!(pending, 0);
        for row in 0..65 {
            let key = (0, CellRef::new(row, 100));
            assert_eq!(calc.graph.nodes[&key].areas, [Area { sheet: 0, range: RangeRef::new(CellRef::new(row, 0), CellRef::new(row, width - 1)) }]);
            assert_eq!(wb.sheet(0).unwrap().cells.get(key.1).unwrap().value, Value::Number(77.0));
            for col in [0, width - 1, width] {
                let mut deps = Vec::new();
                calc.graph.dependents(0, CellRef::new(row, col), &mut deps);
                assert_eq!(deps, if col < width { vec![key] } else { vec![] });
            }
        }
        println!(
            "ordinary intervals width={width} rows=65 input_us={} rebuild_us={} item_capacity_sum={items} tree_capacity_sum={trees} dead_capacity_sum={dead} pending={pending}",
            input_elapsed.as_micros(),
            operation_elapsed.as_micros()
        );
    }
}

#[test]
fn ordinary_rebuild_keeps_python_fallback_and_only_real_cached_cycles() {
    let mut wb = Workbook::new();
    for (col, text) in [(0, "=A1"), (1, "=A1+A1"), (2, "=C1")] {
        put(&mut wb, CellRef::new(0, col), text, Value::Error(CellError::Circ));
    }
    wb.sheet_mut(0)
        .unwrap()
        .cells
        .set(CellRef::new(0, 3), Cell { py: Some(Arc::new(PyCode::new("never_execute"))), value: Value::Number(19.0), ..Cell::default() });
    put(&mut wb, CellRef::new(0, 4), "=)", Value::Number(23.0));
    gridcraft_model::validate_recovery_model(&wb).unwrap();
    let mut calc = Calc::new();
    calc.rebuild(&wb);
    assert_eq!(calc.graph.len(), 4);
    assert_eq!(calc.circular, [(0, CellRef::new(0, 0)), (0, CellRef::new(0, 2))]);
    assert_eq!(calc.graph.nodes[&(0, CellRef::new(0, 1))].areas.len(), 2);
    let fallback = &calc.graph.nodes[&(0, CellRef::new(0, 3))];
    assert!(fallback.areas.is_empty() && !fallback.dynamic);
    assert!(!calc.graph.nodes.contains_key(&(0, CellRef::new(0, 4))));
    for (col, value) in [(3, 19.0), (4, 23.0)] {
        assert_eq!(wb.sheet(0).unwrap().cells.get(CellRef::new(0, col)).unwrap().value, Value::Number(value));
    }
    let mut deps = Vec::new();
    calc.graph.dependents(0, CellRef::new(0, 0), &mut deps);
    deps.sort_unstable();
    assert_eq!(deps, [(0, CellRef::new(0, 0)), (0, CellRef::new(0, 1))]);
    wb.calc.iterative = true;
    calc.rebuild(&wb);
    assert!(calc.circular.is_empty());
}
