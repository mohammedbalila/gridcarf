//! Menu saves explain known losses; script calls remain headless.
use egui_kittest::kittest::Queryable;
use serde_json::json;

fn harness() -> crate::common::Harness {
    let mut h = crate::support::blank_harness(4);
    h.state_mut().run("range.setValues", json!({"range":"A1","values":[["Region"],["East"]]})).unwrap();
    h.state_mut().run("insert.table", json!({"range":"A1:A2","name":"Sales","header":true})).unwrap();
    h.state_mut().run("insert.slicer", json!({"table":"Sales","column":"Region","at":"D1"})).unwrap();
    h
}
#[test]
fn save_losses_menu_explains_and_cancel_preserves_destination() {
    let mut h = harness();
    let path = std::env::temp_dir().join(format!("gridcraft-ui-loss-{}-{}.xlsx", std::process::id(), h.state().session.doc().unwrap().uid));
    std::fs::write(&path, b"original").unwrap();
    h.state_mut().run_or_alert("file.save", json!({"path":path}));
    h.run();
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name.as_str()), Some("saveLosses"));
    assert!(h.query_by_label("Save with losses").is_some());
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel").click();
    h.run();
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert!(h.state().session.doc().unwrap().is_dirty());
    std::fs::remove_file(path).unwrap();
}
#[test]
fn save_losses_programmatic_call_does_not_open_confirmation() {
    let mut h = harness();
    assert!(h.state_mut().run("file.saveBytes", json!({"format":"xlsx"})).is_err());
    assert!(h.state().dialog.is_none());
}

#[test]
fn save_losses_confirmation_writes_only_the_reviewed_revision() {
    let mut h = harness();
    let path = std::env::temp_dir().join(format!("gridcraft-ui-confirm-{}-{}.xlsx", std::process::id(), h.state().session.doc().unwrap().uid));
    h.state_mut().run_or_alert("file.save", json!({"path":path}));
    h.run();
    if let Some(output) = std::env::var_os("GRIDCRAFT_SAVE_LOSSES_SCREENSHOT") {
        h.render().unwrap().save(output).unwrap();
    }
    h.get_by_label("Save with losses").click();
    h.run();
    assert!(h.state().dialog.is_none());
    assert!(!h.state().session.doc().unwrap().is_dirty());
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"PK"));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn save_losses_stale_confirmation_cannot_write_after_an_edit() {
    let mut h = harness();
    let path = std::env::temp_dir().join(format!("gridcraft-ui-stale-{}-{}.xlsx", std::process::id(), h.state().session.doc().unwrap().uid));
    h.state_mut().run_or_alert("file.save", json!({"path":path}));
    h.run();
    h.state_mut().run("cell.set", json!({"cell":"B5", "input":"new content"})).unwrap();
    h.get_by_label("Save with losses").click();
    h.run();
    assert!(!path.exists());
    assert!(h.state().session.doc().unwrap().is_dirty());
    assert!(h.state().message.is_some());
}

#[test]
fn save_losses_control_app_save_forwards_explicit_confirmation() {
    let mut h = harness();
    let path = std::env::temp_dir().join(format!("gridcraft-control-loss-{}-{}.xlsx", std::process::id(), h.state().session.doc().unwrap().uid));
    h.state_mut().run("selection.set", json!({"cell":"F51"})).unwrap();
    h.state_mut().view_mut().unwrap().scroll.y = 1000.0;
    let a = h.state_mut().run("file.analyzeSave", json!({"path":path})).unwrap();
    let ctx = h.ctx.clone();
    let outcome = gridcraft_ui_egui::control::handle(
        h.state_mut(),
        &ctx,
        "app.save",
        &json!({"path":path, "expectedRevision":a["revision"], "expectedDocument":a["document"], "confirmedLosses":["feature.omitted"]}),
    );
    let gridcraft_ui_egui::control::Outcome::Done(result) = outcome else { panic!("unexpected screenshot") };
    assert_eq!(result["ok"], true, "{result}");
    assert!(std::fs::read(&path).unwrap().starts_with(b"PK"));
    std::fs::remove_file(path).unwrap();
    assert!(h.state().dialog.is_none());
}

#[test]
fn save_losses_backstage_download_waits_for_explicit_decision() {
    let mut h = harness();
    let files = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Vec<u8>>::new()));
    let capture = files.clone();
    h.state_mut().services.download = Some(Box::new(move |_, bytes| capture.lock().unwrap().push(bytes.to_vec())));
    gridcraft_ui_egui::backstage::export_as(h.state_mut(), "csv");
    h.run();
    assert!(files.lock().unwrap().is_empty());
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name.as_str()), Some("saveLosses"));
    h.get_by_label("Save with losses").click();
    h.run();
    let files = files.lock().unwrap();
    assert_eq!(files.len(), 1);
    assert!(String::from_utf8_lossy(&files[0]).contains("Region"));
}

#[test]
fn save_losses_analysis_still_rejects_a_subsequent_edit() {
    let mut h = harness();
    h.state_mut().run("selection.set", json!({"cell":"F51"})).unwrap();
    h.state_mut().view_mut().unwrap().scroll.y = 1000.0;
    let a = h.state_mut().run("file.analyzeSave", json!({"path":"Book1.xlsx"})).unwrap();
    h.state_mut().run("cell.set", json!({"cell":"B5", "input":"new content"})).unwrap();
    let error = h
        .state_mut()
        .run(
            "file.saveBytes",
            json!({"format":"xlsx", "expectedRevision":a["revision"], "expectedDocument":a["document"], "confirmedLosses":["feature.omitted"]}),
        )
        .unwrap_err();
    let error: serde_json::Value = serde_json::from_str(&error).unwrap();
    assert_eq!(error["error"], "save.stale");
    assert!(h.state().dialog.is_none());
}
