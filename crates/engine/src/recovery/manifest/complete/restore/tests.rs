use super::super::tests::{bytes, metadata};
use super::*;
use crate::recovery::{decode, restore as public_restore};
#[test]
fn detached_restore_is_always_untrusted_unsaved_even_without_scripts() {
    let wb = Workbook::new();
    let encoded = bytes(&wb, &metadata());
    let restored = public_restore(decode(&encoded).unwrap()).unwrap();
    assert!(restored.is_dirty());
    assert_eq!(restored.trust, crate::Trust::FromFile);
    assert!(!restored.scripts_trusted);
    assert!(restored.path.is_none());
    assert!(restored.undo.is_empty() && restored.redo.is_empty() && restored.unlocked_ranges.is_empty());
    assert_eq!(restored.revision, 1);
}

fn equation(source: String, id: u32) -> gridcraft_model::Shape {
    gridcraft_model::Shape {
        id,
        kind: gridcraft_model::ShapeKind::Equation,
        anchor: gridcraft_model::Anchor::default(),
        fill: gridcraft_model::Color::Auto,
        line: gridcraft_model::Color::Auto,
        text: source,
        math: gridcraft_model::EquationCache::default(),
        format: gridcraft_model::ShapeFormat::default(),
    }
}
#[test]
fn equation_admission_checks_utf8_bytes_for_the_whole_candidate() {
    for source in [String::new(), "x".into(), "x".repeat(4096), "é".repeat(2048)] {
        let mut wb = Workbook::new();
        wb.sheet_mut(0).unwrap().shapes.push(equation(source, 1));
        let encoded = bytes(&wb, &metadata());
        assert!(matches!(public_restore(decode(&encoded).unwrap()), Err(RecoveryError::UnsupportedRepresentation("restore_equation"))));
    }
    for source in ["x".repeat(4097), format!("{}x", "é".repeat(2048))] {
        let mut wb = Workbook::new();
        wb.sheet_mut(0).unwrap().shapes.push(equation(source.clone(), 1));
        let decoded = decode(&bytes(&wb, &metadata())).unwrap();
        let sheet = Arc::downgrade(&decoded.workbook.sheets[0]);
        let doc = public_restore(decoded).unwrap();
        let shape = &doc.wb.sheets[0].shapes[0];
        assert_eq!(shape.text, source);
        assert!(shape.math.get(&source).unwrap().is_err());
        assert!(std::ptr::eq(Arc::as_ptr(&sheet.upgrade().unwrap()), Arc::as_ptr(&doc.wb.sheets[0])));
    }
    for reverse in [false, true] {
        let mut wb = Workbook::new();
        let mut shapes = vec![equation("x".repeat(4097), 1), equation("x".into(), 2)];
        if reverse {
            shapes.reverse();
        }
        wb.sheet_mut(0).unwrap().shapes = shapes;
        assert!(matches!(
            public_restore(decode(&bytes(&wb, &metadata())).unwrap()),
            Err(RecoveryError::UnsupportedRepresentation("restore_equation"))
        ));
    }
}
#[test]
fn context_is_opaque_and_selection_identity_keeps_anonymous_tail() {
    let wb = Workbook::new();
    let mut meta = metadata();
    meta.captured_revision = u64::MAX;
    meta.sequence = 99;
    meta.dirty_at_capture = false;
    meta.original_path_hint = Some(NativePath::Windows(vec![0xd800, 0, 0x41]));
    let doc = public_restore(decode(&bytes(&wb, &meta)).unwrap()).unwrap();
    assert_eq!(doc.recovery_id(), Some("0123456789abcdef0123456789abcdef".parse().unwrap()));
    assert_eq!(doc.recovery_captured_revision(), Some(u64::MAX));
    assert_eq!(doc.revision, 1);
    assert_ne!(doc.wb.sheets[0].uid.get(), wb.sheets[0].uid.get());
    assert_eq!(doc.sheet_selections, vec![Selection::at(CellRef::new(7, 2)); 4]);
    assert_eq!(doc.selection, Selection::default());
    assert_eq!(doc.selection_memory.active_uid(), Some(doc.wb.sheets[0].uid.get()));
    let context = doc.recovery_context.as_ref().unwrap();
    assert!(matches!(&context.original_path_hint,Some(NativePath::Windows(units)) if units==&[0xd800,0,0x41]));
    assert_eq!((context.sequence, context.dirty_at_capture), (99, false));
    assert_eq!(format!("{context:?}"), "RestoreContext { .. }");
    assert!(Arc::ptr_eq(doc.recovery_context.as_ref().unwrap(), doc.clone().recovery_context.as_ref().unwrap()));
    let another = public_restore(decode(&bytes(&wb, &meta)).unwrap()).unwrap();
    assert_ne!(doc.uid, another.uid);
    assert!(Arc::ptr_eq(&doc.wb, &doc.saved));
    assert!(doc.is_dirty());
}
#[test]
fn cached_manual_volatile_malformed_python_and_spills_survive() {
    use gridcraft_model::{Cell, Formula, FormulaEvalMode, PyCode};
    let mut wb = Workbook::new();
    wb.calc.mode = gridcraft_model::CalcMode::Manual;
    let sh = wb.sheet_mut(0).unwrap();
    for (row, text, value) in [(0, "=RAND()", 41.), (1, "=)", 42.), (2, "=1+2", 43.)] {
        let mut formula = Formula::new(text);
        formula.eval_mode = FormulaEvalMode::Legacy;
        sh.cells.set(CellRef::new(row, 0), Cell { formula: Some(Arc::new(formula)), value: Value::Number(value), ..Cell::default() });
    }
    sh.cells.set(
        CellRef::new(3, 0),
        Cell { py: Some(Arc::new(PyCode::new("raise RuntimeError('never run')"))), value: Value::Number(44.), ..Cell::default() },
    );
    sh.spill.insert(CellRef::new(0, 1), Value::Text("retained spill".into()));
    sh.spill_ranges.insert(CellRef::new(0, 0), gridcraft_core::RangeRef::new(CellRef::new(0, 0), CellRef::new(0, 1)));
    let doc = public_restore(decode(&bytes(&wb, &metadata())).unwrap()).unwrap();
    assert_eq!(doc.calc.last_recalc_cells, 0);
    assert_eq!(doc.calc.graph.len(), 3);
    assert_eq!(doc.wb.calc.mode, gridcraft_model::CalcMode::Manual);
    for (row, value) in [(0, 41.), (1, 42.), (2, 43.), (3, 44.)] {
        assert_eq!(doc.wb.sheet(0).unwrap().cells.get(CellRef::new(row, 0)).unwrap().value, Value::Number(value));
    }
    let sh = doc.wb.sheet(0).unwrap();
    assert_eq!(sh.cells.get(CellRef::new(2, 0)).unwrap().formula.as_ref().unwrap().eval_mode, FormulaEvalMode::Legacy);
    assert_eq!(sh.spill.get(&CellRef::new(0, 1)), Some(&Value::Text("retained spill".into())));
    assert_eq!(sh.spill_ranges.get(&CellRef::new(0, 0)), Some(&gridcraft_core::RangeRef::new(CellRef::new(0, 0), CellRef::new(0, 1))));
}
#[test]
fn scan_admission_uses_verified_lengths_and_refusals_do_not_touch_a_session() {
    use gridcraft_model::{Cell, Formula};
    let mut wb = Workbook::new();
    wb.sheet_mut(0).unwrap().shapes.push(equation("x".into(), 1));
    assert_eq!(scan_work(&wb).unwrap(), 20); // 1+3+2+10+4
    let mut session = crate::Session::new();
    session.new_workbook();
    session.execute("cell.set", serde_json::json!({"cell":"A1","input":"sentinel"})).unwrap();
    let old = session.doc().unwrap().wb.clone();
    let revision = session.doc().unwrap().revision;
    assert!(public_restore(decode(&bytes(&wb, &metadata())).unwrap()).is_err());
    assert!(Arc::ptr_eq(&old, &session.doc().unwrap().wb));
    assert_eq!(revision, session.doc().unwrap().revision);
    let mut wb = Workbook::new();
    for row in 0..8193 {
        wb.sheet_mut(0)
            .unwrap()
            .cells
            .set(CellRef::new(row, 0), Cell { formula: Some(Arc::new(Formula::new("1"))), value: Value::Number(9.), ..Cell::default() });
    }
    assert!(matches!(public_restore(decode(&bytes(&wb, &metadata())).unwrap()), Err(RecoveryError::TooLarge("nodes"))));
    assert!(Arc::ptr_eq(&old, &session.doc().unwrap().wb));
}

#[test]
fn actual_selection_reserve_failure_aborts_detached_candidate() {
    let wb = Workbook::new();
    let encoded = bytes(&wb, &metadata());
    assert!(matches!(restore_owned(decode(&encoded).unwrap(), Some(0)), Err(RecoveryError::TooLarge("restore_selection"))));
    let mut session = crate::Session::new();
    session.new_workbook();
    let original = session.doc().unwrap().wb.clone();
    assert!(restore_owned(decode(&encoded).unwrap(), Some(1)).is_err());
    assert!(Arc::ptr_eq(&original, &session.doc().unwrap().wb));
    let doc = restore_owned(decode(&encoded).unwrap(), Some(usize::MAX)).unwrap();
    assert!(doc.is_dirty());
}
#[test]
fn workbook_source_paths_remain_independent_of_opaque_original_hint() {
    let mut wb = Workbook::new();
    wb.source_path = Some(std::path::PathBuf::from("/never-open-recovery-source/book.xlsx"));
    wb.source_dir = Some(std::path::PathBuf::from("/never-open-recovery-source"));
    let doc = public_restore(decode(&bytes(&wb, &metadata())).unwrap()).unwrap();
    assert_eq!(doc.wb.source_path, wb.source_path);
    assert_eq!(doc.wb.source_dir, wb.source_dir);
    assert!(doc.path.is_none());
}
