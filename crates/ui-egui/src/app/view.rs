use super::{CellRef, SheetApp, SheetView};
use gridcraft_engine::Session;
use std::collections::{HashMap, HashSet};

#[cfg(test)]
thread_local! {
    static REACHABILITY_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Nonowning witness for supported Session mutations; sheet identity is numeric, because
/// SheetUid equality deliberately ignores identity. Never keep workbook owners here.
pub(crate) struct ReachableSheets {
    revision: u64,
    roots: Vec<usize>,
    sheets: HashSet<u64>,
}

impl ReachableSheets {
    fn matches(&self, session: &Session, uid: u64) -> bool {
        let Some(d) = session.documents().iter().find(|d| d.uid == uid) else { return false };
        if self.revision == u64::MAX || self.revision != d.revision {
            return false;
        }
        let mut roots = self.roots.iter();
        let mut matches = true;
        session.visit_document_workbooks(uid, |wb| {
            matches &= roots.next().copied() == Some(std::ptr::from_ref(wb) as usize);
        });
        matches && roots.next().is_none()
    }

    fn new(session: &Session, uid: u64, revision: u64) -> Self {
        #[cfg(test)]
        REACHABILITY_BUILDS.with(|count| count.set(count.get() + 1));
        let mut entry = Self { revision, roots: Vec::new(), sheets: HashSet::new() };
        session.visit_document_workbooks(uid, |wb| {
            entry.roots.push(std::ptr::from_ref(wb) as usize);
            entry.sheets.extend(wb.sheets.iter().map(|sh| sh.uid.get()));
        });
        entry
    }
}

impl SheetApp {
    /// Keep scroll only while a live window's document can restore that sheet. Stable passes
    /// compare root tokens, avoiding a scan of every historical workbook's sheet list.
    pub(crate) fn prune_sheet_views(&mut self) {
        let windows: HashMap<_, _> = self.session.windows().iter().map(|w| (w.id, w.doc_uid)).collect();
        let mut documents: HashSet<_> = self.views.keys().filter_map(|(w, _)| windows.get(w).copied()).collect();
        self.reachable_sheets.retain(|uid, _| documents.contains(uid));
        for &uid in &documents {
            let Some(d) = self.session.documents().iter().find(|d| d.uid == uid) else {
                self.reachable_sheets.remove(&uid);
                continue;
            };
            if !self.reachable_sheets.get(&uid).is_some_and(|entry| entry.matches(&self.session, uid)) {
                self.reachable_sheets.insert(uid, ReachableSheets::new(&self.session, uid, d.revision));
            }
        }
        self.views.retain(|(window, sheet), _| {
            windows.get(window).and_then(|uid| self.reachable_sheets.get(uid)).is_some_and(|entry| entry.sheets.contains(sheet))
        });
        documents.clear();
        documents.extend(self.views.keys().filter_map(|(w, _)| windows.get(w).copied()));
        self.reachable_sheets.retain(|uid, _| documents.contains(uid));
    }

    /// The active workbook and sheet: what drafts, text boxes and picture targets belong to.
    pub fn view_key(&self) -> Option<(u64, usize)> {
        self.session.active().map(|d| (d.uid, d.wb.active_sheet()))
    }

    /// The active window and sheet: each window scrolls on its own.
    fn scroll_key(&self) -> Option<(u64, u64)> {
        Some((self.session.active_window()?, self.session.active()?.wb.active()?.uid.get()))
    }

    pub fn view(&self) -> SheetView {
        self.scroll_key().and_then(|k| self.views.get(&k).copied()).unwrap_or_else(|| self.saved_view())
    }

    pub fn view_mut(&mut self) -> Option<&mut SheetView> {
        let k = self.scroll_key()?;
        let saved = self.saved_view();
        Some(self.views.entry(k).or_insert(saved))
    }

    /// The view a sheet opens with: scrolled to the file's saved first visible cell (`topLeftCell`).
    fn saved_view(&self) -> SheetView {
        let Some(sh) = self.session.active().and_then(|d| d.wb.active()) else { return SheetView::default() };
        if sh.view != gridcraft_engine::model::SheetViewMode::Normal {
            return SheetView::default();
        }
        let (fr, fc) = sh.freeze.unwrap_or((0, 0));
        let top = sh.view_top_left;
        let y = sh.row_top(top.row.max(fr)) - sh.row_top(fr);
        let x = sh.col_left(top.col.max(fc)) - sh.col_left(fc);
        SheetView { scroll: egui::vec2(x.max(0.0) as f32, y.max(0.0) as f32) }
    }

    /// Hands the active document's scroll positions (first visible cell per sheet) and active
    /// cell to the engine so a save writes them (`sheetView topLeftCell`). Normal view only:
    /// Page Layout view scrolls over pages, not cells.
    pub fn store_view_state(&mut self) {
        if let Some(uid) = self.session.active().map(|d| d.uid) {
            self.store_document_view_state(uid);
        }
    }

    /// Flush only the window whose selection/sheet is live in this document. Other windows'
    /// scroll positions must not overwrite that view, even if they are newer in the MRU list.
    pub(super) fn store_document_view_state(&mut self, uid: u64) {
        let window = self.session.document_view_window(uid);
        let Some(d) = self.session.documents().iter().find(|d| d.uid == uid) else {
            self.reachable_sheets.remove(&uid);
            return;
        };
        let before_revision = d.revision;
        let mut tops = Vec::new();
        for (si, sh) in d.wb.sheets.iter().enumerate() {
            let Some(v) =
                window.and_then(|w| self.views.get(&(w, sh.uid.get()))).filter(|_| sh.view == gridcraft_engine::model::SheetViewMode::Normal)
            else {
                continue;
            };
            let (fr, fc) = sh.freeze.unwrap_or((0, 0));
            // A hair past the edge so a row that starts exactly at the scroll offset is the one found.
            let row = sh.row_at(sh.row_top(fr) + v.scroll.y.max(0.0) as f64 + 0.01);
            let col = sh.col_at(sh.col_left(fc) + v.scroll.x.max(0.0) as f64 + 0.01);
            tops.push((si, CellRef::new(row, col)));
        }
        // The document was just found, so this can only fail if it was removed; no commands
        // run between collecting and storing the view.
        let rebase = self.reachable_sheets.get(&uid).is_some_and(|entry| entry.revision == before_revision && entry.matches(&self.session, uid));
        if !rebase {
            self.reachable_sheets.remove(&uid);
        }
        if let Err(error) = self.session.store_document_view(uid, &tops) {
            self.reachable_sheets.remove(&uid);
            self.alert(error.to_string());
            return;
        }
        if let Some(d) = self.session.documents().iter().find(|d| d.uid == uid) {
            let after_revision = d.revision;
            // This direct call changes only view metadata and roots 0/1. General view commands
            // (including zoom) can clear redo or evict history, so must never use this rebase.
            let rebased = rebase
                && self.reachable_sheets.get_mut(&uid).is_some_and(|entry| {
                    if entry.revision != before_revision || before_revision == u64::MAX || after_revision == u64::MAX {
                        return false;
                    }
                    let Some(roots) = entry.roots.get_mut(..2) else { return false };
                    let [current, saved] = roots else { return false };
                    *current = std::ptr::from_ref(d.wb.as_ref()) as usize;
                    *saved = std::ptr::from_ref(d.saved.as_ref()) as usize;
                    entry.revision = after_revision;
                    true
                });
            if !rebased {
                self.reachable_sheets.remove(&uid);
            }
            self.rebase_document_view_traces(uid, before_revision, after_revision);
        } else {
            self.reachable_sheets.remove(&uid);
        }
    }
}

#[cfg(test)]
#[path = "../../../engine/tests/support/owned_directory.rs"]
mod owned_directory;

#[cfg(test)]
mod tests {
    use super::*;
    use gridcraft_engine::model::Workbook;
    use serde_json::json;
    use std::sync::Arc;

    fn app() -> SheetApp {
        let mut session = Session::new();
        session.new_workbook();
        let mut app = SheetApp::new(session, Default::default());
        app.ui.autosave = false;
        app.use_prepared_fonts();
        app.follow_active_window();
        app.view_mut().unwrap();
        app
    }

    fn builds() -> usize {
        REACHABILITY_BUILDS.with(std::cell::Cell::get)
    }

    fn roots(session: &Session, uid: u64) -> Vec<usize> {
        let mut roots = Vec::new();
        session.visit_document_workbooks(uid, |wb| roots.push(std::ptr::from_ref(wb) as usize));
        roots
    }

    fn tick(app: &mut SheetApp) {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::pinned_font_definitions());
        let _ = ctx.run_logic(&egui::RawInput::default(), |ctx| app.logic(ctx));
    }

    fn repeated_stored_views(dirty: bool, inactive: bool) {
        let mut app = app();
        let uid = app.session.doc().unwrap().uid;
        let window = app.session.active_window().unwrap();
        let sheet = app.session.doc().unwrap().wb.active().unwrap().uid.get();
        let group = if dirty {
            app.run("cell.set", json!({"cell":"A1","input":"1"})).unwrap();
            app.run("cell.set", json!({"cell":"A1","input":"2"})).unwrap();
            app.run("edit.undo", json!({})).unwrap();
            assert!(!app.session.doc().unwrap().undo.is_empty());
            assert!(!app.session.doc().unwrap().redo.is_empty());
            Some(app.session.group_begin("view fixture").unwrap())
        } else {
            None
        };
        let other = if inactive {
            app.run("file.new", json!({})).unwrap();
            app.view_mut().unwrap();
            Some(app.session.doc().unwrap().uid)
        } else {
            None
        };
        app.prune_sheet_views();
        let initial_builds = builds();
        let initial_roots = app.reachable_sheets[&uid].roots.clone();
        let other_before = other.map(|other| (other, app.reachable_sheets[&other].revision, app.reachable_sheets[&other].roots.clone()));
        for row in 1..=5 {
            let d = app.session.documents().iter().find(|d| d.uid == uid).unwrap();
            let revision = d.revision;
            let previous_current = Arc::as_ptr(&d.wb) as usize;
            let y = d.wb.active().unwrap().row_top(row) as f32;
            app.views.get_mut(&(window, sheet)).unwrap().scroll.y = y;
            if !inactive {
                app.session.run("selection.set", json!({"cell":format!("C{}", row + 1)})).unwrap();
            }
            app.store_document_view_state(uid);
            let d = app.session.documents().iter().find(|d| d.uid == uid).unwrap();
            assert!(d.revision > revision, "each stored-view update must actually change metadata");
            assert_eq!(d.wb.active().unwrap().view_top_left, CellRef::new(row, 0));
            if !inactive {
                assert_eq!(d.wb.active().unwrap().view_active, CellRef::new(row, 2));
            }
            assert_eq!(d.is_dirty(), dirty);
            if !dirty {
                assert_ne!(Arc::as_ptr(&d.wb) as usize, previous_current, "clean view update must exercise COW");
            }
            let entry = &app.reachable_sheets[&uid];
            assert!(entry.matches(&app.session, uid));
            assert_eq!(entry.roots, roots(&app.session, uid));
            assert_eq!(entry.sheets, HashSet::from([sheet]));
            assert_eq!(&entry.roots[if dirty { 1 } else { 2 }..], &initial_roots[if dirty { 1 } else { 2 }..]);
            tick(&mut app);
            assert_eq!(builds(), initial_builds);
        }
        app.store_document_view_state(uid);
        tick(&mut app);
        assert_eq!(builds(), initial_builds, "unchanged view and stable logic reuse membership");
        if let Some((other, revision, roots)) = other_before {
            assert_eq!(app.reachable_sheets[&other].revision, revision);
            assert_eq!(app.reachable_sheets[&other].roots, roots);
        }
        if let Some(group) = group {
            app.session.group_rollback(group).unwrap();
            app.prune_sheet_views();
            assert!(builds() > initial_builds);
            assert!(app.reachable_sheets[&uid].matches(&app.session, uid));
        }
    }

    #[test]
    fn clean_stored_views_rebase_without_membership_rebuilds() {
        repeated_stored_views(false, false);
        repeated_stored_views(false, true);
    }

    #[test]
    fn dirty_stored_views_rebase_without_membership_rebuilds() {
        repeated_stored_views(true, false);
        repeated_stored_views(true, true);
    }

    #[test]
    fn stale_root_revision_and_short_witnesses_are_never_rebased() {
        for cause in ["revision", "group", "saved", "history", "short", "extra", "wrongdoc", "max", "saturating"] {
            let mut app = app();
            let uid = app.session.doc().unwrap().uid;
            if cause == "history" {
                app.session.run("cell.set", json!({"cell":"A1","input":"1"})).unwrap();
            }
            if cause == "saturating" {
                app.session.doc_mut().unwrap().revision = u64::MAX - 1;
            }
            app.prune_sheet_views();
            let before = builds();
            let mut group = None;
            match cause {
                "revision" => {
                    app.session.run("home.insertSheet", json!({"name":"New"})).unwrap();
                }
                "group" => {
                    group = Some(app.session.group_begin("unchanged").unwrap());
                }
                // Direct substitutions below intentionally isolate unchanged-revision witness
                // guards in an owning fixture, not an external mutation contract.
                "saved" => app.session.doc_mut().unwrap().saved = Arc::new(Workbook::new()),
                "history" => app.session.doc_mut().unwrap().undo[0].wb = Arc::new(Workbook::new()),
                "short" => app.reachable_sheets.get_mut(&uid).unwrap().roots.truncate(1),
                "extra" => app.reachable_sheets.get_mut(&uid).unwrap().roots.push(0),
                "wrongdoc" => {
                    let mut other = Session::new();
                    other.new_workbook();
                    let d = other.doc().unwrap();
                    app.reachable_sheets.insert(uid, ReachableSheets::new(&other, d.uid, d.revision));
                }
                "max" => {
                    app.session.doc_mut().unwrap().revision = u64::MAX;
                    app.reachable_sheets.get_mut(&uid).unwrap().revision = u64::MAX;
                }
                _ => assert_eq!(cause, "saturating"),
            }
            app.view_mut().unwrap().scroll.y = 40.0;
            app.store_document_view_state(uid);
            assert!(!app.reachable_sheets.contains_key(&uid), "{cause} must invalidate instead of rebase");
            app.prune_sheet_views();
            assert!(builds() > before, "{cause}");
            let mut expected = HashSet::new();
            app.session.visit_document_workbooks(uid, |wb| expected.extend(wb.sheets.iter().map(|sh| sh.uid.get())));
            assert_eq!(app.reachable_sheets[&uid].sheets, expected, "{cause}");
            if let Some(group) = group {
                let count = builds();
                app.session.group_end(group).unwrap();
                app.prune_sheet_views();
                assert_eq!(builds(), count + 1, "unchanged group end changes root sequence");
            }
        }
    }

    #[test]
    fn revision_only_change_and_saturation_disable_stable_reuse() {
        let mut app = app();
        let uid = app.session.doc().unwrap().uid;
        app.prune_sheet_views();
        let before = builds();
        let old_roots = roots(&app.session, uid);
        app.session.doc_mut().unwrap().revision += 1;
        assert_eq!(roots(&app.session, uid), old_roots);
        app.prune_sheet_views();
        assert_eq!(builds(), before + 1);
        app.session.doc_mut().unwrap().revision = u64::MAX;
        for n in 2..=4 {
            app.prune_sheet_views();
            assert_eq!(builds(), before + n);
        }
    }

    #[test]
    fn group_redo_keeps_deleted_scroll_until_group_release() {
        for rollback in [false, true] {
            let mut app = app();
            app.run("home.insertSheet", json!({"name":"Transient"})).unwrap();
            let uid = app.session.doc().unwrap().uid;
            let key = (app.session.active_window().unwrap(), app.session.doc().unwrap().wb.active().unwrap().uid.get());
            app.view_mut().unwrap().scroll.y = 90.0;
            app.run("edit.undo", json!({})).unwrap();
            let group = app.session.group_begin("captured redo").unwrap();
            // Owning fixture isolates group redo as the sole root containing this sheet.
            app.session.doc_mut().unwrap().redo.clear();
            app.prune_sheet_views();
            assert!(app.views.contains_key(&key));
            let d = app.session.doc().unwrap();
            assert!(d.undo.iter().chain(&d.redo).all(|e| !e.wb.sheets.iter().any(|sh| sh.uid.get() == key.1)));
            assert!(!d.wb.sheets.iter().any(|sh| sh.uid.get() == key.1));
            assert!(!d.saved.sheets.iter().any(|sh| sh.uid.get() == key.1));
            let before = builds();
            if rollback {
                app.session.group_rollback(group).unwrap();
                app.prune_sheet_views();
                assert!(app.views.contains_key(&key), "rollback restores the retained redo");
                assert!(app.reachable_sheets[&uid].matches(&app.session, uid));
                app.run("view.zoom", json!({"percent":125})).unwrap();
            } else {
                app.session.group_end(group).unwrap();
                app.prune_sheet_views();
            }
            assert!(!app.views.contains_key(&key));
            assert!(builds() > before);
        }
    }

    #[test]
    fn view_zoom_releases_last_redo_sheet_root_without_membership_rebase() {
        let mut app = app();
        app.run("home.insertSheet", json!({"name":"Transient"})).unwrap();
        let uid = app.session.doc().unwrap().uid;
        let key = (app.session.active_window().unwrap(), app.session.doc().unwrap().wb.active().unwrap().uid.get());
        app.view_mut().unwrap().scroll.y = 90.0;
        app.run("edit.undo", json!({})).unwrap();
        app.prune_sheet_views();
        assert!(app.views.contains_key(&key));
        let d = app.session.doc().unwrap();
        assert!(!d.wb.sheets.iter().any(|sh| sh.uid.get() == key.1));
        assert!(!d.saved.sheets.iter().any(|sh| sh.uid.get() == key.1));
        assert!(d.undo.iter().all(|e| !e.wb.sheets.iter().any(|sh| sh.uid.get() == key.1)));
        assert!(d.redo.iter().any(|e| e.wb.sheets.iter().any(|sh| sh.uid.get() == key.1)));
        let before = builds();
        app.run("view.zoom", json!({"percent":125})).unwrap();
        assert!(app.session.doc().unwrap().redo.is_empty());
        assert!(!app.views.contains_key(&key));
        assert!(builds() > before);
        assert!(!app.reachable_sheets.get(&uid).is_some_and(|entry| entry.sheets.contains(&key.1)));
    }

    #[test]
    fn saved_only_root_expires_after_genuine_dirty_view_rebase_and_save() {
        let dir = owned_directory::Directory::acquire(
            (0..).map(|n| std::env::temp_dir().join(format!("gridcraft-view-rebase-{}-{n}", std::process::id()))),
        )
        .unwrap();
        let path = dir.join("view.json");
        let mut app = app();
        let uid = app.session.doc().unwrap().uid;
        let key = (app.session.active_window().unwrap(), app.session.doc().unwrap().wb.active().unwrap().uid.get());
        app.run("home.insertSheet", json!({"name":"Survivor"})).unwrap();
        app.run("file.save", json!({"path":path})).unwrap();
        app.run("home.deleteSheet", json!({"sheet":"Sheet1"})).unwrap();
        for n in 0..101 {
            app.run("cell.set", json!({"cell":"A1","input":n.to_string()})).unwrap();
        }
        app.view_mut().unwrap();
        app.prune_sheet_views();
        let d = app.session.doc().unwrap();
        assert!(d.saved.sheets.iter().any(|sh| sh.uid.get() == key.1));
        assert!(d.undo.iter().chain(&d.redo).all(|e| !e.wb.sheets.iter().any(|sh| sh.uid.get() == key.1)));
        let before = builds();
        app.view_mut().unwrap().scroll.y = 60.0;
        app.store_document_view_state(uid);
        assert!(app.reachable_sheets[&uid].matches(&app.session, uid));
        tick(&mut app);
        assert_eq!(builds(), before);
        assert!(app.views.contains_key(&key));
        let revision = app.session.doc().unwrap().revision;
        app.session.run("file.save", json!({"path":path})).unwrap();
        assert_eq!(app.session.doc().unwrap().revision, revision);
        assert!(!app.session.doc().unwrap().is_dirty());
        tick(&mut app);
        assert_eq!(builds(), before + 1);
        assert!(!app.views.contains_key(&key));
    }

    #[test]
    fn no_views_missing_document_and_close_drop_membership_records() {
        let mut app = app();
        let uid = app.session.doc().unwrap().uid;
        app.prune_sheet_views();
        assert!(app.reachable_sheets.contains_key(&uid));
        app.views.clear();
        app.prune_sheet_views();
        assert!(app.reachable_sheets.is_empty());
        app.view_mut().unwrap();
        app.prune_sheet_views();
        app.session.close_document(0);
        app.store_document_view_state(uid);
        assert!(app.reachable_sheets.is_empty());
        app.prune_sheet_views();
        assert!(app.views.is_empty());
        app.store_document_view_state(u64::MAX);
        assert!(app.reachable_sheets.is_empty());
        let empty = ReachableSheets { revision: 0, roots: vec![], sheets: HashSet::new() };
        assert!(!empty.matches(&app.session, uid));
    }
}
