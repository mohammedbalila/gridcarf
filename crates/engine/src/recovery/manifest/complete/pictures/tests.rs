use super::super::tests::{bytes, metadata, picture};
use super::*;
use crate::recovery::decode as decode_recovery;
use gridcraft_model::Sheet;
#[test]
fn complete_global_picture_identity_roundtrips_without_value_interning() {
    for groups in [vec![0, 0], vec![0, 1], vec![0, 1, 0]] {
        let mut source = Workbook::new();
        source.sheets.push(Arc::new(Sheet::new("Other")));
        let a = picture();
        let b = picture();
        for (i, group) in groups.iter().enumerate() {
            Arc::make_mut(&mut source.sheets[i % 2]).cell_pictures.insert(CellRef::new(i as u32, 0), if *group == 0 { a.clone() } else { b.clone() });
        }
        let original = super::super::super::picture_groups(&source, &RecoveryLimits::default()).unwrap();
        let decoded = decode_recovery(&bytes(&source, &metadata())).unwrap();
        assert_eq!(super::super::super::picture_groups(&decoded.workbook, &RecoveryLimits::default()).unwrap(), original);
    }
}

#[test]
fn same_sheet_a_b_a_and_shared_pair_have_exact_reconstructed_groups() {
    for groups in [vec![0, 0], vec![0, 1], vec![0, 1, 0]] {
        let mut source = Workbook::new();
        let a = picture();
        let b = picture();
        for (i, g) in groups.iter().enumerate() {
            Arc::make_mut(&mut source.sheets[0]).cell_pictures.insert(CellRef::new(0, i as u32), if *g == 0 { a.clone() } else { b.clone() });
        }
        let decoded = decode_recovery(&bytes(&source, &metadata())).unwrap();
        assert_eq!(super::super::super::picture_groups(&decoded.workbook, &RecoveryLimits::default()).unwrap(), groups);
    }
}
#[test]
fn invalid_groups_and_each_conflicting_persisted_field_refuse_before_graph() {
    let mut source = Workbook::new();
    let a = picture();
    Arc::make_mut(&mut source.sheets[0]).cell_pictures.insert(CellRef::default(), a.clone());
    Arc::make_mut(&mut source.sheets[0]).cell_pictures.insert(CellRef::new(0, 1), a);
    let encoded = bytes(&source, &metadata());
    for groups in ["[]", "[0]", "[0,0,0]", "[1,1]", "[0,2]", "[0,4294967295]", "[-1,0]", "[-0,0]", "[0.0,0]", "[0,4294967296]", "null"] {
        let bad = super::super::tests::replace_manifest(&encoded, "\"cell_picture_groups\":[0,0]", &format!("\"cell_picture_groups\":{groups}"));
        let mut stats = Stats::default();
        assert!(decode_run(&bad, &RecoveryLimits::default(), &mut stats).is_err());
        assert_eq!(stats.p3_calls, 0);
    }
    for field in 0..3 {
        let mut source = source.clone();
        let mut different = (*picture()).clone();
        match field {
            0 => different.data[19] = 2,
            1 => different.mime = "image/jpeg".into(),
            _ => different.alt = "different".into(),
        }
        Arc::make_mut(&mut source.sheets[0]).cell_pictures.insert(CellRef::new(0, 1), Arc::new(different));
        let raw = super::super::write::manifest_bytes(&source, &metadata(), &RecoveryLimits::default()).unwrap().bytes;
        let mut bad = Vec::new();
        frame::Framing::new(&raw, std::iter::empty(), &RecoveryLimits::default()).unwrap().write_to(&mut bad).unwrap();
        let bad = super::super::tests::replace_manifest(&bad, "\"cell_picture_groups\":[0,1]", "\"cell_picture_groups\":[0,0]");
        let mut stats = Stats::default();
        assert!(matches!(decode_run(&bad, &RecoveryLimits::default(), &mut stats), Err(RecoveryError::Malformed("picture_groups"))));
        assert_eq!((stats.p3_calls, stats.picture_assignments), (0, 0));
    }
}

#[test]
fn retained_native_ancestor_witness_has_prepare_decode_and_real_xlsx_parity() {
    use gridcraft_model::{Cell, PreservedPart, PreservedRel};
    let mut source = Workbook::new();
    let p = picture();
    let sheet = Arc::make_mut(&mut source.sheets[0]);
    for col in 0..2 {
        sheet.cell_pictures.insert(CellRef::new(0, col), p.clone());
        sheet.cells.set(CellRef::new(0, col), Cell::value(Value::Number(1.0)));
    }
    let name = "xl/media/cellImage2.png/child.bin";
    source.preserved = Some(Arc::new(Preserved {
        rels: vec![PreservedRel {
            id: "rIdKept".into(),
            rel_type: "http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml".into(),
            target: name.into(),
            external: false,
        }],
        parts: vec![PreservedPart {
            name: name.into(),
            content_type: "application/octet-stream".into(),
            data: Arc::from(&b"retained-witness"[..]),
            rels: vec![],
        }],
        ..Default::default()
    }));
    let input = bytes(&source, &metadata());
    let out = decode_recovery(&input).unwrap();
    assert_eq!(bytes(&out.workbook, &out.metadata), input);
    assert_eq!(gridcraft_xlsx::validate_recovery_preserved(&out.workbook), Ok(()));
    let xlsx = gridcraft_xlsx::write_xlsx(&out.workbook).unwrap();
    let (read, _) = gridcraft_xlsx::read_xlsx(&xlsx).unwrap();
    assert!(read.preserved.as_ref().unwrap().parts.iter().any(|p| p.data.as_ref() == b"retained-witness"));
    assert_eq!(read.sheets[0].cell_pictures.len(), 2);
    let mut distinct = source.clone();
    Arc::make_mut(&mut distinct.sheets[0]).cell_pictures.insert(CellRef::new(0, 1), picture());
    assert!(matches!(
        crate::recovery::RecoveryCapture { workbook: &distinct, metadata: &metadata() }.prepare(),
        Err(RecoveryError::Malformed("preserved_output_name"))
    ));
    let bad = super::super::tests::replace_manifest(&input, "\"cell_picture_groups\":[0,0]", "\"cell_picture_groups\":[0,1]");
    assert!(matches!(decode_recovery(&bad), Err(RecoveryError::Malformed("preserved_output_name"))));
}

#[test]
fn reconstructed_first_reference_inventory_is_measured_and_small_bound_refuses_before_it() {
    let mut source = Workbook::new();
    let a = picture();
    Arc::make_mut(&mut source.sheets[0]).cell_pictures.insert(CellRef::default(), a.clone());
    Arc::make_mut(&mut source.sheets[0]).cell_pictures.insert(CellRef::new(0, 1), a);
    let mut stats = Stats::default();
    let out = decode_run(&bytes(&source, &metadata()), &RecoveryLimits::default(), &mut stats).unwrap();
    assert_eq!((stats.picture_groups, stats.picture_assignments), (1, 2));
    assert_eq!((stats.vector_request, stats.vector_capacity), (16, 16));
    let mut probe = source.clone();
    let mut rejected = Stats::default();
    assert!(restore(&mut probe, &[0, 0], &RecoveryLimits { nodes: 29, ..RecoveryLimits::default() }, &mut rejected).is_err());
    assert_eq!((rejected.vector_request, rejected.picture_assignments), (0, 0));
    assert!(Arc::ptr_eq(
        out.workbook.sheets[0].cell_pictures.get(&CellRef::default()).unwrap(),
        out.workbook.sheets[0].cell_pictures.get(&CellRef::new(0, 1)).unwrap()
    ));
    use std::io::Write;
    writeln!(std::io::stdout(), "E3_PICTURE_REFERENCES {:?}", stats).unwrap();
}
