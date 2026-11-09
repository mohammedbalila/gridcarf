//! Deterministic regressions also selectable as one bounded stage in fresh test processes.
use super::*;
use gridcraft_core::{CellRef, Value};
use gridcraft_model::{Cell, CellPicture, Formula, FormulaEvalMode, Macro, Sheet, Step, Style, StyleId};
use serde_json::{Value as Json, json};
use std::{hint::black_box, sync::Arc};

const FAMILIES: &[&str] = &[
    "raw_keys",
    "raw_nodes",
    "escaped",
    "typed_binary",
    "macros",
    "rebuild",
    "complete",
    "sheets",
    "cells",
    "styles",
    "depth",
    "macro_records",
    "minimal",
    "payloads",
];
struct Fixture {
    workbook: Option<Workbook>,
    metadata: Metadata,
    input: Vec<u8>,
    limits: RecoveryLimits,
    expected: Option<(&'static str, &'static str)>,
    step_cap: usize,
    inventory: Json,
}
fn metadata() -> Metadata {
    Metadata {
        frozen_active_sheet: None,
        title: "measurement".into(),
        original_path_hint: None,
        file_kind: None,
        dirty_at_capture: true,
        import_issues: vec![],
        selection: Selection::default(),
        sheet_selections: vec![],
        captured_revision: 0,
        id: "00000000000000000000000000000000".parse().unwrap(),
        sequence: 0,
        provenance: RecoveryProvenance::Plain,
    }
}
fn stored(count: usize) -> Workbook {
    let mut workbook = Workbook::new();
    let styles: Vec<_> = (0..count)
        .map(|i| {
            let mut style = Style::default();
            style.font.size = 10.0 + i as f32;
            workbook.styles.intern(style)
        })
        .collect();
    let sheet = Arc::make_mut(&mut workbook.sheets[0]);
    for (i, style) in styles.into_iter().enumerate() {
        let at = CellRef::new((i * 64) as u32, (i % 32) as u32);
        sheet.cells.set(
            at,
            if i % 3 == 0 {
                Cell { style, ..Default::default() }
            } else {
                Cell {
                    style,
                    value: Value::Number(i as f64),
                    formula: Some(Arc::new(Formula {
                        text: "SUM(1,2)".into(),
                        expr: None,
                        array: None,
                        dynamic: false,
                        eval_mode: FormulaEvalMode::Legacy,
                    })),
                    ..Default::default()
                }
            },
        );
    }
    workbook
}
fn sizes(family: &str) -> &'static [usize] {
    match family {
        "escaped" | "typed_binary" | "payloads" => &[4096, 8192, 16384],
        "depth" => &[64],
        "minimal" => &[1],
        "macro_records" => &[gridcraft_model::MAX_MACROS],
        _ => &[64, 128, 256],
    }
}
fn payload_fixture(
    source: &mut Workbook,
    size: usize,
    refused: bool,
    limits: &mut RecoveryLimits,
    expected: &mut Option<(&'static str, &'static str)>,
) {
    use gridcraft_model::{Preserved, PreservedPart, PreservedRel};
    let shared: Arc<[u8]> = vec![0x55; size].into();
    let parts = [Arc::from([]), shared.clone(), shared, Arc::<[u8]>::from(vec![0x55; size])]
        .into_iter()
        .enumerate()
        .map(|(i, data)| PreservedPart { name: format!("customXml/p{i}.bin"), content_type: "application/octet-stream".into(), data, rels: vec![] })
        .collect::<Vec<_>>();
    let rels = parts
        .iter()
        .enumerate()
        .map(|(i, part)| PreservedRel {
            id: format!("rId{i}"),
            rel_type: "http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml".into(),
            target: part.name.clone(),
            external: false,
        })
        .collect();
    source.preserved = Some(Arc::new(Preserved { parts, rels, ..Default::default() }));
    limits.part_bytes = size - usize::from(refused);
    if refused {
        *expected = Some(("TooLarge", "part_bytes"));
    }
}

fn escaped_fixture(size: usize, refused: bool, limits: &mut RecoveryLimits, expected: &mut Option<(&'static str, &'static str)>) -> Vec<u8> {
    limits.manifest = size;
    let fields = (0..size / 32).map(|i| if i % 2 == 0 { "\"\\u00e9\"" } else { "\"\\ud83d\\ude00\"" }).collect::<Vec<_>>();
    let mut bytes = format!("[{}]", fields.join(",")).into_bytes();
    bytes.resize(size + usize::from(refused), b' ');
    if refused {
        *expected = Some(("TooLarge", "manifest"));
    }
    bytes
}

fn fixture(family: &str, size: usize, refused: bool) -> Fixture {
    assert!(FAMILIES.contains(&family) && sizes(family).contains(&size));
    let mut metadata = metadata();
    if family == "minimal" {
        metadata.title.clear();
        metadata.dirty_at_capture = false;
    }
    let mut limits = RecoveryLimits::default();
    let mut workbook = None;
    let mut writer_growth = Json::Null;
    let mut manifest_capacity = None;
    let mut expected = None;
    let mut step_cap = gridcraft_model::MAX_MACRO_STEPS;
    let mut input = match family {
        "raw_keys" => {
            limits.nodes = size;
            let mut fields: Vec<_> = (0..size / 4).map(|i| format!("\"k{i}\":0")).collect();
            if refused {
                fields.push("\"\\u006b0\":0".into());
                expected = Some(("Malformed", "duplicate_key"));
            }
            format!("{{{}}}", fields.join(",")).into_bytes()
        }
        "raw_nodes" => {
            limits.nodes = size;
            let count = size - 1 + usize::from(refused);
            if refused {
                expected = Some(("TooLarge", "nodes"));
            }
            format!("[{}]", vec!["0"; count].join(",")).into_bytes()
        }
        "escaped" => escaped_fixture(size, refused, &mut limits, &mut expected),
        "depth" => {
            let depth = size + usize::from(refused);
            if refused {
                expected = Some(("TooLarge", "depth"));
            }
            format!("{}0{}", "[".repeat(depth), "]".repeat(depth)).into_bytes()
        }
        _ => {
            let mut source = match family {
                "rebuild" | "complete" | "cells" | "styles" => stored(size),
                _ => Workbook::new(),
            };
            match family {
                "typed_binary" => {
                    let picture = Arc::new(CellPicture { data: vec![0x55; size / 64], mime: "image/png".into(), alt: "binary".into() });
                    source.vba_project = Some(vec![0, 255].into());
                    let sheet = Arc::make_mut(&mut source.sheets[0]);
                    for i in 0..6 {
                        sheet.cell_pictures.insert(CellRef::new(0, i), if i % 2 == 0 { picture.clone() } else { Arc::new((*picture).clone()) });
                    }
                    limits.envelope = 2 + 6 * size / 64 - usize::from(refused);
                    if refused {
                        expected = Some(("TooLarge", "base64"));
                    }
                }
                "macros" | "macro_records" => {
                    if family == "macros" {
                        step_cap = size;
                        source.macros = vec![Macro {
                            name: "M".into(),
                            steps: (0..size + usize::from(refused)).map(|_| Step::command("x", json!({"text":"é😀"}))).collect(),
                            ..Default::default()
                        }]
                        .into();
                        if refused {
                            expected = Some(("TooLarge", "macro_steps"));
                        }
                    } else {
                        source.macros = (0..size + usize::from(refused))
                            .map(|i| Macro { name: format!("M{i}"), ..Default::default() })
                            .collect::<Vec<_>>()
                            .into();
                        if refused {
                            expected = Some(("TooLarge", "macros"));
                        }
                    }
                }
                "sheets" => {
                    source.sheets = (0..size).map(|i| Arc::new(Sheet::new(format!("S{i}")))).collect();
                    limits.sheets = size - usize::from(refused);
                    if refused {
                        expected = Some(("TooLarge", "sheets"));
                    }
                }
                "cells" => {
                    limits.cells = size - usize::from(refused);
                    if refused {
                        expected = Some(("TooLarge", "cells"));
                    }
                }
                "styles" if refused => {
                    let invalid = StyleId(source.styles.len() as u32);
                    Arc::make_mut(&mut source.sheets[0]).cells.get_mut(CellRef::default()).unwrap().style = invalid;
                    expected = Some(("Malformed", "model_style"));
                }
                _ => {}
            }
            if family == "payloads" {
                payload_fixture(&mut source, size, refused, &mut limits, &mut expected);
            }
            let bytes = if matches!(family, "complete" | "minimal" | "payloads") {
                let prepared =
                    complete::prepare(&crate::recovery::RecoveryCapture { workbook: &source, metadata: &metadata }, &RecoveryLimits::default())
                        .unwrap();
                manifest_capacity = Some(prepared.manifest.capacity());
                let mut bytes = Vec::new();
                prepared.write_to(&mut bytes).unwrap();
                bytes
            } else {
                let encoded = write::manifest_bytes(&source, &metadata, &RecoveryLimits::default()).unwrap();
                writer_growth = serde_json::to_value(&encoded.growth).unwrap();
                manifest_capacity = Some(encoded.bytes.capacity());
                encoded.bytes
            };
            workbook = Some(source);
            bytes
        }
    };
    let manifest = if matches!(family, "complete" | "minimal" | "payloads") {
        let len = u64::from_le_bytes(input[20..28].try_into().unwrap()) as usize;
        &input[28..28 + len]
    } else {
        input.as_slice()
    };
    let observed = preflight::lexical_audit(manifest, &RecoveryLimits::default());
    let measured_shape = match &observed {
        Ok(stats) => serde_json::to_value(stats).unwrap(),
        Err(error) => json!({"inventory_refusal":error.to_string()}),
    };
    if matches!(family, "complete" | "minimal" | "payloads") {
        limits.nodes = observed.as_ref().unwrap().nodes - usize::from(refused && family != "payloads");
        limits.manifest = manifest.len();
        limits.envelope = input.len();
        if refused && family != "payloads" {
            expected = Some(("TooLarge", "nodes"));
        }
    } else if family == "typed_binary" {
        limits.manifest = size.max(input.len());
    } else if family == "rebuild" {
        limits.nodes = observed.as_ref().unwrap().nodes - usize::from(refused);
        if refused {
            expected = Some(("TooLarge", "nodes"));
        }
    }
    let (physical_cells, spills, styles, sheets, pictures, unique_pictures) = workbook.as_ref().map_or((0, 0, 0, 0, 0, 0), |source| {
        let physical = source.sheets.iter().map(|s| s.cells.iter().count()).sum::<usize>();
        let spills = source.sheets.iter().map(|s| s.spill.len()).sum::<usize>();
        let occurrences = source.sheets.iter().map(|s| s.cell_pictures.len()).sum::<usize>();
        let unique = source.sheets.iter().flat_map(|s| s.cell_pictures.values()).map(Arc::as_ptr).collect::<std::collections::HashSet<_>>().len();
        (physical, spills, source.styles.len(), source.sheets.len(), occurrences, unique)
    });
    let inventory = json!({"input_bytes":input.len(),"manifest_bytes":manifest.len(),"fixture_writer_growth":writer_growth,"fixture_manifest_capacity":manifest_capacity,"physical_cells":physical_cells,"spills":spills,"style_records":styles,
        "sheets":sheets,"picture_occurrences":pictures,"unique_picture_allocations":unique_pictures,"emitted":measured_shape,
        "limits":{"nodes":limits.nodes,"manifest":limits.manifest,"envelope":limits.envelope,"sheets":limits.sheets,"cells":limits.cells,"depth":limits.depth,"steps":step_cap},
        "fixture_inventory_pass":"actual lexical audit before selected operation; included in fixture baseline", "binary_occurrences":"structural stage reports actual encoded/decoded totals; VBA plus six picture occurrences for typed_binary"});
    input.shrink_to_fit();
    Fixture { workbook, metadata, input, limits, expected, step_cap, inventory }
}
fn assert_result(result: &Result<(), RecoveryError>, expected: Option<(&str, &str)>) {
    match (result, expected) {
        (Ok(()), None) => {}
        (Err(RecoveryError::TooLarge(reason)), Some(("TooLarge", expected))) => assert_eq!(*reason, expected),
        (Err(RecoveryError::Malformed(reason)), Some(("Malformed", expected))) => assert_eq!(*reason, expected),
        (other, expected) => panic!("unexpected stage result {other:?}; expected {expected:?}"),
    }
}
fn operation(family: &str, fixture: &mut Fixture) -> (Result<(), RecoveryError>, Json) {
    match family {
        "raw_keys" | "raw_nodes" | "depth" => preflight::measure(&fixture.input, &fixture.limits),
        "escaped" | "typed_binary" => {
            let (result, mut metrics) = preflight::measure(&fixture.input, &fixture.limits);
            metrics["typed_serde_entries"] = json!(0);
            if let Err(error) = result {
                return (Err(error), metrics);
            }
            metrics["typed_serde_entries"] = json!(1);
            let result = if family == "escaped" {
                let values: Vec<String> = serde_json::from_slice(&fixture.input).unwrap();
                assert!(values.iter().any(|value| value == "😀"));
                Ok(())
            } else {
                let value: Manifest = serde_json::from_slice(&fixture.input).unwrap();
                assert_eq!(value.workbook.vba_project.as_ref().unwrap().as_slice(), &[0, 255]);
                assert_eq!(value.workbook.sheets[0].cell_pictures.len(), 6);
                metrics["typed_retained_payload_capacity"] = json!(
                    value.workbook.vba_project.as_ref().unwrap().capacity()
                        + value.workbook.sheets[0].cell_pictures.values().map(|picture| picture.data.capacity()).sum::<usize>()
                );
                Ok(())
            };
            (result, metrics)
        }
        "macros" | "macro_records" => executable::measure(fixture.workbook.as_ref().unwrap(), &fixture.metadata, &fixture.limits, fixture.step_cap),
        "complete" | "minimal" | "payloads" => {
            let (result, metrics) = complete::measure(&fixture.input, &fixture.limits);
            if let Ok(candidate) = &result {
                assert_eq!(candidate.workbook.sheets[0].cells.iter().count(), fixture.inventory["physical_cells"].as_u64().unwrap() as usize);
            }
            if family == "payloads"
                && let Ok(candidate) = &result
            {
                let parts = &candidate.workbook.preserved.as_ref().unwrap().parts;
                assert_eq!(parts.len(), 4);
                assert!(!Arc::ptr_eq(&parts[1].data, &parts[2].data));
                assert_eq!(parts[1].data, parts[2].data);
                assert_eq!(metrics["payload_copies"], 4);
            }
            (result.map(drop), metrics)
        }
        _ => {
            let (result, metrics) = rebuild::measure(fixture.workbook.take().unwrap(), &fixture.metadata, &fixture.limits);
            if let Ok(candidate) = &result {
                assert_eq!(candidate.sheets.len(), fixture.inventory["sheets"].as_u64().unwrap() as usize);
            }
            (result.map(drop), metrics)
        }
    }
}
fn run_case(family: &str, size: usize, refused: bool, mode: &str) {
    assert!(["startup", "fixture", "operation"].contains(&mode));
    if mode == "startup" {
        println!("E5B_METRICS {}", json!({"family":family,"size":size,"refused":refused,"mode":mode,"fixture_entered":false}));
        return;
    }
    let mut fixture = fixture(family, size, refused);
    let mut record = json!({"family":family,"size":size,"refused":refused,"mode":mode,"inventory":fixture.inventory,
        "uninstrumented":"serde/Arc/BTreeMap/helper heap and allocator overhead; process RSS is not exact heap", "metrics":null});
    if mode == "operation" {
        let (result, metrics) = operation(family, &mut fixture);
        assert_result(&result, fixture.expected);
        record["result"] = json!(match &result {
            Ok(()) => "accepted".to_string(),
            Err(error) => error.to_string(),
        });
        if refused {
            match family {
                "raw_nodes" | "depth" | "escaped" => assert_eq!(metrics["structural_entries"], 0),
                "typed_binary" => assert_eq!(metrics["typed_serde_entries"], 0),
                "macros" | "macro_records" => {
                    assert_eq!(metrics["parse_entries"], 0);
                    assert_eq!(metrics["check_entries"], 0);
                }
                "complete" | "minimal" | "payloads" => {
                    assert_eq!(metrics["payload_copies"], 0);
                    assert_eq!(metrics["d2_decode"], 0);
                    assert_eq!(metrics["p3_calls"], 0);
                }
                "rebuild" | "sheets" | "cells" => {
                    assert_eq!(metrics["model_before"], 0);
                    assert_eq!(metrics["store_calls"], 0);
                }
                "styles" => {
                    assert_eq!(metrics["model_before"], 1);
                    assert_eq!(metrics["style_calls"], 0);
                    assert_eq!(metrics["store_calls"], 0);
                }
                _ => {}
            }
        } else {
            match family {
                "macros" | "macro_records" => assert!(metrics["parse_entries"].as_u64().unwrap() > 0),
                "complete" | "minimal" | "payloads" => {
                    assert_eq!(metrics["derived_calls"], 1);
                    assert_eq!(metrics["final_compare"], 1);
                }
                "rebuild" | "cells" | "styles" => {
                    assert!(metrics["style_entries"].as_u64().unwrap() > 0);
                    assert!(metrics["store_calls"].as_u64().unwrap() > 0);
                }
                _ => {}
            }
        }
        record["metrics"] = metrics;
    }
    black_box(&fixture);
    println!("E5B_METRICS {record}");
}
#[test]
fn bounded_stage_campaign() {
    if let Ok(family) = std::env::var("GRIDCRAFT_E5B_FAMILY") {
        assert!(FAMILIES.contains(&family.as_str()));
        let size: usize = std::env::var("GRIDCRAFT_E5B_SIZE").unwrap().parse().unwrap();
        assert!(sizes(&family).contains(&size));
        let refused = match std::env::var("GRIDCRAFT_E5B_CASE").unwrap().as_str() {
            "accepted" => false,
            "refused" => true,
            _ => panic!("case"),
        };
        let mode = std::env::var("GRIDCRAFT_E5B_MODE").unwrap();
        run_case(&family, size, refused, &mode);
    } else {
        // Normal CI executes every accepted baseline and targeted boundary, without environment.
        for family in FAMILIES {
            for size in sizes(family) {
                for refused in [false, true] {
                    run_case(family, *size, refused, "operation");
                }
            }
        }
    }
}
