use super::*;
use crate::*;
use gridcraft_core::{Array, CellRef, MAX_COLS, MAX_ROWS, RangeRef, Value};
use std::sync::Arc;

fn rect(r0: u32, c0: u32, r1: u32, c1: u32) -> RangeRef {
    RangeRef { start: CellRef::new(r0, c0), end: CellRef::new(r1, c1) }
}

#[test]
fn recovery_metadata_print_retains_inert_fields_fit_zero_and_break_order() {
    let mut wb = Workbook::new();
    let print = &mut wb.sheet_mut(0).unwrap().print;
    *print = PrintSettings {
        orientation: Orientation::Landscape,
        paper: "unknown paper".into(),
        margins: [-0.0, f32::MAX, 0.0, 1.0, 2.0, 3.0],
        print_area: Some(RangeRef::all()),
        title_rows: Some((0, MAX_ROWS - 1)),
        title_cols: Some((0, MAX_COLS - 1)),
        scale: 0,
        fit_to: Some((0, u16::MAX)),
        gridlines: true,
        headings: true,
        center_h: true,
        center_v: true,
        header: "&arbitrary\0".into(),
        footer: String::new(),
        row_breaks: vec![MAX_ROWS - 1, 1, 1],
        col_breaks: vec![MAX_COLS - 1, 1, 1],
        different_first: true,
        different_odd_even: true,
        first_header: "first".into(),
        first_footer: "first footer".into(),
        even_header: "even".into(),
        even_footer: "even footer".into(),
        hf_scale_with_doc: false,
        hf_align_with_margins: false,
        page_order: PageOrder::OverThenDown,
    };
    let before = wb.clone();
    assert_eq!(validate_workbook_metadata(&wb), Ok(()));
    assert_eq!(wb, before);
    assert_eq!(wb.sheets[0].print.margins[0].to_bits(), (-0.0f32).to_bits());
    for scale in [0, 1, 9, 400, u16::MAX] {
        wb.sheet_mut(0).unwrap().print.scale = scale;
        assert_eq!(validate_workbook_metadata(&wb), Ok(()));
    }
    for fit in [None, Some((0, 0)), Some((u16::MAX, 0))] {
        wb.sheet_mut(0).unwrap().print.fit_to = fit;
        assert_eq!(validate_workbook_metadata(&wb), Ok(()));
    }
}

#[test]
fn recovery_metadata_print_checks_actual_axes_ranges_and_all_six_margins() {
    for field in 0..6 {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0] {
            let mut wb = Workbook::new();
            wb.sheet_mut(0).unwrap().print.margins[field] = bad;
            assert_eq!(validate_workbook_metadata(&wb), Err(RecoveryModelError::Print));
            assert_eq!(wb.sheets[0].print.margins[field].to_bits(), bad.to_bits());
        }
    }
    for row_axis in [true, false] {
        let limit = if row_axis { MAX_ROWS } else { MAX_COLS };
        for bad in [0, limit, u32::MAX] {
            let mut wb = Workbook::new();
            let p = &mut wb.sheet_mut(0).unwrap().print;
            if row_axis {
                p.row_breaks.push(bad);
            } else {
                p.col_breaks.push(bad);
            }
            assert_eq!(validate_workbook_metadata(&wb), Err(RecoveryModelError::Print));
        }
        for bad in [(1, 0), (0, limit), (u32::MAX, u32::MAX)] {
            let mut wb = Workbook::new();
            let p = &mut wb.sheet_mut(0).unwrap().print;
            if row_axis {
                p.title_rows = Some(bad);
            } else {
                p.title_cols = Some(bad);
            }
            assert_eq!(validate_workbook_metadata(&wb), Err(RecoveryModelError::Print));
        }
    }
    for bad in [rect(1, 0, 0, 0), rect(0, 0, MAX_ROWS, 0)] {
        let mut wb = Workbook::new();
        wb.sheet_mut(0).unwrap().print.print_area = Some(bad);
        assert!(validate_workbook_metadata(&wb).is_err());
    }
}

#[test]
fn recovery_metadata_passwords_and_empty_overlapping_edit_ranges_are_inert() {
    for password in [
        PasswordHash::Legacy(u16::MAX),
        PasswordHash::Iterated { algorithm: "future\0".into(), hash: "not base64".into(), salt: "opaque".into(), spin_count: u32::MAX },
    ] {
        let mut wb = Workbook::new();
        wb.protected_structure = true;
        wb.structure_password = Some(password.clone());
        let sheet = wb.sheet_mut(0).unwrap();
        sheet.protection = Some(SheetProtection {
            password: Some(password.clone()),
            legacy_password_hash: Some(u64::MAX),
            select_locked: true,
            select_unlocked: true,
            format_cells: true,
            format_columns: true,
            format_rows: true,
            insert_columns: true,
            insert_rows: true,
            delete_columns: true,
            delete_rows: true,
            sort: true,
            autofilter: true,
        });
        sheet.edit_ranges = vec![
            EditRange { title: "".into(), ranges: vec![], password: None, security_descriptor: Some("opaque ACL".into()) },
            EditRange { title: "".into(), ranges: vec![RangeRef::all(), RangeRef::all()], password: Some(password), security_descriptor: None },
        ];
        let before = wb.clone();
        assert_eq!(validate_workbook_metadata(&wb), Ok(()));
        assert_eq!(wb, before);
        wb.sheet_mut(0).unwrap().edit_ranges[1].ranges[1] = rect(1, 1, 0, 0);
        assert_eq!(validate_workbook_metadata(&wb), Err(RecoveryModelError::Range));
    }
}

#[test]
fn recovery_metadata_phonetics_use_exact_identity_utf16_units_and_preserve_properties() {
    let mut wb = Workbook::new();
    wb.phonetics = vec![
        PhoneticString {
            text: "A😀B".into(),
            runs: vec![PhoneticRun { start: 2, end: 3, text: String::new() }, PhoneticRun { start: 1, end: 4, text: "overlap".into() }],
            props: Some(PhoneticProps { font_id: u32::MAX, kind: Some("future".into()), alignment: Some("unknown".into()) }),
        },
        PhoneticString { text: "a😀B".into(), runs: vec![], props: Some(PhoneticProps::default()) },
        PhoneticString { text: "".into(), runs: vec![], props: None },
    ];
    let before = wb.clone();
    assert_eq!(validate_workbook_metadata(&wb), Ok(()));
    assert_eq!(wb, before);
    wb.phonetics.push(wb.phonetics[0].clone());
    assert_eq!(validate_workbook_metadata(&wb), Err(RecoveryModelError::UnsupportedRepresentation(RecoveryRepresentation::AmbiguousIdentity)));
    wb.phonetics.pop();
    for (start, end) in [(0, 0), (3, 2), (0, 5), (u32::MAX, u32::MAX)] {
        wb.phonetics[0].runs[0].start = start;
        wb.phonetics[0].runs[0].end = end;
        assert_eq!(validate_workbook_metadata(&wb), Err(RecoveryModelError::Phonetic));
    }
}

#[test]
fn recovery_metadata_python_count_matches_existing_formula_encoder_at_boundaries() {
    let mut cases = vec!["".into(), "\r\nx\r".into(), "\"".repeat(1200) + &"y".repeat(5800)];
    for count in [1, 254, 255, 256, 510, 511, 4000, 8078, 8079, 8080, 8192, 8193] {
        for c in ['x', '😀', '"'] {
            cases.push(c.to_string().repeat(count));
        }
    }
    for code in cases {
        for object in [false, true] {
            let py = PyCode { code: code.clone(), object, output: None, stale: true };
            assert_eq!(python_code_fits(&py.code), py.fits(), "{} chars, object {object}", py.code.chars().count());
            assert_eq!(validate_python(&py).is_ok(), py.fits());
            assert_eq!(py.code, code);
        }
    }
}

#[test]
fn recovery_metadata_python_checks_outputs_without_result_or_execution_helpers() {
    for object in [false, true] {
        for stale in [false, true] {
            for convertible in [false, true] {
                let array = Arc::new(Array { rows: 1, cols: 2, data: vec![Value::Number(-0.0), Value::from("cached")] });
                let py = PyCode {
                    code: "unparseable(\r\n".into(),
                    object,
                    stale,
                    output: Some(PyOutput {
                        value: Value::Array(array.clone()),
                        type_name: "future type".into(),
                        preview: "arbitrary\0".into(),
                        convertible,
                    }),
                };
                assert_eq!(validate_python(&py), Ok(()));
                let Value::Array(after) = &py.output.as_ref().unwrap().value else { panic!("array") };
                assert!(Arc::ptr_eq(after, &array));
                assert_eq!(Arc::strong_count(&array), 2);
                let Value::Number(n) = after.data[0] else { panic!("number") };
                assert_eq!(n.to_bits(), (-0.0f64).to_bits());
            }
        }
    }
    let limit = crate::python::MAX_RESULT_CELLS;
    let mut py = PyCode {
        output: Some(PyOutput {
            value: Value::Array(Arc::new(Array { rows: 1, cols: limit, data: vec![Value::Empty; limit] })),
            ..PyOutput::default()
        }),
        ..PyCode::default()
    };
    assert_eq!(validate_python(&py), Ok(()));
    py.output.as_mut().unwrap().value = Value::Array(Arc::new(Array { rows: 1, cols: limit + 1, data: vec![Value::Empty; limit + 1] }));
    assert_eq!(validate_python(&py), Err(RecoveryModelError::Python));
    for value in [
        Value::Number(f64::INFINITY),
        Value::Array(Arc::new(Array { rows: 0, cols: 1, data: vec![] })),
        Value::Array(Arc::new(Array::scalar(Value::Array(Arc::new(Array::scalar(Value::Empty)))))),
    ] {
        py.output.as_mut().unwrap().value = value;
        assert!(validate_python(&py).is_err());
    }
}
