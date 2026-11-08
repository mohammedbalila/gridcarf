use super::*;
fn workbook_fields(source: &Workbook, actual: &ValidatedRecovery) {
    macro_rules! book { ($($field:ident),+)=>{$(assert_eq!(actual.workbook.$field,source.$field,stringify!($field));)+}; }
    book!(
        names,
        view,
        theme,
        date_system,
        calc,
        show_objects,
        remove_personal_info,
        props,
        protected_structure,
        structure_password,
        custom_lists,
        cell_styles,
        phonetics,
        vba_project,
        csv_delimiter,
        macros,
        macros_read,
        macros_unread,
        preserved,
        source_dir,
        source_path
    );
}

pub(super) fn same(source: &Workbook, metadata: &Metadata, actual: &ValidatedRecovery) {
    workbook_fields(source, actual);
    assert_eq!(actual.workbook.active_sheet(), source.active_sheet());
    assert_eq!(actual.workbook.styles.len(), source.styles.len());
    for i in 0..source.styles.len() {
        assert_eq!(actual.workbook.styles.get(gridcraft_model::StyleId(i as u32)), source.styles.get(gridcraft_model::StyleId(i as u32)));
    }
    assert_eq!(actual.workbook.sheets.len(), source.sheets.len());
    for (a, b) in actual.workbook.sheets.iter().zip(&source.sheets) {
        assert_ne!(a.uid.get(), b.uid.get());
        assert_eq!(a.as_ref(), b.as_ref());
    }
    metadata_fields(metadata, actual);
}

fn metadata_fields(metadata: &Metadata, actual: &ValidatedRecovery) {
    assert_eq!(serde_json::to_vec(&actual.metadata).unwrap(), serde_json::to_vec(metadata).unwrap());
    macro_rules! meta {($($field:ident),+)=>{$(assert_eq!(actual.metadata.$field,metadata.$field,stringify!($field));)+};}
    meta!(title, dirty_at_capture, import_issues, selection, sheet_selections, provenance);
    assert_eq!(actual.metadata.file_kind.map(|v| std::mem::discriminant(&v)), metadata.file_kind.map(|v| std::mem::discriminant(&v)));
    match (&actual.metadata.original_path_hint, &metadata.original_path_hint) {
        (None, None) => {}
        (Some(NativePath::Unix(a)), Some(NativePath::Unix(b))) => assert_eq!(a, b),
        (Some(NativePath::Windows(a)), Some(NativePath::Windows(b))) => assert_eq!(a, b),
        _ => panic!("native hint tag"),
    }
    assert_eq!(actual.metadata.title, metadata.title);
    assert_eq!(actual.id(), metadata.id);
    assert_eq!(actual.sequence(), metadata.sequence);
    assert_eq!(actual.captured_revision(), metadata.captured_revision);
    assert_eq!(actual.metadata.selection, metadata.selection);
    assert_eq!(actual.metadata.sheet_selections, metadata.sheet_selections);
}

pub(super) fn ordinary(actual: &ValidatedRecovery) {
    let w = &actual.workbook;
    let s = &w.sheets[0];
    let gridcraft_model::CfRule::Expression { style, .. } = &s.cond_formats[0].rule else { panic!("CF") };
    let f32s = [
        w.styles.get(gridcraft_model::StyleId(1)).font.size,
        w.styles.get(gridcraft_model::StyleId(2)).font.size,
        w.cell_styles[0].1.font.size,
        style.font.size,
        s.cols[&16383].size.unwrap(),
        s.charts[0].anchor.height,
        s.images[0].anchor.height,
        s.shapes[0].anchor.height,
        s.slicers[0].anchor.height,
        s.controls[0].anchor.height,
        s.shapes[0].format.font_size.unwrap(),
        s.shapes[0].format.text_outline.unwrap().1,
        s.charts[0].series[0].trendlines[0].width.unwrap(),
        s.print.margins[0],
    ];
    assert_eq!(f32s.map(f32::to_bits), [0x80000000; 14]);
    let Value::Number(python) = s.cells.get(CellRef::new(1, 0)).unwrap().py.as_ref().unwrap().output.as_ref().unwrap().value else {
        panic!("Python")
    };
    let Value::Array(array) = &s.cells.get(CellRef::default()).unwrap().value else { panic!("array") };
    let Value::Number(array) = array.data[0] else { panic!("number") };
    let Value::Number(edge) = s.cells.get(CellRef::new(1048575, 16383)).unwrap().value else { panic!("edge") };
    assert_eq!(
        [
            w.calc.max_change,
            s.comments[&CellRef::new(1, 1)].note_box.unwrap().size_pt.unwrap().0,
            s.charts[0].series[0].trendlines[0].intercept.unwrap(),
            python,
            array,
            edge
        ]
        .map(f64::to_bits),
        [0x8000000000000000; 6]
    );
    let formula = s.cells.get(CellRef::new(6, 0)).unwrap();
    assert_eq!(formula.value, Value::Number(42.0));
    assert_eq!(formula.formula.as_ref().unwrap().text, "RAND()");
    assert!(formula.formula.as_ref().unwrap().expr.is_some());
    assert_eq!(s.cells.get(CellRef::new(64, 0)).unwrap().style, gridcraft_model::StyleId(2));
    assert_eq!(s.cells.iter().count(), 23);
    assert_eq!(s.cells.get(CellRef::new(1, 0)).unwrap().py.as_ref().unwrap().code, "x\r\n\"😀\"");
    let gridcraft_model::CfRule::DataBar { min: gridcraft_model::CfValueKind::Number(bar), .. } = &s.cond_formats[15].rule else { panic!("bar") };
    let gridcraft_model::CfRule::ColorScale { stops } = &s.cond_formats[18].rule else { panic!("scale") };
    let gridcraft_model::CfValueKind::Number(scale) = stops[0].0 else { panic!("stop") };
    let gridcraft_model::StepKind::If { value, .. } = &w.macros[0].steps[2].kind else { panic!("if") };
    assert_eq!([*bar, scale, value.as_f64().unwrap()].map(f64::to_bits), [0x8000000000000000; 3]);
    assert_eq!(s.controls[1].anchor.height.to_bits(), 0);
    assert!(w.macros_read.is_empty());
    assert_eq!(w.macros.len(), 2);
    assert_eq!(w.macros_unread[0]["v"].as_u64(), Some(u64::MAX));
}
