//! Functional ownership evidence: scalar cache observations never clean up or retain payloads.
use super::*;
use gridcraft_engine::{Session, Trust, core::Value};
use serde_json::{Value as Json, json};

#[path = "../../../engine/tests/support/lifetime_fixture.rs"]
mod lifetime_fixture;
use lifetime_fixture::{CYCLES, Probes, SAMPLES, WARMUP, assert_pasted, at, edits, estimated_reachable, fixture};

fn run(session: &mut Session, command: &str, params: Json) -> Json {
    let result = session.execute(command, params).unwrap();
    assert_eq!(session.depth(), 0);
    result
}

struct Sentinel {
    doc: u64,
    window: u64,
    sheet: u64,
    selection: gridcraft_engine::Selection,
    scroll: egui::Vec2,
}

fn ticks(app: &mut SheetApp, ctx: &egui::Context) {
    for _ in 0..2 {
        let _ = ctx.run_logic(&egui::RawInput::default(), |ctx| app.logic(ctx));
    }
}

fn setup() -> (SheetApp, egui::Context, Sentinel) {
    let mut session = Session::new();
    session.new_workbook();
    let mut app = SheetApp::new(session, Default::default());
    app.ui.autosave = false;
    app.use_prepared_fonts();
    app.run("selection.set", json!({"cell":"E11"})).unwrap();
    app.view_mut().unwrap().scroll = egui::vec2(12.0, 40.0);
    let sentinel = Sentinel {
        doc: app.session.doc().unwrap().uid,
        window: app.session.active_window().unwrap(),
        sheet: app.session.doc().unwrap().wb.active().unwrap().uid.get(),
        selection: app.session.doc().unwrap().selection.clone(),
        scroll: app.view().scroll,
    };
    let ctx = egui::Context::default();
    ctx.set_fonts(crate::theme::pinned_font_definitions());
    crate::theme::apply(&ctx, false);
    ticks(&mut app, &ctx);
    (app, ctx, sentinel)
}

fn target(app: &mut SheetApp, ctx: &egui::Context, retained: usize) -> (u64, u64, u64, Probes) {
    app.session.add_document(fixture(Trust::FromFile));
    app.follow_active_window();
    let (uid, first) = (app.session.doc().unwrap().uid, app.session.active_window().unwrap());
    let data = app.session.doc().unwrap().wb.active().unwrap().uid.get();
    let second = app.run("view.newWindow", json!({})).unwrap()["window"].as_u64().unwrap();
    let mut probes = Probes::default();
    edits(&mut app.session, &mut probes);
    assert_eq!(probes.live()[0], 9);
    let mut output = ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0))), ..Default::default() },
        |ui| crate::grid::show(app, ui),
    );
    assert_eq!(output.shapes.iter().filter(|s| matches!(&s.shape, egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default())).count(), 2);
    output.textures_delta.clear();
    drop(output);
    let cell = crate::cell_pictures::lifetime_evidence(ctx).unwrap();
    assert_eq!((cell[0], cell[2], cell[3], cell[4]), (retained + 1, (retained + 1) * 32, retained + 1, retained + 1));
    let floating = crate::floating_pictures::lifetime_evidence(ctx, uid).unwrap();
    assert_eq!((floating[0], floating[2], floating[3], floating[6]), (1, 32, 1, 1));
    assert_eq!(app.grid.cf.key.unwrap().0, uid);
    app.view_mut().unwrap().scroll = egui::vec2(5.0, 20.0);
    app.open_dialog("zoom", json!({}));
    app.enter_window(first);
    app.run("sheet.activate", json!({"sheet":"Lifetime Aux"})).unwrap();
    assert_ne!(app.session.doc().unwrap().wb.active().unwrap().uid.get(), data);
    app.view_mut().unwrap().scroll = egui::vec2(25.0, 80.0);
    app.enter_window(second);
    assert_eq!(app.session.doc().unwrap().wb.active().unwrap().uid.get(), data);
    assert_eq!(app.view().scroll, egui::vec2(5.0, 20.0));
    probes.document(&app.session);
    ticks(app, ctx);
    (uid, first, second, probes)
}

fn close_target(app: &mut SheetApp, ctx: &egui::Context, s: &Sentinel, ids: (u64, u64, u64), probes: &mut Probes) {
    let (uid, first, last) = ids;
    app.enter_window(s.window);
    assert_eq!(app.parked[&last].grid.cf.key.unwrap().0, uid);
    assert!(app.parked[&last].dialog.is_some());
    app.session.hide_window(first);
    app.session.hide_window(last);
    assert_eq!(app.run("window.close", json!({"window":first,"force":true})).unwrap()["closed"], true);
    assert!(app.session.document_index(uid).is_some());
    assert!(probes.live()[0] > 0);
    app.enter_window(last);
    let sh = app.session.doc().unwrap().wb.active().unwrap();
    assert_eq!(sh.value(at("B1")), Value::Number(1007.0));
    assert_eq!(sh.value(at("C1")), Value::Number(1008.0));
    probes.document(&app.session);
    app.enter_window(s.window);
    assert!(estimated_reachable(&app.session) > 64 * 1024);
    assert_eq!(run(&mut app.session, "window.close", json!({"window":last,"force":true}))["closed"], true);
    // Session close has not run any app cleanup: observe the stale state before logic alone.
    assert!(app.parked.contains_key(&last));
    assert!(app.views.keys().any(|(window, _)| *window == last));
    assert!(app.reachable_sheets.contains_key(&uid));
    assert!(app.autosave_states.contains_key(&uid));
    assert_eq!(crate::floating_pictures::lifetime_evidence(ctx, uid).unwrap()[3], 1);
    ticks(app, ctx);
}

fn released(app: &SheetApp, ctx: &egui::Context, s: &Sentinel, target: u64, retained: usize) {
    assert_eq!((app.session.documents().len(), app.session.windows().len()), (1, 1));
    assert_eq!((app.session.active_window(), app.ui_window()), (Some(s.window), Some(s.window)));
    assert_eq!(app.session.doc().unwrap().uid, s.doc);
    assert_eq!(app.session.doc().unwrap().selection, s.selection);
    assert_eq!(app.view().scroll, s.scroll);
    assert!(app.session.document_index(target).is_none() && !app.session.in_group(target));
    assert!(app.session.macro_debug.is_none());
    assert_eq!(app.session.depth(), 0);
    assert!(app.parked.is_empty());
    assert_eq!(app.views.len(), 1);
    assert!(app.views.contains_key(&(s.window, s.sheet)));
    assert_eq!(app.autosave_states.len(), 1);
    assert!(app.autosave_states.contains_key(&s.doc));
    assert!(app.reachable_sheets.keys().all(|uid| *uid == s.doc));
    assert!(!app.reachable_sheets.contains_key(&target));
    assert!(app.grid.cf.key.is_none_or(|key| key.0 == s.doc));
    let cell = crate::cell_pictures::lifetime_evidence(ctx).unwrap();
    assert_eq!((cell[0], cell[2], cell[3], cell[4]), (retained, retained * 32, retained, retained));
    let floating = crate::floating_pictures::lifetime_evidence(ctx, target).unwrap();
    assert_eq!((floating[0], floating[2], floating[3], floating[4], floating[6]), (0, 0, 0, 1, 0));
}

fn sample(app: &SheetApp, ctx: &egui::Context, phase: &str, cycle: usize) {
    let cell = crate::cell_pictures::lifetime_evidence(ctx).unwrap();
    let document = app.session.doc().unwrap().uid;
    let floating = crate::floating_pictures::lifetime_evidence(ctx, document).unwrap();
    let inline = app.parked.capacity() * std::mem::size_of::<(u64, WindowUi)>()
        + app.views.capacity() * std::mem::size_of::<((u64, u64), crate::SheetView)>()
        + app.reachable_sheets.capacity() * std::mem::size_of::<(u64, crate::app::ReachableSheets)>()
        + app.autosave_states.capacity() * std::mem::size_of::<(u64, crate::app::AutoSaveState)>();
    println!(
        "M03D_SAMPLE phase={phase} cycle={cycle} observed_document={document} docs={} windows={} parked_len={} parked_capacity={} views_len={} views_capacity={} membership_len={} membership_capacity={} autosave_len={} autosave_capacity={} current_cf_instances={} current_cf_key={:?} parked_cf_instances={} estimated_workbook_bytes={} ui_inline_capacity_estimate={} cell_len_capacity_decodedbytes_livesources_decodedlen_inline={cell:?} floating_len_capacity_decodedbytes_targetentries_docslen_docscapacity_decodedlen_inline={floating:?} cf_innerbytes=unavailable",
        app.session.documents().len(),
        app.session.windows().len(),
        app.parked.len(),
        app.parked.capacity(),
        app.views.len(),
        app.views.capacity(),
        app.reachable_sheets.len(),
        app.reachable_sheets.capacity(),
        app.autosave_states.len(),
        app.autosave_states.capacity(),
        usize::from(app.grid.cf.key.is_some()),
        app.grid.cf.key,
        app.parked.values().filter(|window| window.grid.cf.key.is_some()).count(),
        estimated_reachable(&app.session),
        inline
    );
}

fn owner_ids(session: &Session) -> (Option<usize>, Option<usize>) {
    (
        session.clipboard.as_ref().map(|clipboard| std::sync::Arc::as_ptr(&clipboard.wb) as usize),
        session.format_painter.as_ref().map(|painter| std::sync::Arc::as_ptr(&painter.0) as usize),
    )
}

#[test]
fn m03d_combined_ui_owner_cache_cycles() {
    for retained_phase in [false, true] {
        let (mut app, ctx, s) = setup();
        let retained = if retained_phase {
            let (uid, first, last, mut all) = target(&mut app, &ctx, 0);
            app.run("selection.set", json!({"range":"A1:C2"})).unwrap();
            app.run("edit.copy", json!({})).unwrap();
            app.run("edit.formatPainter", json!({"sticky":true})).unwrap();
            let mut intentional = Probes::default();
            intentional.workbook(&app.session.clipboard.as_ref().unwrap().wb);
            intentional.workbook(&app.session.format_painter.as_ref().unwrap().0);
            assert_eq!(intentional.live(), [1, 2, 2, 1, 1, 1, 1, 1, 2, 2, 1, 1, 0, 0]);
            all.document(&app.session);
            close_target(&mut app, &ctx, &s, (uid, first, last), &mut all);
            assert_eq!(all.live(), intentional.live());
            released(&app, &ctx, &s, uid, 1);
            intentional
        } else {
            app.run("edit.clearClipboard", json!({})).unwrap();
            Probes::default()
        };
        let intentional = retained.live();
        let owners = owner_ids(&app.session);
        let baseline = estimated_reachable(&app.session);
        let phase = if retained_phase { "retained" } else { "cleared" };
        let mut started = crate::monotonic_ms();
        for completed in 0..=WARMUP + CYCLES {
            if completed == WARMUP {
                started = crate::monotonic_ms();
            }
            if completed >= WARMUP && SAMPLES.contains(&(completed - WARMUP)) {
                sample(&app, &ctx, phase, completed - WARMUP);
            }
            if completed == WARMUP + CYCLES {
                break;
            }
            let (uid, first, last, mut probes) = target(&mut app, &ctx, usize::from(retained_phase));
            if completed >= WARMUP && SAMPLES.contains(&(completed - WARMUP + 1)) {
                sample(&app, &ctx, &format!("{phase}_before_close"), completed - WARMUP + 1);
            }
            close_target(&mut app, &ctx, &s, (uid, first, last), &mut probes);
            probes.assert_dead();
            released(&app, &ctx, &s, uid, usize::from(retained_phase));
            assert_eq!(retained.live(), intentional);
            assert_eq!(owner_ids(&app.session), owners);
            assert_eq!(estimated_reachable(&app.session), baseline);
        }
        println!(
            "M03D_COMPLETE phase={phase} warmup={WARMUP} measured={CYCLES} checked={} measured_elapsed_seconds={:.6}",
            WARMUP + CYCLES,
            (crate::monotonic_ms() - started) / 1000.0
        );
        if retained_phase {
            app.session.new_workbook();
            app.follow_active_window();
            let destination = app.session.active_window().unwrap();
            app.run("edit.paste", json!({"at":"A1"})).unwrap();
            assert_pasted(&app.session, 1007.0);
            app.run("edit.formatPainter", json!({"apply":"E10:G11"})).unwrap();
            let d = app.session.doc().unwrap();
            let sh = d.wb.active().unwrap();
            for cell in gridcraft_engine::core::RangeRef::parse("E10:G11").unwrap().iter() {
                assert_eq!(sh.value(cell), Value::Empty);
                assert!(d.wb.styles.get(sh.style_id(cell)).font.bold);
                assert!(!sh.comments.contains_key(&cell) && !sh.cell_pictures.contains_key(&cell));
            }
            assert!(app.session.format_painter.is_some());
            app.enter_window(s.window);
            run(&mut app.session, "window.close", json!({"window":destination,"force":true}));
        }
        app.run("edit.clearClipboard", json!({})).unwrap();
        assert!(app.session.clipboard.is_none() && app.session.format_painter.is_none());
        ticks(&mut app, &ctx);
        retained.assert_dead();
        released(&app, &ctx, &s, u64::MAX, 0);
    }
}
