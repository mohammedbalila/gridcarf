//! Shared synthetic lifetime payloads and nonowning probes; isolated tests need the workspace source layout.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::{Arc, Weak};

use gridcraft_engine::core::{Array, CellRef, RangeRef, Value};
use gridcraft_engine::formula::Expr;
use gridcraft_engine::model::{
    Anchor, Cell, CellPicture, CfRule, Comment, CondFormat, Formula, Image, Preserved, PreservedPart, PyCode, RetainedBytes, Sheet, Style, Workbook,
};
use gridcraft_engine::{DocState, Session, Trust};
use serde_json::{Value as Json, json};

use super::run;

pub(super) const WARMUP: usize = 16;
pub(super) const CYCLES: usize = 1_000;
pub(super) const SAMPLES: [usize; 6] = [0, 100, 250, 500, 750, 1_000];

pub(super) fn at(cell: &str) -> CellRef {
    CellRef::parse(cell).unwrap()
}

pub(super) fn png() -> Vec<u8> {
    let image = image::RgbaImage::from_fn(4, 2, |x, _| if x < 2 { image::Rgba([240, 60, 40, 255]) } else { image::Rgba([20, 100, 240, 128]) });
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(image).write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    bytes
}

pub(super) fn preserved(name: &str) -> Arc<Preserved> {
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
pub(super) fn fixture(trust: Trust) -> DocState {
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

pub(super) struct Handles<T: ?Sized>(Vec<Weak<T>>);

impl<T: ?Sized> Default for Handles<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T: ?Sized> Handles<T> {
    pub(super) fn add(&mut self, owner: &Arc<T>) {
        let weak = Arc::downgrade(owner);
        if !self.0.iter().any(|old| old.ptr_eq(&weak)) {
            self.0.push(weak);
        }
    }

    pub(super) fn live(&self) -> usize {
        self.0.iter().filter(|w| w.strong_count() != 0).count()
    }
}

#[derive(Default)]
pub(super) struct Probes {
    pub(super) workbooks: Handles<Workbook>,
    pub(super) sheets: Handles<Sheet>,
    pub(super) notes: Handles<BTreeMap<CellRef, Comment>>,
    pub(super) texts: Handles<str>,
    pub(super) formulas: Handles<Formula>,
    pub(super) expressions: Handles<Expr>,
    pub(super) pictures: Handles<CellPicture>,
    pub(super) floating: Handles<Vec<u8>>,
    pub(super) preserved: Handles<Preserved>,
    pub(super) parts: Handles<[u8]>,
    pub(super) vba: Handles<Vec<u8>>,
    pub(super) unread_macros: Handles<Vec<Json>>,
    pub(super) arrays: Handles<Array>,
    pub(super) python: Handles<PyCode>,
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

    pub(super) fn workbook(&mut self, wb: &Arc<Workbook>) {
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

    pub(super) fn document(&mut self, s: &Session) {
        let d = s.doc().unwrap();
        self.workbook(&d.wb);
        self.workbook(&d.saved);
        for h in d.undo.iter().chain(&d.redo) {
            self.workbook(&h.wb);
        }
    }

    pub(super) fn live(&self) -> [usize; 14] {
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

    pub(super) fn assert_dead(&self) {
        assert_eq!(self.live(), [0; 14], "target workbook/shared payload owners remain");
        // strong_count zero is checked above; upgrade independently checks all workbook roots.
        assert!(self.workbooks.0.iter().all(|w| w.upgrade().is_none()));
    }
}

pub(super) fn edits(s: &mut Session, probes: &mut Probes) {
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
pub(super) fn estimated_reachable(s: &Session) -> usize {
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

pub(super) fn assert_pasted(s: &Session, number: f64) {
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
