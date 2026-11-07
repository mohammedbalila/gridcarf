//! Cells.

use std::sync::Arc;

use gridcraft_core::{RangeRef, Value};
use gridcraft_formula::Expr;
use serde::{Deserialize, Serialize};

use crate::python::PyCode;
use crate::style::StyleId;

/// Evaluation semantics, independent of XLSX dynamic-array export metadata.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FormulaEvalMode {
    /// New GridCraft formulas, dynamic arrays and legacy Ctrl+Shift+Enter formulas.
    #[default]
    Array,
    /// Ordinary imported XLSX formulas use implicit intersection in scalar contexts.
    Legacy,
}

/// A parsed formula. `text` is the canonical formula text without the leading `=`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Formula {
    pub text: String,
    #[serde(skip)]
    pub expr: Option<Arc<Expr>>,
    /// Legacy array formula (Ctrl+Shift+Enter) covering this range; the formula lives in the
    /// top-left cell.
    pub array: Option<RangeRef>,
    /// Read from a file that marks this formula as a dynamic-array formula (cell metadata `cm`).
    /// The writer keeps the mark, so a formula Excel evaluates as an array is never saved as a
    /// legacy formula (which Excel evaluates with implicit intersection). Formulas entered or
    /// rewritten in GridCraft start without it, and the writer decides from their shape.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dynamic: bool,
    /// Missing in older JSON: retain GridCraft's existing array evaluation by default.
    #[serde(default)]
    pub eval_mode: FormulaEvalMode,
}

impl Formula {
    /// Parses `text` (with or without `=`). Unparseable formulas keep their text and evaluate to
    /// `#NAME?`.
    pub fn new(text: &str) -> Formula {
        let body = text.strip_prefix('=').unwrap_or(text);
        match gridcraft_formula::parse(body) {
            Ok(e) => Formula {
                text: gridcraft_formula::print(&e),
                expr: Some(Arc::new(e)),
                array: None,
                dynamic: false,
                eval_mode: FormulaEvalMode::Array,
            },
            Err(_) => Formula { text: body.to_string(), expr: None, array: None, dynamic: false, eval_mode: FormulaEvalMode::Array },
        }
    }
    pub fn from_expr(e: Expr) -> Formula {
        Formula { text: gridcraft_formula::print(&e), expr: Some(Arc::new(e)), array: None, dynamic: false, eval_mode: FormulaEvalMode::Array }
    }
    /// The parsed expression, re-parsing after deserialization.
    pub fn expr(&self) -> Option<Expr> {
        self.expr.as_deref().cloned().or_else(|| gridcraft_formula::parse(&self.text).ok())
    }
    /// The parsed expression without copying it (re-parses after deserialization).
    pub fn expr_arc(&self) -> Option<Arc<Expr>> {
        self.expr.clone().or_else(|| gridcraft_formula::parse(&self.text).ok().map(Arc::new))
    }
    /// Runs `f` on the parsed expression, borrowing the cached tree or, after deserialization,
    /// a freshly parsed one that is dropped afterwards. Unlike [`expr`](Self::expr) it never copies
    /// the tree, and unlike [`expr_arc`](Self::expr_arc) it needs no reference count or `Arc`
    /// allocation. `None` when the text does not parse.
    pub fn with_expr<R>(&self, f: impl FnOnce(&Expr) -> R) -> Option<R> {
        match &self.expr {
            Some(e) => Some(f(e)),
            None => gridcraft_formula::parse(&self.text).ok().map(|e| f(&e)),
        }
    }
}

/// A static picture stored as cell content, independent of floating drawing anchors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellPicture {
    /// Encoded PNG/JPEG bytes, base64 in JSON. Sheet entries share the payload through Arc.
    #[serde(with = "crate::b64")]
    pub data: Vec<u8>,
    pub mime: String,
    pub alt: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    /// Constant value, or the cached result of the formula or Python code.
    pub value: Value,
    pub formula: Option<Arc<Formula>>,
    pub style: StyleId,
    /// Python code, for a Python cell (which has no formula).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub py: Option<Arc<PyCode>>,
}

impl Cell {
    pub fn value(v: Value) -> Cell {
        Cell { value: v, formula: None, style: StyleId::DEFAULT, py: None }
    }
    pub fn formula(f: Formula) -> Cell {
        Cell { value: Value::Empty, formula: Some(Arc::new(f)), style: StyleId::DEFAULT, py: None }
    }
    /// A value cell with this style.
    pub fn styled(v: Value, style: StyleId) -> Cell {
        Cell { value: v, formula: None, style, py: None }
    }
    /// No value, no formula, no Python code and the default style: storing it is pointless.
    pub fn is_blank(&self) -> bool {
        self.value.is_empty() && self.formula.is_none() && self.style == StyleId::DEFAULT && self.py.is_none()
    }
    /// Whether the cell holds content (a value, a formula or Python code), formats aside.
    pub fn has_content(&self) -> bool {
        !self.value.is_empty() || self.formula.is_some() || self.py.is_some()
    }
    /// Removes the content (value, formula and Python code), keeping the style.
    pub fn clear_content(&mut self) {
        self.value = Value::Empty;
        self.formula = None;
        self.py = None;
    }
    /// What the formula bar shows: a formula, a Python cell's code, or the value.
    pub fn input_text(&self) -> String {
        if let Some(py) = &self.py {
            return py.code.clone();
        }
        if let Some(f) = &self.formula {
            return format!("={}", f.text);
        }
        match &self.value {
            Value::Text(t) => {
                // Text that would parse as something else is shown with its apostrophe by Excel
                // only when entered that way; we show it plainly.
                t.to_string()
            }
            v => v.display(),
        }
    }
    /// What the formula bar shows when not editing: [`input_text`](Self::input_text), with a
    /// legacy array formula in braces (`{=A1:A2*2}`).
    pub fn bar_text(&self) -> String {
        match &self.formula {
            Some(f) if f.array.is_some() => format!("{{={}}}", f.text),
            _ => self.input_text(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Formula;
    use std::sync::Arc;

    #[test]
    fn with_expr_borrows_the_cached_tree_without_cloning() {
        let formula = Formula::new("=IF(A1>2,SUM(B1:C3),A1*3)");
        let cached = formula.expr.as_ref().unwrap();
        let owners = Arc::strong_count(cached);
        assert_eq!(
            formula.with_expr(|expr| {
                assert!(std::ptr::eq(expr, cached.as_ref()), "callback must receive the cached tree itself");
                assert_eq!(Arc::strong_count(cached), owners, "callback must not clone the Arc handle");
            }),
            Some(())
        );
    }

    #[test]
    fn with_expr_parses_text_without_populating_the_cache() {
        let mut formula = Formula::new("=SUM(A1:B2)+3");
        formula.expr = None;
        assert_eq!(formula.with_expr(gridcraft_formula::print).as_deref(), Some("SUM(A1:B2)+3"));
        assert!(formula.expr.is_none());
    }

    #[test]
    fn with_expr_does_not_call_back_for_malformed_text() {
        let formula = Formula::new("=NOPE(");
        let mut called = false;
        assert_eq!(formula.with_expr(|_| called = true), None);
        assert!(!called);
    }
}
