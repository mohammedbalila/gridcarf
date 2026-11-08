//! Export-time implicit-intersection analysis, without evaluating the workbook.
//!
//! A range is harmless in SUM, but not in SQRT or arithmetic. Keep the result's shape separate
//! from affected operations inside it: SUM(range * range) returns a scalar yet needs array
//! evaluation. Legacy array-aware contexts such as SUMPRODUCT already preserve those operations.

use gridcraft_core::CellRef;
use gridcraft_formula::{BinOp, Expr, SheetSel, StructItem, UnOp};
use gridcraft_model::{Formula, Workbook};

#[derive(Clone, Copy, Default)]
struct Shape {
    array: bool,
    affected: bool,
}

impl Shape {
    fn array() -> Self {
        Self { array: true, affected: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AnalysisError {
    Limit,
    Allocation,
}
#[derive(Clone, Copy)]
pub(crate) enum AnalysisCharge {
    Work(usize, usize),
    Allocate(usize),
    Release(usize),
}
type Charge<'a> = Option<&'a mut dyn FnMut(AnalysisCharge) -> Result<(), AnalysisError>>;

pub(crate) fn needs_array(wb: &Workbook, sheet: usize, at: CellRef, formula: &Formula) -> bool {
    needs_array_impl(wb, sheet, at, formula, None).unwrap_or(true)
}
pub(crate) fn needs_array_checked(
    wb: &Workbook,
    sheet: usize,
    at: CellRef,
    formula: &Formula,
    charge: &mut dyn FnMut(AnalysisCharge) -> Result<(), AnalysisError>,
) -> Result<bool, AnalysisError> {
    needs_array_impl(wb, sheet, at, formula, Some(charge))
}
fn needs_array_impl(wb: &Workbook, sheet: usize, at: CellRef, formula: &Formula, charge: Charge<'_>) -> Result<bool, AnalysisError> {
    let mut scan = Scan { wb, at, remaining: 16_384, locals: Vec::new(), locals_requested: 0, charge, error: None, live: 0 };
    if !scan.spend(|| (1, 0)) {
        return Err(scan.error.unwrap_or(AnalysisError::Limit));
    }
    if formula.eval_mode == gridcraft_model::FormulaEvalMode::Legacy {
        return Ok(false);
    }
    if let Some(expr) = formula.expr.as_deref() {
        return scan.finish(expr, sheet);
    }
    let (result, request) = scan.parse(&formula.text, false);
    let parsed = result;
    let answer = if let Some(expr) = parsed.as_ref() { scan.analyze(expr, sheet) } else { false };
    drop(parsed);
    scan.release(request);
    scan.result(answer)
}
struct Scan<'a, 'b> {
    wb: &'a Workbook,
    at: CellRef,
    remaining: usize,
    locals: Vec<(String, Shape)>,
    locals_requested: usize,
    charge: Charge<'b>,
    error: Option<AnalysisError>,
    live: usize,
}
impl<'a> Scan<'a, '_> {
    fn request(&mut self, r: AnalysisCharge) -> bool {
        if self.error.is_some() {
            return false;
        }
        if let Some(c) = self.charge.as_mut()
            && let Err(e) = c(r)
        {
            self.error = Some(e);
            return false;
        }
        true
    }
    fn spend(&mut self, measure: impl FnOnce() -> (usize, usize)) -> bool {
        if self.charge.is_none() {
            return true;
        }
        let (events, bytes) = measure();
        self.request(AnalysisCharge::Work(events, bytes))
    }
    fn allocate(&mut self, n: usize) -> bool {
        if self.request(AnalysisCharge::Allocate(n)) {
            self.live = self.live.saturating_add(n);
            true
        } else {
            false
        }
    }
    fn release(&mut self, n: usize) {
        if self.charge.is_some() {
            if let Some(c) = self.charge.as_mut() {
                let _ = c(AnalysisCharge::Release(n));
            }
            self.live = self.live.saturating_sub(n);
        }
    }
    fn reserve<T>(&mut self, v: &mut Vec<T>, requested: &mut usize, additional: usize) -> bool {
        if self.charge.is_none() {
            v.reserve(additional);
            return true;
        }
        let Some(needed) = v.len().checked_add(additional) else {
            self.error = Some(AnalysisError::Allocation);
            return false;
        };
        if needed <= *requested {
            return true;
        }
        let cap = needed.max(requested.saturating_mul(2)).max(4);
        let Some(bytes) = cap.checked_mul(std::mem::size_of::<T>()) else {
            self.error = Some(AnalysisError::Allocation);
            return false;
        };
        if !self.allocate(bytes) {
            return false;
        }
        let old = *requested * std::mem::size_of::<T>();
        if v.try_reserve_exact(cap - v.len()).is_err() {
            self.error = Some(AnalysisError::Allocation);
            return false;
        }
        *requested = cap;
        self.release(old);
        true
    }
    fn parse(&mut self, text: &str, name: bool) -> (Option<Expr>, usize) {
        let mut request = 0;
        if self.charge.is_some() {
            if !self.spend(|| (1, 0)) {
                return (None, 0);
            }
            let body = text.strip_prefix('=').unwrap_or(text);
            if body.len() > 4 * 8192 {
                return (None, 0);
            }
            if !self.spend(|| (1, body.len())) {
                return (None, 0);
            }
            if body.chars().count() > 8192 {
                return (None, 0);
            }
            // Lexer character/token vectors, significant-space token copies, AST boxes/vectors
            // and owned spellings are conservatively requested at512 bytes/input byte. Parser
            // internals remain the audited infallible dependency allocation exception.
            let Some(n) = body.len().checked_add(1).and_then(|n| n.checked_mul(512)) else {
                self.error = Some(AnalysisError::Limit);
                return (None, 0);
            };
            // Fence the audited token/AST layouts and charge the parser's bounded passes
            // before entering its infallible allocation internals.
            if std::mem::size_of::<gridcraft_formula::lexer::Token>() > 72 || std::mem::size_of::<Expr>() > 104 {
                self.error = Some(AnalysisError::Limit);
                return (None, 0);
            }
            if !self.spend(|| ((body.len() + 1) * 32, (body.len() + 1) * 128)) || !self.allocate(n) {
                return (None, 0);
            }
            request = n;
        }
        let result = if name { gridcraft_formula::parse_name(text) } else { gridcraft_formula::parse(text) };
        (result.ok(), request)
    }
    fn parsed_shape(&mut self, text: &str, name: bool, sheet: usize, context: bool, depth: usize) -> Shape {
        let (expr, n) = self.parse(text, name);
        let result = expr.as_ref().map(|e| self.expr(e, sheet, context, depth)).unwrap_or_else(Shape::array);
        drop(expr);
        self.release(n);
        result
    }
    fn result(mut self, value: bool) -> Result<bool, AnalysisError> {
        drop(std::mem::take(&mut self.locals));
        let live = self.live;
        self.release(live);
        self.error.map_or(Ok(value), Err)
    }
    fn finish(mut self, e: &Expr, sheet: usize) -> Result<bool, AnalysisError> {
        let value = self.analyze(e, sheet);
        self.result(value)
    }
    fn analyze(&mut self, e: &Expr, sheet: usize) -> bool {
        let shape = self.expr(e, sheet, false, 0);
        if shape.array || shape.affected {
            return true;
        }
        if self.charge.is_none() {
            let mut found = false;
            e.walk(&mut |e| found |= matches!(e, Expr::Unary(UnOp::At, _)));
            return found;
        }
        let mut pending = Vec::new();
        let mut requested = 0;
        if !self.reserve(&mut pending, &mut requested, 1) {
            return true;
        }
        pending.push(e);
        let mut found = false;
        while let Some(e) = pending.pop() {
            if !self.spend(|| (1, 0)) {
                break;
            }
            if matches!(e, Expr::Unary(UnOp::At, _)) {
                found = true;
                break;
            }
            match e {
                Expr::Array(rows) => {
                    if !self.spend(|| (rows.len(), 0)) {
                        break;
                    }
                    for row in rows.iter().rev() {
                        for x in row.iter().rev() {
                            if !self.pending(&mut pending, &mut requested, x) {
                                break;
                            }
                        }
                        if self.error.is_some() {
                            break;
                        }
                    }
                }
                Expr::Unary(_, x) | Expr::Paren(x) => {
                    self.pending(&mut pending, &mut requested, x);
                }
                Expr::Binary(_, a, b) => {
                    self.pending(&mut pending, &mut requested, b);
                    self.pending(&mut pending, &mut requested, a);
                }
                Expr::Call(_, args) => {
                    for x in args.iter().rev() {
                        if !self.pending(&mut pending, &mut requested, x) {
                            break;
                        }
                    }
                }
                Expr::Invoke(c, args) => {
                    for x in args.iter().rev() {
                        if !self.pending(&mut pending, &mut requested, x) {
                            break;
                        }
                    }
                    self.pending(&mut pending, &mut requested, c);
                }
                _ => {}
            }
            if self.error.is_some() {
                break;
            }
        }
        drop(pending);
        self.release(requested * std::mem::size_of::<&Expr>());
        found
    }
    fn pending<'e>(&mut self, pending: &mut Vec<&'e Expr>, requested: &mut usize, e: &'e Expr) -> bool {
        if !self.spend(|| (1, 0)) || !self.reserve(pending, requested, 1) {
            return false;
        }
        pending.push(e);
        true
    }
    fn name(&mut self, key: &str, sheet: usize) -> Option<&'a gridcraft_model::DefinedName> {
        for scope in [Some(sheet), None] {
            for n in &self.wb.names {
                if !self.spend(|| (1, key.len().saturating_add(n.name.len()))) {
                    return None;
                }
                if n.scope == scope && n.name.eq_ignore_ascii_case(key) {
                    return Some(n);
                }
            }
        }
        None
    }
    fn table(&mut self, key: &str, sheet: Option<usize>) -> Option<&'a gridcraft_model::Table> {
        if !self.spend(|| (1, key.len())) {
            return None;
        }
        for (i, s) in self.wb.sheets.iter().enumerate() {
            if sheet.is_some_and(|x| x != i) {
                continue;
            }
            for t in &s.tables {
                if !self.spend(|| (1, key.len().saturating_add(t.name.len()))) {
                    return None;
                }
                if if key.is_empty() { t.range.contains(self.at) } else { t.name.eq_ignore_ascii_case(key) } {
                    return Some(t);
                }
            }
        }
        None
    }
    fn sheet(&mut self, raw: &str) -> Option<usize> {
        if !self.spend(|| (1, raw.len())) {
            return None;
        }
        let raw = raw.trim();
        let inner = raw.strip_prefix('\'').and_then(|s| s.strip_suffix('\''));
        for (i, s) in self.wb.sheets.iter().enumerate() {
            if !self.spend(|| (1, raw.len().saturating_add(s.name.len()))) {
                return None;
            }
            let mut chars = inner.unwrap_or(raw).chars().peekable();
            let decoded = std::iter::from_fn(|| {
                let c = chars.next()?;
                if inner.is_some() && c == '\'' && chars.peek() == Some(&'\'') {
                    chars.next();
                }
                Some(c.to_ascii_lowercase())
            });
            if decoded.eq(s.name.chars().map(|c| c.to_ascii_lowercase())) {
                return Some(i);
            }
        }
        None
    }
    fn expr(&mut self, e: &Expr, sheet: usize, array_context: bool, depth: usize) -> Shape {
        // Names can be cyclic, and a short chain of repeated names can expand exponentially.
        // Preserve array evaluation when analysis cannot safely finish.
        if !self.spend(|| (1, 0)) || depth >= 64 || self.remaining == 0 {
            return Shape { array: true, affected: true };
        }
        self.remaining -= 1;
        match e {
            Expr::Ref(r) => Shape { array: !r.range().is_single() || matches!(r.sheet, SheetSel::Span(_, _)), affected: false },
            Expr::Array(rows) => Shape { array: rows.len() > 1 || rows.first().is_some_and(|r| r.len() > 1), affected: false },
            Expr::Name(n) => {
                if !self.spend(|| (1, n.len())) {
                    return Shape::array();
                }
                for i in (0..self.locals.len()).rev() {
                    let local_len = self.locals[i].0.len();
                    if !self.spend(|| (1, n.len().saturating_add(local_len))) {
                        return Shape::array();
                    }
                    if self.locals[i].0.eq_ignore_ascii_case(n) {
                        return self.locals[i].1;
                    }
                }
                let (scope, bare) = match n.rsplit_once('!') {
                    Some((s, name)) => (self.sheet(s).unwrap_or(sheet), name),
                    None => (sheet, n.as_str()),
                };
                if let Some(def) = self.name(bare, scope) {
                    if !self.spend(|| (1, def.formula.len())) {
                        return Shape::array();
                    }
                    return self.parsed_shape(def.formula.trim_start_matches('='), true, def.scope.unwrap_or(scope), array_context, depth + 1);
                }
                Shape { array: self.table(bare, None).is_some(), affected: false }
            }
            Expr::Struct(r) => {
                let table = if r.table.is_empty() { self.table("", Some(sheet)) } else { self.table(&r.table, None) };
                if !self.spend(|| {
                    (r.specifiers.len(), r.col_start.as_ref().map_or(0, String::len).saturating_add(r.col_end.as_ref().map_or(0, String::len)))
                }) {
                    return Shape::array();
                }
                let one_row = r.specifiers.iter().any(|s| matches!(s, StructItem::ThisRow | StructItem::Headers | StructItem::Totals));
                let one_col = match (&r.col_start, &r.col_end) {
                    (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                    (Some(_), None) => true,
                    _ => table.is_some_and(|t| t.columns.len() == 1),
                };
                Shape { array: !(one_row && one_col), affected: false }
            }
            Expr::Paren(x) => self.expr(x, sheet, array_context, depth + 1),
            Expr::Unary(op, x) => {
                let child = self.expr(x, sheet, array_context, depth + 1);
                match op {
                    UnOp::At => Shape { array: false, affected: child.affected },
                    // A trim reference is a reference operator, like `:`.
                    UnOp::Spill | UnOp::Trim(_) => Shape { array: true, affected: child.affected },
                    _ => Shape { array: child.array, affected: child.affected || (child.array && !array_context) },
                }
            }
            Expr::Binary(op, a, b) => {
                // A flat chain such as A1+A2+… nests one level per operator on its left: follow
                // it in a loop, so a long chain is neither deep recursion nor past the depth
                // limit (which would write a plain sum as an array formula).
                let mut spine = Vec::new();
                let mut requested = 0;
                if !self.reserve(&mut spine, &mut requested, 1) {
                    return Shape::array();
                }
                spine.push((*op, b.as_ref()));
                let mut left = a.as_ref();
                while let Expr::Binary(op, a, b) = left {
                    if !self.spend(|| (1, 0)) || self.remaining == 0 {
                        drop(spine);
                        self.release(requested * std::mem::size_of::<(BinOp, &Expr)>());
                        return Shape { array: true, affected: true };
                    }
                    self.remaining -= 1;
                    if !self.reserve(&mut spine, &mut requested, 1) {
                        return Shape::array();
                    }
                    spine.push((*op, b.as_ref()));
                    left = a.as_ref();
                }
                let mut acc = self.expr(left, sheet, array_context, depth + 1);
                while let Some((op, b)) = spine.pop() {
                    let b = self.expr(b, sheet, array_context, depth + 1);
                    let reference = matches!(op, BinOp::Range | BinOp::Union | BinOp::Intersect);
                    let array = reference || acc.array || b.array;
                    acc = Shape { array, affected: acc.affected || b.affected || (!reference && array && !array_context) };
                }
                drop(spine);
                self.release(requested * std::mem::size_of::<(BinOp, &Expr)>());
                acc
            }
            Expr::Call(name, args) => self.call(name, args, sheet, array_context, depth + 1),
            Expr::Invoke(_, _) => Shape { array: true, affected: false },
            _ => Shape::default(),
        }
    }

    fn call(&mut self, name: &str, args: &[Expr], sheet: usize, array_context: bool, depth: usize) -> Shape {
        if !self.spend(|| (args.len().saturating_add(1), name.len())) {
            return Shape::array();
        }
        if name == "LET"
            && args.len() >= 3
            && let Some((last, bindings)) = args.split_last()
        {
            let saved = self.locals.len();
            let mut affected = false;
            for pair in bindings.as_chunks::<2>().0 {
                if let [Expr::Name(n), value] = pair {
                    let shape = self.expr(value, sheet, array_context, depth);
                    affected |= shape.affected;
                    let value = if self.charge.is_some() {
                        if !self.spend(|| (1, n.len())) || !self.allocate(n.len()) {
                            return Shape::array();
                        }
                        let mut s = String::new();
                        if s.try_reserve_exact(n.len()).is_err() {
                            self.error = Some(AnalysisError::Allocation);
                            return Shape::array();
                        }
                        s.push_str(n);
                        s
                    } else {
                        n.clone()
                    };
                    let mut locals = std::mem::take(&mut self.locals);
                    let mut requested = self.locals_requested;
                    let reserved = self.reserve(&mut locals, &mut requested, 1);
                    self.locals_requested = requested;
                    if !reserved {
                        self.locals = locals;
                        return Shape::array();
                    }
                    locals.push((value, shape));
                    self.locals = locals;
                }
            }
            let mut result = self.expr(last, sheet, array_context, depth);
            result.affected |= affected;
            while self.locals.len() > saved {
                if let Some((name, _)) = self.locals.pop() {
                    let requested = name.len();
                    drop(name);
                    self.release(requested);
                }
            }
            return result;
        }
        let legacy_array =
            matches!(name, "SUMPRODUCT" | "MMULT" | "MINVERSE" | "MDETERM" | "TRANSPOSE" | "FREQUENCY" | "LINEST" | "LOGEST" | "TREND" | "GROWTH");
        let mut affected = false;
        let mut lifted = false;
        let mut shapes = Vec::new();
        let mut requested = 0;
        if args.len() > 255 {
            return Shape { array: true, affected: true };
        }
        if !self.reserve(&mut shapes, &mut requested, args.len()) {
            return Shape::array();
        }
        for (i, arg) in args.iter().enumerate() {
            let context = array_context || legacy_array || (name == "INDEX" && i == 0);
            let shape = self.expr(arg, sheet, context, depth);
            affected |= shape.affected;
            if shape.array && scalar_arg(name, i) {
                lifted = true;
                affected |= !context;
            }
            shapes.push(shape);
        }
        let first_array = shapes.first().is_some_and(|s| s.array);
        let array = match name {
            "INDIRECT" => match args.first() {
                Some(Expr::Text(text)) if !matches!(args.get(1), Some(Expr::Bool(false))) => {
                    self.parsed_shape(text, false, sheet, array_context, depth).array
                }
                _ => true,
            },
            "OFFSET" => {
                // Omitted dimensions retain the source reference's shape; unknown dimensions
                // might become multiple cells on a later calculation.
                if args.len() <= 3 { first_array } else { !args.get(3).is_some_and(one) || args.get(4).map_or(first_array, |e| !one(e)) }
            }
            "INDEX" => {
                let row = args.get(1);
                let col = args.get(2);
                let row_positive = row.is_some_and(|e| positive_index(e, shapes.get(1).copied()));
                let col_positive = col.is_some_and(|e| positive_index(e, shapes.get(2).copied()));
                !row_positive || col.is_some_and(|_| !col_positive) || (col.is_none() && !args.first().is_some_and(single_column))
            }
            "ROW" | "COLUMN" => {
                affected |= first_array && !array_context;
                first_array
            }
            "IF" | "IFERROR" | "IFNA" | "IFS" | "SWITCH" | "CHOOSE" => shapes.iter().any(|s| s.array),
            "XLOOKUP" => {
                lifted
                    || match (args.get(1), args.get(2)) {
                        (Some(Expr::Ref(lookup)), Some(Expr::Ref(result))) => {
                            if lookup.range().width() == 1 {
                                result.range().width() > 1
                            } else {
                                result.range().height() > 1
                            }
                        }
                        _ => true,
                    }
            }
            "FILTER" | "SORT" | "SORTBY" | "UNIQUE" | "SEQUENCE" | "RANDARRAY" | "VSTACK" | "HSTACK" | "TOROW" | "TOCOL" | "WRAPROWS"
            | "WRAPCOLS" | "TAKE" | "DROP" | "CHOOSEROWS" | "CHOOSECOLS" | "EXPAND" | "GROUPBY" | "PIVOTBY" | "TRANSPOSE" | "MMULT" | "MINVERSE"
            | "FREQUENCY" | "LINEST" | "LOGEST" | "TREND" | "GROWTH" | "MAP" | "SCAN" | "BYROW" | "BYCOL" | "MAKEARRAY" | "ANCHORARRAY" => true,
            _ => lifted || self.name(name, sheet).is_some(),
        };
        drop(shapes);
        self.release(requested * std::mem::size_of::<Shape>());
        Shape { array, affected }
    }
}

fn positive(e: &Expr) -> bool {
    matches!(e, Expr::Number(n) if *n >= 1.0)
}

fn positive_index(e: &Expr, shape: Option<Shape>) -> bool {
    // MATCH returns a one-based position or an error, never INDEX's zero "whole row" selector.
    positive(e) || (matches!(e, Expr::Call(name, _) if matches!(name.as_str(), "MATCH" | "XMATCH")) && shape.is_some_and(|s| !s.array))
}

fn one(e: &Expr) -> bool {
    matches!(e, Expr::Number(n) if *n == 1.0)
}

fn single_column(e: &Expr) -> bool {
    matches!(e, Expr::Ref(r) if r.range().width() == 1)
}

/// Legacy reference/array-taking parameters. Other parameters consume a single value.
/// This is file-format policy, not a second evaluator: unknown results are never calculated.
fn scalar_arg(name: &str, i: usize) -> bool {
    match name {
        "SUM" | "AVERAGE" | "AVERAGEA" | "MIN" | "MINA" | "MAX" | "MAXA" | "COUNT" | "COUNTA" | "COUNTBLANK" | "PRODUCT" | "SUMSQ" | "MEDIAN"
        | "MODE" | "MODE.SNGL" | "STDEV" | "STDEV.S" | "STDEV.P" | "STDEVP" | "VAR" | "VAR.S" | "VAR.P" | "VARP" | "AVEDEV" | "DEVSQ" | "GEOMEAN"
        | "HARMEAN" | "KURT" | "SKEW" | "SKEW.P" | "AND" | "OR" | "XOR" | "SUMPRODUCT" | "SUMX2MY2" | "SUMX2PY2" | "SUMXMY2" | "CORREL" | "COVAR"
        | "COVARIANCE.P" | "COVARIANCE.S" | "PEARSON" | "RSQ" | "SLOPE" | "INTERCEPT" | "STEYX" | "ROWS" | "COLUMNS" | "AREAS" | "ROW" | "COLUMN"
        | "TRANSPOSE" | "ISREF" | "MMULT" | "MINVERSE" | "MDETERM" | "FREQUENCY" | "CONCAT" | "VSTACK" | "HSTACK" | "ANCHORARRAY" => false,
        "SUMIF" | "AVERAGEIF" => i == 1,
        "COUNTIF" => i != 0,
        "SUMIFS" | "AVERAGEIFS" | "MINIFS" | "MAXIFS" => i > 0 && i.is_multiple_of(2),
        "COUNTIFS" => i % 2 == 1,
        "INDEX" | "LARGE" | "SMALL" | "PERCENTILE" | "PERCENTILE.INC" | "PERCENTILE.EXC" | "QUARTILE" | "QUARTILE.INC" | "QUARTILE.EXC" | "IRR"
        | "MIRR" | "RANK" | "RANK.EQ" | "RANK.AVG" => i != if name.starts_with("RANK") { 1 } else { 0 },
        "VLOOKUP" | "HLOOKUP" | "MATCH" | "XMATCH" => i != 1,
        "LOOKUP" | "NPV" | "XNPV" | "SUBTOTAL" => i == 0,
        "XLOOKUP" => i != 1 && i != 2,
        "XIRR" => i > 1,
        "OFFSET" => i != 0,
        "AGGREGATE" | "TEXTJOIN" => i < 2,
        "FILTER" | "SORTBY" => i > 1,
        "SORT" | "UNIQUE" | "TAKE" | "DROP" | "CHOOSECOLS" | "CHOOSEROWS" | "TOROW" | "TOCOL" | "WRAPROWS" | "WRAPCOLS" | "EXPAND" => i != 0,
        "LINEST" | "LOGEST" | "TREND" | "GROWTH" => i > 1,
        "FORECAST" | "FORECAST.LINEAR" => i == 0,
        "FORECAST.ETS" | "FORECAST.ETS.CONFINT" => i != 1 && i != 2,
        "FORECAST.ETS.SEASONALITY" | "FORECAST.ETS.STAT" => i > 1,
        _ => true,
    }
}
