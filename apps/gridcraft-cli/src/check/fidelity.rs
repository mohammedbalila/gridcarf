use super::calc::formula_text;
use super::package::{REBUILT_KINDS, relationship_kinds, validate_package};
use super::{BTreeMap, SourcePackage};
use super::{BTreeSet, Category, Cell, CellError, CellRef, Diffs, Options, RoundTrip, Sheet, Value, Workbook, number_text, quote_sheet};
use gridcraft_engine::model::{Formula, FormulaEvalMode, Style, StyleId};

// ---------------------------------------------------------------- round trip

/// Writes `wb` with GridCraft's XLSX writer, validates the package against itself and against
/// `source` (the file `wb` was read from), reads it back and compares.
pub fn round_trip(wb: &Workbook, source: &[u8], opts: &Options) -> RoundTrip {
    let bytes = match gridcraft_engine::io::save_bytes(wb, "roundtrip.xlsx") {
        Ok(b) => b,
        Err(e) => return RoundTrip { error: Some(format!("write failed: {e}")), categories: vec![], ..Default::default() },
    };
    let mut package = Diffs::new("package", opts.max_report);
    match validate_package(&bytes) {
        Ok(issues) => issues.into_iter().for_each(|i| package.push(|| i)),
        Err(e) => package.push(|| e),
    }
    // Non-OPC inputs (JSON/CSV/BIFF) have no relationship graph to compare. Never infer
    // source-package preservation from equality of the already imported model.
    let source_package = if source.starts_with(b"PK") {
        match relationship_kinds(source).and_then(|before| relationship_kinds(&bytes).map(|after| (before, after))) {
            Ok((before, after)) => {
                for (key, kind) in &before {
                    if !after.contains_key(key) && !REBUILT_KINDS.contains(&key.as_str()) {
                        package.push(|| format!("lost relationship {kind}"));
                    }
                }
                SourcePackage::Compared { relationship_kinds: before.len() }
            }
            Err(error) => {
                package.push(|| format!("source relationship coverage unavailable: {error}"));
                SourcePackage::Unavailable { error }
            }
        }
    } else {
        SourcePackage::NotApplicable
    };
    let back = match gridcraft_engine::io::open_bytes("roundtrip.xlsx", &bytes) {
        Ok((b, _)) => b,
        Err(e) => {
            return RoundTrip { error: Some(format!("reading the written file failed: {e}")), categories: vec![package.done()], source_package };
        }
    };
    let mut categories = diff_workbooks(wb, &back, opts);
    categories.push(package.done());
    RoundTrip { error: None, categories, source_package }
}

pub(super) fn value_show(v: &Value) -> String {
    match v {
        Value::Empty => "(blank)".to_string(),
        Value::Text(t) => serde_json::to_string(t.as_ref()).unwrap_or_default(),
        Value::Number(n) => number_text(*n),
        Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Value::Error(e) => e.as_str().to_string(),
        Value::Array(_) => v.scalar().display(),
    }
}

fn opt_formula(c: Option<&Cell>) -> String {
    c.filter(|c| c.formula.is_some()).map(formula_text).unwrap_or_default()
}

/// Per-sheet comparison of two lists, reporting what one side has and the other lacks.
fn diff_sets(d: &mut Diffs, sheet: &str, a: &BTreeSet<String>, b: &BTreeSet<String>) {
    for x in a.difference(b) {
        d.push(|| format!("{sheet}: lost {x}"));
    }
    for x in b.difference(a) {
        d.push(|| format!("{sheet}: gained {x}"));
    }
}

/// A sheet's conditional-format rules in precedence order, as comparable text. The priority number
/// itself is left out, so renumbering without changing the order isn't a difference.
fn cond_format_list(s: &Sheet) -> Vec<String> {
    let mut cfs: Vec<_> = s.cond_formats.iter().collect();
    cfs.sort_by_key(|c| c.priority);
    cfs.iter().map(|c| format!("{:?} {:?} stopIfTrue={}", c.ranges, c.rule, c.stop_if_true)).collect()
}

/// One payload difference per sheet/category; examples stay bounded even for image data.
fn diff_payload<T: PartialEq + ?Sized>(d: &mut Diffs, sheet: &str, a: &T, b: &T) {
    if a != b {
        let feature = d.name;
        d.push(|| format!("{sheet}: {feature} payload changed"));
    }
}

/// Resolve style table indices before comparing: interning order isn't document content.
fn style<'a>(wb: &'a Workbook, c: Option<&Cell>) -> &'a Style {
    wb.styles.get(c.map(|c| c.style).unwrap_or_default())
}

fn line_styles<'a>(wb: &'a Workbook, lines: &BTreeMap<u32, gridcraft_engine::model::LineInfo>) -> BTreeMap<u32, &'a Style> {
    lines
        .iter()
        .filter_map(|(i, line)| line.style.map(|id| (*i, wb.styles.get(id))))
        .filter(|(_, s)| **s != *wb.styles.get(StyleId::DEFAULT))
        .collect()
}

/// Ignore only the Array→Legacy normalization permitted by the writer's scalar classifier.
/// Keep actual CSE extents and dynamic/array-sensitive evaluation distinct, independently of
/// cached values: equal results for today's inputs don't prove the modes interchangeable.
fn formula_mode(
    wb: &Workbook,
    sheet: usize,
    cell: CellRef,
    f: Option<&Formula>,
) -> Option<(FormulaEvalMode, Option<gridcraft_engine::core::RangeRef>)> {
    f.map(|f| {
        let mode = if f.eval_mode == FormulaEvalMode::Array
            && f.array.is_none()
            && !f.dynamic
            && !gridcraft_engine::io::formula_needs_array(wb, sheet, cell, f)
        {
            FormulaEvalMode::Legacy
        } else {
            f.eval_mode
        };
        (mode, f.array)
    })
}

/// A value as the file format can hold it: `#SPILL!`/`#CALC!` are cached as `#VALUE!` and an
/// unresolved circular reference as 0 (what Excel writes), so those aren't round-trip losses.
fn as_cached(v: Value) -> Value {
    match v {
        Value::Error(CellError::Spill | CellError::Calc) => Value::Error(CellError::Value),
        Value::Error(CellError::Circ) => Value::Number(0.0),
        v => v,
    }
}

fn num_fmt<'a>(wb: &'a Workbook, c: Option<&Cell>) -> &'a str {
    wb.styles.get(c.map(|c| c.style).unwrap_or_default()).num_fmt.as_str()
}

/// Feature-by-feature comparison of a workbook (`a`) with what reading it back gave (`b`). Sheets
/// are paired by position.
pub fn diff_workbooks(a: &Workbook, b: &Workbook, opts: &Options) -> Vec<Category> {
    let m = opts.max_report;
    let mut sheets = Diffs::new("sheets", m);
    let (an, bn): (Vec<&str>, Vec<&str>) = (a.sheets.iter().map(|s| s.name.as_str()).collect(), b.sheets.iter().map(|s| s.name.as_str()).collect());
    if an != bn {
        sheets.push(|| format!("{an:?} → {bn:?}"));
    }
    let mut values = Diffs::new("values", m);
    let mut formulas = Diffs::new("formulas", m);
    let mut num_fmts = Diffs::new("numFmts", m);
    let mut merges = Diffs::new("merges", m);
    let mut tables = Diffs::new("tables", m);
    let mut cond_formats = Diffs::new("condFormats", m);
    let mut validations = Diffs::new("validations", m);
    let mut charts = Diffs::new("charts", m);
    let mut comments = Diffs::new("comments", m);
    let mut hyperlinks = Diffs::new("hyperlinks", m);
    let mut pivots = Diffs::new("pivots", m);
    let mut slicers = Diffs::new("slicers", m);
    let mut controls = Diffs::new("controls", m);
    let mut pictures = Diffs::new("pictures", m);
    let mut shapes = Diffs::new("shapes", m);
    let mut styles = Diffs::new("styles", m);
    let mut print_settings = Diffs::new("printSettings", m);
    let mut formula_modes = Diffs::new("formulaModes", m);
    diff_payload(&mut styles, "workbook default", a.styles.get(StyleId::DEFAULT), b.styles.get(StyleId::DEFAULT));
    diff_payload(&mut styles, "workbook theme", &a.theme, &b.theme);
    diff_payload(&mut styles, "named cell styles", &a.cell_styles, &b.cell_styles);
    for (si, (sa, sb)) in a.sheets.iter().zip(b.sheets.iter()).enumerate() {
        let name = quote_sheet(&sa.name);
        let mut cell = |c: CellRef, ca: Option<&Cell>, cb: Option<&Cell>| {
            let cached = |x: Option<&Cell>| as_cached(x.map(|x| x.value.scalar()).unwrap_or_default());
            let (va, vb) = (cached(ca), cached(cb));
            if va != vb {
                values.push(|| format!("{name}!{}: {} → {}", c.a1(), value_show(&va), value_show(&vb)));
            }
            let (fa, fb) = (opt_formula(ca), opt_formula(cb));
            if fa != fb {
                formulas.push(|| format!("{name}!{}: {fa} → {fb}", c.a1()));
            }
            if style(a, ca) != style(b, cb) {
                styles.push(|| format!("{name}!{}: style changed", c.a1()));
            }
            let (ma, mb) =
                (formula_mode(a, si, c, ca.and_then(|c| c.formula.as_deref())), formula_mode(b, si, c, cb.and_then(|c| c.formula.as_deref())));
            if ma != mb {
                formula_modes.push(|| format!("{name}!{}: {ma:?} → {mb:?}", c.a1()));
            }
            let (na, nb) = (num_fmt(a, ca), num_fmt(b, cb));
            if na != nb {
                num_fmts.push(|| format!("{name}!{}: {na} → {nb}", c.a1()));
            }
        };
        for (c, ca) in sa.cells.iter() {
            cell(c, Some(ca), sb.cell(c));
        }
        for (c, cb) in sb.cells.iter() {
            if sa.cell(c).is_none() {
                cell(c, None, Some(cb));
            }
        }
        let set = |f: &dyn Fn(&Sheet) -> Vec<String>, s: &Sheet| f(s).into_iter().collect::<BTreeSet<String>>();
        let merge_list = |s: &Sheet| -> Vec<String> { s.merges.iter().map(|r| r.a1()).collect() };
        diff_sets(&mut merges, &name, &set(&merge_list, sa), &set(&merge_list, sb));
        // These ids are generated package/object handles. References by table/pivot name,
        // linked cell and XML-map bindings remain in the payload and must still compare.
        let table_list = |s: &Sheet| {
            s.tables
                .iter()
                .cloned()
                .map(|mut t| {
                    t.id = 0;
                    t
                })
                .collect::<Vec<_>>()
        };
        diff_payload(&mut tables, &name, &table_list(sa), &table_list(sb));
        let (cfa, cfb) = (cond_format_list(sa), cond_format_list(sb));
        if cfa.len() != cfb.len() {
            cond_formats.push(|| format!("{name}: {} → {}", cfa.len(), cfb.len()));
        } else if let Some((x, y)) = cfa.iter().zip(&cfb).find(|(x, y)| x != y) {
            cond_formats.push(|| format!("{name}: {x} → {y}"));
        }
        diff_payload(&mut validations, &name, &sa.validations, &sb.validations);
        let chart_list = |s: &Sheet| {
            s.charts
                .iter()
                .cloned()
                .map(|mut c| {
                    c.id = 0;
                    c
                })
                .collect::<Vec<_>>()
        };
        diff_payload(&mut charts, &name, &chart_list(sa), &chart_list(sb));
        diff_payload(&mut comments, &name, &sa.comments, &sb.comments);
        diff_payload(&mut hyperlinks, &name, &sa.hyperlinks, &sb.hyperlinks);
        let pivot_list = |s: &Sheet| {
            s.pivots
                .iter()
                .cloned()
                .map(|mut p| {
                    p.id = 0;
                    for filter in &mut p.filters {
                        if let Some(selected) = &mut filter.selected {
                            selected.sort();
                            selected.dedup();
                        }
                    }
                    p
                })
                .collect::<Vec<_>>()
        };
        diff_payload(&mut pivots, &name, &pivot_list(sa), &pivot_list(sb));
        let slicer_list = |s: &Sheet| {
            s.slicers
                .iter()
                .cloned()
                .map(|mut s| {
                    s.id = 0;
                    if let Some(selected) = &mut s.selected {
                        selected.sort();
                        selected.dedup();
                    }
                    s
                })
                .collect::<Vec<_>>()
        };
        diff_payload(&mut slicers, &name, &slicer_list(sa), &slicer_list(sb));
        let control_list = |s: &Sheet, other: &Sheet| {
            s.controls
                .iter()
                .cloned()
                .enumerate()
                .map(|(i, mut c)| {
                    // The writer materializes the source's automatic display name even when the
                    // importer renumbers its id. Only match that name against its unnamed peer;
                    // arbitrary user names (including names containing numbers) remain significant.
                    if c.name.trim().is_empty()
                        || other.controls.get(i).is_some_and(|peer| peer.name.trim().is_empty() && c.name == peer.display_name())
                    {
                        c.name.clear();
                    }
                    c.id = 0;
                    c
                })
                .collect::<Vec<_>>()
        };
        diff_payload(&mut controls, &name, &control_list(sa, sb), &control_list(sb, sa));
        let image_list = |s: &Sheet| {
            s.images
                .iter()
                .cloned()
                .map(|mut i| {
                    i.id = 0;
                    i
                })
                .collect::<Vec<_>>()
        };
        diff_payload(&mut pictures, &name, &image_list(sa), &image_list(sb));
        diff_payload(&mut pictures, &name, &sa.cell_pictures, &sb.cell_pictures);
        let shape_list = |s: &Sheet| {
            s.shapes
                .iter()
                .cloned()
                .map(|mut s| {
                    s.id = 0;
                    s
                })
                .collect::<Vec<_>>()
        };
        diff_payload(&mut shapes, &name, &shape_list(sa), &shape_list(sb));
        diff_payload(&mut styles, &format!("{name} rows"), &line_styles(a, &sa.rows), &line_styles(b, &sb.rows));
        diff_payload(&mut styles, &format!("{name} columns"), &line_styles(a, &sa.cols), &line_styles(b, &sb.cols));
        diff_payload(&mut print_settings, &name, &sa.print, &sb.print);
    }
    let mut names = Diffs::new("names", m);
    let name_list = |wb: &Workbook| -> BTreeSet<String> {
        wb.names
            .iter()
            .map(|n| {
                let scope = n.scope.map(|i| format!("{}!", quote_sheet(wb.sheet(i).map(|s| s.name.as_str()).unwrap_or("?")))).unwrap_or_default();
                format!("{scope}{}={} hidden={} comment={:?}", n.name, n.formula, n.hidden, n.comment)
            })
            .collect()
    };
    diff_sets(&mut names, "workbook", &name_list(a), &name_list(b));
    vec![
        sheets.done(),
        values.done(),
        formulas.done(),
        num_fmts.done(),
        merges.done(),
        names.done(),
        tables.done(),
        cond_formats.done(),
        validations.done(),
        charts.done(),
        comments.done(),
        hyperlinks.done(),
        pivots.done(),
        slicers.done(),
        controls.done(),
        pictures.done(),
        shapes.done(),
        styles.done(),
        print_settings.done(),
        formula_modes.done(),
    ]
}
