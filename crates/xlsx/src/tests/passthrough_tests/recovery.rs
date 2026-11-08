//! Recovery graph checks use synthetic retained model data, independently of a recovery codec.

use super::*;
use crate::passthrough::recovery::{Allowance, Error, LIMITS, OwnerIndex, part_key, preflight};
use gridcraft_model::{Preserved, PreservedPart, PreservedRel, Workbook};

fn part(name: &str) -> PreservedPart {
    PreservedPart { name: name.into(), content_type: "application/xml".into(), data: Arc::from(&b"<root/>"[..]), rels: vec![] }
}

fn rel(id: &str, kind: &str, target: &str) -> PreservedRel {
    PreservedRel { id: id.into(), rel_type: format!("{NS_R}/{kind}"), target: target.into(), external: false }
}

#[test]
fn passthrough_recovery_characterizes_writer_local_loss_and_shared_owners() {
    assert_eq!(normalize("/customXml\\ITEM.xml"), "customxml/item.xml");
    let (wb, _) = read_xlsx(&rich_package_with(CONTENT_TYPES, true)).unwrap();
    let a = wb.sheets[0].preserved.as_deref().unwrap();
    let b = wb.sheets[1].preserved.as_deref().unwrap();
    assert_eq!(a.parts, b.parts);
    let u = Unpacked::new(&write_xlsx(&wb).unwrap());
    u.assert_valid();
    assert_eq!(u.find(OLE).len(), 2, "writer emits independent owner copies");

    let mut wb = Workbook::new();
    wb.preserved = Some(Arc::new(Preserved {
        rels: vec![rel("a", "customXml", "customXml/a.xml"), rel("b", "customXml", "customXml/A.xml")],
        parts: vec![part("customXml/a.xml"), PreservedPart { data: Arc::from(&b"<different/>"[..]), ..part("customXml/A.xml") }],
        ..Default::default()
    }));
    let out = Unpacked::new(&write_xlsx(&wb).unwrap());
    assert_eq!(out.find(b"<root/>").len(), 1);
    assert!(out.find(b"<different/>").is_empty(), "ordinary writer deduplicates without validating");
}

#[test]
fn passthrough_recovery_opc_spelling_retains_literal_iri_and_ascii_identity() {
    for name in [
        "customXml/a.xml",
        "customXml/é.xml",
        "customXml/É.xml",
        "customXml/e\u{301}.xml",
        "customXml/中文.xml",
        "a/*",
        "a/%20%23%3F%3A%25%2A",
        "a/%252e",
    ] {
        assert_eq!(part_key(name), Ok(name.to_ascii_lowercase()), "{name}");
    }
    for name in [
        "",
        "/a",
        "\\a",
        "a\\b",
        "a//b",
        "a/./b",
        "a/../b",
        "a/b.",
        "a?b",
        "a#b",
        "a/ b",
        "a/\0",
        "a/%",
        "a/%A",
        "a/%Q1",
        "a/\u{E000}",
        "a/\u{FDD0}",
        "a/\u{EFFFE}",
    ] {
        assert_eq!(part_key(name), Err(Error::PartName), "{name:?}");
    }
    for name in ["C:book.xml", "file:book.xml"] {
        assert_eq!(part_key(name), Err(Error::UnsupportedPartName));
    }
}

#[test]
fn passthrough_recovery_percent_encoding_is_one_lexical_pass() {
    for name in ["a/%41", "a/%7e", "a/%2e", "a/%2F", "a/%5c", "a/%C3%A9", "a/%E4%B8%AD", "a/%F0%90%80%80"] {
        assert_eq!(part_key(name), Err(Error::PartName), "{name}");
    }
    for name in ["a/%00", "a/%1F", "a/%7F", "a/%80", "a/%FF", "a/%ED%A0%80", "a/%EE%80%80"] {
        assert_eq!(part_key(name), Err(Error::UnsupportedPartName), "{name}");
    }
}

#[test]
fn passthrough_recovery_indices_do_not_collapse_local_copies_or_ids() {
    for (first, second) in [("customXml/a.xml", "customXml/A.xml"), ("a/%2A", "a/%2a")] {
        let p = Preserved { parts: vec![part(first), part(second)], ..Default::default() };
        assert_eq!(OwnerIndex::new(&p).err(), Some(Error::DuplicatePart));
    }
}

#[test]
fn passthrough_recovery_relationship_ids_are_unique_per_list() {
    let edge = rel("same", "customXml", "customXml/a.xml");
    let p = Preserved { rels: vec![edge.clone(), edge], parts: vec![part("customXml/a.xml")], ..Default::default() };
    assert_eq!(OwnerIndex::new(&p).err(), Some(Error::DuplicateRelationship));
}

#[test]
fn passthrough_recovery_part_prefixes_and_relationship_shadows_fail() {
    for (a, b) in [("customXml/a", "customXml/A/b.xml"), ("customXml/a", "customXml/a/b/c.xml")] {
        let p = Preserved { parts: vec![part(a), part(b)], ..Default::default() };
        assert_eq!(OwnerIndex::new(&p).err(), Some(Error::DuplicatePart));
    }
    for name in ["_rels/.rels", "customXml/_RELS/item.xml.RELS"] {
        assert_eq!(part_key(name), Err(Error::PartName));
    }
}

#[test]
fn passthrough_recovery_vml_is_a_separate_charged_local_part() {
    let p = Preserved { parts: vec![part("customXml/a.xml")], vml: Some(part("customXml/A.xml")), ..Default::default() };
    assert_eq!(OwnerIndex::new(&p).err(), Some(Error::DuplicatePart));
}

#[test]
fn passthrough_recovery_aggregate_prepass_charges_shared_arcs_and_vml() {
    let p = Arc::new(Preserved { parts: vec![part("customXml/a.xml")], vml: Some(part("xl/a.vml")), ..Default::default() });
    let mut wb = Workbook::new();
    for _ in 0..2 {
        let mut s = Sheet::new("s");
        s.preserved = Some(p.clone());
        wb.sheets.push(Arc::new(s));
    }
    assert_eq!(preflight(&wb, crate::passthrough::recovery::Limits { parts: 3, ..LIMITS }), Err(Error::PartLimit));
    assert_eq!(preflight(&wb, crate::passthrough::recovery::Limits { bytes: 27, ..LIMITS }), Err(Error::ByteLimit));
}

#[test]
fn passthrough_recovery_combined_sheet_and_relationship_budgets_are_global() {
    let mut wb = Workbook::new();
    wb.sheets.clear();
    let p = Arc::new(Preserved { rels: vec![rel("x", "queryTable", "a.xml")], parts: vec![part("a.xml")], ..Default::default() });
    for _ in 0..2 {
        let mut s = Sheet::new("s");
        s.preserved = Some(p.clone());
        wb.sheets.push(Arc::new(s));
    }
    assert_eq!(preflight(&wb, crate::passthrough::recovery::Limits { total_rels: 1, ..LIMITS }), Err(Error::RelationshipLimit));
    wb.preserved = Some(Arc::new(Preserved { sheets: vec![Default::default()], ..Default::default() }));
    assert_eq!(preflight(&wb, crate::passthrough::recovery::Limits { sheets: 2, ..LIMITS }), Err(Error::SheetLimit));
}

fn chain(depth: usize) -> Preserved {
    Preserved {
        rels: vec![rel("root", "customXml", "customXml/n0.xml")],
        parts: (0..=depth)
            .map(|n| PreservedPart {
                rels: if n == depth { vec![] } else { vec![rel("next", "customXml", &format!("customXml/n{}.xml", n + 1))] },
                ..part(&format!("customXml/n{n}.xml"))
            })
            .collect(),
        ..Default::default()
    }
}

fn walk(p: &Preserved) -> Result<Vec<u8>, Error> {
    OwnerIndex::new(p)?.walk(p.package_rels.iter().chain(&p.rels).map(|r| (r, Allowance::Ordinary)))
}

#[test]
fn passthrough_recovery_first_discovery_depth_16_and_17() {
    assert!(walk(&chain(16)).is_ok());
    assert_eq!(walk(&chain(17)), Err(Error::ChainLimit));
}

#[test]
fn passthrough_recovery_ordered_roots_cannot_use_a_later_shortcut() {
    let mut p = chain(17);
    p.rels.push(rel("shortcut", "customXml", "customXml/n17.xml"));
    assert_eq!(walk(&p), Err(Error::ChainLimit));
    p.rels.reverse();
    assert!(walk(&p).is_ok());
}

#[test]
fn passthrough_recovery_context_is_not_borrowed_from_chart_discovery() {
    let p = Preserved {
        rels: vec![rel("chart", "chartsheet", "xl/c.xml"), rel("plain", "customXml", "xl/c.xml")],
        parts: vec![PreservedPart { rels: vec![rel("draw", "drawing", "xl/d.xml")], ..part("xl/c.xml") }, part("xl/d.xml")],
        ..Default::default()
    };
    let index = OwnerIndex::new(&p).unwrap();
    assert_eq!(index.walk([(&p.rels[0], Allowance::Chart), (&p.rels[1], Allowance::Ordinary)]), Err(Error::Relationship));
}

#[test]
fn passthrough_recovery_visited_target_still_checks_incoming_edge_policy() {
    let p = Preserved {
        rels: vec![rel("first", "customXml", "a.xml"), rel("bad", "worksheet", "a.xml")],
        parts: vec![part("a.xml")],
        ..Default::default()
    };
    assert_eq!(walk(&p), Err(Error::Relationship));
}

#[test]
fn passthrough_recovery_orphan_part_refuses_instead_of_pruning() {
    let p = Preserved { parts: vec![part("a.xml")], ..Default::default() };
    assert_eq!(walk(&p), Err(Error::UnreachablePart));
}

#[test]
fn passthrough_recovery_owner_slots_and_roots_are_local() {
    let mut wb = Workbook::new();
    wb.preserved = Some(Arc::new(Preserved { vml: Some(part("a.vml")), ..Default::default() }));
    assert_eq!(preflight(&wb, LIMITS), Err(Error::Owner));
    wb.preserved = None;
    Arc::make_mut(&mut wb.sheets[0]).preserved =
        Some(Arc::new(Preserved { package_rels: vec![rel("x", "custom-properties", "a.xml")], ..Default::default() }));
    assert_eq!(preflight(&wb, LIMITS), Err(Error::Owner));
}

#[test]
fn passthrough_recovery_header_context_does_not_admit_charts() {
    let p = Preserved {
        rels: vec![rel("hf", "vmlDrawing", "a.vml")],
        parts: vec![PreservedPart { rels: vec![rel("chart", "chart", "a.xml")], ..part("a.vml") }, part("a.xml")],
        ..Default::default()
    };
    assert_eq!(OwnerIndex::new(&p).unwrap().walk([(&p.rels[0], Allowance::HeaderVml)]), Err(Error::Relationship));
}

#[test]
fn passthrough_recovery_unknown_duplicate_and_wrong_owner_slots_fail() {
    for slots in [vec!["controls"], vec!["fileVersion", "fileVersion"], vec!["invented"]] {
        let mut wb = Workbook::new();
        wb.preserved = Some(Arc::new(Preserved {
            elements: slots.into_iter().map(|slot| gridcraft_model::PreservedXml { slot: slot.into(), xml: "<x/>".into() }).collect(),
            ..Default::default()
        }));
        assert_eq!(preflight(&wb, LIMITS), Err(Error::Owner));
    }
}

#[test]
fn passthrough_recovery_wrong_owner_root_kinds_fail() {
    for (package, kind) in [(false, "image"), (true, "connections")] {
        let mut wb = Workbook::new();
        let mut p = Preserved { parts: vec![part("a.xml")], ..Default::default() };
        if package {
            p.package_rels.push(rel("x", kind, "a.xml"));
        } else {
            p.rels.push(rel("x", kind, "a.xml"));
        }
        wb.preserved = Some(Arc::new(p));
        assert_eq!(preflight(&wb, LIMITS), Err(Error::Owner));
    }
}

#[test]
fn passthrough_recovery_vml_keeps_its_stricter_reader_limit() {
    let mut wb = Workbook::new();
    Arc::make_mut(&mut wb.sheets[0]).preserved = Some(Arc::new(Preserved {
        vml: Some(PreservedPart { data: Arc::from(vec![0; 16 * 1024 * 1024 + 1]), ..part("a.vml") }),
        ..Default::default()
    }));
    assert_eq!(preflight(&wb, LIMITS), Err(Error::ByteLimit));
}

#[test]
fn passthrough_recovery_index_counts_vml_toward_part_cap() {
    let p = Preserved { parts: (0..4096).map(|n| part(&format!("customXml/{n}.xml"))).collect(), vml: Some(part("a.vml")), ..Default::default() };
    assert_eq!(OwnerIndex::new(&p).err(), Some(Error::PartLimit));
}

#[test]
fn passthrough_recovery_indices_keep_duplicate_ids_in_independent_scopes() {
    let p = Preserved {
        package_rels: vec![rel("same", "custom-properties", "customXml/a.xml")],
        rels: vec![rel("same", "customXml", "customXml/a.xml")],
        parts: vec![part("customXml/a.xml")],
        vml: Some(PreservedPart { rels: vec![rel("same", "image", "customXml/a.xml")], ..part("a.vml") }),
        ..Default::default()
    };
    let index = OwnerIndex::new(&p).unwrap();
    assert_eq!(index.package[0].rel_type, format!("{NS_R}/custom-properties"));
    assert_eq!(index.rels[0].rel_type, format!("{NS_R}/customXml"));
    assert_eq!(index.vml_rels[0].rel_type, format!("{NS_R}/image"));
    assert_eq!(index.part("CUSTOMXML/A.XML").unwrap().part.data.as_ref(), b"<root/>");
    assert!(index.part("other.xml").is_none());
}

#[test]
fn passthrough_recovery_raw_count_guard_precedes_owner_inspection() {
    let mut wb = Workbook::new();
    wb.preserved = Some(Arc::new(Preserved { rels: vec![rel("x", "image", "a.xml")], parts: vec![part("a.xml")], ..Default::default() }));
    assert_eq!(preflight(&wb, crate::passthrough::recovery::Limits { parts: 0, ..LIMITS }), Err(Error::PartLimit));
}

#[test]
fn passthrough_recovery_imported_positive_owner_closures_and_cross_owner_refusal() {
    for bytes in [rich_package(), rich_package_with(CONTENT_TYPES, true), xml_map_package(), header_picture_package(), chartsheet_package()] {
        let (wb, _) = read_xlsx(&bytes).unwrap();
        assert_eq!(preflight(&wb, LIMITS), Ok(()));
        for p in wb.preserved.iter().chain(wb.sheets.iter().filter_map(|s| s.preserved.as_ref())) {
            let index = OwnerIndex::new(p).unwrap();
            let roots = p
                .package_rels
                .iter()
                .chain(&p.rels)
                .map(|r| {
                    let allowance = match r.rel_type.rsplit('/').next().unwrap() {
                        "chartsheet" => Allowance::Chart,
                        "vmlDrawing" => Allowance::HeaderVml,
                        _ => Allowance::Ordinary,
                    };
                    (r, allowance)
                })
                .chain(index.vml_rels.iter().map(|r| (*r, Allowance::Ordinary)));
            assert!(index.walk(roots).is_ok());
        }
    }
    let (wb, _) = read_xlsx(&rich_package_with(CONTENT_TYPES, true)).unwrap();
    let mut b = wb.sheets[1].preserved.as_ref().unwrap().as_ref().clone();
    b.parts.retain(|q| q.content_type != "image/x-emf");
    assert_eq!(walk(&b), Err(Error::MissingTarget), "owner B cannot borrow owner A's existing image");
    let mut a = wb.sheets[0].preserved.as_ref().unwrap().as_ref().clone();
    a.parts.iter_mut().find(|q| q.content_type == "image/x-emf").unwrap().data = Arc::from(&b"different image"[..]);
    assert!(walk(&a).is_ok(), "divergent independently owned copies remain legitimate");
}

#[test]
fn passthrough_recovery_cycles_sharing_and_within_root_bfs_shortcut_terminate() {
    let mut p = chain(7);
    p.parts[7].rels.push(rel("back", "customXml", "customXml/n0.xml"));
    for n in 1..1000 {
        p.rels.push(rel(&format!("r{n}"), "customXml", "customXml/n0.xml"));
    }
    assert!(walk(&p).is_ok());
    let mut p = chain(17);
    p.parts[0].rels.push(rel("short", "customXml", "customXml/n17.xml"));
    assert!(walk(&p).is_ok(), "BFS discovers the short edge while expanding the same root");
}

#[test]
fn passthrough_recovery_external_targets_are_inert_strings() {
    let mut p = chain(0);
    p.parts[0].rels.push(PreservedRel { external: true, ..rel("external", "externalLinkPath", "../../never\\opened/%Q1?x=1&y=2") });
    assert!(walk(&p).is_ok());
    p.parts[0].rels.push(rel("missing", "customXml", "customXml/missing.xml"));
    assert_eq!(walk(&p), Err(Error::MissingTarget));
    p.parts[0].rels.pop();
    p.parts[0].rels.push(rel("alias", "customXml", "/customXml/n0.xml"));
    assert_eq!(walk(&p), Err(Error::PartName));
}

#[test]
fn passthrough_recovery_context_discovery_has_an_independent_depth_budget() {
    let mut p = chain(17);
    p.rels.insert(0, rel("chart-short", "customXml", "customXml/n17.xml"));
    let index = OwnerIndex::new(&p).unwrap();
    assert_eq!(index.walk([(&p.rels[0], Allowance::Chart), (&p.rels[1], Allowance::Ordinary)]), Err(Error::ChainLimit));
}

#[test]
fn passthrough_recovery_small_budgets_have_inclusive_boundaries() {
    let mut wb = Workbook::new();
    wb.preserved = Some(Arc::new(Preserved { rels: vec![rel("x", "customXml", "a.xml")], parts: vec![part("a.xml")], ..Default::default() }));
    let exact = crate::passthrough::recovery::Limits { bytes: 7, part_bytes: 7, parts: 1, rels: 1, total_rels: 1, sheets: 1, ..LIMITS };
    assert_eq!(preflight(&wb, exact), Ok(()));
    for (limits, error) in [
        (crate::passthrough::recovery::Limits { bytes: 6, ..exact }, Error::ByteLimit),
        (crate::passthrough::recovery::Limits { part_bytes: 6, ..exact }, Error::ByteLimit),
        (crate::passthrough::recovery::Limits { parts: 0, ..exact }, Error::PartLimit),
        (crate::passthrough::recovery::Limits { rels: 0, ..exact }, Error::RelationshipLimit),
        (crate::passthrough::recovery::Limits { total_rels: 0, ..exact }, Error::RelationshipLimit),
        (crate::passthrough::recovery::Limits { sheets: 0, ..exact }, Error::SheetLimit),
    ] {
        assert_eq!(preflight(&wb, limits), Err(error));
    }
    assert_eq!(
        (LIMITS.bytes, LIMITS.part_bytes, LIMITS.parts, LIMITS.rels, LIMITS.total_rels, LIMITS.sheets, LIMITS.text, LIMITS.fragment),
        (256 * 1024 * 1024, 64 * 1024 * 1024, 4096, 10_000, 100_000, 1024, 32 * 1024, 4 * 1024 * 1024)
    );
    assert_eq!(part_key(&"a".repeat(32 * 1024 + 1)), Err(Error::TextLimit));
}

#[test]
fn passthrough_recovery_metadata_checks_xml_scalars_without_mime_whitelist() {
    let mut wb = Workbook::new();
    let mut p = Preserved { rels: vec![rel("x", "customXml", "a.xml")], parts: vec![part("a.xml")], ..Default::default() };
    p.parts[0].content_type = "application/x-test\" <&>".into();
    wb.preserved = Some(Arc::new(p.clone()));
    assert_eq!(preflight(&wb, LIMITS), Ok(()));
    p.rels[0].external = true;
    for bad in ["", "raw\0text", "raw\u{FFFF}text"] {
        p.rels[0].target = bad.into();
        wb.preserved = Some(Arc::new(p.clone()));
        assert_eq!(preflight(&wb, LIMITS), Err(Error::Relationship));
    }
}

#[test]
fn passthrough_recovery_iri_interval_boundaries_do_not_use_unicode_categories() {
    for scalar in [0xA0, 0xD7FF, 0xF900, 0xFDCF, 0xFDF0, 0xFFEF, 0x10000, 0x1FFFD, 0xE1000, 0xEFFFD] {
        assert!(part_key(&format!("a/{}", char::from_u32(scalar).unwrap())).is_ok());
    }
    for scalar in [0x9F, 0xE000, 0xF8FF, 0xFDD0, 0xFDEF, 0xFFF0, 0x1FFFE, 0xE0000, 0xE0FFF, 0xEFFFE, 0xF0000] {
        assert_eq!(part_key(&format!("a/{}", char::from_u32(scalar).unwrap())), Err(Error::PartName));
    }
    assert_eq!(part_key("a/%00%Q1"), Err(Error::PartName), "invalid percent syntax retains malformed classification");
}

#[test]
fn passthrough_recovery_prefix_check_handles_intervening_lexical_siblings() {
    let p = Preserved { parts: vec![part("customXml/a"), part("customXml/a-other"), part("customXml/a/b.xml")], ..Default::default() };
    assert_eq!(OwnerIndex::new(&p).err(), Some(Error::DuplicatePart));
}

#[test]
fn passthrough_recovery_part_vml_and_package_id_scopes_reject_duplicates() {
    let edges = vec![rel("same", "customXml", "a.xml"), rel("same", "customXml", "b.xml")];
    for p in [
        Preserved { package_rels: edges.clone(), ..Default::default() },
        Preserved { parts: vec![PreservedPart { rels: edges.clone(), ..part("c.xml") }], ..Default::default() },
        Preserved { vml: Some(PreservedPart { rels: edges, ..part("c.vml") }), ..Default::default() },
    ] {
        assert_eq!(OwnerIndex::new(&p).err(), Some(Error::DuplicateRelationship));
    }
}

#[test]
fn passthrough_recovery_checked_arithmetic_and_reservation_fail_finitely() {
    use crate::passthrough::recovery::{charge, reserve};
    let mut total = usize::MAX;
    assert_eq!(charge(&mut total, 1, usize::MAX, Error::ByteLimit), Err(Error::ByteLimit));
    assert_eq!(reserve(&mut Vec::<u8>::new(), usize::MAX), Err(Error::Allocation));
}

#[test]
fn passthrough_recovery_fragment_and_metadata_bounds_precede_indices() {
    let p = Preserved {
        rels: vec![rel("x", "customXml", "a.xml")],
        parts: vec![part("a.xml")],
        elements: vec![gridcraft_model::PreservedXml { slot: "fileVersion".into(), xml: "<fileVersion/>".into() }],
        ..Default::default()
    };
    let mut wb = Workbook::new();
    wb.preserved = Some(Arc::new(p.clone()));
    assert_eq!(preflight(&wb, crate::passthrough::recovery::Limits { fragment: 14, ..LIMITS }), Ok(()));
    assert_eq!(preflight(&wb, crate::passthrough::recovery::Limits { fragment: 13, ..LIMITS }), Err(Error::TextLimit));
    for field in 0..5 {
        let mut p = p.clone();
        let target = match field {
            0 => &mut p.rels[0].id,
            1 => &mut p.rels[0].target,
            2 => &mut p.rels[0].rel_type,
            3 => &mut p.parts[0].name,
            _ => &mut p.parts[0].content_type,
        };
        *target = "a".repeat(32 * 1024 + 1);
        wb.preserved = Some(Arc::new(p));
        assert_eq!(preflight(&wb, LIMITS), Err(Error::TextLimit));
    }
}

fn output_map(label: &str, wb: &Workbook) -> Unpacked {
    let bytes = write_xlsx(wb).unwrap();
    let out = Unpacked::new(&bytes);
    let mut names: Vec<_> = out.parts.keys().cloned().collect();
    names.sort();
    if let Ok(root) = std::env::var("GRIDCRAFT_P3_MAPS_DIR") {
        let dir = std::path::Path::new(&root).join(label);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("__members.txt"), names.join("\n")).unwrap();
        for (i, name) in names.iter().enumerate() {
            std::fs::write(dir.join(format!("{i:04}.bin")), &out.parts[name]).unwrap();
        }
    }
    if let Ok(root) = std::env::var("GRIDCRAFT_P3_OLD_MAPS_DIR") {
        let mut dir = std::path::Path::new(&root).join(label);
        if !dir.exists() {
            dir = std::path::Path::new(&root).parent().unwrap().join("old-writer-nativefamilies-v2-maps").join(label);
        }
        assert_eq!(std::fs::read_to_string(dir.join("__members.txt")).unwrap(), names.join("\n"), "{label} names");
        for (i, name) in names.iter().enumerate() {
            let mut expected = std::fs::read(dir.join(format!("{i:04}.bin"))).unwrap();
            if label == "charts" && name == "xl/chartsheets/_rels/kept4.xml.rels" {
                // The only authorized legacy fixture correction: second pageSetup gains rId2.
                let added =
                    format!("<Relationship Id=\"rId2\" Type=\"{NS_R}/printerSettings\" Target=\"/xl/printerSettings/printerSettings1.bin\"/>");
                expected = String::from_utf8(expected).unwrap().replace("</Relationships>", &format!("{added}</Relationships>")).into_bytes();
            }
            assert_eq!(expected, out.parts[name], "{label}: {name}");
        }
    }
    out
}

fn image(bytes: &[u8], id: u32) -> gridcraft_model::Image {
    gridcraft_model::Image { id, anchor: Default::default(), data: Arc::new(bytes.to_vec()), mime: "image/png".into(), alt: String::new() }
}

fn png() -> Vec<u8> {
    let mut b = vec![0; 33];
    b[..16].copy_from_slice(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR");
    b[19] = 1;
    b[23] = 1;
    b
}

fn named_owner(name: &str, workbook: bool) -> Arc<Preserved> {
    Arc::new(Preserved {
        rels: vec![rel("x", if workbook { "customXml" } else { "queryTable" }, name)],
        parts: vec![part(name)],
        ..Default::default()
    })
}

#[test]
fn passthrough_recovery_old_writer_claim_literals_and_body_maps() {
    let mut wb = Workbook::new();
    wb.sheets.clear();
    wb.preserved = Some(named_owner("customXml/item.xml", true));
    for name in ["customXml/item.xml", "xl/media/original.png", "customXml/item.xml"] {
        let mut s = Sheet::new(format!("S{}", wb.sheets.len()));
        s.preserved = Some(named_owner(name, false));
        wb.sheets.push(Arc::new(s));
    }
    let out = output_map("suffix", &wb);
    for name in ["customXml/item.xml", "customXml/item_1.xml", "xl/media/kept2.png", "customXml/item_3.xml"] {
        assert!(out.parts.contains_key(name), "{name}");
    }
    let mut wb = Workbook::new();
    wb.preserved = Some(Arc::new(Preserved {
        rels: vec![rel("a", "customXml", "customXml/a.xml"), rel("b", "customXml", "customXml/a_1.xml")],
        parts: vec![part("customXml/a.xml"), part("customXml/a_1.xml")],
        ..Default::default()
    }));
    Arc::make_mut(&mut wb.sheets[0]).preserved = Some(named_owner("customXml/a.xml", false));
    assert!(output_map("retry", &wb).parts.contains_key("customXml/a_2.xml"));
    for owned in [false, true] {
        let mut wb = Workbook::new();
        let folder = if owned { "customXml" } else { "other" };
        wb.preserved = Some(named_owner(&format!("{folder}/a"), true));
        Arc::make_mut(&mut wb.sheets[0]).preserved = Some(named_owner(&format!("{folder}/a/b.xml"), false));
        let out = output_map(if owned { "prefix-conflict" } else { "prefix-renamed" }, &wb);
        if owned {
            assert!(out.parts.contains_key("customXml/a") && out.parts.contains_key("customXml/a/b.xml"));
        } else {
            assert!(out.parts.contains_key("other/kept1") && out.parts.contains_key("other/a/kept2.xml"));
        }
    }
}

#[test]
fn passthrough_recovery_old_writer_control_vml_and_image_maps() {
    let mut wb = Workbook::new();
    wb.preserved = Some(named_owner("xl/ctrlProps/ctrlProp1.xml", true));
    Arc::make_mut(&mut wb.sheets[0]).controls.push(gridcraft_model::FormControl { id: 1, ..Default::default() });
    let mut second = Sheet::new("Second");
    second.preserved = Some(named_owner("xl/ctrlProps/ctrlProp2.xml", false));
    second.controls.push(gridcraft_model::FormControl { id: 2, ..Default::default() });
    wb.sheets.push(Arc::new(second));
    let out = output_map("controls", &wb);
    for n in [
        "xl/ctrlProps/ctrlProp1.xml",
        "xl/ctrlProps/ctrlProp2.xml",
        "xl/ctrlProps/ctrlProp2_1.xml",
        "xl/ctrlProps/ctrlProp3.xml",
        "xl/drawings/vmlDrawing1.vml",
        "xl/drawings/vmlDrawing2.vml",
    ] {
        assert!(out.parts.contains_key(n), "{n}");
    }
    for (label, bytes) in [
        ("rich", rich_package()),
        ("twins", rich_package_with(CONTENT_TYPES, true)),
        ("header", header_picture_package()),
        ("charts", chartsheet_package()),
        ("xmlmap", xml_map_package()),
    ] {
        let (wb, _) = read_xlsx(&bytes).unwrap();
        output_map(label, &wb);
    }
    for distinct in [false, true] {
        let mut wb = Workbook::new();
        let s = Arc::make_mut(&mut wb.sheets[0]);
        let mut p = Preserved {
            rels: vec![rel("pic", "image", "xl/media/imageParent/source.png")],
            parts: vec![PreservedPart { data: Arc::from(&b"A"[..]), content_type: "image/png".into(), ..part("xl/media/imageParent/source.png") }],
            ..Default::default()
        };
        p.elements.push(gridcraft_model::PreservedXml { slot: "picture".into(), xml: format!("<picture xmlns:r='{NS_R}' r:id='pic'/>") });
        s.preserved = Some(Arc::new(p));
        s.images.push(image(b"A", 1));
        if distinct {
            s.images.push(image(b"B", 2));
        }
        let out = output_map(if distinct { "image-distinct" } else { "image-reused" }, &wb);
        assert!(out.parts.contains_key("xl/media/imageParent/kept1.png"));
        assert_eq!(out.parts.contains_key("xl/media/image1.png"), distinct);
        assert!(out.text("xl/drawings/_rels/drawing1.xml.rels").contains("imageParent/kept1.png"));
    }
}

#[test]
fn passthrough_recovery_old_writer_native_and_picture_identity_maps() {
    use gridcraft_core::{CellRef, Value};
    use gridcraft_model::{Cell, CellPicture, PivotTable};
    for separate in [false, true] {
        let mut wb = Workbook::new();
        let s = Arc::make_mut(&mut wb.sheets[0]);
        let p = Arc::new(CellPicture { data: png(), mime: "image/png".into(), alt: String::new() });
        s.cell_pictures.insert(CellRef::new(0, 0), p.clone());
        s.cell_pictures.insert(CellRef::new(1, 0), if separate { Arc::new(p.as_ref().clone()) } else { p });
        s.images.push(image(&png(), 1));
        let out = output_map(if separate { "picture-distinct" } else { "picture-shared" }, &wb);
        assert!(out.parts.contains_key("xl/media/cellImage1.png"));
        assert_eq!(out.parts.contains_key("xl/media/cellImage2.png"), separate);
        assert!(out.parts.contains_key("xl/media/image1.png"));
    }
    for rows in [false, true] {
        let mut wb = Workbook::new();
        let s = Arc::make_mut(&mut wb.sheets[0]);
        s.cells.set(CellRef::new(0, 0), Cell::value(Value::text("Head")));
        if rows {
            s.cells.set(CellRef::new(1, 0), Cell::value(Value::Number(1.)));
        }
        s.pivots.push(PivotTable { source: "A1:A2".into(), name: "P".into(), ..Default::default() });
        let out = output_map(if rows { "pivot-row" } else { "pivot-empty" }, &wb);
        for n in ["xl/pivotCache/pivotCacheRecords1.xml", "xl/pivotCache/_rels/pivotCacheDefinition1.xml.rels", "xl/pivotTables/pivotTable1.xml"] {
            assert!(out.parts.contains_key(n));
        }
    }
    let out = output_map("default", &Workbook::new());
    for n in [
        "xl/styles.xml",
        "xl/sharedStrings.xml",
        "xl/theme/theme1.xml",
        "xl/workbook.xml",
        "docProps/core.xml",
        "docProps/app.xml",
        "_rels/.rels",
        "xl/_rels/workbook.xml.rels",
        "xl/worksheets/sheet1.xml",
    ] {
        assert!(out.parts.contains_key(n));
    }
    assert!(!out.parts.contains_key("xl/metadata.xml") && !out.parts.contains_key("xl/python.xml") && !out.parts.contains_key("docProps/custom.xml"));
}

#[test]
fn passthrough_recovery_shared_claim_kernel_geometry_retry_and_max_are_legacy_exact() {
    use crate::passthrough::{ClaimName, claim_with};
    use std::collections::HashSet;
    let mut n = 0;
    let mut used = HashSet::from(["customxml/a.xml".to_string(), "customxml/a_1.xml".to_string()]);
    let actual: Result<String, ()> = claim_with("customXml/a.xml", &mut n, |r| {
        let s = r.to_string();
        assert_eq!(Some(s.len()), r.byte_len());
        Ok((used.insert(s.to_ascii_lowercase()), s))
    });
    assert_eq!(actual, Ok("customXml/a_2.xml".into()));
    assert_eq!(n, 2);
    n = u32::MAX;
    let result: Result<String, ()> = claim_with("customXml/a.xml", &mut n, |r| Ok((false, r.to_string())));
    assert_eq!(result, Ok("customXml/a_4294967295.xml".into()));
    let r = ClaimName::Renamed { directory: "xl", stem: "", extension: Some("é"), owned: false, number: u32::MAX };
    assert_eq!(r.byte_len(), Some(r.to_string().len()));
}

#[test]
fn passthrough_recovery_claim_plan_refuses_actual_native_prefix_not_absent_names() {
    use crate::passthrough::recovery::claims;
    for (name, bad) in [("xl/worksheets/sheet1.xml/leaf.bin", true), ("xl/worksheets/sheet99.xml/leaf.bin", false)] {
        let mut wb = Workbook::new();
        wb.preserved = Some(named_owner(name, true));
        assert_eq!(claims::validate(&wb), if bad { Err(Error::OutputName) } else { Ok(()) });
    }
}

#[test]
fn passthrough_recovery_claim_plan_accounts_native_control_and_cross_owner_renames() {
    use crate::passthrough::recovery::claims;
    for owned in [true, false] {
        let folder = if owned { "customXml" } else { "other" };
        let mut wb = Workbook::new();
        wb.preserved = Some(named_owner(&format!("{folder}/a"), true));
        Arc::make_mut(&mut wb.sheets[0]).preserved = Some(named_owner(&format!("{folder}/a/b"), false));
        assert_eq!(claims::validate(&wb), if owned { Err(Error::OutputName) } else { Ok(()) });
    }
    let mut wb = Workbook::new();
    wb.preserved = Some(named_owner("xl/ctrlProps/ctrlProp1.xml", true));
    Arc::make_mut(&mut wb.sheets[0]).controls.push(Default::default());
    assert_eq!(claims::validate(&wb), Ok(()));
    Arc::make_mut(&mut wb.sheets[0]).preserved = Some(named_owner("xl/ctrlProps/ctrlProp2.xml/leaf", false));
    assert_eq!(claims::validate(&wb), Err(Error::OutputName));
}

#[test]
fn passthrough_recovery_checked_array_shape_matches_writer_and_refuses_budget() {
    use crate::array_formula::{AnalysisError, needs_array_checked};
    use gridcraft_core::CellRef;
    use gridcraft_model::Formula;
    let wb = Workbook::new();
    for text in ["1+2", "SUM(A1:A3)", "A1:A3*2", "@A1", "LET(x,1,LET(x,A1:A3,SUM(x)))", "INDIRECT(\"A1:A3\")"] {
        let f = Formula::new(text);
        let expected = crate::formula_needs_array(&wb, 0, CellRef::new(0, 0), &f);
        let mut ok = |_| Ok(());
        assert_eq!(needs_array_checked(&wb, 0, CellRef::new(0, 0), &f, &mut ok), Ok(expected), "{text}");
        let mut fail = |_| Err(AnalysisError::Limit);
        assert_eq!(needs_array_checked(&wb, 0, CellRef::new(0, 0), &f, &mut fail), Err(AnalysisError::Limit));
    }
}

#[test]
fn passthrough_recovery_native_queried_families_match_actual_package() {
    use crate::passthrough::recovery::claims;
    for (path, prepare) in [
        ("xl/python.xml", 0),
        ("xl/metadata.xml", 1),
        ("xl/pivotCache/pivotCacheRecords1.xml", 2),
        ("xl/media/image1.png", 3),
        ("xl/media/cellImage2.png", 4),
    ] {
        let mut wb = Workbook::new();
        let s = Arc::make_mut(&mut wb.sheets[0]);
        match prepare {
            0 => {
                s.cells.set(
                    gridcraft_core::CellRef::new(0, 0),
                    gridcraft_model::Cell {
                        py: Some(Arc::new(gridcraft_model::PyCode { code: "1".into(), ..Default::default() })),
                        ..Default::default()
                    },
                );
            }
            1 => {
                s.cells.set(
                    gridcraft_core::CellRef::new(0, 0),
                    gridcraft_model::Cell { formula: Some(Arc::new(gridcraft_model::Formula::new("@A2"))), ..Default::default() },
                );
            }
            2 => s.pivots.push(gridcraft_model::PivotTable { source: "A1:A2".into(), ..Default::default() }),
            3 => s.images.push(image(b"image", 1)),
            _ => {
                for row in 0..2 {
                    s.cell_pictures.insert(
                        gridcraft_core::CellRef::new(row, 0),
                        Arc::new(gridcraft_model::CellPicture { data: png(), mime: "image/png".into(), alt: String::new() }),
                    );
                }
            }
        }
        assert!(Unpacked::new(&write_xlsx(&wb).unwrap()).parts.contains_key(path), "{path}");
        wb.preserved = Some(named_owner(&format!("{path}/leaf.bin"), true));
        assert_eq!(claims::validate(&wb), Err(Error::OutputName), "{path}");
    }
}

fn conditional_native_workbook() -> Workbook {
    use gridcraft_core::{CellRef, RangeRef, Value};
    use gridcraft_model::*;
    let mut wb = Workbook::new();
    wb.vba_project = Some(Arc::new(vec![1, 2, 3]));
    wb.styles.intern(Style { num_fmt: NumFmt::new("checkbox"), ..Default::default() });
    let s = Arc::make_mut(&mut wb.sheets[0]);
    s.cells.set(CellRef::new(0, 0), Cell { py: Some(Arc::new(PyCode::new("1"))), ..Default::default() });
    s.cells.set(CellRef::new(1, 0), Cell { formula: Some(Arc::new(Formula::new("@A1"))), ..Default::default() });
    s.cells.set(CellRef::new(2, 0), Cell::value(Value::Number(2.)));
    s.tables.push(Table {
        id: 1,
        name: "NativeTable".into(),
        range: RangeRef::parse("C1:C3").unwrap(),
        header_row: true,
        totals_row: false,
        columns: vec![],
        style: String::new(),
        banded_rows: false,
        banded_cols: false,
        first_col: false,
        last_col: false,
        filter_button: false,
        filter: vec![],
        table_type: None,
        connection_id: None,
    });
    s.pivots.push(PivotTable { source: "A1:A3".into(), name: "Pivot".into(), ..Default::default() });
    Arc::make_mut(&mut s.comments).insert(
        CellRef::new(0, 1),
        Comment {
            author: "Author".into(),
            text: "Text".into(),
            replies: vec![],
            threaded: true,
            resolved: false,
            visible: false,
            dates: vec![],
            note_box: None,
        },
    );
    for (i, kind) in [ChartKind::ColumnClustered, ChartKind::Histogram].into_iter().enumerate() {
        s.charts.push(Chart {
            id: i as u32 + 1,
            kind,
            anchor: Default::default(),
            title: None,
            series: vec![],
            legend: Default::default(),
            data_labels: false,
            data_table: None,
            gridlines: false,
            style: 2,
            hole_size: 75,
            first_slice_angle: 0,
            x_title: None,
            y_title: None,
            source: None,
            by_rows: false,
            pivot: None,
        });
    }
    s.images.push(image(b"A", 1));
    s.cell_pictures.insert(CellRef::new(3, 0), Arc::new(CellPicture { data: png(), mime: "image/png".into(), alt: String::new() }));
    wb
}

#[test]
fn passthrough_recovery_old_writer_conditional_native_full_maps() {
    let wb = conditional_native_workbook();
    let out = output_map("conditional-native", &wb);
    for n in [
        "xl/python.xml",
        "xl/metadata.xml",
        "xl/vbaProject.bin",
        "xl/featurePropertyBag/featurePropertyBag.xml",
        "xl/persons/person.xml",
        "xl/comments1.xml",
        "xl/threadedComments/threadedComment1.xml",
        "xl/drawings/vmlDrawing1.vml",
        "xl/drawings/drawing1.xml",
        "xl/drawings/_rels/drawing1.xml.rels",
        "xl/tables/table1.xml",
        "xl/charts/chart1.xml",
        "xl/charts/chartEx2.xml",
        "xl/charts/style2.xml",
        "xl/charts/colors2.xml",
        "xl/charts/_rels/chartEx2.xml.rels",
        "xl/pivotCache/pivotCacheDefinition1.xml",
        "xl/pivotCache/pivotCacheRecords1.xml",
        "xl/pivotTables/_rels/pivotTable1.xml.rels",
        "xl/richData/rdrichvalue.xml",
        "xl/richData/rdrichvaluestructure.xml",
        "xl/richData/richValueRel.xml",
        "xl/richData/_rels/richValueRel.xml.rels",
        "xl/media/image1.png",
        "xl/media/cellImage1.png",
        "xl/worksheets/_rels/sheet1.xml.rels",
    ] {
        assert!(out.parts.contains_key(n), "{n}");
    }
    for n in [
        "xl/charts/chart2.xml",
        "xl/charts/chartEx1.xml",
        "xl/tables/table2.xml",
        "xl/media/image2.png",
        "xl/media/cellImage2.png",
        "docProps/custom.xml",
    ] {
        assert!(!out.parts.contains_key(n), "{n}");
    }
}

#[test]
fn passthrough_recovery_native_every_characterized_name_and_absence_is_queried() {
    use crate::passthrough::recovery::claims;
    let wb = conditional_native_workbook();
    let out = Unpacked::new(&write_xlsx(&wb).unwrap());
    for path in out.parts.keys().filter(|p| p.as_str() != "[Content_Types].xml") {
        let mut candidate = wb.clone();
        candidate.preserved = Some(named_owner(&format!("{path}/leaf.bin"), true));
        assert_eq!(claims::validate(&candidate), Err(Error::OutputName), "{path}");
    }
    for path in [
        "xl/charts/chart2.xml",
        "xl/charts/chartEx1.xml",
        "xl/tables/table2.xml",
        "xl/media/image2.png",
        "xl/media/cellImage2.png",
        "xl/persons/person2.xml",
        "xl/pivotCache/pivotCacheRecords2.xml",
        "xl/worksheets/sheet99.xml",
        "docProps/custom.xml",
    ] {
        let mut candidate = wb.clone();
        candidate.preserved = Some(named_owner(&format!("{path}/leaf.bin"), true));
        assert_eq!(claims::validate(&candidate), Ok(()), "absent {path}");
    }
}

#[test]
fn passthrough_recovery_nested_retained_image_suppresses_native_until_distinct_bytes() {
    use crate::passthrough::recovery::claims;
    for distinct in [false, true] {
        let mut wb = Workbook::new();
        let s = Arc::make_mut(&mut wb.sheets[0]);
        let name = "xl/media/image1.png/original.png";
        s.preserved = Some(Arc::new(Preserved {
            rels: vec![rel("x", "image", name)],
            parts: vec![PreservedPart { data: Arc::from(&b"A"[..]), content_type: "image/png".into(), ..part(name) }],
            ..Default::default()
        }));
        s.images.push(image(b"A", 1));
        if distinct {
            s.images.push(image(b"B", 2));
        }
        let out = Unpacked::new(&write_xlsx(&wb).unwrap());
        assert_eq!(out.parts.contains_key("xl/media/image1.png"), distinct);
        assert!(out.parts.contains_key("xl/media/image1.png/kept1.png"));
        assert_eq!(claims::validate(&wb), if distinct { Err(Error::OutputName) } else { Ok(()) });
    }
}

#[test]
fn passthrough_recovery_checked_array_cache_names_scopes_spine_and_live_storage() {
    use crate::array_formula::{AnalysisCharge, AnalysisError, needs_array_checked};
    use crate::passthrough::recovery::claims::Work;
    use gridcraft_core::CellRef;
    use gridcraft_model::{DefinedName, Formula, FormulaEvalMode};
    let mut wb = Workbook::new();
    Arc::make_mut(&mut wb.sheets[0]).name = "O'Brien".into();
    wb.names.push(DefinedName { name: "RangeName".into(), scope: None, formula: "A1:A3".into(), comment: String::new(), hidden: false });
    wb.names.push(DefinedName { name: "RangeName".into(), scope: Some(0), formula: "1".into(), comment: String::new(), hidden: false });
    wb.names.push(DefinedName { name: "Cycle".into(), scope: None, formula: "Cycle".into(), comment: String::new(), hidden: false });
    let mut cases = vec![
        "RangeName".to_string(),
        "'O''Brien'!RangeName".into(),
        "Cycle".into(),
        "LET(x,A1:A3,LET(x,1,x))".into(),
        "INDIRECT(\"A1:A3\")".into(),
        "SUM({1,2})+@A1".into(),
    ];
    cases.push(std::iter::repeat_n("A1", 1000).collect::<Vec<_>>().join("+"));
    for text in cases {
        let f = Formula::new(&text);
        for cached in [true, false] {
            let mut f = f.clone();
            if !cached {
                f.expr = None;
            }
            let expected = crate::formula_needs_array(&wb, 0, CellRef::new(0, 0), &f);
            let mut work = Work::default();
            let before = work.scratch;
            let value = needs_array_checked(&wb, 0, CellRef::new(0, 0), &f, &mut |c| match c {
                AnalysisCharge::Work(e, b) => work.touch(e, b).map_err(|_| AnalysisError::Limit),
                AnalysisCharge::Allocate(n) => work.allocate(n).map_err(|_| AnalysisError::Limit),
                AnalysisCharge::Release(n) => {
                    work.release(n);
                    Ok(())
                }
            });
            assert_eq!(value, Ok(expected), "{cached}: {text}");
            assert_eq!(work.scratch, before, "released {text}");
        }
    }
    let mut f = Formula::new("@A1");
    f.eval_mode = FormulaEvalMode::Legacy;
    assert_eq!(needs_array_checked(&wb, 0, CellRef::new(0, 0), &f, &mut |_| Ok(())), Ok(false));
    let mut f = Formula::new("LET(x,1,x)");
    f.expr = None;
    let mut allocated = false;
    assert_eq!(
        needs_array_checked(&wb, 0, CellRef::new(0, 0), &f, &mut |c| if matches!(c, AnalysisCharge::Allocate(_)) {
            allocated = true;
            Err(AnalysisError::Limit)
        } else {
            Ok(())
        }),
        Err(AnalysisError::Limit)
    );
    assert!(allocated);
    let f = Formula { expr: None, text: "x".repeat(4 * 8192 + 1), ..Formula::new("1") };
    assert_eq!(
        needs_array_checked(&wb, 0, CellRef::new(0, 0), &f, &mut |c| if matches!(c, AnalysisCharge::Allocate(_)) {
            panic!("parser not requested after proven refusal")
        } else {
            Ok(())
        }),
        Ok(false)
    );
}

#[test]
fn passthrough_recovery_claim_work_limits_refuse_without_guessing_and_retry() {
    use crate::passthrough::recovery::claims::{self, Work};
    let mut wb = conditional_native_workbook();
    wb.preserved = Some(named_owner("xl/python.xml/leaf.bin", true));
    for work in [Work { events: 0, ..Default::default() }, Work { bytes: 0, ..Default::default() }, Work { scratch: 0, ..Default::default() }] {
        assert_eq!(claims::validate_with(&wb, work), Err(Error::WorkLimit));
    }
    assert_eq!(claims::validate(&wb), Err(Error::OutputName));
}

#[test]
fn passthrough_recovery_scanner_scalar_entity_namespace_refusals_are_finite() {
    use crate::passthrough::recovery::scanner::{Failure, scan_recovery};
    for (xml, want) in [
        ("<x>ordinary &amp; &#65; &#xE9;</x>", Ok(())),
        ("<x a='&lt;&amp;' />", Ok(())),
        ("<x>\u{1}</x>", Err(Failure::Malformed)),
        ("<x a='&#1;'/>", Err(Failure::Malformed)),
        ("<x>&missing;</x>", Err(Failure::Malformed)),
        ("<x>&#0;</x>", Err(Failure::Malformed)),
        ("<x>bare & text</x>", Err(Failure::Malformed)),
        ("<x a='<'/>", Err(Failure::Malformed)),
        ("<x><![CDATA[valid]]></x>", Err(Failure::Unsupported)),
        ("<x><?valid instruction?></x>", Err(Failure::Unsupported)),
        ("<!DOCTYPE x [<!ENTITY e 'text'>]><x>&e;</x>", Err(Failure::Unsupported)),
        ("<?xml version='1.0' encoding='iso-8859-1'?><x/>", Err(Failure::Unsupported)),
        ("<x/><y/>", Err(Failure::Malformed)),
        ("<x xmlns:a='urn:n' xmlns:b='urn:n' a:z='1' b:z='2'/>", Err(Failure::Malformed)),
        ("<x:root/>", Err(Failure::Malformed)),
        ("<1invalid/>", Err(Failure::Malformed)),
    ] {
        assert_eq!(scan_recovery(xml.as_bytes(), false, false, |_, _| Ok(())), want, "{xml}");
    }
    assert_eq!(scan_recovery(&[0xff, 0xfe, b'<', 0, b'x', 0, b'/', 0, b'>', 0], false, false, |_, _| Ok(())), Err(Failure::Unsupported));
    assert_eq!(scan_recovery(b"<x>\xff</x>", false, false, |_, _| Ok(())), Err(Failure::Malformed));
}

#[test]
fn passthrough_recovery_scanner_writer_remap_representation_and_real_limits() {
    use crate::passthrough::recovery::scanner::{Failure, scan_recovery};
    let rebinding = format!("<x xmlns:r='{NS_R}'><a xmlns:r='urn:other' r:id='inert'/></x>");
    assert_eq!(scan_recovery(rebinding.as_bytes(), true, true, |_, _| Ok(())), Err(Failure::Unsupported));
    assert_eq!(scan_recovery(rebinding.as_bytes(), false, false, |_, _| Ok(())), Ok(()));
    let encoded = format!("<picture xmlns:r='{NS_R}' r:id='r&#49;'/>");
    assert_eq!(scan_recovery(encoded.as_bytes(), true, true, |_, _| Ok(())), Err(Failure::Unsupported));
    assert_eq!(scan_recovery(encoded.as_bytes(), false, false, |_, _| Ok(())), Ok(()));
    for xml in [
        format!("<{} />", "x".repeat(1025)),
        format!("<x a='{}'/>", "a".repeat(8193)),
        format!("{}{}", "<x>".repeat(257), "</x>".repeat(257)),
        " ".repeat(4 * 1024 * 1024 + 1),
    ] {
        assert_eq!(scan_recovery(xml.as_bytes(), true, false, |_, _| Ok(())), Err(Failure::Limit));
    }
    let depth = format!("{}{}", "<x>".repeat(256), "</x>".repeat(256));
    assert_eq!(scan_recovery(depth.as_bytes(), false, false, |_, _| Ok(())), Ok(()));
}

#[test]
fn passthrough_recovery_scanner_prefix_returns_and_reserved_namespace_refuse() {
    use crate::passthrough::recovery::scanner::{Failure, scan_recovery};
    let xml = format!("<x xmlns:r='urn:inert'><a xmlns:r='{NS_R}' r:id='known'/><b r:id='inert'/></x>");
    assert_eq!(scan_recovery(xml.as_bytes(), true, true, |_, _| Ok(())), Err(Failure::Unsupported));
    assert_eq!(scan_recovery(xml.as_bytes(), false, false, |_, _| Ok(())), Ok(()));
    assert_eq!(scan_recovery(b"<xmlns:root/>", false, false, |_, _| Ok(())), Err(Failure::Malformed));
}

#[test]
fn passthrough_recovery_scanner_legacy_raw_spans_admission_and_borrowed_text() {
    use crate::passthrough::recovery::scanner::{Failure, RecoveryToken, Token, scan, scan_recovery};
    let xml = b"<picture xmlns:r='http://schemas.openxmlformats.org/officeDocument/2006/relationships' r:id='a&amp;b'> raw text </picture>";
    let mut seen = false;
    scan(xml, true, |path, token| {
        if let Token::Start(tag) = token {
            assert_eq!(path[0].ns, NS_MAIN);
            let f = tag.field(NS_R, "id").unwrap();
            assert_eq!(f.value, "a&b");
            assert_eq!(&xml[f.span.clone()], b"a&amp;b");
        } else if let Token::Text(f) = token {
            assert_eq!(f.value, " raw text ");
            assert_eq!(&xml[f.span], f.value.as_bytes());
            seen = true;
        }
        Ok(())
    })
    .unwrap();
    assert!(seen);
    for xml in ["<x>\u{1}</x>", "<x a='&#1;'/>"] {
        assert!(scan(xml.as_bytes(), false, |_, _| Ok(())).is_ok());
        assert_eq!(scan_recovery(xml.as_bytes(), false, false, |_, _| Ok(())), Err(Failure::Malformed));
    }
    for xml in ["<x>&amp;</x>", "<x><![CDATA[text]]></x>", "<x><?valid instruction?></x>"] {
        assert!(scan(xml.as_bytes(), false, |_, _| Ok(())).is_err());
    }
    let mut text = String::new();
    scan_recovery(b"<x>A&amp;&#66;</x>", false, false, |_, t| {
        match t {
            RecoveryToken::Text(s, _) => text.push_str(s),
            RecoveryToken::Reference(c) => text.push(c),
            _ => {}
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(text, "A&B");
    let mut xml = String::from("<x>");
    xml.push_str(&"a".repeat(4 * 1024 * 1024));
    xml.push_str("</x>");
    let input = xml.as_ptr() as usize;
    scan_recovery(xml.as_bytes(), false, false, |_, t| {
        if let RecoveryToken::Text(s, span) = t {
            assert_eq!(s.as_ptr() as usize, input + span.start);
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(scan_recovery(xml.as_bytes(), true, false, |_, _| Ok(())), Err(Failure::Limit));
    assert_eq!(scan_recovery(b"<?xml version='1.0' encoding='iso-8859-1'?><x>\xff</x>", false, false, |_, _| Ok(())), Err(Failure::Unsupported));
}

#[test]
fn passthrough_recovery_scanner_inclusive_attribute_name_and_event_limits() {
    use crate::passthrough::recovery::scanner::{Failure, scan_recovery};
    for (attrs, want) in [(128, Ok(())), (129, Err(Failure::Limit))] {
        let xml = format!("<x {}/>", (0..attrs).map(|i| format!("a{i}='v'")).collect::<Vec<_>>().join(" "));
        assert_eq!(scan_recovery(xml.as_bytes(), false, false, |_, _| Ok(())), want);
    }
    for xml in [format!("<{} />", "x".repeat(1024)), format!("<x a='{}'/>", "a".repeat(8192))] {
        assert_eq!(scan_recovery(xml.as_bytes(), false, false, |_, _| Ok(())), Ok(()));
    }
    let mut xml = String::from("<x>");
    xml.push_str(&"<y/>".repeat(crate::xml::MAX_XML_NODES - 3));
    xml.push_str("</x>");
    assert_eq!(scan_recovery(xml.as_bytes(), false, false, |_, _| Ok(())), Ok(()));
    xml.insert_str(3, "<y/>");
    assert_eq!(scan_recovery(xml.as_bytes(), false, false, |_, _| Ok(())), Err(Failure::Limit));
}

#[test]
fn passthrough_recovery_logical_growth_and_table_geometry_preserve_finite_failures() {
    use crate::passthrough::recovery::claims::{self, Work};
    let mut work = Work { scratch: 11, ..Default::default() };
    let mut values = Vec::new();
    for n in 0u8..4 {
        work.push(&mut values, n).unwrap();
    }
    assert_eq!(work.scratch, 7);
    assert_eq!(work.push(&mut values, 4), Err(Error::WorkLimit));
    assert_eq!(values, [0, 1, 2, 3]);
    let mut work = Work { scratch: 12, ..Default::default() };
    let mut values = Vec::new();
    for n in 0u8..5 {
        work.push(&mut values, n).unwrap();
    }
    assert_eq!(work.scratch, 4);
    assert_eq!(values, [0, 1, 2, 3, 4]);
    let mut wb = conditional_native_workbook();
    let s = Arc::make_mut(&mut wb.sheets[0]);
    s.tables[0].range.start.row = 4;
    s.tables[0].range.end.row = 1;
    assert_eq!(crate::sheet_write::table_is_written(&s.tables[0]), None);
    assert!(matches!(write_xlsx(&wb), Err(crate::IoError::TooLarge(_))));
    wb.preserved = Some(named_owner("xl/tables/table1.xml/leaf.bin", true));
    assert_eq!(claims::validate(&wb), Err(Error::Owner));
}

#[test]
fn passthrough_recovery_complete_private_positive_graphs_and_real_dangling_chart() {
    use crate::passthrough::recovery::check_all;
    assert_eq!(check_all(&Workbook::new()), Ok(()));
    for bytes in [rich_package(), rich_package_with(CONTENT_TYPES, true), header_picture_package(), xml_map_package()] {
        let (wb, _) = read_xlsx(&bytes).unwrap();
        assert_eq!(check_all(&wb), Ok(()));
    }
    let (mut wb, _) = read_xlsx(&chartsheet_package()).unwrap();
    assert_eq!(check_all(&wb), Ok(()));
    let p = Arc::make_mut(wb.preserved.as_mut().unwrap());
    p.parts.iter_mut().find(|q| q.name == "xl/chartsheets/sheet2.xml").unwrap().rels.retain(|r| r.id != "rId2");
    assert_eq!(check_all(&wb), Err(Error::MissingTarget), "historical missing-rId2 clone remains hostile");
}

#[test]
fn passthrough_recovery_shared_pivot_borrowed_axis_and_sheet_name_controls() {
    use gridcraft_core::{CellRef, Value};
    use gridcraft_model::{Cell, PivotTable};
    let mut wb = Workbook::new();
    let s = Arc::make_mut(&mut wb.sheets[0]);
    s.cells.set(CellRef::new(0, 0), Cell::value(Value::text("Header")));
    s.cells.set(CellRef::new(1, 0), Cell::value(Value::Number(2.)));
    for source in ["A1:A2", "$A$1:$A$2", " A:A ", "$A:$A", "2:1", "$2:$1", "A2:A1", "XFD:XFD", "0:2", "A::B", "$A$:B", "1048577:1048578"] {
        let mut candidate = wb.clone();
        Arc::make_mut(&mut candidate.sheets[0]).pivots.push(PivotTable { source: source.into(), ..Default::default() });
        let actual = Unpacked::new(&write_xlsx(&candidate).unwrap()).parts.contains_key("xl/pivotCache/pivotCacheDefinition1.xml");
        let fact = crate::pivot::PivotWriter::recovery_name_facts(&candidate, 0, &candidate.sheets[0].pivots[0], false, &mut |_, _| Ok::<_, ()>(()))
            .unwrap()
            .is_some();
        assert_eq!(fact, actual, "{source}");
    }
    for text in ["'O''Brien'!A1,Data!A2", "[1]Sheet!A1", "'A!B'!A1", "SUM( Data!A1,Other!A2)", "'unterminated", "'x''y'!A1"] {
        let mut borrowed = Vec::new();
        let result: Result<(), ()> = crate::passthrough::recovery::visit_sheet_names(text, |raw, quoted| {
            borrowed.push(if quoted { raw.replace("''", "'") } else { raw.to_string() });
            Ok(())
        });
        result.unwrap();
        assert_eq!(borrowed, crate::passthrough::sheet_names_in(text));
    }
}

#[test]
fn passthrough_recovery_complete_slots_and_local_opaque_references() {
    use crate::passthrough::recovery::check_all;
    use gridcraft_model::PreservedXml;
    for (xml, want) in [
        (format!("<picture xmlns:r='{NS_R}' r:id='pic'/>"), Ok(())),
        (format!("<picture xmlns='' xmlns:r='{NS_R}' r:id='pic'/>"), Err(Error::Slot)),
        (format!("<picture xmlns:r='{NS_R}' r:id='missing'/>"), Err(Error::MissingTarget)),
        (format!("<controls xmlns:r='{NS_R}' r:id='pic'/>"), Err(Error::Slot)),
        (format!("<picture xmlns:r='{NS_R}' r:id='p&#105;c'/>"), Err(Error::UnsupportedXml)),
    ] {
        let mut wb = Workbook::new();
        Arc::make_mut(&mut wb.sheets[0]).preserved = Some(Arc::new(Preserved {
            rels: vec![rel("pic", "image", "a.xml")],
            parts: vec![part("a.xml")],
            elements: vec![PreservedXml { slot: "picture".into(), xml }],
            ..Default::default()
        }));
        assert_eq!(check_all(&wb), want);
    }
    let mut wb = Workbook::new();
    Arc::make_mut(&mut wb.sheets[0]).preserved =
        Some(Arc::new(Preserved { rels: vec![rel("unused", "printerSettings", "a.bin")], parts: vec![part("a.bin")], ..Default::default() }));
    assert_eq!(check_all(&wb), Err(Error::Slot));
    for body in [format!("<root xmlns:r='{NS_R}' r:id='missing'/>"), "<root>bare & text</root>".into()] {
        let mut wb = Workbook::new();
        let mut p = Preserved { rels: vec![rel("root", "customXml", "customXml/a.xml")], parts: vec![part("customXml/a.xml")], ..Default::default() };
        p.parts[0].data = Arc::from(body.into_bytes());
        wb.preserved = Some(Arc::new(p));
        assert!(matches!(check_all(&wb), Err(Error::MissingTarget | Error::Xml)));
    }
    let mut wb = Workbook::new();
    wb.preserved = Some(Arc::new(Preserved {
        elements: vec![
            PreservedXml { slot: "fileVersion".into(), xml: "<fileVersion appName='a&amp;b'/>".into() },
            PreservedXml {
                slot: "ext".into(),
                xml: "<ext uri='{B58B0392-4F1F-4190-BB64-5DF3571DCE5F}' xmlns:c='http://schemas.microsoft.com/office/spreadsheetml/2018/calcfeatures'><c:calcFeatures><c:feature name='test'/></c:calcFeatures></ext>".into(),
            },
        ],
        ..Default::default()
    }));
    assert_eq!(check_all(&wb), Ok(()));
}

#[test]
fn passthrough_recovery_complete_vml_definitions_references_and_merge_geometry() {
    use crate::passthrough::recovery::check_all;
    let (original, _) = read_xlsx(&rich_package()).unwrap();
    assert_eq!(check_all(&original), Ok(()));
    for (from, to, want) in [
        ("_x0000_s1026", "_x0000_s1025", Error::Vml),
        ("o:relid=\"rId1\"", "o:relid=\"missing\"", Error::MissingTarget),
        ("_x0000_s1026", "_x0000_s18446744073709551616", Error::Vml),
        ("</xml>", "</xml >", Error::UnsupportedXml),
        ("_x0000_s1025", "_x0000_s&#49;025", Error::UnsupportedXml),
    ] {
        let mut wb = original.clone();
        let p = Arc::make_mut(Arc::make_mut(&mut wb.sheets[0]).preserved.as_mut().unwrap());
        let q = p.vml.as_mut().unwrap();
        q.data = Arc::from(std::str::from_utf8(&q.data).unwrap().replace(from, to).into_bytes());
        assert_eq!(check_all(&wb), Err(want), "{from} to {to}");
    }
    let mut wb = original.clone();
    let s = Arc::make_mut(&mut wb.sheets[0]);
    Arc::make_mut(&mut s.comments).clear();
    s.controls.clear();
    let q = Arc::make_mut(s.preserved.as_mut().unwrap()).vml.as_mut().unwrap();
    q.data = Arc::from(std::str::from_utf8(&q.data).unwrap().replace("</xml>", "</xml >").into_bytes());
    assert_eq!(check_all(&wb), Ok(()), "lone opaque VML has no literal merge requirement");
    let mut wb = original;
    Arc::make_mut(&mut wb.sheets[0]).controls.push(Default::default());
    assert_eq!(check_all(&wb), Ok(()), "retained objects, notes and modeled controls coexist");
    let (mut wb, _) = read_xlsx(&header_picture_package()).unwrap();
    let p = Arc::make_mut(Arc::make_mut(&mut wb.sheets[0]).preserved.as_mut().unwrap());
    let q = p.parts.iter_mut().find(|q| q.content_type.ends_with("vmlDrawing")).unwrap();
    q.data = Arc::from(std::str::from_utf8(&q.data).unwrap().replace("data=\"1\"", "data=\"4294967296\"").into_bytes());
    assert_eq!(check_all(&wb), Err(Error::VmlLimit));
}

#[test]
fn passthrough_recovery_complete_chart_entry_identity_names_and_formula_controls() {
    use crate::passthrough::recovery::check_all;
    let (original, _) = read_xlsx(&chartsheet_package()).unwrap();
    assert_eq!(check_all(&original), Ok(()));
    for change in 0..9 {
        let mut wb = original.clone();
        let p = Arc::make_mut(wb.preserved.as_mut().unwrap());
        match change {
            0 => p.sheets[0].name = "x".repeat(32),
            1 => p.sheets[0].name = "data".into(),
            2 => p.sheets[1].name = p.sheets[0].name.clone(),
            3 => p.sheets[0].sheet_id = 0,
            4 => p.sheets[0].sheet_id = 1,
            5 => p.sheets[1].sheet_id = p.sheets[0].sheet_id,
            6 => p.sheets[0].before = wb.sheets.len() + 1,
            7 => {
                p.sheets[0].before = 2;
                p.sheets[1].before = 1;
            }
            _ => p.rels.iter_mut().find(|r| r.id == p.sheets[0].rel_id).unwrap().external = true,
        };
        assert_eq!(check_all(&wb), Err(Error::ChartSheet), "case {change}");
    }
    let mut wb = original.clone();
    Arc::make_mut(&mut wb.sheets[0]).file_id = 0;
    assert_eq!(check_all(&wb), Ok(()), "new worksheet file_id0 is legitimate");
    for formula in ["Missing!$A$1", "'[1]Unopened'!$A$1"] {
        let mut wb = original.clone();
        let p = Arc::make_mut(wb.preserved.as_mut().unwrap());
        let q = p.parts.iter_mut().find(|q| q.name == "xl/charts/chart1.xml").unwrap();
        q.data = Arc::from(chart_xml(formula).into_bytes());
        assert_eq!(check_all(&wb), if formula.starts_with("'[") { Ok(()) } else { Err(Error::ChartSheet) });
    }
}

#[test]
fn passthrough_recovery_complete_ordered_roots_and_cross_owner_locality() {
    use crate::passthrough::recovery::check_all;
    for reverse in [false, true] {
        let mut p = chain(17);
        p.rels.push(rel("shortcut", "customXml", "customXml/n17.xml"));
        if reverse {
            p.rels.reverse();
        }
        let mut wb = Workbook::new();
        wb.preserved = Some(Arc::new(p));
        assert_eq!(check_all(&wb), if reverse { Ok(()) } else { Err(Error::ChainLimit) });
    }
    let (mut wb, _) = read_xlsx(&rich_package_with(CONTENT_TYPES, true)).unwrap();
    let p = Arc::make_mut(Arc::make_mut(&mut wb.sheets[1]).preserved.as_mut().unwrap());
    p.parts.retain(|q| q.content_type != "image/x-emf");
    assert_eq!(check_all(&wb), Err(Error::MissingTarget));
}

#[test]
#[cfg(unix)]
fn passthrough_recovery_complete_save_as_keeps_distinct_original_context_and_inert_links() {
    use crate::passthrough::recovery::check_all;
    let (mut wb, _) = read_xlsx(&rich_package()).unwrap();
    wb.source_dir = Some("/p3-unopened/source".into());
    wb.source_path = Some("/p3-unopened/different/Current.xlsx".into());
    let p = Arc::make_mut(wb.preserved.as_mut().unwrap());
    let q = p.parts.iter_mut().find(|q| q.name == "xl/externalLinks/externalLink1.xml").unwrap();
    q.rels[0].target = "Source.xlsx".into();
    q.rels.push(PreservedRel { external: true, ..rel("inert", "hyperlink", "notes.xlsx") });
    let original = wb.clone();
    assert_eq!(check_all(&wb), Ok(()));
    assert_eq!(wb.preserved, original.preserved);
    assert_eq!(wb.source_dir, original.source_dir);
    assert_eq!(wb.source_path, original.source_path);
    let changed = crate::rebase_links(&wb, std::path::Path::new("/p3-unopened/destination")).unwrap();
    assert_eq!(check_all(&changed), Ok(()));
    let q = changed.preserved.as_ref().unwrap().parts.iter().find(|q| q.name == "xl/externalLinks/externalLink1.xml").unwrap();
    assert_eq!(q.rels[0].target, "../source/Source.xlsx");
    assert_eq!(q.rels[1].target, "notes.xlsx");
    assert_eq!(changed.source_dir, wb.source_dir);
    assert_eq!(changed.source_path, wb.source_path);
    assert_eq!(wb.preserved, original.preserved);
}

#[test]
fn passthrough_recovery_vml_excluded_encoding_is_not_malformed_utf8() {
    use crate::passthrough::recovery::check_all;
    for bytes in [
        vec![0xff, 0xfe, b'<', 0, b'x', 0, b'm', 0, b'l', 0, b'/', 0, b'>', 0],
        b"<?xml version='1.0' encoding='iso-8859-1'?><xml>\xff</xml>".to_vec(),
    ] {
        let (mut wb, _) = read_xlsx(&rich_package()).unwrap();
        Arc::make_mut(Arc::make_mut(&mut wb.sheets[0]).preserved.as_mut().unwrap()).vml.as_mut().unwrap().data = Arc::from(bytes);
        assert_eq!(check_all(&wb), Err(Error::UnsupportedXml));
    }
}

#[test]
fn passthrough_recovery_complete_public_signature_and_pivot_lookup_bounds() {
    let validate: fn(&Workbook) -> Result<(), crate::RecoveryPreservedError> = crate::validate_recovery_preserved;
    assert_eq!(validate(&Workbook::new()), Ok(()));
    use gridcraft_model::{DefinedName, PivotTable};
    let mut wb = conditional_native_workbook();
    Arc::make_mut(&mut wb.sheets[0]).name = "O'Brien".into();
    Arc::make_mut(&mut wb.sheets[0]).pivots.clear();
    for (name, scope, formula) in [("Local", Some(0), "A1:A3"), ("Global", None, "Local"), ("Cycle", None, "Cycle")] {
        wb.names.push(DefinedName { name: name.into(), scope, formula: formula.into(), comment: String::new(), hidden: false });
    }
    for source in ["NativeTable", "Local", "Global", "'O''Brien'!A1:A3", "Cycle", "Missing", "A1:K1000000"] {
        let mut candidate = wb.clone();
        Arc::make_mut(&mut candidate.sheets[0]).pivots.push(PivotTable { source: source.into(), ..Default::default() });
        let actual = Unpacked::new(&write_xlsx(&candidate).unwrap()).parts.contains_key("xl/pivotCache/pivotCacheDefinition1.xml");
        let fact = crate::pivot::PivotWriter::recovery_name_facts(&candidate, 0, &candidate.sheets[0].pivots[0], false, &mut |_, _| Ok::<_, ()>(()))
            .unwrap()
            .is_some();
        assert_eq!(fact, actual, "{source}");
    }
    let pt = PivotTable { source: "A1:J1000000".into(), ..Default::default() };
    assert_eq!(crate::pivot::PivotWriter::recovery_name_facts(&wb, 0, &pt, false, &mut |_, _| Ok::<_, ()>(())), Ok(Some(false)));
    let mut work = crate::passthrough::recovery::claims::Work::default();
    assert_eq!(crate::pivot::PivotWriter::recovery_name_facts(&wb, 0, &pt, true, &mut |e, b| work.touch(e, b)), Err(Error::WorkLimit));
}

#[test]
fn passthrough_recovery_scanner_document_space_and_declaration_grammar() {
    use crate::passthrough::recovery::scanner::{Failure, scan_recovery};
    for xml in [
        "\u{a0}<x/>",
        "<x/>\u{2003}",
        "<?xml version='1.0' standalone='maybe'?><x/>",
        "<?xml version='1.0' garbage='x'?><x/>",
        "<?xml version='1.0' standalone='yes' encoding='UTF-8'?><x/>",
        "   <?xml version='1.0'?><x/>",
        "<?XML version='1.0'?><x/>",
    ] {
        assert_eq!(scan_recovery(xml.as_bytes(), false, false, |_, _| Ok(())), Err(Failure::Malformed), "{xml}");
    }
    for xml in [" \t\r\n<x/>", "<?xml version='1.0' encoding='UTF-8' standalone='yes'?><x/>"] {
        assert_eq!(scan_recovery(xml.as_bytes(), false, false, |_, _| Ok(())), Ok(()));
    }
}

#[test]
fn passthrough_recovery_scanner_attribute_normalization_preserves_legacy_raw_values() {
    use crate::passthrough::recovery::scanner::{RecoveryToken, Token, scan, scan_recovery};
    let xml = b"<x a='x\tY\r\nz' b='x&#9;Y'/>";
    scan_recovery(xml, false, false, |_, t| {
        if let RecoveryToken::Start(tag) = t {
            assert_eq!(tag.field("", "a").unwrap().value, "x Y z");
            assert_eq!(tag.field("", "b").unwrap().value, "x\tY");
        }
        Ok(())
    })
    .unwrap();
    scan(xml, false, |_, t| {
        if let Token::Start(tag) = t {
            assert_eq!(tag.field("", "a").unwrap().value, "x\tY\r\nz");
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn passthrough_recovery_chart_text_line_endings_match_actual_retention() {
    for (formula, want) in [("'Line\rBreak'!$A$1", Err(Error::ChartSheet)), ("'Line&#13;Break'!$A$1", Ok(()))] {
        let (mut wb, _) = read_xlsx(&chartsheet_package()).unwrap();
        Arc::make_mut(&mut wb.sheets[0]).name = "Line\rBreak".into();
        let p = Arc::make_mut(wb.preserved.as_mut().unwrap());
        p.parts.iter_mut().find(|q| q.name == "xl/charts/chart1.xml").unwrap().data = Arc::from(chart_xml(formula).into_bytes());
        let out = Unpacked::new(&write_xlsx(&wb).unwrap());
        assert_eq!(written_sheets(&out).iter().any(|(n, _)| n == "Chart1"), want.is_ok());
        assert_eq!(crate::validate_recovery_preserved(&wb), want);
    }
}

#[test]
fn passthrough_recovery_raw_merge_vml_close_in_trailing_comment_is_unrepresentable() {
    let (mut wb, _) = read_xlsx(&rich_package()).unwrap();
    let p = Arc::make_mut(Arc::make_mut(&mut wb.sheets[0]).preserved.as_mut().unwrap());
    let q = p.vml.as_mut().unwrap();
    q.data = Arc::from(format!("{}<!--</xml>-->", std::str::from_utf8(&q.data).unwrap()).into_bytes());
    let out = Unpacked::new(&write_xlsx(&wb).unwrap());
    let root = xml::parse(&out.parts["xl/drawings/vmlDrawing1.vml"]).unwrap();
    assert_eq!(root.kids("shape").count(), 2, "the modeled note was inserted inside a comment");
    assert_eq!(crate::validate_recovery_preserved(&wb), Err(Error::UnsupportedXml));
}

#[test]
fn passthrough_recovery_raw_merge_controls_close_in_comment_is_unrepresentable() {
    let (mut wb, _) = read_xlsx(&rich_package()).unwrap();
    let sh = Arc::make_mut(&mut wb.sheets[0]);
    sh.controls.push(Default::default());
    let p = Arc::make_mut(sh.preserved.as_mut().unwrap());
    let e = p.elements.iter_mut().find(|e| e.slot == "controls").unwrap();
    e.xml = e.xml.replace("</controls>", "<!--</controls>--></controls>");
    let out = Unpacked::new(&write_xlsx(&wb).unwrap());
    let part = sheet_part(&out, "Data");
    let root = xml::parse(&out.parts[&part]).unwrap();
    assert_eq!(root.child("controls").unwrap().kids("control").count(), 1, "modeled control was inserted inside a comment");
    assert_eq!(crate::validate_recovery_preserved(&wb), Err(Error::UnsupportedXml));
}

#[test]
fn passthrough_recovery_remapped_literal_double_quote_needs_finite_refusal() {
    use crate::passthrough::recovery::scanner::{Failure, scan_recovery};
    let (mut wb, _) = read_xlsx(&rich_package()).unwrap();
    let p = Arc::make_mut(Arc::make_mut(&mut wb.sheets[0]).preserved.as_mut().unwrap());
    let e = p.elements.iter_mut().find(|e| e.slot == "controls").unwrap();
    e.xml = e.xml.replace("name=\"Check Box 1\"", "name='Check \" Box 1'");
    let out = Unpacked::new(&write_xlsx(&wb).unwrap());
    let sheet = sheet_part(&out, "Data");
    assert_eq!(
        scan_recovery(&out.parts[&sheet], false, false, |_, _| Ok(())),
        Err(Failure::Malformed),
        "actual start-tag rebuild does not escape a raw nonrel double quote"
    );
    assert_eq!(crate::validate_recovery_preserved(&wb), Err(Error::UnsupportedXml));
    let mut wb = Workbook::new();
    let mut p = Preserved { rels: vec![rel("x", "customXml", "customXml/a.xml")], parts: vec![part("customXml/a.xml")], ..Default::default() };
    let body = b"<root name='literal\"quote'/>";
    p.parts[0].data = Arc::from(body.as_slice());
    wb.preserved = Some(Arc::new(p));
    assert_eq!(crate::validate_recovery_preserved(&wb), Ok(()));
    assert_eq!(Unpacked::new(&write_xlsx(&wb).unwrap()).parts["customXml/a.xml"], body);
}

#[test]
fn passthrough_recovery_raw_merge_safe_prefix_comments_and_choice_remain_valid() {
    for choice in [false, true] {
        let (mut wb, _) = read_xlsx(&rich_package()).unwrap();
        let sh = Arc::make_mut(&mut wb.sheets[0]);
        sh.controls.push(gridcraft_model::FormControl {
            kind: gridcraft_model::ControlKind::Slider,
            anchor: gridcraft_model::Anchor { cell: gridcraft_core::CellRef::new(8, 1), width: 120.0, height: 20.0, ..Default::default() },
            ..Default::default()
        });
        let p = Arc::make_mut(sh.preserved.as_mut().unwrap());
        let e = p.elements.iter_mut().find(|e| e.slot == "controls").unwrap();
        e.xml =
            e.xml.replacen("<controls", &format!("<s:controls xmlns:s='{NS_MAIN}'"), 1).replace("</controls>", "<!-- harmless end --> </s:controls>");
        if choice {
            e.xml = format!(
                "<mc:AlternateContent xmlns:mc='http://schemas.openxmlformats.org/markup-compatibility/2006' xmlns:x14='http://schemas.microsoft.com/office/spreadsheetml/2009/9/main'><mc:Choice Requires='x14'>{}</mc:Choice><mc:Fallback>{}</mc:Fallback></mc:AlternateContent>",
                e.xml, e.xml
            );
        }
        let q = p.vml.as_mut().unwrap();
        q.data = Arc::from(std::str::from_utf8(&q.data).unwrap().replace("</xml>", "<!-- </xml> before true close --></xml>").into_bytes());
        assert_eq!(crate::validate_recovery_preserved(&wb), Ok(()));
        let out = Unpacked::new(&write_xlsx(&wb).unwrap());
        let part = sheet_part(&out, "Data");
        let root = xml::parse(&out.parts[&part]).unwrap();
        let controls = if choice {
            root.kids("AlternateContent").find_map(|e| e.child("Choice").and_then(|c| c.child("controls"))).unwrap()
        } else {
            root.child("controls").unwrap()
        };
        assert_eq!(controls.kids("control").count(), 1, "retained entry remains direct; generated entries use AlternateContent");
        let (reopened, _) = read_xlsx(&write_xlsx(&wb).unwrap()).unwrap();
        assert_eq!(reopened.sheets[0].controls.len(), 1);
    }
    let (mut wb, _) = read_xlsx(&rich_package()).unwrap();
    let sh = Arc::make_mut(&mut wb.sheets[0]);
    Arc::make_mut(&mut sh.comments).clear();
    let p = Arc::make_mut(sh.preserved.as_mut().unwrap());
    let q = p.vml.as_mut().unwrap();
    q.data = Arc::from(format!("{}<!--</xml>-->", std::str::from_utf8(&q.data).unwrap()).into_bytes());
    let e = p.elements.iter_mut().find(|e| e.slot == "controls").unwrap();
    e.xml = e.xml.replace("</controls>", "<!--</controls>--></controls>");
    assert_eq!(crate::validate_recovery_preserved(&wb), Ok(()), "no modeled merge, raw comments stay inert");
    let mut wb = Workbook::new();
    wb.preserved = Some(Arc::new(Preserved {
        elements: vec![gridcraft_model::PreservedXml { slot: "fileVersion".into(), xml: "<fileVersion appName='raw\"quote'/>".into() }],
        ..Default::default()
    }));
    assert_eq!(crate::validate_recovery_preserved(&wb), Ok(()), "fileVersion is emitted raw");
}

#[test]
fn passthrough_recovery_known_chart_role_keeps_formula_checks_when_mime_case_differs() {
    let (mut wb, _) = read_xlsx(&chartsheet_package()).unwrap();
    let p = Arc::make_mut(wb.preserved.as_mut().unwrap());
    let q = p.parts.iter_mut().find(|q| q.name == "xl/charts/chart1.xml").unwrap();
    q.content_type.make_ascii_uppercase();
    q.data = Arc::from(chart_xml("Missing!$A$1").into_bytes());
    assert_eq!(crate::validate_recovery_preserved(&wb), Err(Error::ChartSheet));
}

#[test]
fn passthrough_recovery_control_insertion_requires_the_main_default_namespace() {
    use crate::passthrough::recovery::scanner::{RecoveryToken, scan_recovery};
    let (mut wb, _) = read_xlsx(&rich_package()).unwrap();
    let sh = Arc::make_mut(&mut wb.sheets[0]);
    sh.controls.push(Default::default());
    let p = Arc::make_mut(sh.preserved.as_mut().unwrap());
    let e = p.elements.iter_mut().find(|e| e.slot == "controls").unwrap();
    e.xml = e
        .xml
        .replacen("<controls", &format!("<s:controls xmlns:s='{NS_MAIN}' xmlns=''"), 1)
        .replace("</controls>", "</s:controls>")
        .replace("<control ", "<s:control ");
    let out = Unpacked::new(&write_xlsx(&wb).unwrap());
    let part = sheet_part(&out, "Data");
    let (mut all, mut main) = (0, 0);
    scan_recovery(&out.parts[&part], false, false, |path, t| {
        if let RecoveryToken::Start(_) = t
            && path.last().is_some_and(|n| n.local == "control")
        {
            all += 1;
            if path.last().unwrap().ns == NS_MAIN {
                main += 1;
            }
        }
        Ok(())
    })
    .unwrap();
    assert_eq!((all, main), (3, 1), "both generated Choice/Fallback control entries inherited the reset default");
    assert_eq!(crate::validate_recovery_preserved(&wb), Err(Error::UnsupportedXml));
    Arc::make_mut(&mut wb.sheets[0]).controls.clear();
    assert_eq!(crate::validate_recovery_preserved(&wb), Ok(()), "opaque prefixed controls are valid without insertion");
}

#[test]
fn passthrough_recovery_control_insertion_empty_choice_is_not_a_fallback_target() {
    for (choice, want) in [("<controls/>", Err(Error::UnsupportedXml)), ("", Ok(()))] {
        let mut wb = Workbook::new();
        let sh = Arc::make_mut(&mut wb.sheets[0]);
        sh.controls.push(Default::default());
        sh.preserved = Some(Arc::new(Preserved {
            elements: vec![gridcraft_model::PreservedXml {
                slot: "controls".into(),
                xml: format!(
                    "<mc:AlternateContent xmlns:mc='http://schemas.openxmlformats.org/markup-compatibility/2006'><mc:Choice Requires='x14' xmlns:x14='http://schemas.microsoft.com/office/spreadsheetml/2009/9/main'>{choice}</mc:Choice><mc:Fallback><controls></controls></mc:Fallback></mc:AlternateContent>"
                ),
            }],
            ..Default::default()
        }));
        assert_eq!(crate::validate_recovery_preserved(&wb), want);
    }
}

#[test]
fn passthrough_recovery_chart_entries_can_share_one_valid_local_relationship() {
    let (mut wb, _) = read_xlsx(&chartsheet_package()).unwrap();
    let p = Arc::make_mut(wb.preserved.as_mut().unwrap());
    p.sheets.truncate(1);
    let mut twin = p.sheets[0].clone();
    twin.name = "TwinChart".into();
    twin.sheet_id = 10;
    p.sheets.push(twin);
    p.rels.retain(|r| r.id != "rId5");
    p.parts.retain(|q| !["xl/chartsheets/sheet2.xml", "xl/drawings/drawing2.xml", "xl/charts/chart2.xml"].contains(&q.name.as_str()));
    let out = Unpacked::new(&write_xlsx(&wb).unwrap());
    assert!(written_sheets(&out).iter().any(|(n, _)| n == "TwinChart"));
    assert_eq!(crate::validate_recovery_preserved(&wb), Ok(()));
}

#[test]
fn passthrough_recovery_generated_prefix_controls_require_rel_binding_for_r() {
    use crate::passthrough::recovery::scanner::{RecoveryToken, scan_recovery};
    let (mut wb, _) = read_xlsx(&rich_package()).unwrap();
    let sh = Arc::make_mut(&mut wb.sheets[0]);
    sh.controls.push(Default::default());
    let p = Arc::make_mut(sh.preserved.as_mut().unwrap());
    let e = p.elements.iter_mut().find(|e| e.slot == "controls").unwrap();
    e.xml = e.xml.replace(&format!("xmlns:r=\"{NS_R}\""), &format!("xmlns:k=\"{NS_R}\" xmlns:r=\"urn:inert\"")).replace("r:id=", "k:id=");
    let out = Unpacked::new(&write_xlsx(&wb).unwrap());
    let part = sheet_part(&out, "Data");
    let (mut all, mut linked) = (0, 0);
    scan_recovery(&out.parts[&part], false, false, |path, t| {
        if let RecoveryToken::Start(tag) = t
            && path.last().is_some_and(|n| n.ns == NS_MAIN && n.local == "control")
        {
            all += 1;
            if tag.field(NS_R, "id").is_some() {
                linked += 1;
            }
        }
        Ok(())
    })
    .unwrap();
    assert_eq!((all, linked), (3, 1), "generated r:id binds to the retained inert URI");
    assert_eq!(crate::validate_recovery_preserved(&wb), Err(Error::UnsupportedXml));
    Arc::make_mut(&mut wb.sheets[0]).controls.clear();
    assert_eq!(crate::validate_recovery_preserved(&wb), Ok(()));
}

#[test]
fn passthrough_recovery_generated_prefix_vml_require_native_bindings_only_for_merge() {
    use crate::passthrough::recovery::scanner::{Failure, scan_recovery};
    let (mut wb, _) = read_xlsx(&rich_package()).unwrap();
    let p = Arc::make_mut(Arc::make_mut(&mut wb.sheets[0]).preserved.as_mut().unwrap());
    let q = p.vml.as_mut().unwrap();
    let text = std::str::from_utf8(&q.data)
        .unwrap()
        .replace("xmlns:v=", "xmlns:w=")
        .replace("v:", "w:")
        .replace("xmlns:o=", "xmlns:f=")
        .replace("o:", "f:")
        .replace("xmlns:x=", "xmlns:z=")
        .replace("x:", "z:");
    q.data = Arc::from(text.into_bytes());
    let out = Unpacked::new(&write_xlsx(&wb).unwrap());
    assert_eq!(scan_recovery(&out.parts["xl/drawings/vmlDrawing1.vml"], false, false, |_, _| Ok(())), Err(Failure::Malformed));
    assert_eq!(crate::validate_recovery_preserved(&wb), Err(Error::UnsupportedXml));
    Arc::make_mut(&mut Arc::make_mut(&mut wb.sheets[0]).comments).clear();
    assert_eq!(crate::validate_recovery_preserved(&wb), Ok(()), "lone alias-prefixed VML is emitted raw");
}

#[test]
fn passthrough_recovery_remapped_xmlns_prefixed_relationship_refuses() {
    use gridcraft_model::PreservedXml;
    let image =
        PreservedPart { name: "xl/media/alias.png".into(), content_type: "image/png".into(), data: Arc::from(&b"P3-alias-image"[..]), rels: vec![] };
    let raw = format!("<root xmlns:xmlnsfoo='{NS_R}' xmlnsfoo:id='old'/>");
    let mut body = part("customXml/alias.xml");
    body.data = Arc::from(raw.as_bytes());
    body.rels = vec![rel("old", "image", &image.name)];
    let mut wb = Workbook::new();
    wb.preserved =
        Some(Arc::new(Preserved { rels: vec![rel("root", "customXml", &body.name)], parts: vec![body, image.clone()], ..Default::default() }));
    assert_eq!(crate::validate_recovery_preserved(&wb), Ok(()), "raw opaque XML is not remapped");
    let out = Unpacked::new(&write_xlsx(&wb).unwrap());
    let names = out.find(raw.as_bytes());
    assert_eq!(names.len(), 1);
    let (kind, target) = out.target(names[0], "old");
    assert_eq!(kind, format!("{NS_R}/image"));
    assert_eq!(out.parts[&target], image.data.as_ref());
    out.assert_valid();

    for prefix in ["r", "xmlfoo", "Xmlnsfoo", "xmlnsfoo", "xmlns_foo"] {
        let mut wb = Workbook::new();
        Arc::make_mut(&mut wb.sheets[0]).preserved = Some(Arc::new(Preserved {
            rels: vec![rel("old", "image", &image.name)],
            parts: vec![image.clone()],
            elements: vec![PreservedXml { slot: "picture".into(), xml: format!("<picture xmlns:{prefix}='{NS_R}' {prefix}:id='old'/>") }],
            ..Default::default()
        }));
        let out = Unpacked::new(&write_xlsx(&wb).unwrap());
        let sheet = sheet_part(&out, &wb.sheets[0].name);
        let relationships = out.rels(&sheet);
        let image_rel: Vec<_> = relationships.iter().filter(|r| r.1 == format!("{NS_R}/image")).collect();
        assert_eq!(image_rel.len(), 1);
        let (id, _, target, external) = image_rel[0];
        assert!(!external);
        assert!(id.strip_prefix("rId").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())));
        assert_ne!(id, "old");
        assert_eq!(out.parts[target], image.data.as_ref());
        let value = attr_of(&out.text(&sheet), "picture", &format!("{prefix}:id"));
        let excluded = prefix.starts_with("xmlns");
        assert_eq!(value, if excluded { "old" } else { id });
        assert_eq!(relationships.iter().any(|r| r.0 == value), !excluded);
        eprintln!("prefix={prefix}; actual picture ID={value}; emitted relationship ID={id}; target={target}");
        assert_eq!(crate::validate_recovery_preserved(&wb), if excluded { Err(Error::UnsupportedXml) } else { Ok(()) }, "{prefix}");
    }
}
