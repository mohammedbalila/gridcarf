//! Table styles: how a cell inside a table looks (our own palettes, keyed by the familiar
//! `TableStyleLight1…21 / Medium1…28 / Dark1…11` names so files round-trip).

use gridcraft_core::{CellRef, RangeRef};
use gridcraft_model::{BorderLine, BorderStyle, Cell, Color, Fill, Formula, PatternType, Style, Table, Workbook};

/// (family, number) from a style name, e.g. `TableStyleMedium2` → ("Medium", 2).
pub fn parse_style_name(name: &str) -> (&'static str, u32) {
    let rest = name.strip_prefix("TableStyle").unwrap_or(name);
    for fam in ["Light", "Medium", "Dark"] {
        if let Some(n) = rest.strip_prefix(fam) {
            let k = n.parse::<u32>().unwrap_or(2).max(1); // the gallery starts at 1; 0 would underflow `n - 1`
            return (fam, k);
        }
    }
    ("Medium", 2)
}

/// Accent theme slot (4..=9) for a style number; 1 and every 7th style are neutral (dk1).
fn accent_for(n: u32) -> Option<u8> {
    let k = (n.saturating_sub(1)) % 7;
    if k == 0 { None } else { Some(3 + k as u8) }
}

/// Header/band/body colours for a table style.
pub struct TableLook {
    pub header_fill: Option<Color>,
    pub header_font: Color,
    pub header_bold: bool,
    pub band_fill: Option<Color>,
    pub body_fill: Option<Color>,
    pub line: Option<Color>,
    pub total_top: Option<Color>,
}

pub fn look(style: &str) -> TableLook {
    let (fam, n) = parse_style_name(style);
    let acc = accent_for(n);
    let base = |tint: i16| match acc {
        Some(a) => Color::Theme(a, tint),
        None => Color::Theme(1, if tint == 0 { 0 } else { tint.clamp(-1000, 1000) }),
    };
    let neutral_light = |tint: i16| match acc {
        Some(a) => Color::Theme(a, tint),
        None => Color::Theme(0, -((1000 - tint).clamp(0, 1000) / 6)),
    };
    match fam {
        "Light" => {
            let group = (n - 1) / 7; // 0: lines only, 1: banded header line, 2: grid
            TableLook {
                header_fill: None,
                header_font: if group == 0 { base(-250) } else { Color::Auto },
                header_bold: true,
                band_fill: Some(neutral_light(800)),
                body_fill: None,
                line: Some(base(0)),
                total_top: Some(base(0)),
            }
        }
        "Dark" => TableLook {
            header_fill: Some(Color::Theme(1, 0)),
            header_font: Color::Theme(0, 0),
            header_bold: true,
            band_fill: Some(base(-250)),
            body_fill: Some(base(0)),
            line: None,
            total_top: Some(Color::Theme(0, 0)),
        },
        _ => TableLook {
            header_fill: Some(base(0)),
            header_font: Color::Theme(0, 0),
            header_bold: true,
            band_fill: Some(neutral_light(800)),
            body_fill: None,
            line: if (n - 1) / 7 == 0 { None } else { Some(base(400)) },
            total_top: Some(base(0)),
        },
    }
}

/// The style a table gives a cell (merged over the cell's own style: explicit fills/fonts win).
pub fn table_cell_style(wb: &Workbook, sheet: usize, t: &Table, c: CellRef) -> Style {
    let own = wb.sheet(sheet).map(|sh| wb.styles.get(sh.style_id(c)).clone()).unwrap_or_default();
    let mut s = own.clone();
    let lk = look(&t.style);
    let is_header = t.header_row && c.row == t.range.start.row;
    let is_total = t.totals_row && c.row == t.range.end.row;
    let data_row = c.row.saturating_sub(t.range.start.row + t.header_row as u32);
    let data_col = c.col - t.range.start.col;
    let explicit_fill = own.fill.pattern != PatternType::None;
    if is_header {
        if let Some(f) = lk.header_fill
            && !explicit_fill
        {
            s.fill = Fill::solid(f);
        }
        if own.font.color == Color::Auto {
            s.font.color = lk.header_font;
        }
        s.font.bold = s.font.bold || lk.header_bold;
        if let Some(l) = lk.line {
            s.borders.bottom = BorderLine { style: BorderStyle::Thin, color: l };
        }
    } else if is_total {
        s.font.bold = true;
        if let Some(l) = lk.total_top {
            s.borders.top = BorderLine { style: BorderStyle::Double, color: l };
        }
    } else {
        let band = (t.banded_rows && data_row.is_multiple_of(2)) || (t.banded_cols && data_col.is_multiple_of(2));
        if !explicit_fill {
            if band {
                if let Some(f) = lk.band_fill {
                    s.fill = Fill::solid(f);
                }
            } else if let Some(f) = lk.body_fill {
                s.fill = Fill::solid(f);
            }
        }
        if let Some(l) = lk.line
            && t.style.contains("Light")
            && c.row == t.range.end.row
        {
            s.borders.bottom = BorderLine { style: BorderStyle::Thin, color: l };
        }
    }
    if (t.first_col && c.col == t.range.start.col) || (t.last_col && c.col == t.range.end.col) {
        s.font.bold = true;
    }
    if t.style.starts_with("TableStyleDark") && !is_header && own.font.color == Color::Auto {
        s.font.color = Color::Theme(0, 0);
    }
    s
}

/// Gallery list of style names.
pub fn gallery() -> Vec<String> {
    let mut v = Vec::new();
    for n in 1..=21 {
        v.push(format!("TableStyleLight{n}"));
    }
    for n in 1..=28 {
        v.push(format!("TableStyleMedium{n}"));
    }
    for n in 1..=11 {
        v.push(format!("TableStyleDark{n}"));
    }
    v
}

/// Editing a table's header row renames the column, as in Excel: an empty or repeated header
/// gets a unique name written back into the cell, and structured references follow the rename.
pub(crate) fn sync_headers(cx: &mut crate::cmd::Ctx) {
    if cx.wb.sheets.iter().all(|s| s.tables.is_empty()) {
        return;
    }
    // (sheet, table name, table range, old column name, new column name)
    let mut renames: Vec<(usize, String, RangeRef, String, String)> = Vec::new();
    let heads: Vec<(usize, RangeRef)> = (0..cx.wb.sheets.len())
        .flat_map(|si| cx.wb.sheet(si).into_iter().flat_map(move |sh| sh.tables.iter().filter(|t| t.header_row).map(move |t| (si, t.range))))
        .map(|(si, r)| (si, RangeRef::new(r.start, CellRef::new(r.start.row, r.end.col))))
        .collect();
    // A structural edit can move anything into a header row: then every header is checked.
    let changed: std::collections::BTreeSet<(usize, CellRef)> = if cx.structural {
        heads.iter().flat_map(|(s, r)| r.iter().map(move |c| (*s, c))).collect()
    } else {
        cx.changed.iter().filter(|(si, c)| heads.iter().any(|(s, r)| s == si && r.contains(*c))).copied().collect()
    };
    // Per table: lower-case column name → column, and the next number to try for a repeated name
    // (both kept up to date, so a wide header row stays linear).
    type Names = (std::collections::HashMap<String, usize>, std::collections::HashMap<String, usize>);
    let mut names: std::collections::HashMap<(usize, usize), Names> = std::collections::HashMap::new();
    for (si, c) in changed {
        let Some(sh) = cx.wb.sheet(si) else { continue };
        let Some(ti) = sh.tables.iter().position(|t| t.header_row && c.row == t.range.start.row && t.range.contains(c)) else { continue };
        let Some(t) = sh.tables.get(ti) else { continue };
        let col = (c.col - t.range.start.col) as usize;
        let Some(old) = t.columns.get(col).map(|x| x.name.clone()) else { continue };
        // A text header names its column as it is: a number format only changes how it looks (an
        // accounting format pads text with spaces), and the file writes the name into the cell.
        let value = sh.value(c);
        let text = match value.as_text() {
            Some(t) => t.to_string(),
            None => crate::display::cell_text(&cx.wb, sh, c),
        };
        // Headers hold constant text: a formula (or a spilled value) there becomes its text.
        let is_text = value.is_text() && sh.cell(c).is_some_and(|x| x.formula.is_none());
        if text == old && is_text {
            continue;
        }
        let (taken, next) = names
            .entry((si, ti))
            .or_insert_with(|| (t.columns.iter().enumerate().map(|(i, x)| (x.name.to_lowercase(), i)).collect(), Default::default()));
        if taken.get(&old.to_lowercase()) == Some(&col) {
            taken.remove(&old.to_lowercase());
        }
        let base = if text.trim().is_empty() { format!("Column{}", col + 1) } else { text.clone() };
        let k = next.entry(base.to_lowercase()).or_insert(2);
        let mut name = base.clone();
        while taken.contains_key(&name.to_lowercase()) {
            name = format!("{base}{k}");
            *k += 1;
        }
        taken.insert(name.to_lowercase(), col);
        let (table, range) = (t.name.clone(), t.range);
        let Some(sh) = cx.wb.sheet_mut(si) else { continue };
        if let Some(x) = sh.tables.get_mut(ti).and_then(|t| t.columns.get_mut(col)) {
            x.name = name.clone();
        }
        // Headers are text: a number typed there, or a generated name, is written back as text.
        if name != text || !is_text {
            let style = sh.style_id(c);
            sh.cells.set(c, Cell { value: gridcraft_core::Value::text(name.as_str()), style, ..Default::default() });
        }
        if !old.eq_ignore_ascii_case(&name) {
            renames.push((si, table, range, old, name));
        }
    }
    if renames.is_empty() {
        return;
    }
    // Structured references follow the new names.
    for i in 0..cx.wb.sheets.len() {
        let formulas: Vec<(CellRef, std::sync::Arc<Formula>)> =
            cx.wb.sheets.get(i).map(|s| s.cells.iter().filter_map(|(c, x)| x.formula.clone().map(|f| (c, f))).collect()).unwrap_or_default();
        for (c, f) in formulas {
            let Some(e) = f.expr() else { continue };
            let mut hit = false;
            let ne = e.map(&mut |x| match x {
                gridcraft_formula::Expr::Struct(mut st) => {
                    for (si, table, range, old, name) in &renames {
                        let this = if st.table.is_empty() { i == *si && range.contains(c) } else { st.table.eq_ignore_ascii_case(table) };
                        if this {
                            for end in [&mut st.col_start, &mut st.col_end].into_iter().flatten() {
                                if end.eq_ignore_ascii_case(old) {
                                    *end = name.clone();
                                    hit = true;
                                }
                            }
                        }
                    }
                    gridcraft_formula::Expr::Struct(st)
                }
                o => o,
            });
            if hit && let Some(cell) = cx.wb.sheet_mut(i).and_then(|s| s.cells.get_mut(c)) {
                cell.formula = Some(std::sync::Arc::new(Formula { eval_mode: f.eval_mode, ..Formula::from_expr(ne) }));
            }
        }
        // Stored calculated-column text binds local references to its owning table.
        // Read before mutating so tables on unrelated shared sheets stay shared.
        let mut updates = Vec::new();
        if let Some(sh) = cx.wb.sheet(i) {
            for (ti, t) in sh.tables.iter().enumerate() {
                for (ci, col) in t.columns.iter().enumerate() {
                    let Some(text) = &col.formula else { continue };
                    let Ok(e) = gridcraft_formula::parse(text) else { continue };
                    let mut hit = false;
                    let ne = e.map(&mut |x| match x {
                        gridcraft_formula::Expr::Struct(mut st) => {
                            for (si, table, range, old, name) in &renames {
                                let this = if st.table.is_empty() { i == *si && t.range == *range } else { st.table.eq_ignore_ascii_case(table) };
                                if this {
                                    for end in [&mut st.col_start, &mut st.col_end].into_iter().flatten() {
                                        if end.eq_ignore_ascii_case(old) {
                                            *end = name.clone();
                                            hit = true;
                                        }
                                    }
                                }
                            }
                            gridcraft_formula::Expr::Struct(st)
                        }
                        o => o,
                    });
                    if hit {
                        updates.push((ti, ci, gridcraft_formula::print(&ne)));
                    }
                }
            }
        }
        if !updates.is_empty()
            && let Some(sh) = cx.wb.sheet_mut(i)
        {
            for (ti, ci, text) in updates {
                if let Some(col) = sh.tables.get_mut(ti).and_then(|t| t.columns.get_mut(ci)) {
                    col.formula = Some(text);
                }
            }
        }
    }
    cx.structural = true;
}

#[cfg(test)]
mod tests {
    use gridcraft_core::Value;
    use serde_json::json;

    use crate::Session;

    fn calculated_columns() -> Session {
        use gridcraft_model::{Formula, FormulaEvalMode};
        let mut s = Session::new();
        s.new_workbook();
        s.execute("range.setValues", json!({"range":"A1", "values":[["Sku","Price","Local","Span"],[101,5,0,0],[102,7,0,0]]})).unwrap();
        s.execute("range.setValues", json!({"range":"F1", "values":[["Item","Cost","Local","Bad","Other"],[102,0,0,0,0],[101,0,0,0,0]]})).unwrap();
        s.execute("insert.table", json!({"range":"A1:D3","header":true,"name":"Prices"})).unwrap();
        s.execute("insert.table", json!({"range":"F1:J3","header":true,"name":"Orders"})).unwrap();
        s.execute("cell.set", json!({"cell":"N1","input":"=SUM(Prices[Price])"})).unwrap();
        s.execute("home.insertSheet", json!({"name":"Remote"})).unwrap();
        s.execute("range.setValues", json!({"range":"A1", "values":[["Sku","Price","Local","Span"],[1,2,0,0]]})).unwrap();
        s.execute("insert.table", json!({"range":"A1:D3","header":true,"name":"Mirror"})).unwrap();
        s.execute("home.insertSheet", json!({"name":"Untouched"})).unwrap();
        s.execute("range.setValues", json!({"range":"A1", "values":[["Price","Cost"],[2,3]]})).unwrap();
        s.execute("insert.table", json!({"range":"A1:B2","header":true,"name":"Unrelated"})).unwrap();
        s.execute("sheet.activate", json!({"sheet":0})).unwrap();
        let wb = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().wb);
        let sh = wb.sheet_mut(0).unwrap();
        sh.tables[0].columns[2].formula = Some("[@Price]*2".into());
        sh.tables[0].columns[3].formula = Some("SUM(Prices[[Sku]:[Price]])".into());
        for (col, text) in [(1, "INDEX(Prices[Price],MATCH(Orders[@Item],Prices[Sku],0))"), (2, "[@Price]*2"), (3, "Prices["), (4, " [@Other] + 1 ")]
        {
            sh.tables[1].columns[col].formula = Some(text.into());
        }
        let cell = sh.cells.get_mut(gridcraft_core::CellRef::new(0, 13)).unwrap();
        let f = cell.formula.as_ref().unwrap();
        cell.formula = Some(std::sync::Arc::new(Formula { eval_mode: FormulaEvalMode::Legacy, ..(**f).clone() }));
        let sh = wb.sheet_mut(1).unwrap();
        sh.tables[0].columns[1].formula = Some("SUM(prices[price])".into());
        sh.tables[0].columns[2].formula = Some("[@Price]*2".into());
        let sh = wb.sheet_mut(2).unwrap();
        sh.tables[0].columns[0].formula = Some("[@Price]*2".into());
        sh.tables[0].columns[1].formula = Some("SUM(Unrelated[Price])".into());
        s
    }

    fn stored(s: &Session, sheet: usize, table: usize, column: usize) -> &str {
        s.doc().unwrap().wb.sheet(sheet).unwrap().tables[table].columns[column].formula.as_deref().unwrap()
    }

    fn assert_stored_file(s: &Session, table: &str, col: usize, model: &str, native: &str) {
        use std::io::Read;
        let wb = &s.doc().unwrap().wb;
        let (si, ti) = wb.table(table).unwrap();
        assert_eq!(stored(s, si, ti, col), model);
        let bytes = gridcraft_xlsx::write_xlsx(wb).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        let mut found = false;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).unwrap();
            if !entry.name().starts_with("xl/tables/table") {
                continue;
            }
            let mut xml = String::new();
            entry.read_to_string(&mut xml).unwrap();
            if xml.contains(&format!("name=\"{table}\"")) {
                assert!(xml.contains(&format!("<calculatedColumnFormula>{native}</calculatedColumnFormula>")), "{xml}");
                found = true;
            }
        }
        assert!(found, "missing native table {table}");
        let (back, _) = gridcraft_xlsx::read_xlsx(&bytes).unwrap();
        let (si, ti) = back.table(table).unwrap();
        assert_eq!(back.sheet(si).unwrap().tables[ti].columns[col].formula.as_deref(), Some(model));
    }

    fn assert_legacy_cell(s: &Session, expected: &str) {
        let sh = s.doc().unwrap().wb.sheet(0).unwrap();
        let cell = sh.cell(gridcraft_core::CellRef::new(0, 13)).unwrap();
        let f = cell.formula.as_ref().unwrap();
        assert_eq!(f.text, expected);
        assert_eq!(f.eval_mode, gridcraft_model::FormulaEvalMode::Legacy);
        assert_eq!(cell.value, Value::Number(12.0));
    }

    #[test]
    fn header_rename_updates_stored_calculated_column_formulas() {
        let mut s = calculated_columns();
        let untouched = s.doc().unwrap().wb.sheets[2].clone();
        s.execute("cell.set", json!({"cell":"B1","input":"Amount"})).unwrap();
        assert_stored_file(
            &s,
            "Orders",
            1,
            "INDEX(Prices[Amount],MATCH(Orders[@Item],Prices[Sku],0))",
            "INDEX(Prices[Amount],MATCH(Orders[[#This Row],[Item]],Prices[Sku],0))",
        );
        assert_stored_file(&s, "Prices", 2, "[@Amount]*2", "[[#This Row],[Amount]]*2");
        assert_eq!(stored(&s, 0, 0, 3), "SUM(Prices[[Sku]:[Amount]])");
        assert_eq!(stored(&s, 1, 0, 1), "SUM(prices[Amount])");
        assert_eq!(stored(&s, 0, 1, 2), "[@Price]*2");
        assert_eq!(stored(&s, 1, 0, 2), "[@Price]*2");
        assert_eq!(stored(&s, 0, 1, 3), "Prices[");
        assert_eq!(stored(&s, 0, 1, 4), " [@Other] + 1 ");
        assert!(std::sync::Arc::ptr_eq(&untouched, &s.doc().unwrap().wb.sheets[2]));
        assert_legacy_cell(&s, "SUM(Prices[Amount])");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(stored(&s, 0, 0, 2), "[@Price]*2");
        assert_eq!(stored(&s, 0, 1, 1), "INDEX(Prices[Price],MATCH(Orders[@Item],Prices[Sku],0))");
        assert_legacy_cell(&s, "SUM(Prices[Price])");
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(stored(&s, 0, 0, 2), "[@Amount]*2");
        assert_legacy_cell(&s, "SUM(Prices[Amount])");
        let mut s = calculated_columns();
        s.execute("cell.set", json!({"cell":"A1","input":"Code"})).unwrap();
        assert_eq!(stored(&s, 0, 0, 3), "SUM(Prices[[Code]:[Price]])");
    }

    #[test]
    fn table_rename_updates_stored_calculated_column_formulas() {
        let mut s = calculated_columns();
        let untouched = s.doc().unwrap().wb.sheets[2].clone();
        s.execute("table.rename", json!({"table":"Prices","name":"PriceList"})).unwrap();
        assert_stored_file(
            &s,
            "Orders",
            1,
            "INDEX(PriceList[Price],MATCH(Orders[@Item],PriceList[Sku],0))",
            "INDEX(PriceList[Price],MATCH(Orders[[#This Row],[Item]],PriceList[Sku],0))",
        );
        assert_eq!(stored(&s, 0, 0, 2), "[@Price]*2");
        assert_eq!(stored(&s, 1, 0, 1), "SUM(PriceList[price])");
        assert_eq!(stored(&s, 0, 1, 3), "Prices[");
        assert_eq!(stored(&s, 0, 1, 4), " [@Other] + 1 ");
        assert!(std::sync::Arc::ptr_eq(&untouched, &s.doc().unwrap().wb.sheets[2]));
        assert_legacy_cell(&s, "SUM(PriceList[Price])");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.doc().unwrap().wb.sheet(0).unwrap().tables[0].name, "Prices");
        assert_eq!(stored(&s, 0, 1, 1), "INDEX(Prices[Price],MATCH(Orders[@Item],Prices[Sku],0))");
        assert_legacy_cell(&s, "SUM(Prices[Price])");
        s.execute("edit.redo", json!({})).unwrap();
        assert_legacy_cell(&s, "SUM(PriceList[Price])");
        assert_eq!(stored(&s, 0, 1, 1), "INDEX(PriceList[Price],MATCH(Orders[@Item],PriceList[Sku],0))");
        s.execute("table.rename", json!({"table":"Orders","name":"Sales"})).unwrap();
        assert_eq!(stored(&s, 0, 1, 1), "INDEX(PriceList[Price],MATCH(Sales[@Item],PriceList[Sku],0))");
    }

    #[test]
    fn stored_calculated_column_rename_preserves_names_strings_and_whole_table_refs() {
        for (before, after) in [
            ("ROWS(Prices[])+[@Item]", "ROWS(PriceList[])+[@Item]"),
            ("LET(Prices,2,Prices)+ROWS(Prices[])", "LET(Prices,2,Prices)+ROWS(PriceList[])"),
            ("Prices", "Prices"),
            ("\"Prices[Price]\"", "\"Prices[Price]\""),
            ("SUM(Orders[Cost])+SUM(Prices[Price])", "SUM(Orders[Cost])+SUM(PriceList[Price])"),
        ] {
            let mut s = calculated_columns();
            std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().wb).sheet_mut(0).unwrap().tables[1].columns[1].formula = Some(before.into());
            s.execute("table.rename", json!({"table":"Prices","name":"PriceList"})).unwrap();
            assert_eq!(stored(&s, 0, 1, 1), after);
        }
    }

    fn native_calculated_columns(protected: bool) -> Session {
        use gridcraft_core::{CellRef, RangeRef};
        use gridcraft_model::{Cell, Formula, WhatIfTable};
        let mut s = calculated_columns();
        let wb = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().wb);
        let sh = wb.sheet_mut(0).unwrap();
        sh.cells.set(
            CellRef::new(0, 19),
            if protected {
                Cell { formula: Some(std::sync::Arc::new(Formula::new("SUM(Prices[Price])"))), value: Value::Number(12.0), ..Default::default() }
            } else {
                Cell::value(Value::Number(12.0))
            },
        );
        sh.cells.set(CellRef::new(9, 25), Cell::value(Value::Number(3.0)));
        sh.cells.set(CellRef::new(1, 19), Cell::value(Value::Number(6.0)));
        sh.cells.set(CellRef::new(2, 20), Cell::value(Value::Number(8.0)));
        sh.what_if_tables.push(WhatIfTable {
            anchor: CellRef::new(1, 19),
            range: RangeRef::new(CellRef::new(1, 19), CellRef::new(2, 20)),
            formula_text: "TABLE(Z10,)".into(),
            input1: Some(CellRef::new(9, 25)),
            input2: None,
            two_variable: Some(false),
            row_oriented: None,
            input1_deleted: Some(false),
            input2_deleted: None,
            calculate: Some(true),
        });
        let bytes = gridcraft_xlsx::write_xlsx(wb).unwrap();
        let (back, _) = crate::io::open_bytes("synthetic.xlsx", &bytes).unwrap();
        let mut imported = Session::new();
        imported.add_document(crate::DocState::new(back, None, "synthetic".into(), crate::Trust::FromFile));
        imported
    }

    #[test]
    fn calculated_column_renames_preserve_disjoint_native_whatif_tables() {
        for (cmd, params) in [("cell.set", json!({"cell":"B1","input":"Amount"})), ("table.rename", json!({"table":"Prices","name":"PriceList"}))] {
            let mut s = native_calculated_columns(false);
            let definitions = s.doc().unwrap().wb.sheet(0).unwrap().what_if_tables.clone();
            s.execute(cmd, params).unwrap();
            assert_eq!(s.doc().unwrap().wb.sheet(0).unwrap().what_if_tables, definitions);
            let bytes = crate::io::save_bytes(&s.doc().unwrap().wb, "synthetic.xlsx").unwrap();
            let (back, report) = crate::io::open_bytes_with_report("synthetic.xlsx", &bytes, None, false).unwrap();
            let sh = back.sheet(0).unwrap();
            assert_eq!(sh.what_if_tables, definitions);
            assert_eq!(sh.value(gridcraft_core::CellRef::new(1, 19)), Value::Number(6.0));
            assert_eq!(sh.value(gridcraft_core::CellRef::new(2, 20)), Value::Number(8.0));
            assert!(report.issues.iter().any(|i| i.feature == "what-if data table" && i.effect == gridcraft_model::CompatibilityEffect::Static));
        }
    }

    #[test]
    fn calculated_column_renames_refuse_protected_native_headers_atomically() {
        for (cmd, params) in [("cell.set", json!({"cell":"B1","input":"Amount"})), ("table.rename", json!({"table":"Prices","name":"PriceList"}))] {
            let mut s = native_calculated_columns(true);
            s.execute("cell.set", json!({"cell":"Z20","input":"1"})).unwrap();
            s.execute("edit.undo", json!({})).unwrap();
            let d = s.doc().unwrap();
            let before = d.wb.clone();
            let selection = d.selection.clone();
            let history = (d.undo.len(), d.redo.len());
            let dirty = d.is_dirty();
            let error = s.execute(cmd, params).unwrap_err();
            assert!(error.to_string().contains("what-if"), "{error}");
            let d = s.doc().unwrap();
            assert_eq!(&*d.wb, &*before);
            assert!(std::sync::Arc::ptr_eq(&d.wb, &before));
            assert_eq!(d.selection, selection);
            assert_eq!((d.undo.len(), d.redo.len()), history);
            assert_eq!(d.is_dirty(), dirty);
        }
    }

    #[test]
    fn editing_a_header_renames_the_column() {
        let mut s = Session::new();
        s.new_workbook();
        s.execute("range.setValues", json!({"range": "A1", "values": [["a", "b", "c"], [1, 2, 3]]})).unwrap();
        s.execute("insert.table", json!({"range": "A1:C2", "header": true})).unwrap();
        s.execute("cell.set", json!({"cell": "E1", "input": "=SUM(Table1[a])"})).unwrap();
        s.execute("cell.set", json!({"cell": "A1", "input": "zz"})).unwrap();
        s.execute("edit.clearContents", json!({"range": "B1"})).unwrap();
        s.execute("cell.set", json!({"cell": "C1", "input": "ZZ"})).unwrap();
        let get = |s: &mut Session, a: &str| s.execute("cell.get", json!({"cell": a})).unwrap();
        assert_eq!(get(&mut s, "E1")["formula"], "=SUM(Table1[zz])");
        assert_eq!(get(&mut s, "E1")["value"], json!(1.0));
        let d = s.doc().unwrap();
        let sh = d.wb.sheet(0).unwrap();
        let names: Vec<&str> = sh.tables[0].columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["zz", "Column2", "ZZ2"]);
        assert_eq!(sh.value(gridcraft_core::CellRef::new(0, 1)), Value::from("Column2"));
        assert_eq!(sh.value(gridcraft_core::CellRef::new(0, 2)), Value::from("ZZ2"));
        s.execute("range.setValues", json!({"range": "A1", "values": [["x", "x", 5]]})).unwrap();
        let sh = s.doc().unwrap().wb.sheet(0).unwrap();
        let names: Vec<&str> = sh.tables[0].columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["x", "x2", "5"]);
        assert_eq!(sh.value(gridcraft_core::CellRef::new(0, 2)), Value::from("5"));
    }

    #[test]
    fn go_to_a_table_selects_its_data() {
        let mut s = Session::new();
        s.new_workbook();
        s.execute("range.setValues", json!({"range": "A1", "values": [["a", "b"], [1, 2], [3, 4]]})).unwrap();
        s.execute("insert.table", json!({"range": "A1:B3", "header": true})).unwrap();
        assert_eq!(s.execute("edit.goTo", json!({"reference": "Table1"})).unwrap()["selection"], "A2:B3");
    }
}

#[cfg(test)]
mod style_tests {
    use super::*;

    #[test]
    fn zero_numbered_styles_do_not_underflow() {
        assert_eq!(parse_style_name("TableStyleLight0"), ("Light", 1));
        assert_eq!(parse_style_name("TableStyleMedium0"), ("Medium", 1));
        for name in ["TableStyleLight0", "TableStyleMedium0", "TableStyleDark0"] {
            let _ = look(name);
        }
    }
}
