use std::sync::Arc;

use gridcraft_core::{CellRef, Value};
use gridcraft_engine::Session;
use gridcraft_model::{CfRule, CondFormat, Hyperlink, LineInfo, Style, Validation};
use serde_json::json;

#[test]
fn feature_allocations_are_charged() {
    for feature in ["styles", "rows", "columns", "charts", "validation", "conditional formats", "spill", "hyperlinks", "workbook strings"] {
        let mut s = Session::new();
        s.new_workbook();
        let wb = Arc::make_mut(&mut s.doc_mut().unwrap().wb);
        let payload = "x".repeat(32 * 1024);
        match feature {
            "styles" => {
                wb.styles.intern(Style { font: gridcraft_model::Font { name: payload, ..Default::default() }, ..Default::default() });
            }
            "workbook strings" => wb.props.description = payload,
            _ => {
                let sh = wb.sheet_mut(0).unwrap();
                match feature {
                    "rows" => sh.rows.extend((0..2048).map(|r| (r, LineInfo::default()))),
                    "columns" => sh.cols.extend((0..2048).map(|r| (r, LineInfo::default()))),
                    "charts" => sh.charts.push(
                        serde_json::from_value(json!({
                            "id":1,"kind":"ColumnClustered","anchor":{"cell":{"row":0,"col":0},"dx":0,"dy":0,"width":100,"height":100},
                            "title":payload,"series":[],"legend":"Right","data_labels":false,"gridlines":false,"style":0,
                            "hole_size":75,"first_slice_angle":0,"by_rows":false
                        }))
                        .unwrap(),
                    ),
                    "validation" => sh.validations.push(Validation { input_message: payload, ..Default::default() }),
                    "conditional formats" => sh.cond_formats.push(CondFormat {
                        ranges: vec![],
                        rule: CfRule::Expression { formula: payload, style: Box::default() },
                        priority: 0,
                        stop_if_true: false,
                    }),
                    "spill" => {
                        sh.spill.extend((0..2048).map(|r| (CellRef::new(r, 0), Value::Number(r as f64))));
                    }
                    "hyperlinks" => {
                        sh.hyperlinks.insert(CellRef::new(0, 0), Hyperlink { target: payload, tooltip: None, external: true });
                    }
                    _ => unreachable!(),
                }
            }
        }
        // A real command snapshots the populated model. Non-shared metadata is cloned;
        // sharing the live notes and cell values must not hide these owned allocations.
        s.execute("cell.set", json!({"cell":"B1", "input":"1"})).unwrap();
        let first = s.doc().unwrap().undo.last().unwrap().retained_bytes;
        assert!(first >= 32 * 1024, "{feature}: only {first} bytes charged");
    }
}

#[test]
fn shared_payload_charged_once() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("review.newNote", json!({"cell":"A1","text":"x".repeat(256 * 1024)})).unwrap();
    s.doc_mut().unwrap().undo.clear();
    for i in 0..10 {
        s.execute("cell.set", json!({"cell":"B1","input":i.to_string()})).unwrap();
    }
    let d = s.doc().unwrap();
    assert_eq!(d.undo.len(), 10);
    assert!(d.undo.iter().map(|h| h.retained_bytes).sum::<usize>() < 256 * 1024);
}
