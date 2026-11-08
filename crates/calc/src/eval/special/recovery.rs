use super::{Area, Classification, Expr, PrecedentAction, RangeRef, Reference, SheetSel, Workbook, precedent_action};
#[cfg(test)]
use super::{CellRef, precedents};
use crate::recalc::recovery::{Budget, RecoveryCalcError, Result};
struct Borrowed<'a>(&'a mut Budget);
impl Classification for Borrowed<'_> {
    type Error = RecoveryCalcError;
    fn eq(&mut self, a: &str, b: &str) -> Result<bool> {
        self.0.text(a, b)?;
        Ok(a == b)
    }
    fn lookup(&mut self, name: &str) -> Result<Option<bool>> {
        self.0.text(name, "")?;
        let mut n = name.trim();
        loop {
            let mut stripped = false;
            for prefix in ["_XLFN.", "_XLWS."] {
                self.0.text(n, prefix)?;
                if n.get(..prefix.len()).is_some_and(|p| p.eq_ignore_ascii_case(prefix)) {
                    n = &n[prefix.len()..];
                    stripped = true;
                    break;
                }
            }
            if !stripped {
                break;
            }
        }
        // Fixed cold registry initialization is excluded from Calc-owned requested payload.
        for spec in gridcraft_functions::all() {
            self.0.text(n, spec.name)?;
            if n.eq_ignore_ascii_case(spec.name) {
                return Ok(Some(spec.category == gridcraft_functions::Category::Database));
            }
        }
        Ok(None)
    }
}
fn sheet_index(wb: &Workbook, name: &str, b: &mut Budget) -> Result<Option<usize>> {
    for (i, sheet) in wb.sheets.iter().enumerate() {
        b.text(name, &sheet.name)?;
        if sheet.name.eq_ignore_ascii_case(name) {
            return Ok(Some(i));
        }
    }
    Ok(None)
}
fn emit(out: &mut Vec<Area>, area: Area, b: &mut Budget) -> Result<()> {
    b.area()?;
    b.push(out, area)
}
fn reference(wb: &Workbook, sheet: usize, r: &Reference, range: RangeRef, out: &mut Vec<Area>, b: &mut Budget) -> Result<()> {
    match &r.sheet {
        SheetSel::Current => emit(out, Area { sheet, range }, b)?,
        SheetSel::Named(n) => {
            if let Some(sheet) = sheet_index(wb, n, b)? {
                emit(out, Area { sheet, range }, b)?;
            }
        }
        SheetSel::Span(a, z) => {
            // Evaluate both endpoints, including missing first endpoints, like ordinary lookup.
            let a = sheet_index(wb, a, b)?;
            let z = sheet_index(wb, z, b)?;
            if let (Some(a), Some(z)) = (a, z) {
                for sheet in a.min(z)..=a.max(z) {
                    emit(out, Area { sheet, range }, b)?;
                }
            }
        }
    }
    Ok(())
}
fn schedule<'a>(pending: &mut Vec<&'a Expr>, e: &'a Expr, b: &mut Budget) -> Result<()> {
    b.schedule()?;
    b.push(pending, e)
}
pub(crate) fn recovery_precedents(wb: &Workbook, sheet: usize, e: &Expr, b: &mut Budget) -> Result<(Vec<Area>, bool)> {
    collect(wb, sheet, e, b, 1)
}
fn collect(wb: &Workbook, sheet: usize, e: &Expr, b: &mut Budget, depth: usize) -> Result<(Vec<Area>, bool)> {
    b.work(1)?;
    if depth > b.limits.depth {
        return Err(RecoveryCalcError::Limit("depth"));
    }
    let (mut out, mut pending, mut dynamic) = (Vec::new(), Vec::new(), false);
    schedule(&mut pending, e, b)?;
    while !pending.is_empty() {
        b.work(1)?;
        let Some(x) = pending.pop() else {
            break;
        };
        b.work(1)?;
        match precedent_action(x, &mut Borrowed(b))? {
            PrecedentAction::Reference(r, range) => reference(wb, sheet, r, range, &mut out, b)?,
            PrecedentAction::Dynamic => dynamic = true,
            PrecedentAction::Range(a, z) => {
                let mut boxes: Vec<Area> = Vec::new();
                for child in [a, z] {
                    // Admission occurs before recursive entry, not merely inside the callee.
                    b.work(1)?;
                    if depth >= b.limits.depth {
                        return Err(RecoveryCalcError::Limit("depth"));
                    }
                    let (inner, _) = collect(wb, sheet, child, b, depth + 1)?;
                    for area in inner {
                        b.area()?;
                        let mut found = false;
                        for existing in &mut boxes {
                            b.work(1)?;
                            if existing.sheet == area.sheet {
                                existing.range = existing.range.union(&area.range);
                                found = true;
                                break;
                            }
                        }
                        if !found {
                            emit(&mut boxes, area, b)?;
                        }
                    }
                }
                for area in boxes {
                    emit(&mut out, area, b)?;
                }
            }
            PrecedentAction::None => {}
        }
        match x {
            Expr::Array(rows) => {
                for row in rows.iter().rev() {
                    b.work(1)?;
                    for child in row.iter().rev() {
                        schedule(&mut pending, child, b)?;
                    }
                }
            }
            Expr::Unary(_, e) | Expr::Paren(e) => schedule(&mut pending, e, b)?,
            Expr::Binary(_, a, z) => {
                schedule(&mut pending, z, b)?;
                schedule(&mut pending, a, b)?;
            }
            Expr::Call(_, args) => {
                for child in args.iter().rev() {
                    schedule(&mut pending, child, b)?;
                }
            }
            Expr::Invoke(c, args) => {
                for child in args.iter().rev() {
                    schedule(&mut pending, child, b)?;
                }
                schedule(&mut pending, c, b)?;
            }
            _ => {}
        }
    }
    Ok((out, dynamic))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schedules_are_shared_across_collectors_before_any_emission() {
        let wb = Workbook::new();
        let e = gridcraft_formula::parse("=A1").unwrap();
        let mut budget = Budget::new();
        budget.schedules = 262143;
        let first = recovery_precedents(&wb, 0, &e, &mut budget).unwrap();
        assert_eq!(first.0.len(), 1);
        assert!(recovery_precedents(&wb, 0, &e, &mut budget).is_err());
    }
    #[test]
    fn actual_dynamic_ranges_keep_inner_and_outer_visits() {
        let wb = Workbook::new();
        let e = gridcraft_formula::parse("=A1:INDEX(B2:B4,1)").unwrap();
        let (areas, dynamic) = recovery_precedents(&wb, 0, &e, &mut Budget::new()).unwrap();
        assert_eq!(
            areas,
            [
                Area { sheet: 0, range: RangeRef::new(CellRef::new(0, 0), CellRef::new(3, 1)) },
                Area { sheet: 0, range: RangeRef::new(CellRef::new(0, 0), CellRef::new(0, 0)) },
                Area { sheet: 0, range: RangeRef::new(CellRef::new(1, 1), CellRef::new(3, 1)) },
            ]
        );
        assert!(!dynamic);
        assert_eq!((areas, dynamic), precedents(&wb, 0, &e));
    }
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    #[test]
    fn collector_matches_ordinary_semantics_and_order() {
        let mut wb = Workbook::new();
        wb.sheets.push(std::sync::Arc::new(gridcraft_model::Sheet::new("Other")));
        wb.sheets.push(std::sync::Arc::new(gridcraft_model::Sheet::new("Last")));
        for text in [
            "=A1",
            "=sheet1!A1",
            "=missing!A1",
            "=Last:Sheet1!A1",
            "=Sheet1:Last!A1",
            "=SUM(A1:A2)",
            "=SUM(A1:BL2)",
            "=SUM(A1:BM2)",
            "=SUMIF(A1:B3,1,D2)",
            "=AVERAGEIF(A1:A3,1,D2)",
            "=SUMIF(A1,1,4)",
            "=SUMIF(A1,1,INDEX(B1:B2,1))",
            "=NOW()+NoSuchName",
            "=DSUM(A1:B5,1,D1:D2)",
            "=Unknown(A1)",
            "=A1:INDEX(B2:INDEX(C3:C5,1),1)",
            "={1,2;3,4}",
        ] {
            let e = gridcraft_formula::parse(text).unwrap();
            let mut b = Budget::new();
            assert_eq!(recovery_precedents(&wb, 0, &e, &mut b).unwrap(), precedents(&wb, 0, &e), "{text}");
            assert!(b.schedules > 0);
        }
        for name in ["SUM", "sum", " _xLfN._XLws._xlfn.SUM ", "_xludf.SUM", "ÜSUM", "nosuchfn", "_xlfn.DSUM", "SUMIF", "_xlfn.SUMIF"] {
            let e = Expr::Call(name.into(), vec![gridcraft_formula::parse("A1").unwrap(), Expr::Number(1.), gridcraft_formula::parse("D1").unwrap()]);
            assert_eq!(recovery_precedents(&wb, 0, &e, &mut Budget::new()).unwrap(), precedents(&wb, 0, &e), "{name}");
        }
    }
    #[test]
    fn fences_precede_emission_schedule_and_recursive_entry() {
        let wb = Workbook::new();
        let e = gridcraft_formula::parse("A1+B1").unwrap();
        let mut b = Budget::new();
        b.limits.schedules = 1;
        assert_eq!(recovery_precedents(&wb, 0, &e, &mut b), Err(RecoveryCalcError::Limit("schedules")));
        assert_eq!(b.schedules, 1);
        assert_eq!(b.areas, 0);
        assert_eq!(b.requests, 1);
        let e = gridcraft_formula::parse("A1").unwrap();
        let mut b = Budget::new();
        b.limits.areas = 0;
        assert_eq!(recovery_precedents(&wb, 0, &e, &mut b), Err(RecoveryCalcError::Limit("areas")));
        assert_eq!(b.requests, 1); // pending exists; the output allocation was never requested
        let e = gridcraft_formula::parse("A1:INDEX(B1,1)").unwrap();
        let mut b = Budget::new();
        b.limits.depth = 1;
        assert_eq!(recovery_precedents(&wb, 0, &e, &mut b), Err(RecoveryCalcError::Limit("depth")));
        assert_eq!(b.schedules, 1);
        assert_eq!(b.areas, 0);
        let mut b = Budget::new();
        b.limits.depth = 2;
        assert!(recovery_precedents(&wb, 0, &e, &mut b).is_ok());
        assert_eq!(b.schedules, 9); // root plus both inner traversals plus ordinary child revisits
    }
    #[test]
    fn full_failed_name_operands_and_checked_work_are_shared() {
        let wb = Workbook::new();
        let e = Expr::Call("nonesuch".into(), vec![]);
        let mut b = Budget::new();
        recovery_precedents(&wb, 0, &e, &mut b).unwrap();
        let used = b.names;
        assert!(used > 8 * 491);
        b.limits.names = used;
        assert_eq!(recovery_precedents(&wb, 0, &e, &mut b), Err(RecoveryCalcError::Limit("names")));
        assert_eq!(b.names, used);
        let mut b = Budget::new();
        b.work = usize::MAX;
        assert_eq!(recovery_precedents(&wb, 0, &e, &mut b), Err(RecoveryCalcError::Limit("work")));
        assert_eq!(b.requests, 0);
        let mut b = Budget::new();
        b.schedules = usize::MAX;
        assert_eq!(recovery_precedents(&wb, 0, &e, &mut b), Err(RecoveryCalcError::Limit("schedules")));
        assert_eq!(b.requests, 0);
    }
    #[test]
    fn storage_refusal_precedes_real_pending_allocation() {
        let wb = Workbook::new();
        let e = gridcraft_formula::parse("A1").unwrap();
        let mut b = Budget::new();
        b.limits.schedules = 0;
        assert!(recovery_precedents(&wb, 0, &e, &mut b).is_err());
        assert_eq!(b.requests, 0);
        let mut b = Budget::new();
        b.limits.storage = std::mem::size_of::<&Expr>() - 1;
        assert_eq!(recovery_precedents(&wb, 0, &e, &mut b), Err(RecoveryCalcError::Limit("storage")));
        assert_eq!(b.requests, 0);
        assert_eq!(b.storage, 0);
        let mut b = Budget::new();
        b.refuse_request = Some(2);
        assert_eq!(recovery_precedents(&wb, 0, &e, &mut b), Err(RecoveryCalcError::Allocation));
        assert_eq!(b.requests, 2);
    }
}
