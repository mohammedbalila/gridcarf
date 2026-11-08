use super::super::{RecoveryLimits, duplicates};
use super::*;

fn wire(pairs: &str) -> String {
    format!("{{\"workbook\":{{\"sheets\":[{{\"cell_pictures\":{pairs}}}]}}}}")
}

#[test]
fn pair_shape_coordinate_primitives_and_duplicates_are_checked_before_map_collapse() {
    let limits = RecoveryLimits::default();
    for pairs in [
        r#"[[{"row":0,"col":0},{}],[{"col":0,"row":0},{}]]"#,
        r#"[[]]"#,
        r#"[[{"row":0,"col":0}]]"#,
        r#"[[{"row":0,"col":0},{},null]]"#,
        r#"[[null,{}]]"#,
        r#"[[{"row":0},{}]]"#,
        r#"[[{"row":-1,"col":0},{}]]"#,
        r#"[[{"row":1.0,"col":0},{}]]"#,
        r#"[[{"row":4294967296,"col":0},{}]]"#,
        r#"[[{"row":1048576,"col":0},{}]]"#,
        r#"[[{"row":0,"col":16384},{}]]"#,
        r#"[[{"row":0,"row":1,"col":0},{}]]"#,
        r#"[[{"row":0,"col":0},[]]]"#,
    ] {
        assert!(duplicates::audit(wire(pairs).as_bytes(), &limits).is_err(), "{pairs}");
    }
    let accepted =
        r#"[[{"col":0,"r\u006fw":0},{"data":"AQ==","mime":"x","alt":"same"}],[{"row":0,"col":1},{"data":"AQ==","mime":"x","alt":"same"}]]"#;
    let result = duplicates::audit(wire(accepted).as_bytes(), &limits).unwrap();
    assert_eq!(result.pictures, 2);
    assert_eq!(result.binary_decoded, 2);
    assert_eq!((result.requests.current, result.requests.actual_current), (0, 0));
    let cross = r#"{"workbook":{"sheets":[{"cell_pictures":[[{"row":0,"col":0},{}]]},{"cell_pictures":[[{"row":0,"col":0},{}]]}]}}"#;
    assert_eq!(duplicates::audit(cross.as_bytes(), &limits).unwrap().pictures, 2);
}

#[test]
fn coordinate_storage_uses_actual_counts_not_source_picture_node_lower_bound() {
    let mut coordinates = Coordinates::default();
    let mut requests = super::super::scratch::Requests::default();
    let limits = RecoveryLimits { nodes: 1, ..RecoveryLimits::default() };
    coordinates.push(gridcraft_core::CellRef::default(), &limits, &mut requests).unwrap();
    let calls = requests.calls;
    assert!(coordinates.push(gridcraft_core::CellRef::new(0, 1), &limits, &mut requests).is_err());
    assert_eq!(requests.calls, calls);
    coordinates.release(&mut requests);
    assert_eq!((requests.current, requests.actual_current), (0, 0));
}

#[test]
fn actual_picture_nodes_not_stored_cells_bound_minimal_raw_pairs() {
    let raw = wire(r#"[[{"row":1048575,"col":16383},{}],[{"row":0,"col":0},{}]]"#);
    let observed = super::super::lexical_audit(raw.as_bytes(), &RecoveryLimits::default()).unwrap();
    let limits = RecoveryLimits { nodes: observed.nodes, cells: 0, ..RecoveryLimits::default() };
    assert_eq!(duplicates::audit(raw.as_bytes(), &limits).unwrap().pictures, 2);
    assert!(duplicates::audit(raw.as_bytes(), &RecoveryLimits { nodes: observed.nodes - 1, ..limits }).is_err());
    let unknown_duplicate = wire(r#"[[{"row":0,"col":0,"future":{"x":0,"x":1}},{}]]"#);
    assert!(duplicates::audit(unknown_duplicate.as_bytes(), &limits).is_err());
    assert!(duplicates::audit(br#"{"future":{"cell_pictures":[[null,[]],[null,[]]]}}"#, &RecoveryLimits::default()).is_ok());
}

#[test]
fn coordinate_sort_inventory_and_duplicate_refusal_keep_original_observations() {
    let limits = RecoveryLimits::default();
    let mut coordinates = Coordinates::default();
    let mut requests = super::super::scratch::Requests::default();
    for col in (0..17).rev() {
        coordinates.push(gridcraft_core::CellRef::new(0, col), &limits, &mut requests).unwrap();
    }
    coordinates.check(&limits, &mut requests).unwrap();
    assert_eq!(coordinates.values.len(), 17);
    use std::io::Write;
    writeln!(
        std::io::stdout(),
        "C2_PICTURE_INVENTORY len={} requested={:?} capacity={:?} merge_len={} requests={:?}",
        coordinates.values.len(),
        coordinates.requested,
        [coordinates.values.capacity(), coordinates.merge.capacity()],
        coordinates.merge.len(),
        requests
    )
    .unwrap();
    coordinates.push(gridcraft_core::CellRef::new(0, 16), &limits, &mut requests).unwrap();
    assert!(coordinates.check(&limits, &mut requests).is_err());
    coordinates.release(&mut requests);
    assert_eq!((requests.current, requests.actual_current), (0, 0));
}

#[test]
fn pair_roles_are_local_and_every_extra_child_is_audited() {
    for first in ["[]", "null", "0", "true", "\"A1\""] {
        assert!(duplicates::audit(wire(&format!("[[{first},{{}}]]")).as_bytes(), &RecoveryLimits::default()).is_err());
    }
    for axis in ["null", "true", "\"0\"", "[]", "{}", "-0"] {
        assert!(duplicates::audit(wire(&format!("[[{{\"row\":{axis},\"col\":0}},{{}}]]")).as_bytes(), &RecoveryLimits::default()).is_err());
    }
    let extra = wire(r#"[[{"row":0,"col":0},{},{"x":0,"x":1}]]"#);
    assert!(matches!(duplicates::audit(extra.as_bytes(), &RecoveryLimits::default()), Err(super::super::RecoveryError::Malformed("duplicate_key"))));
    assert!(duplicates::audit(wire("[{}]").as_bytes(), &RecoveryLimits::default()).is_err());
}
