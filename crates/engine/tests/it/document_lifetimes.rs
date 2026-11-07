//! Document closure must drop snapshots and shared payloads; clipboard/painter are intentional owners.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::{Arc, Weak};

use gridcraft_core::{Array, CellRef, RangeRef, Value};
use gridcraft_engine::{DocState, Session, Trust};
use gridcraft_formula::Expr;
use gridcraft_model::{
    Anchor, Cell, CellPicture, CfRule, Comment, CondFormat, Formula, Image, Preserved, PreservedPart, PyCode, RetainedBytes, Sheet, Style, Workbook,
};
use serde_json::{Value as Json, json};

const WARMUP: usize = 16;
const CYCLES: usize = 1_000;
const SAMPLES: [usize; 6] = [0, 100, 250, 500, 750, 1_000];

fn run(s: &mut Session, command: &str, params: Json) -> Json {
    let result = s.execute(command, params).unwrap();
    assert_eq!(s.depth(), 0, "probe only after command-local snapshot captures are dropped");
    result
}

fn at(cell: &str) -> CellRef {
    CellRef::parse(cell).unwrap()
}

fn png() -> Vec<u8> {
    let image = image::RgbaImage::from_fn(4, 2, |x, _| if x < 2 { image::Rgba([240, 60, 40, 255]) } else { image::Rgba([20, 100, 240, 128]) });
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(image).write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    bytes
}

fn preserved(name: &str) -> Arc<Preserved> {
    Arc::new(Preserved {
        parts: vec![PreservedPart {
            name: name.into(),
            content_type: "application/xml".into(),
            data: Arc::from(b"<m03>fixture</m03>".as_slice()),
            rels: vec![],
        }],
        ..Default::default()
    })
}

// Fresh allocations on every call, including notes, encoded images and opaque roots. This is
// a model lifetime fixture, not an imported package or an executable/valid VBA project.
fn fixture(trust: Trust) -> DocState {
    let mut wb = Workbook::new();
    wb.sheet_mut(0).unwrap().name = "Lifetime Data".into();
    wb.sheets.push(Arc::new(Sheet::new("Lifetime Aux")));
    let mut style = Style::default();
    style.font.bold = true;
    let style = wb.styles.intern(style);
    let sh = wb.sheet_mut(0).unwrap();
    let text: Arc<str> = Arc::from("t".repeat(4096));
    for row in 0..256 {
        sh.set_value(CellRef::new(row, 0), Value::Text(text.clone()));
        sh.set_value(CellRef::new(row, 1), Value::Number(f64::from(row + 1)));
    }
    sh.set_cell(at("C1"), Cell::formula(Formula::new("=B1+1")));
    for cell in RangeRef::parse("A1:C2").unwrap().iter() {
        sh.set_style(cell, style);
    }
    for row in 0..64 {
        sh.comments_mut().insert(
            CellRef::new(row, 0),
            Comment {
                author: "M03".into(),
                text: "n".repeat(1024),
                replies: vec![],
                threaded: false,
                resolved: false,
                visible: false,
                dates: vec![],
                note_box: None,
            },
        );
    }
    sh.set_picture(at("C2"), Arc::new(CellPicture { data: png(), mime: "image/png".into(), alt: "M03 cell picture".into() }));
    sh.images.push(Image {
        id: 1,
        anchor: Anchor { cell: at("E4"), width: 40.0, height: 20.0, ..Default::default() },
        data: Arc::new(png()),
        mime: "image/png".into(),
        alt: "M03 floating picture".into(),
    });
    sh.cond_formats.push(CondFormat {
        ranges: vec![RangeRef::parse("B1:B256").unwrap()],
        rule: CfRule::Top10 { bottom: false, percent: false, rank: 10, style: Box::default() },
        priority: 0,
        stop_if_true: false,
    });
    wb.preserved = Some(preserved("customXml/m03-workbook.xml"));
    wb.sheet_mut(1).unwrap().preserved = Some(preserved("customXml/m03-sheet.xml"));
    wb.vba_project = Some(Arc::new(b"M03 synthetic VBA root".to_vec()));
    DocState::new(wb, None, "M03 target".into(), trust)
}

struct Handles<T: ?Sized>(Vec<Weak<T>>);

impl<T: ?Sized> Default for Handles<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T: ?Sized> Handles<T> {
    fn add(&mut self, owner: &Arc<T>) {
        let weak = Arc::downgrade(owner);
        if !self.0.iter().any(|old| old.ptr_eq(&weak)) {
            self.0.push(weak);
        }
    }

    fn live(&self) -> usize {
        self.0.iter().filter(|w| w.strong_count() != 0).count()
    }
}

#[derive(Default)]
struct Probes {
    workbooks: Handles<Workbook>,
    sheets: Handles<Sheet>,
    notes: Handles<BTreeMap<CellRef, Comment>>,
    texts: Handles<str>,
    formulas: Handles<Formula>,
    expressions: Handles<Expr>,
    pictures: Handles<CellPicture>,
    floating: Handles<Vec<u8>>,
    preserved: Handles<Preserved>,
    parts: Handles<[u8]>,
    vba: Handles<Vec<u8>>,
    unread_macros: Handles<Vec<Json>>,
    arrays: Handles<Array>,
    python: Handles<PyCode>,
}

impl Probes {
    fn value(&mut self, value: &Value) {
        match value {
            Value::Text(text) => self.texts.add(text),
            Value::Array(array) => {
                self.arrays.add(array);
                for value in array.iter() {
                    self.value(value);
                }
            }
            _ => {}
        }
    }

    fn kept(&mut self, kept: &Option<Arc<Preserved>>) {
        if let Some(kept) = kept {
            self.preserved.add(kept);
            for part in kept.parts.iter().chain(&kept.vml) {
                self.parts.add(&part.data);
            }
        }
    }

    fn workbook(&mut self, wb: &Arc<Workbook>) {
        self.workbooks.add(wb);
        self.kept(&wb.preserved);
        if let Some(vba) = &wb.vba_project {
            self.vba.add(vba);
        }
        self.unread_macros.add(&wb.macros_unread);
        for sh in &wb.sheets {
            self.sheets.add(sh);
            self.notes.add(&sh.comments);
            self.kept(&sh.preserved);
            for (_, cell) in sh.cells.iter() {
                self.value(&cell.value);
                if let Some(formula) = &cell.formula {
                    self.formulas.add(formula);
                    if let Some(expr) = &formula.expr {
                        self.expressions.add(expr);
                    }
                }
                if let Some(py) = &cell.py {
                    self.python.add(py);
                    if let Some(output) = &py.output {
                        self.value(&output.value);
                    }
                }
            }
            for value in sh.spill.values() {
                self.value(value);
            }
            for picture in sh.cell_pictures.values() {
                self.pictures.add(picture);
            }
            for image in &sh.images {
                self.floating.add(&image.data);
            }
        }
    }

    fn document(&mut self, s: &Session) {
        let d = s.doc().unwrap();
        self.workbook(&d.wb);
        self.workbook(&d.saved);
        for h in d.undo.iter().chain(&d.redo) {
            self.workbook(&h.wb);
        }
    }

    fn live(&self) -> [usize; 14] {
        [
            self.workbooks.live(),
            self.sheets.live(),
            self.notes.live(),
            self.texts.live(),
            self.formulas.live(),
            self.expressions.live(),
            self.pictures.live(),
            self.floating.live(),
            self.preserved.live(),
            self.parts.live(),
            self.vba.live(),
            self.unread_macros.live(),
            self.arrays.live(),
            self.python.live(),
        ]
    }

    fn assert_dead(&self) {
        assert_eq!(self.live(), [0; 14], "target workbook/shared payload owners remain");
        // strong_count zero is checked above; upgrade independently checks all workbook roots.
        assert!(self.workbooks.0.iter().all(|w| w.upgrade().is_none()));
    }
}

fn sentinel() -> (Session, u64, u64) {
    let mut s = Session::new();
    s.new_workbook();
    let uid = s.doc().unwrap().uid;
    let window = s.active_window().unwrap();
    (s, uid, window)
}

fn assert_sentinel(s: &Session, uid: u64, window: u64) {
    assert_eq!(s.documents().len(), 1);
    assert_eq!(s.windows().len(), 1);
    assert_eq!(s.doc().unwrap().uid, uid);
    assert_eq!(s.active_window(), Some(window));
    assert!(!s.in_group(uid));
    assert!(s.macro_debug.is_none());
    assert_eq!(s.depth(), 0);
}

fn add(s: &mut Session, trust: Trust) -> u64 {
    s.add_document(fixture(trust));
    s.active_window().unwrap()
}

fn close(s: &mut Session, window: u64) {
    assert_eq!(run(s, "window.close", json!({"window": window, "force": true}))["closed"], true);
}

fn edits(s: &mut Session, probes: &mut Probes) {
    probes.document(s);
    for edit in 1..=8 {
        run(s, "cell.set", json!({"cell": "B1", "input": (1000 + edit).to_string()}));
        probes.document(s);
    }
    run(s, "edit.undo", json!({"steps": 2}));
    probes.document(s);
    run(s, "edit.redo", json!({"steps": 1}));
    probes.document(s);
    let d = s.doc().unwrap();
    assert_eq!((d.undo.len(), d.redo.len()), (7, 1));
    assert_eq!(d.wb.active().unwrap().value(at("B1")), Value::Number(1007.0));
    assert_eq!(d.wb.active().unwrap().value(at("C1")), Value::Number(1008.0));
}

// Sum deduplicated reachable Workbook allocations, not process heap/RSS. This context must
// leave scope before a close; it borrows the real initialized band inventories.
fn estimated_reachable(s: &Session) -> usize {
    let mut context = RetainedBytes::default();
    let mut bytes = 0usize;
    for d in s.documents() {
        for wb in std::iter::once(&d.wb).chain(std::iter::once(&d.saved)).chain(d.undo.iter().chain(&d.redo).map(|h| &h.wb)) {
            bytes = bytes.saturating_add(context.workbook(wb));
        }
    }
    for wb in s.clipboard.iter().map(|c| &c.wb).chain(s.format_painter.iter().map(|p| &p.0)) {
        bytes = bytes.saturating_add(context.workbook(wb));
    }
    bytes
}

#[test]
fn fixture_has_fixed_shape_and_fresh_payloads() {
    let first = fixture(Trust::Local);
    let second = fixture(Trust::Local);
    let sh = first.wb.active().unwrap();
    assert_eq!(first.wb.sheets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Lifetime Data", "Lifetime Aux"]);
    assert_eq!(sh.value(at("B256")), Value::Number(256.0));
    assert_eq!(sh.comments.len(), 64);
    assert!(sh.comments.values().all(|n| n.text.len() == 1024));
    let Value::Text(text) = sh.value_ref(at("A1")).unwrap() else { panic!("shared text missing") };
    assert_eq!(text.len(), 4096);
    let Value::Text(last) = sh.value_ref(at("A256")).unwrap() else { panic!("shared text missing") };
    assert!(Arc::ptr_eq(text, last));
    assert!(sh.cell(at("C1")).unwrap().formula.as_ref().unwrap().expr.is_some());
    let picture = &sh.cell_pictures[&at("C2")];
    assert_eq!(image::load_from_memory(&picture.data).unwrap().width(), 4);
    assert_eq!(image::load_from_memory(&picture.data).unwrap().height(), 2);
    assert_ne!(picture.data.as_ptr(), sh.images[0].data.as_ptr());
    let mut one = Probes::default();
    one.document_for_fixture(&first);
    let mut two = Probes::default();
    two.document_for_fixture(&second);
    assert_eq!(one.live(), [1, 2, 2, 1, 1, 1, 1, 1, 2, 2, 1, 1, 0, 0]);
    assert_eq!(two.live(), one.live());
    one.workbook(&second.wb);
    assert_eq!(one.live(), [2, 4, 4, 2, 2, 2, 2, 2, 4, 4, 2, 2, 0, 0], "no master payload across cycles");
    eprintln!("M03 fixture PNG bytes={} hex={}", picture.data.len(), picture.data.iter().map(|b| format!("{b:02x}")).collect::<String>());
    drop((first, second));
    one.assert_dead();
    two.assert_dead();
}

impl Probes {
    fn document_for_fixture(&mut self, d: &DocState) {
        self.workbook(&d.wb);
        self.workbook(&d.saved);
    }
}

#[test]
fn closed_documents_release_owned_snapshots() {
    let (mut s, uid, sentinel) = sentinel();
    let target = add(&mut s, Trust::Local);
    let target_uid = s.doc().unwrap().uid;
    let mut probes = Probes::default();
    edits(&mut s, &mut probes);
    assert_eq!(probes.workbooks.live(), 9);
    assert!(estimated_reachable(&s) > 64 * 1024);
    run(&mut s, "edit.clearClipboard", json!({}));
    s.activate_window(sentinel);
    close(&mut s, target);
    assert!(!s.in_group(target_uid));
    assert_sentinel(&s, uid, sentinel);
    probes.assert_dead();
}

#[test]
fn closing_one_window_keeps_document_usable() {
    let (mut s, uid, sentinel) = sentinel();
    let first = add(&mut s, Trust::Local);
    let target_uid = s.doc().unwrap().uid;
    let second = run(&mut s, "view.newWindow", json!({}))["window"].as_u64().unwrap();
    let mut probes = Probes::default();
    probes.document(&s);
    s.activate_window(sentinel);
    close(&mut s, first);
    assert!(s.document_index(target_uid).is_some());
    assert_eq!((s.documents().len(), s.windows().len()), (2, 2));
    assert_eq!(probes.workbooks.live(), 1);
    s.activate_window(second);
    run(&mut s, "cell.set", json!({"cell": "B1", "input": "77"}));
    assert_eq!(s.doc().unwrap().wb.active().unwrap().value(at("C1")), Value::Number(78.0));
    probes.document(&s);
    s.activate_window(sentinel);
    close(&mut s, second);
    assert_sentinel(&s, uid, sentinel);
    probes.assert_dead();
}

#[test]
fn closing_document_removes_all_windows() {
    let (mut s, uid, sentinel) = sentinel();
    add(&mut s, Trust::Local);
    run(&mut s, "view.newWindow", json!({}));
    let mut probes = Probes::default();
    probes.document(&s);
    run(&mut s, "file.close", json!({"document": true, "force": true}));
    assert_sentinel(&s, uid, sentinel);
    probes.assert_dead();
}

#[test]
fn closing_open_group_releases_snapshots() {
    let (mut s, uid, sentinel) = sentinel();
    let target = add(&mut s, Trust::Local);
    let mut probes = Probes::default();
    edits(&mut s, &mut probes);
    let token = s.group_begin("M03 open group").unwrap();
    let target_uid = token.doc();
    run(&mut s, "cell.set", json!({"cell": "B1", "input": "2001"}));
    probes.document(&s);
    assert!(s.group_is_open(&token));
    assert_eq!(s.doc().unwrap().redo.len(), 1, "group owns the preexisting redo snapshot");
    s.activate_window(sentinel);
    close(&mut s, target);
    assert!(!s.group_is_open(&token));
    assert!(!s.in_group(target_uid));
    assert_sentinel(&s, uid, sentinel);
    probes.assert_dead();
}

#[test]
fn closing_real_debugger_releases_start_and_guard() {
    let (mut s, uid, sentinel) = sentinel();
    let target = add(&mut s, Trust::Local);
    run(
        &mut s,
        "macro.save",
        json!({"name": "M03Walk", "output": {"onConflict": "fail"}, "steps": [
            {"command": "cell.set", "params": {"cell": "E1", "input": "first"}},
            {"command": "cell.set", "params": {"cell": "E2", "input": "second"}}
        ]}),
    );
    let mut probes = Probes::default();
    probes.document(&s);
    let target_uid = s.doc().unwrap().uid;
    assert_eq!(run(&mut s, "macro.debug", json!({"action": "start", "name": "M03Walk", "onConflict": "fail"}))["state"], "paused");
    assert_eq!(s.macro_debug.as_ref().unwrap().doc, target_uid);
    assert!(s.in_group(target_uid));
    // A non-overwrite run installs Machine.guard.start; overwrite would skip that owner.
    assert_eq!(run(&mut s, "macro.debug", json!({"action": "step"}))["state"], "paused");
    assert_eq!(s.doc().unwrap().wb.active().unwrap().value(at("E1")), Value::from("first"));
    probes.document(&s);
    s.activate_window(sentinel);
    close(&mut s, target);
    assert!(!s.in_group(target_uid));
    assert_sentinel(&s, uid, sentinel);
    probes.assert_dead();
}

fn copied_source(s: &mut Session, command: &str) -> (Probes, Probes) {
    let source = add(s, Trust::FromFile);
    let mut all = Probes::default();
    edits(s, &mut all);
    run(s, command, json!({"range": "A1:C2"}));
    let mut retained = Probes::default();
    retained.workbook(&s.clipboard.as_ref().unwrap().wb);
    close(s, source);
    assert_eq!(all.live(), retained.live(), "only the clipboard version and its shared payloads survive");
    (all, retained)
}

fn assert_pasted(s: &Session, number: f64) {
    let d = s.doc().unwrap();
    let sh = d.wb.active().unwrap();
    assert_eq!(sh.value(at("A1")), Value::from("t".repeat(4096)));
    assert_eq!(sh.value(at("B1")), Value::Number(number));
    assert_eq!(sh.value(at("C1")), Value::Number(number + 1.0));
    assert_eq!(sh.cell(at("C1")).unwrap().formula.as_ref().unwrap().text, "B1+1");
    assert!(d.wb.styles.get(sh.style_id(at("A1"))).font.bold);
    assert_eq!(sh.comments[&at("A1")].text, "n".repeat(1024));
    assert_eq!(sh.comments[&at("A2")].text, "n".repeat(1024));
    assert_eq!(sh.cell_pictures[&at("C2")].data, png());
    assert_eq!(sh.cell_pictures[&at("C2")].alt, "M03 cell picture");
    assert!(sh.images.is_empty(), "range paste is not an object paste");
    assert!(d.wb.vba_project.is_none() && d.wb.preserved.is_none());
    assert_eq!(d.trust, Trust::FromFile);
}

#[test]
fn clipboard_survives_source_close() {
    let (mut s, uid, sentinel) = sentinel();
    let (all, retained) = copied_source(&mut s, "edit.copy");
    assert_eq!(retained.live(), [1, 2, 2, 1, 1, 1, 1, 1, 2, 2, 1, 1, 0, 0]);
    s.new_workbook();
    let dest = s.active_window().unwrap();
    run(&mut s, "edit.paste", json!({"at": "A1"}));
    assert_pasted(&s, 1007.0);
    assert!(Arc::ptr_eq(
        &s.clipboard.as_ref().unwrap().wb.active().unwrap().cell_pictures[&at("C2")],
        &s.doc().unwrap().wb.active().unwrap().cell_pictures[&at("C2")]
    ));
    close(&mut s, dest);
    assert_eq!(all.live(), retained.live());
    run(&mut s, "edit.clearClipboard", json!({}));
    assert!(s.clipboard.is_none() && s.format_painter.is_none());
    assert_sentinel(&s, uid, sentinel);
    all.assert_dead();
    retained.assert_dead();
}

#[test]
fn source_closed_cut_keeps_copy_like_behavior() {
    for command in ["edit.paste", "edit.pasteSpecial"] {
        let (mut s, uid, sentinel) = sentinel();
        let (all, retained) = copied_source(&mut s, "edit.cut");
        s.new_workbook();
        let dest = s.active_window().unwrap();
        run(&mut s, command, json!({"at": "A1", "what": "all"}));
        assert_pasted(&s, 1007.0);
        assert!(s.clipboard.as_ref().unwrap().cut, "only same-document cut-move consumes the clipboard");
        close(&mut s, dest);
        run(&mut s, "edit.clearClipboard", json!({}));
        assert_sentinel(&s, uid, sentinel);
        all.assert_dead();
        retained.assert_dead();
    }
}

#[test]
fn painter_survives_source_close_until_escape() {
    let (mut s, uid, sentinel) = sentinel();
    let source = add(&mut s, Trust::Local);
    run(&mut s, "selection.set", json!({"range": "A1:C2"}));
    run(&mut s, "edit.formatPainter", json!({"sticky": true}));
    let mut probes = Probes::default();
    probes.workbook(&s.format_painter.as_ref().unwrap().0);
    close(&mut s, source);
    assert_eq!(probes.workbooks.live(), 1);
    assert!(s.clipboard.is_none());
    s.new_workbook();
    let dest = s.active_window().unwrap();
    run(&mut s, "edit.formatPainter", json!({"apply": "A1:C2"}));
    let sh = s.doc().unwrap().wb.active().unwrap();
    assert!(s.doc().unwrap().wb.styles.get(sh.style_id(at("A1"))).font.bold);
    assert_eq!(sh.value(at("A1")), Value::Empty);
    assert!(sh.comments.is_empty() && sh.cell_pictures.is_empty());
    assert!(s.format_painter.is_some());
    close(&mut s, dest);
    run(&mut s, "edit.clearClipboard", json!({}));
    assert!(s.clipboard.is_none() && s.format_painter.is_none());
    assert_sentinel(&s, uid, sentinel);
    probes.assert_dead();
}

#[test]
fn clipboard_and_painter_clear_together() {
    let (mut s, uid, sentinel) = sentinel();
    let source = add(&mut s, Trust::Local);
    run(&mut s, "selection.set", json!({"range": "A1:C2"}));
    run(&mut s, "edit.copy", json!({}));
    run(&mut s, "edit.formatPainter", json!({"sticky": true}));
    let mut probes = Probes::default();
    probes.document(&s);
    close(&mut s, source);
    assert!(s.clipboard.is_some() && s.format_painter.is_some());
    assert_eq!(probes.workbooks.live(), 1);
    run(&mut s, "edit.clearClipboard", json!({}));
    assert!(s.clipboard.is_none() && s.format_painter.is_none());
    assert_sentinel(&s, uid, sentinel);
    probes.assert_dead();
}

#[test]
fn source_closed_multi_area_paste() {
    let (mut s, uid, sentinel) = sentinel();
    let mut d = fixture(Trust::FromFile);
    let sh = Arc::make_mut(&mut d.wb).sheet_mut(0).unwrap();
    sh.set_cell(at("D1"), Cell::formula(Formula::new("=B1+10")));
    sh.set_picture(at("D2"), Arc::new(CellPicture { data: png(), mime: "image/png".into(), alt: "packed picture".into() }));
    d.calc.recalc_all(Arc::make_mut(&mut d.wb));
    s.add_document(d);
    let source = s.active_window().unwrap();
    let mut all = Probes::default();
    all.document(&s);
    run(&mut s, "edit.copy", json!({"range": "A1"}));
    assert!(s.execute("edit.cut", json!({"range": "A1:B2,D1:D2"})).is_err());
    assert_eq!(s.clipboard.as_ref().unwrap().range.a1(), "A1");
    run(&mut s, "edit.copy", json!({"range": "D1:D2,A1:B2"}));
    assert_eq!(s.clipboard.as_ref().unwrap().areas.len(), 2);
    assert!(!Arc::ptr_eq(&s.doc().unwrap().wb, &s.clipboard.as_ref().unwrap().wb));
    let mut packed = Probes::default();
    packed.workbook(&s.clipboard.as_ref().unwrap().wb);
    all.workbook(&s.clipboard.as_ref().unwrap().wb);
    close(&mut s, source);
    assert_eq!(all.live(), packed.live());
    s.new_workbook();
    let dest = s.active_window().unwrap();
    assert!(s.execute("edit.pasteSpecial", json!({"link": true, "at": "A1"})).is_err());
    run(&mut s, "edit.paste", json!({"at": "A1"}));
    let sh = s.doc().unwrap().wb.active().unwrap();
    assert_eq!(sh.value(at("B1")), Value::Number(1.0));
    assert_eq!(sh.value(at("C1")), Value::Number(11.0));
    assert!(sh.cell(at("C1")).unwrap().formula.is_none(), "packed formulas become values");
    assert_eq!(sh.cell_pictures[&at("C2")].alt, "packed picture");
    assert_eq!(sh.comments[&at("A1")].text.len(), 1024);
    assert!(s.doc().unwrap().wb.styles.get(sh.style_id(at("A1"))).font.bold);
    close(&mut s, dest);
    run(&mut s, "edit.clearClipboard", json!({}));
    assert_sentinel(&s, uid, sentinel);
    all.assert_dead();
    packed.assert_dead();
}

#[test]
fn source_closed_paste_special_modes() {
    for what in ["values", "formulas", "formats", "notes"] {
        let (mut s, uid, sentinel) = sentinel();
        let (all, retained) = copied_source(&mut s, "edit.copy");
        s.new_workbook();
        let dest = s.active_window().unwrap();
        run(&mut s, "cell.set", json!({"cell": "A1", "input": "destination"}));
        run(&mut s, "edit.pasteSpecial", json!({"what": what, "at": "A1"}));
        let d = s.doc().unwrap();
        let sh = d.wb.active().unwrap();
        if matches!(what, "values" | "formulas") {
            assert_eq!(sh.value(at("B1")), Value::Number(1007.0));
            assert_eq!(sh.value(at("C1")), Value::Number(1008.0));
            assert_eq!(sh.cell(at("C1")).unwrap().formula.is_some(), what == "formulas");
            assert_eq!(sh.cell_pictures[&at("C2")].data, png());
        } else {
            assert_eq!(sh.value(at("A1")), Value::from("destination"));
            assert_eq!(sh.value(at("B1")), Value::Empty);
            assert!(sh.cell_pictures.is_empty());
        }
        assert_eq!(sh.comments.contains_key(&at("A1")), what == "notes");
        assert_eq!(d.wb.styles.get(sh.style_id(at("A1"))).font.bold, what == "formats");
        assert_eq!(d.trust, if what == "formulas" { Trust::FromFile } else { Trust::Local });
        close(&mut s, dest);
        run(&mut s, "edit.clearClipboard", json!({}));
        assert_sentinel(&s, uid, sentinel);
        all.assert_dead();
        retained.assert_dead();
    }
}

fn cycle(s: &mut Session, uid: u64, sentinel: u64, keep_clipboard: bool, sample: Option<usize>) {
    let first = add(s, Trust::Local);
    let target_uid = s.doc().unwrap().uid;
    let second = run(s, "view.newWindow", json!({}))["window"].as_u64().unwrap();
    let mut probes = Probes::default();
    edits(s, &mut probes);
    let before = probes.live();
    assert_eq!(before[0], 9);
    if let Some(completed) = sample {
        eprintln!(
            "M03 before_close keep_clipboard={keep_clipboard} cycle={completed} docs={} windows={} undo=7 redo=1 target={before:?} estimated_reachable_bytes={}",
            s.documents().len(),
            s.windows().len(),
            estimated_reachable(s)
        );
    }
    if !keep_clipboard {
        run(s, "edit.clearClipboard", json!({}));
        assert!(s.clipboard.is_none() && s.format_painter.is_none());
    }
    s.activate_window(sentinel);
    close(s, first);
    assert!(s.document_index(target_uid).is_some());
    close(s, second);
    assert!(!s.in_group(target_uid));
    assert_sentinel(s, uid, sentinel);
    probes.assert_dead();
}

fn owner_cycles(keep_clipboard: bool) {
    let (mut s, uid, sentinel) = sentinel();
    let retained = if keep_clipboard {
        let (_, retained) = copied_source(&mut s, "edit.copy");
        retained
    } else {
        Probes::default()
    };
    let intentional = retained.live();
    for _ in 0..WARMUP {
        cycle(&mut s, uid, sentinel, keep_clipboard, None);
        assert_eq!(retained.live(), intentional);
    }
    let baseline = estimated_reachable(&s);
    for completed in 0..=CYCLES {
        if SAMPLES.contains(&completed) {
            assert_eq!(estimated_reachable(&s), baseline, "reachable workbook estimate must return to the retained baseline");
            eprintln!(
                "M03 owners keep_clipboard={keep_clipboard} cycle={completed} docs=1 windows=1 target_live=0 retained={intentional:?} estimated_reachable_bytes={baseline}"
            );
        }
        if completed < CYCLES {
            let sample = SAMPLES.contains(&(completed + 1)).then_some(completed + 1);
            cycle(&mut s, uid, sentinel, keep_clipboard, sample);
            assert_eq!(retained.live(), intentional);
        }
    }
    if keep_clipboard {
        s.new_workbook();
        let dest = s.active_window().unwrap();
        run(&mut s, "edit.paste", json!({"at": "A1"}));
        assert_pasted(&s, 1007.0);
        close(&mut s, dest);
    }
    run(&mut s, "edit.clearClipboard", json!({}));
    assert_sentinel(&s, uid, sentinel);
    retained.assert_dead();
}

#[test]
fn cleared_owner_cycles() {
    owner_cycles(false);
}

#[test]
fn retained_clipboard_owner_cycles() {
    owner_cycles(true);
}
