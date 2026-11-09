use super::*;
use crate::recovery::{RecoveryCapture, decode as decode_recovery};
use gridcraft_core::{CellRef, Value};
use gridcraft_model::{Cell, CellPicture};
use std::sync::Arc;

mod goldens;

pub(super) fn metadata() -> Metadata {
    Metadata {
        frozen_active_sheet: None,
        title: "complete".into(),
        original_path_hint: Some(NativePath::Windows(vec![0xd800, 0])),
        file_kind: Some(RecoveryFileKind::Sylk),
        dirty_at_capture: true,
        import_issues: vec![],
        selection: Selection::default(),
        sheet_selections: vec![Selection::at(CellRef::new(7, 2)); 4],
        captured_revision: 0,
        id: "0123456789abcdef0123456789abcdef".parse().unwrap(),
        sequence: 0,
        provenance: RecoveryProvenance::Plain,
    }
}
pub(super) fn bytes(workbook: &Workbook, metadata: &Metadata) -> Vec<u8> {
    let capture = RecoveryCapture { workbook, metadata };
    let prepared = capture.prepare().unwrap();
    let mut bytes = Vec::new();
    prepared.write_to(&mut bytes).unwrap();
    assert_eq!(bytes.len(), prepared.len());
    bytes
}
pub(super) fn png() -> Vec<u8> {
    let mut b = vec![0; 33];
    b[..16].copy_from_slice(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR");
    b[19] = 1;
    b[23] = 1;
    b
}
pub(super) fn picture() -> Arc<CellPicture> {
    Arc::new(CellPicture { data: png(), mime: "image/png".into(), alt: "kept".into() })
}

#[test]
fn complete_facades_roundtrip_fresh_uid_full_memory_and_derived_state() {
    let mut source = Workbook::new();
    let meta = metadata();
    let sheet = Arc::make_mut(&mut source.sheets[0]);
    sheet.file_id = 0;
    sheet.cells.set(
        CellRef::default(),
        Cell {
            value: Value::Number(-0.0),
            formula: Some(Arc::new(gridcraft_model::Formula {
                text: "1+2".into(),
                expr: None,
                array: None,
                dynamic: false,
                eval_mode: gridcraft_model::FormulaEvalMode::Legacy,
            })),
            ..Default::default()
        },
    );
    let encoded = bytes(&source, &meta);
    let decoded = decode_recovery(&encoded).unwrap();
    assert_eq!(decoded.id(), meta.id);
    assert_eq!(decoded.sequence(), 0);
    assert_eq!(decoded.captured_revision(), 0);
    assert_ne!(source.sheets[0].uid.get(), decoded.workbook.sheets[0].uid.get());
    assert_eq!(decoded.metadata.sheet_selections, meta.sheet_selections);
    let cell = decoded.workbook.sheets[0].cells.get(CellRef::default()).unwrap();
    let Value::Number(n) = cell.value else { panic!("number") };
    assert_eq!(n.to_bits(), (-0.0f64).to_bits());
    assert!(cell.formula.as_ref().unwrap().expr.is_some());
    assert_eq!(cell.formula.as_ref().unwrap().text, "1+2");
    assert_eq!(bytes(&decoded.workbook, &decoded.metadata), encoded);
}

#[test]
fn complete_rejects_toy_and_ineligible_before_any_sink_write() {
    let limits = RecoveryLimits::default();
    let mut toy = Vec::new();
    frame::Framing::new(b"{}", [&b"abc"[..]].into_iter(), &limits).unwrap().write_to(&mut toy).unwrap();
    assert_eq!(toy.len(), 41);
    assert!(decode_recovery(&toy).is_err());
    let source = Workbook::new();
    let mut meta = metadata();
    meta.provenance = RecoveryProvenance::EncryptedOrigin;
    assert!(matches!(
        RecoveryCapture { workbook: &source, metadata: &meta }.prepare(),
        Err(RecoveryError::Ineligible(RecoveryProvenance::EncryptedOrigin))
    ));
}

pub(super) fn replace_manifest(bytes: &[u8], from: &str, to: &str) -> Vec<u8> {
    let length = u64::from_le_bytes(bytes[20..28].try_into().unwrap()) as usize;
    let text = std::str::from_utf8(&bytes[28..28 + length]).unwrap();
    assert!(text.contains(from), "missing replacement {from}");
    let new = text.replacen(from, to, 1);
    let mut result = bytes[..28].to_vec();
    result.extend_from_slice(new.as_bytes());
    result.extend_from_slice(&bytes[28 + length..]);
    let total = result.len() as u64;
    result[12..20].copy_from_slice(&total.to_le_bytes());
    result[20..28].copy_from_slice(&(new.len() as u64).to_le_bytes());
    result
}

#[test]
fn complete_required_wrappers_schema_and_final_writer_forms_refuse() {
    let input = bytes(&Workbook::new(), &metadata());
    for (from, to) in [
        ("{\"workbook\":", "{\"unknown\":0,\"workbook\":"),
        ("\"original_path_hint\":{\"windows\":[55296,0]},", ""),
        ("\"source_path\":null,", ""),
        ("\"cell_picture_groups\":[],", ""),
        ("\"sheets\":[{\"ordinal\":0,\"file_id\":0,\"spill\":{},\"spill_ranges\":{},\"preserved\":null}]", "\"sheets\":[]"),
        ("\"ordinal\":0", "\"ordinal\":1"),
        ("\"workbook\":{", "\"workbook\":{\"ignored_nested\":0,"),
        ("\"title\":\"complete\"", "\"title\":\"complet\\u0065\""),
        ("\"title\":\"complete\"", "\"\\u0074itle\":\"complete\""),
        ("\"size\":11.0", "\"size\":11e0"),
        ("\"major_font_ea\":\"\",", ""),
        ("\"provenance\":\"Plain\"", "\"provenance\":\"Future\""),
    ] {
        let mut stats = Stats::default();
        let result = decode_run(&replace_manifest(&input, from, to), &RecoveryLimits::default(), &mut stats);
        assert!(result.is_err(), "{from}");
        if ["\"workbook\":{", "\"title\":\"complete\"", "\"size\":11.0", "\"major_font_ea\":\"\","].contains(&from) {
            assert!(matches!(result, Err(RecoveryError::UnsupportedRepresentation("manifest_writer_form"))), "{from}");
            assert_eq!(stats.final_compare, 1);
        }
    }
    let length = u64::from_le_bytes(input[20..28].try_into().unwrap()) as usize;
    let value: serde_json::Value = serde_json::from_slice(&input[28..28 + length]).unwrap();
    let reordered = serde_json::to_vec(&value).unwrap();
    let mut bad = Vec::new();
    frame::Framing::new(&reordered, std::iter::empty(), &RecoveryLimits::default()).unwrap().write_to(&mut bad).unwrap();
    assert!(matches!(decode_recovery(&bad), Err(RecoveryError::UnsupportedRepresentation("manifest_writer_form"))));
}
#[test]
fn complete_model_and_preserved_graph_errors_are_real_rejections() {
    let source = Workbook::new();
    let input = bytes(&source, &metadata());
    let bad = replace_manifest(&input, "\"bands\":{}", "\"bands\":{\"0\":{\"rows\":{}}}");
    let mut stats = Stats::default();
    assert!(matches!(decode_run(&bad, &RecoveryLimits::default(), &mut stats), Err(RecoveryError::Malformed("model_storage"))));
    assert_eq!(stats.derived_calls, 0);
    let mut source = Workbook::new();
    source.preserved = Some(Arc::new(Preserved {
        parts: vec![gridcraft_model::PreservedPart {
            name: "unreachable.bin".into(),
            content_type: "application/octet-stream".into(),
            data: Arc::from(&b"x"[..]),
            rels: vec![],
        }],
        ..Default::default()
    }));
    assert!(matches!(
        RecoveryCapture { workbook: &source, metadata: &metadata() }.prepare(),
        Err(RecoveryError::Malformed("preserved_unreachable_part"))
    ));
    let manifest = write::manifest_bytes(&source, &metadata(), &RecoveryLimits::default()).unwrap().bytes;
    let mut bad = Vec::new();
    frame::Framing::new(&manifest, source_frames(&source, &RecoveryLimits::default()).unwrap(), &RecoveryLimits::default())
        .unwrap()
        .write_to(&mut bad)
        .unwrap();
    assert!(matches!(decode_recovery(&bad), Err(RecoveryError::Malformed("preserved_unreachable_part"))));
}
#[test]
fn full_semantics_survive_actual_adoption_and_derived_rebuild() {
    use gridcraft_model::{DefinedName, Formula, FormulaEvalMode, PyCode, PyOutput, Style, StyleId};
    let mut source = Workbook::new();
    let meta = metadata();
    let mut style = Style::default();
    style.font.size = -0.0;
    source.styles = serde_json::from_value(serde_json::json!({"styles":[style,style]})).unwrap();
    let sheet = Arc::make_mut(&mut source.sheets[0]);
    sheet.file_id = u32::MAX;
    for (i, text) in ["1+2", "SUM(", &"x".repeat(8193)].into_iter().enumerate() {
        sheet.cells.set(
            CellRef::new(i as u32, 0),
            Cell {
                value: Value::Number(if i == 0 { 42.0 } else { -0.0 }),
                style: StyleId(1),
                formula: Some(Arc::new(Formula {
                    text: text.into(),
                    expr: None,
                    array: Some(RangeRef::new(CellRef::new(i as u32, 0), CellRef::new(i as u32, 1))),
                    dynamic: true,
                    eval_mode: FormulaEvalMode::Legacy,
                })),
                ..Default::default()
            },
        );
    }
    sheet.cells.set(CellRef::new(1024, 2), Cell { style: StyleId(1), ..Default::default() });
    sheet.cells.set(
        CellRef::new(4, 0),
        Cell {
            value: Value::Number(99.0),
            py: Some(Arc::new(PyCode {
                code: "x\r\ny\rz".into(),
                object: true,
                stale: true,
                output: Some(PyOutput {
                    value: Value::Text("output".into()),
                    type_name: "object".into(),
                    preview: " preview ".into(),
                    convertible: false,
                }),
            })),
            ..Default::default()
        },
    );
    sheet.spill.insert(CellRef::new(1, 1), Value::Text("spill".into()));
    sheet.spill_ranges.insert(CellRef::new(1, 0), RangeRef::new(CellRef::new(1, 0), CellRef::new(1, 1)));
    sheet.shapes.push(gridcraft_model::Shape {
        id: 7,
        kind: gridcraft_model::ShapeKind::Rectangle,
        anchor: Default::default(),
        fill: gridcraft_model::Color::Auto,
        line: gridcraft_model::Color::Auto,
        text: "kept".into(),
        format: gridcraft_model::ShapeFormat { font_size: Some(-0.0), ..Default::default() },
        math: Default::default(),
    });
    source.names.push(DefinedName { name: "_gc_macro_0_0".into(), scope: None, formula: "kept".into(), comment: "literal".into(), hidden: true });
    source.macros = vec![gridcraft_model::Macro { name: "M".into(), ..Default::default() }].into();
    source.macros_unread = Arc::new(vec![serde_json::json!({"v":u64::MAX,"name":"Future","extra":-0.0}), serde_json::json!({"v":2,"name":"Future"})]);
    let input = bytes(&source, &meta);
    let out = decode_recovery(&input).unwrap();
    assert_eq!(bytes(&out.workbook, &out.metadata), input);
    assert!(out.workbook.macros_read.is_empty());
    assert_eq!(out.workbook.macros, source.macros);
    assert_eq!(out.workbook.macros_unread, source.macros_unread);
    assert_eq!(out.workbook.names, source.names);
    let sheet = &out.workbook.sheets[0];
    assert_eq!(sheet.file_id, u32::MAX);
    assert_eq!(sheet.shapes[0].format.font_size.unwrap().to_bits(), (-0.0f32).to_bits());
    assert_eq!(sheet.cells.iter().count(), 5);
    assert_eq!(sheet.spill, source.sheets[0].spill);
    assert_eq!(sheet.spill_ranges, source.sheets[0].spill_ranges);
    for i in 0..3 {
        let cell = sheet.cells.get(CellRef::new(i, 0)).unwrap();
        let old = source.sheets[0].cells.get(CellRef::new(i, 0)).unwrap();
        let Value::Number(n) = cell.value else { panic!("number") };
        let Value::Number(before) = old.value else { panic!("number") };
        assert_eq!(n.to_bits(), before.to_bits());
        let f = cell.formula.as_ref().unwrap();
        let before = old.formula.as_ref().unwrap();
        assert_eq!((&f.text, f.array, f.dynamic, f.eval_mode), (&before.text, before.array, before.dynamic, before.eval_mode));
        assert_eq!(cell.formula.as_ref().unwrap().expr.is_some(), i == 0);
    }
    assert_eq!(sheet.cells.get(CellRef::new(4, 0)).unwrap().py, source.sheets[0].cells.get(CellRef::new(4, 0)).unwrap().py);
    assert_eq!(out.workbook.styles.get(StyleId(1)).font.size.to_bits(), (-0.0f32).to_bits());
    let mut probe = out.workbook.styles.clone();
    assert_eq!(probe.intern(probe.get(StyleId(0)).clone()), StyleId(1));
    assert_eq!(probe.len(), 2);
    assert_eq!(bytes(&source, &meta), input);
    assert!(source.sheets[0].cells.get(CellRef::default()).unwrap().formula.as_ref().unwrap().expr.is_none());
    let bad = replace_manifest(&input, "\"name\":\"M\"", "\"name\":\" M \"");
    assert!(decode_recovery(&bad).is_err());
    let bad = replace_manifest(&input, "\"v\":1,\"name\":\"M\"", "\"v\":2,\"name\":\"M\"");
    assert!(decode_recovery(&bad).is_err());
}
#[test]
fn prepared_write_uses_frozen_storage_and_preserves_sink_error_kinds() {
    use std::io::{self, Write};
    let mut source = Workbook::new();
    source.sheets.push(Arc::new(gridcraft_model::Sheet::new("Other")));
    let meta = metadata();
    let prepared = RecoveryCapture { workbook: &source, metadata: &meta }.prepare().unwrap();
    let ptr = prepared.manifest.as_ptr();
    let cap = prepared.manifest.capacity();
    source.set_active_sheet(1);
    let mut one = Vec::new();
    prepared.write_to(&mut one).unwrap();
    let mut two = Vec::new();
    prepared.write_to(&mut two).unwrap();
    assert_eq!(one, two);
    assert_eq!((prepared.manifest.as_ptr(), prepared.manifest.capacity()), (ptr, cap));
    assert_eq!(decode_recovery(&one).unwrap().workbook.active_sheet(), 0);
    struct Sink {
        left: usize,
        zero: bool,
        kind: io::ErrorKind,
    }
    impl Write for Sink {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            if self.left == 0 {
                return if self.zero { Ok(0) } else { Err(self.kind.into()) };
            }
            let n = self.left.min(b.len()).min(3);
            self.left -= n;
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    for (zero, kind) in [(true, io::ErrorKind::WriteZero), (false, io::ErrorKind::PermissionDenied)] {
        let mut sink = Sink { left: 31, zero, kind };
        assert!(matches!(prepared.write_to(&mut sink),Err(RecoveryError::Io(actual)) if actual==kind));
    }
}
#[test]
fn actual_validated_workbook_xlsm_opaque_vba_and_save_as_rebase_roundtrip() {
    use gridcraft_model::{PreservedPart, PreservedRel, PreservedXml};
    const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    let mut source = Workbook::new();
    source.source_dir = Some("/links".into());
    source.source_path = Some("/links/Original.xlsm".into());
    source.vba_project = Some(Arc::new(b"synthetic-vba-only".to_vec()));
    let link=PreservedPart {name:"xl/externalLinks/externalLink1.xml".into(),content_type:"application/vnd.openxmlformats-officedocument.spreadsheetml.externalLink+xml".into(),data:Arc::from(format!(r#"<externalLink xmlns="{MAIN}" xmlns:r="{REL}"><externalBook r:id="rIdPath"><sheetNames><sheetName val="Remote"/></sheetNames></externalBook></externalLink>"#).as_bytes()),rels:vec![PreservedRel {id:"rIdPath".into(),rel_type:format!("{REL}/externalLinkPath"),target:"Source.xlsx".into(),external:true}]};
    let opaque = PreservedPart {
        name: "customXml/opaque.bin".into(),
        content_type: "application/octet-stream".into(),
        data: Arc::from(&b"synthetic-opaque\0"[..]),
        rels: vec![],
    };
    source.preserved = Some(Arc::new(Preserved {
        rels: vec![
            PreservedRel { id: "rIdLink".into(), rel_type: format!("{REL}/externalLink"), target: link.name.clone(), external: false },
            PreservedRel { id: "rIdOpaque".into(), rel_type: format!("{REL}/customXml"), target: opaque.name.clone(), external: false },
        ],
        parts: vec![link, opaque],
        elements: vec![PreservedXml {
            slot: "externalReferences".into(),
            xml: format!(r#"<externalReferences xmlns="{MAIN}" xmlns:r="{REL}"><externalReference r:id="rIdLink"/></externalReferences>"#),
        }],
        ..Default::default()
    }));
    let validated = decode_recovery(&bytes(&source, &metadata())).unwrap();
    let output = crate::io::save_file_bytes(&validated.workbook, "/links/moved/Recovered.xlsm").unwrap();
    let (read, _) = gridcraft_xlsx::read_xlsx(&output).unwrap();
    assert_eq!(read.vba_project, source.vba_project);
    let preserved = read.preserved.as_ref().unwrap();
    assert!(preserved.parts.iter().any(|p| p.data.as_ref() == b"synthetic-opaque\0"));
    assert!(preserved.parts.iter().flat_map(|p| &p.rels).any(|r| r.external && r.target == "../Source.xlsx"));
    assert_eq!(validated.workbook.source_path, source.source_path);
    assert_eq!(validated.workbook.source_dir, source.source_dir);
}

#[test]
fn exact_node_and_metadata_bounds_run_at_complete_seams_before_materialization() {
    let mut source = Workbook::new();
    let p = picture();
    Arc::make_mut(&mut source.sheets[0]).cell_pictures.insert(CellRef::default(), p);
    let meta = metadata();
    let input = bytes(&source, &meta);
    let nodes = observe_manifest(&source, &meta, &RecoveryLimits::default(), false).unwrap().nodes;
    let exact = RecoveryLimits { nodes, ..RecoveryLimits::default() };
    assert!(decode(&input, &exact).is_ok());
    assert!(prepare(&RecoveryCapture { workbook: &source, metadata: &meta }, &exact).is_ok());
    let lower = RecoveryLimits { nodes: nodes - 1, ..RecoveryLimits::default() };
    let mut stats = Stats::default();
    assert!(decode_run(&input, &lower, &mut stats).is_err());
    assert_eq!((stats.vector_request, stats.payload_copies, stats.native_conversions, stats.d2_decode), (0, 0, 0, 0));
    let mut meta = metadata();
    meta.title = "a".repeat(32);
    let cap = RecoveryLimits { metadata: 32, ..RecoveryLimits::default() };
    assert!(prepare(&RecoveryCapture { workbook: &source, metadata: &meta }, &cap).is_ok());
    assert!(decode(&bytes(&source, &meta), &cap).is_ok());
    meta.title.push('a');
    assert!(prepare(&RecoveryCapture { workbook: &source, metadata: &meta }, &cap).is_err());
    let mut stats = Stats::default();
    assert!(decode_run(&bytes(&source, &meta), &cap, &mut stats).is_err());
    assert_eq!(stats.payload_copies, 0);
    let validated = {
        let temporary_input = bytes(&source, &metadata());
        decode_recovery(&temporary_input).unwrap()
    };
    assert_eq!(validated.metadata.title, "complete");
}

#[test]
fn decoder_diagnostic_is_useful_structural_and_redacts_sensitive_content() {
    let mut source = Workbook::new();
    let mut meta = metadata();
    meta.title = "PRIVATE_TITLE_SENTINEL".into();
    meta.sequence = 7;
    meta.captured_revision = u64::MAX;
    meta.original_path_hint = Some(NativePath::Unix(b"/PRIVATE_PATH_SENTINEL".to_vec()));
    let sheet = Arc::make_mut(&mut source.sheets[0]);
    sheet.name = "PRIVATE_SHEET_SENTINEL".into();
    sheet.cells.set(CellRef::default(), Cell { value: Value::Text("PRIVATE_CELL_SENTINEL".into()), ..Default::default() });
    source.macros_unread = Arc::new(vec![serde_json::json!({"v":2,"script":"PRIVATE_SCRIPT_SENTINEL"})]);
    let decoded = decode_recovery(&bytes(&source, &meta)).unwrap();
    let diagnostic = format!("{decoded:?}");
    assert_eq!(diagnostic, format!("ValidatedRecovery {{ id: {:?}, sequence: 7, captured_revision: {}, worksheets: 1 }}", meta.id, u64::MAX));
    for secret in ["PRIVATE_TITLE_SENTINEL", "PRIVATE_PATH_SENTINEL", "PRIVATE_SHEET_SENTINEL", "PRIVATE_CELL_SENTINEL", "PRIVATE_SCRIPT_SENTINEL"] {
        assert!(!diagnostic.contains(secret));
    }
}
