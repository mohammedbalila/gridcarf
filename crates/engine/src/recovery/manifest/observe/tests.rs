use super::*;
use serde::ser::{SerializeMap, SerializeSeq};
use std::{cell::Cell, collections::BTreeMap, sync::Arc};

fn parity<T: Serialize + ?Sized>(value: &T) -> AuditStats {
    let limits = RecoveryLimits::default();
    let raw = serde_json::to_vec(value).unwrap();
    let mut lexical = preflight::lexical_audit(&raw, &limits).unwrap();
    let observed = observe(value, &limits).unwrap();
    lexical.number_bytes = 0;
    lexical.max_number = 0;
    assert_eq!(observed, lexical);
    observed
}

#[test]
fn literal_transparent_external_and_adjacent_shapes_match_actual_writer() {
    #[derive(Serialize)]
    struct New<T>(T);
    #[derive(Serialize)]
    enum External {
        Unit,
        New(u8),
        Tuple(u8, String),
        Struct { v: Option<u8> },
    }
    parity(&());
    parity(&None::<u8>);
    parity(&Some(New(Arc::new("é"))));
    parity(&External::Unit);
    parity(&External::New(1));
    parity(&External::Tuple(2, "中".into()));
    parity(&External::Struct { v: None });
    parity(&gridcraft_model::Color::Rgb(0x123456));
    parity(&gridcraft_core::Value::Number(-0.0));
    parity(&gridcraft_core::Value::Text("x".into()));
    let value: gridcraft_core::Value =
        serde_json::from_value(serde_json::json!({"t":"Array","v":{"rows":1,"cols":1,"data":[{"t":"Number","v":1.0}]}})).unwrap();
    parity(&value);
    let mut integers = BTreeMap::new();
    integers.insert(u128::MAX, i128::MIN);
    parity(&integers);
    let shared = Arc::new(vec![1u8, 2]);
    parity(&vec![shared; 3]);
}

#[test]
fn real_flattened_steps_and_internal_control_actions_are_forwarded_once() {
    for raw in [
        r#"{"type":"command","id":"x","params":{"data":1}}"#,
        r#"{"type":"repeat","over":"A1","of":"rows","max":1,"body":[]}"#,
        r#"{"type":"if","cell":"A1","op":"eq","value":null,"then":[],"else":[]}"#,
        r#"{"type":"stop","message":"x","rollback":true}"#,
    ] {
        let step: gridcraft_model::Step = serde_json::from_str(raw).unwrap();
        parity(&step);
    }
    for raw in [r#"{"type":"command","id":"x","params":{}}"#, r#"{"type":"macro","name":"m","input":"selection"}"#] {
        let action: gridcraft_model::ControlAction = serde_json::from_str(raw).unwrap();
        parity(&action);
    }
}

struct Depth(usize);
impl Serialize for Depth {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if self.0 == 0 {
            s.serialize_some(&0)
        } else {
            let mut out = s.serialize_seq(Some(1))?;
            out.serialize_element(&Depth(self.0 - 1))?;
            out.end()
        }
    }
}
#[test]
fn real_depth_nodes_hints_and_lengths_fence_child_entry() {
    assert_eq!(observe(&Depth(64), &RecoveryLimits::default()).unwrap().max_depth, 64);
    assert!(matches!(observe(&Depth(65), &RecoveryLimits::default()), Err(RecoveryError::TooLarge("depth"))));
    let values = vec![BTreeMap::<String, u8>::new(); 20];
    assert_eq!(observe(&values, &RecoveryLimits { nodes: 21, ..RecoveryLimits::default() }).unwrap().nodes, 21);
    assert!(observe(&values, &RecoveryLimits { nodes: 20, ..RecoveryLimits::default() }).is_err());
    struct Count<'a>(&'a Cell<usize>);
    impl Serialize for Count<'_> {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            self.0.set(self.0.get() + 1);
            s.serialize_unit()
        }
    }
    let entered = Cell::new(0);
    assert!(observe(&[Count(&entered)], &RecoveryLimits { nodes: 1, ..RecoveryLimits::default() }).is_err());
    assert_eq!(entered.get(), 0);
    struct Wrong(usize, usize);
    impl Serialize for Wrong {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            let mut a = s.serialize_seq(Some(self.0))?;
            for _ in 0..self.1 {
                a.serialize_element(&())?;
            }
            a.end()
        }
    }
    for value in [Wrong(0, 1), Wrong(2, 1)] {
        assert!(matches!(observe(&value, &RecoveryLimits::default()), Err(RecoveryError::Malformed("serializer_length"))));
    }
}

#[test]
fn finite_scalars_key_forms_and_collect_str_have_explicit_contracts() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(matches!(observe(&value, &RecoveryLimits::default()), Err(RecoveryError::UnsupportedRepresentation("nonfinite"))));
    }
    parity(&[-0.0, 0.0, f64::MIN_POSITIVE]);
    #[derive(Serialize, PartialEq, Eq, PartialOrd, Ord)]
    struct Key(u32);
    let mut map = BTreeMap::new();
    map.insert(Key(2), true);
    parity(&map);
    struct Display<'a>(&'a Cell<usize>);
    impl fmt::Display for Display<'_> {
        fn fmt(&self, _: &mut fmt::Formatter) -> fmt::Result {
            self.0.set(1);
            Ok(())
        }
    }
    impl Serialize for Display<'_> {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.collect_str(self)
        }
    }
    let entered = Cell::new(0);
    assert!(matches!(observe(&Display(&entered), &RecoveryLimits::default()), Err(RecoveryError::UnsupportedRepresentation("collect_str"))));
    assert_eq!(entered.get(), 0);
    struct BadKey;
    impl Serialize for BadKey {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            let mut m = s.serialize_map(Some(1))?;
            m.serialize_entry(&vec![0], &())?;
            m.end()
        }
    }
    assert!(matches!(observe(&BadKey, &RecoveryLimits::default()), Err(RecoveryError::UnsupportedRepresentation("map_key"))));
}

#[test]
fn actual_private_empty_store_buckets_are_visible_without_iterating_cells() {
    for raw in [r#"{"len":0,"bands":{}}"#, r#"{"len":0,"bands":{"0":{"rows":{}}}}"#, r#"{"len":0,"bands":{"0":{"rows":{"0":{}}}}}"#] {
        let store: gridcraft_model::CellStore = serde_json::from_str(raw).unwrap();
        let stats = parity(&store);
        assert!(stats.nodes >= 5);
        assert!(observe(&store, &RecoveryLimits { nodes: stats.nodes - 1, ..RecoveryLimits::default() }).is_err());
    }
}

#[test]
fn bytes_native_units_and_general_metadata_routes_match_real_emitted_shape() {
    struct Bytes;
    impl Serialize for Bytes {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_bytes(&[255, 0])
        }
    }
    parity(&Bytes);
    let wire = serde_json::json!({"metadata":{"original_path_hint":{"windows":[55296,0]},"title":"é"},
        "workbook":{"sheets":[{"cells":{"bands":{"0":{"rows":{"0":{"0":{"value":{"t":"Number","v":-0.0}}}}}}}}]},
        "sidecars":{"workbook":{"macros_unread":[{"metadata":{"title":"content"},"data":"?"}]}}});
    let stats = parity(&wire);
    assert_eq!((stats.native_bytes, stats.sheets, stats.cells), (4, 1, 1));
    assert!(observe(&wire, &RecoveryLimits { native_units: 3, ..RecoveryLimits::default() }).is_err());
    assert!(observe(&wire, &RecoveryLimits { metadata: 1, ..RecoveryLimits::default() }).is_err());
    assert!(observe(&wire, &RecoveryLimits { cells: 0, ..RecoveryLimits::default() }).is_err());
    let observed = observe(&wire, &RecoveryLimits::default()).unwrap();
    use std::io::Write;
    writeln!(std::io::stdout(), "D1_OBSERVED {:?}", observed).unwrap();
}

#[test]
fn compound_protocol_mismatches_and_unknown_lengths_refuse_before_child_entry() {
    struct Bad(u8);
    impl Serialize for Bad {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            let mut m = s.serialize_map(Some(1))?;
            match self.0 {
                0 => m.serialize_value(&())?,
                1 => {
                    m.serialize_key("x")?;
                    m.serialize_key("y")?;
                }
                2 => {
                    m.serialize_key("x")?;
                }
                _ => {}
            }
            m.end()
        }
    }
    for kind in 0..4 {
        assert!(observe(&Bad(kind), &RecoveryLimits::default()).is_err());
    }
    struct Unknown<'a>(&'a Cell<usize>);
    impl Serialize for Unknown<'_> {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            struct Child<'a>(&'a Cell<usize>);
            impl Serialize for Child<'_> {
                fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                    self.0.set(1);
                    s.serialize_unit()
                }
            }
            let mut array = s.serialize_seq(None)?;
            array.serialize_element(&Child(self.0))?;
            array.end()
        }
    }
    let entered = Cell::new(0);
    assert!(observe(&Unknown(&entered), &RecoveryLimits { nodes: 1, ..RecoveryLimits::default() }).is_err());
    assert_eq!(entered.get(), 0);
    struct Huge;
    impl Serialize for Huge {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_map(Some(usize::MAX))?.end()
        }
    }
    assert!(matches!(observe(&Huge, &RecoveryLimits::default()), Err(RecoveryError::TooLarge("nodes"))));
}

#[test]
fn key_forwarding_char_bool_unit_variants_and_fixed_integer_extremes_match_writer() {
    #[derive(Serialize)]
    enum Unit {
        Key,
    }
    struct Keys;
    impl Serialize for Keys {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            let mut m = s.serialize_map(Some(5))?;
            m.serialize_entry(&Unit::Key, &())?;
            m.serialize_entry(&'é', &())?;
            m.serialize_entry(&false, &())?;
            m.serialize_entry(&i128::MIN, &())?;
            m.serialize_entry(&u128::MAX, &())?;
            m.end()
        }
    }
    parity(&Keys);
    assert!(matches!(
        observe(&gridcraft_core::Value::Number(f64::NAN), &RecoveryLimits::default()),
        Err(RecoveryError::UnsupportedRepresentation("nonfinite"))
    ));
    use std::io::Write;
    writeln!(
        std::io::stdout(),
        "D1_FIXED_STATE state={} observer={} compound={} key={}",
        std::mem::size_of::<State<'_>>(),
        std::mem::size_of::<Observer<'_, '_>>(),
        std::mem::size_of::<Compound<'_, '_>>(),
        std::mem::size_of::<key::Key<'_, '_>>()
    )
    .unwrap();
}
