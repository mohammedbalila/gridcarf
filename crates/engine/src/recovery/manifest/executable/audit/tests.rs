use super::*;

#[test]
fn iterative_raw_step_roles_count_only_real_current_children_before_helpers() {
    let limits = RecoveryLimits::default();
    let caps = Caps { steps: 2, ..Caps::default() };
    let mut audit = Audit::new(&limits, &caps);
    assert!(audit.json(&serde_json::json!({"steps":[{},{},{}]}), Role::Macro).is_err());
    let mut audit = Audit::new(&limits, &caps);
    audit.json(&serde_json::json!({"steps":[{"type":"command","params":{"steps":[{},{},{}]}}]}), Role::Macro).unwrap();
    assert_eq!(audit.steps, 1);
    let mut audit = Audit::new(&limits, &caps);
    audit.json(&serde_json::json!({"v":2,"steps":[{},{},{}]}), Role::General).unwrap();
    assert_eq!(audit.steps, 0);
}
#[test]
fn four_controls_and_terminal_fifth_include_disabled_both_branches() {
    let limits = RecoveryLimits::default();
    let caps = Caps::default();
    let mut steps = vec![Step::command("x", serde_json::json!({}))];
    for _ in 0..4 {
        steps = vec![Step {
            kind: StepKind::Repeat { over: "A1".into(), of: Default::default(), max: 1, body: steps },
            enabled: false,
            breakpoint: false,
            note: String::new(),
        }];
    }
    let m = Macro { name: "M".into(), steps, ..Default::default() };
    let mut audit = Audit::new(&limits, &caps);
    audit.typed(&m).unwrap();
    assert_eq!(audit.steps, 5);
    let mut too = m.clone();
    too.steps = vec![Step::new(StepKind::Repeat { over: "A1".into(), of: Default::default(), max: 1, body: too.steps })];
    assert!(Audit::new(&limits, &caps).typed(&too).is_err());
    let mut raw = serde_json::json!({"type":"command","id":"x","params":{}});
    for _ in 0..4 {
        raw = serde_json::json!({"type":"if","value":null,"then":[raw],"else":[{"type":"stop","message":"x"}],"enabled":false});
    }
    let mut audit = Audit::new(&limits, &caps);
    audit.json(&serde_json::json!({"steps":[raw]}), Role::Macro).unwrap();
    assert_eq!(audit.steps, 9);
}
#[test]
fn generic_node_depth_and_string_bounds_stop_explicit_borrowed_walk() {
    let caps = Caps::default();
    let exact = serde_json::json!({"x":null});
    let limits = RecoveryLimits { nodes: 3, ..RecoveryLimits::default() };
    Audit::new(&limits, &caps).json(&exact, Role::General).unwrap();
    assert!(Audit::new(&RecoveryLimits { nodes: 2, ..limits }, &caps).json(&exact, Role::General).is_err());
    let mut deep = serde_json::json!(null);
    for _ in 0..64 {
        deep = serde_json::json!([deep]);
    }
    Audit::new(&RecoveryLimits::default(), &caps).json(&deep, Role::General).unwrap();
    deep = serde_json::json!([deep]);
    assert!(Audit::new(&RecoveryLimits::default(), &caps).json(&deep, Role::General).is_err());
    let text = serde_json::json!({"x":"é"});
    Audit::new(&RecoveryLimits { manifest: 3, ..RecoveryLimits::default() }, &caps).json(&text, Role::General).unwrap();
    assert!(Audit::new(&RecoveryLimits { manifest: 2, ..RecoveryLimits::default() }, &caps).json(&text, Role::General).is_err());
}
