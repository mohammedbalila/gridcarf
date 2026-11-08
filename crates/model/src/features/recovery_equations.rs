//! Paired call-site controls for the shared no-layout path used after engine admission.
use super::{EquationCache, Shape, ShapeFormat, ShapeKind};
use crate::{Anchor, Color, Workbook};
use std::sync::Arc;
thread_local! {
    pub(super) static LAYOUT_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
fn shape(text: String, id: u32) -> Shape {
    Shape {
        id,
        kind: ShapeKind::Equation,
        anchor: Anchor::default(),
        fill: Color::Auto,
        line: Color::Auto,
        text,
        math: EquationCache::default(),
        format: ShapeFormat::default(),
    }
}
#[test]
fn short_sources_really_enter_math_layout_and_cached_refresh_does_not() {
    for source in [String::new(), "x".into(), "x".repeat(4096), "é".repeat(2048)] {
        let mut wb = Workbook::new();
        wb.sheet_mut(0).unwrap().shapes.push(shape(source, 1));
        LAYOUT_CALLS.with(|calls| calls.set(0));
        wb.refresh_equations();
        assert_eq!(LAYOUT_CALLS.with(|calls| calls.get()), 1);
        wb.refresh_equations();
        assert_eq!(LAYOUT_CALLS.with(|calls| calls.get()), 1);
    }
}
#[test]
fn overlong_sources_keep_shared_sheet_and_cache_without_entering_math_layout() {
    for source in ["x".repeat(4097), format!("{}x", "é".repeat(2048))] {
        let mut wb = Workbook::new();
        wb.sheet_mut(0).unwrap().shapes.push(shape(source.clone(), 1));
        let before = wb.sheets[0].clone();
        LAYOUT_CALLS.with(|calls| calls.set(0));
        let hashes = super::equation_cache_tests::HASHES.with(|hashes| hashes.get());
        wb.refresh_equations();
        assert_eq!(LAYOUT_CALLS.with(|calls| calls.get()), 0);
        assert_eq!(super::equation_cache_tests::HASHES.with(|hashes| hashes.get()), hashes);
        assert!(Arc::ptr_eq(&before, &wb.sheets[0]));
        let equation = &wb.sheets[0].shapes[0];
        assert_eq!(equation.text, source);
        assert!(equation.math.get(&source).unwrap().is_err());
        assert!(equation.math.get("x").is_none());
    }
}
#[test]
fn mixed_sources_prove_whole_candidate_admission_is_needed() {
    for reverse in [false, true] {
        let mut wb = Workbook::new();
        let mut shapes = vec![shape("x".repeat(4097), 1), shape("x".into(), 2)];
        if reverse {
            shapes.reverse();
        }
        wb.sheet_mut(0).unwrap().shapes = shapes;
        LAYOUT_CALLS.with(|calls| calls.set(0));
        wb.refresh_equations();
        assert_eq!(LAYOUT_CALLS.with(|calls| calls.get()), 1);
    }
}
