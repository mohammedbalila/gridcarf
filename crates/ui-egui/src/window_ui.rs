//! The interface state each window keeps for itself.
//!
//! [`SheetApp`]'s per-window fields (the cell draft, dialog, Backstage view, grid gestures…) hold
//! the state of one window: the engine's active window. Every other window's state is parked in a
//! [`WindowUi`] and swapped in when that window is activated or drawn, the way the engine swaps a
//! window's selection and sheet into its workbook. So a draft, a dialog or a message stays in the
//! window it belongs to.

use gridcraft_engine::core::CellRef;

use crate::{Notice, SheetApp, backstage, dialogs, editor, grid, keytips, text_box};

#[cfg(test)]
mod cf_tests {
    use super::*;

    #[test]
    fn cf_statistics_view_rebase_matches_current_and_parked_document_revisions() {
        let mut session = gridcraft_engine::Session::new();
        session.new_workbook();
        let mut app = SheetApp::new(session, Default::default());
        let doc = app.session.doc().unwrap();
        let (uid, revision, sheet) = (doc.uid, doc.revision, doc.wb.active().unwrap().uid.get());
        app.grid.cf.key = Some((uid, revision, sheet));
        for (window, owner, rev) in [(100, uid, revision), (101, uid, revision.saturating_sub(1)), (102, uid + 1, revision)] {
            let mut parked = WindowUi::default();
            parked.grid.cf.key = Some((owner, rev, sheet));
            app.parked.insert(window, parked);
        }
        app.rebase_document_view_traces(uid, revision, revision + 1);
        assert_eq!(app.grid.cf.key, Some((uid, revision + 1, sheet)));
        assert_eq!(app.parked[&100].grid.cf.key, Some((uid, revision + 1, sheet)));
        assert_eq!(app.parked[&101].grid.cf.key, Some((uid, revision.saturating_sub(1), sheet)));
        assert_eq!(app.parked[&102].grid.cf.key, Some((uid + 1, revision, sheet)));
    }
}

/// One window's parked interface state (see the module docs).
#[derive(Default)]
pub struct WindowUi {
    editor: Option<editor::EditState>,
    text_box_editor: Option<text_box::EditState>,
    dialog: Option<dialogs::Dialog>,
    backstage: Option<backstage::Backstage>,
    message: Option<(String, String)>,
    toast: Option<(String, f64)>,
    notice: Option<Notice>,
    grid: grid::GridState,
    name_box: Option<String>,
    keytips: keytips::KeyTips,
    selected_object: Option<gridcraft_engine::model::ObjRef>,
    tab_run: Option<(u64, usize, u32, CellRef)>,
    open_switch_windows: bool,
    /// The ribbon tab this window shows. A contextual tab depends on this window's selection, so
    /// another window's frame must not reset it.
    ribbon_tab: String,
    /// The status bar's Ready/Enter/Edit, which follows the window's cell draft.
    mode: gridcraft_engine::Mode,
    /// A key tip's control request waiting for the frame that draws this window's ribbon.
    keytip_invoke_next: Option<&'static str>,
}

impl WindowUi {
    /// A new window's state. The last border, fill and font colour picked carry over, as they are
    /// the user's recent choices rather than the window's; so does the ribbon tab.
    fn fresh(like: &grid::GridState, ribbon_tab: &str) -> WindowUi {
        WindowUi {
            ribbon_tab: ribbon_tab.to_string(),
            grid: grid::GridState {
                last_border: like.last_border.clone(),
                last_fill: like.last_fill.clone(),
                last_font_color: like.last_font_color.clone(),
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

impl SheetApp {
    /// AutoSave leaves drafts in their own document, including parked native windows.
    pub(crate) fn document_has_draft(&self, uid: u64) -> bool {
        self.editor.as_ref().is_some_and(|e| e.doc == uid)
            || self.text_box_editor.as_ref().is_some_and(|e| e.document() == uid)
            || self
                .parked
                .values()
                .any(|w| w.editor.as_ref().is_some_and(|e| e.doc == uid) || w.text_box_editor.as_ref().is_some_and(|e| e.document() == uid))
    }

    /// Only a proven view transition is harmless to arrows and conditional statistics. Match
    /// document and prior revision so content-invalidated caches stay stale, including parked ones.
    pub(crate) fn rebase_document_view_traces(&mut self, uid: u64, before_revision: u64, after_revision: u64) {
        for grid in std::iter::once(&mut self.grid).chain(self.parked.values_mut().map(|w| &mut w.grid)) {
            grid.cf.rebase_view(uid, before_revision, after_revision);
            if let Some(trace) = grid.trace.as_mut()
                && trace.uid == uid
                && trace.revision == before_revision
            {
                trace.revision = after_revision;
            }
        }
    }

    /// Exchanges the per-window fields with `w`. Destructured in full, so a field added to
    /// [`WindowUi`] can't be left out of the swap.
    fn swap_window_ui(&mut self, w: &mut WindowUi) {
        let WindowUi {
            editor,
            text_box_editor,
            dialog,
            backstage,
            message,
            toast,
            notice,
            grid,
            name_box,
            keytips,
            selected_object,
            tab_run,
            open_switch_windows,
            ribbon_tab,
            mode,
            keytip_invoke_next,
        } = w;
        std::mem::swap(&mut self.editor, editor);
        std::mem::swap(&mut self.text_box_editor, text_box_editor);
        std::mem::swap(&mut self.dialog, dialog);
        std::mem::swap(&mut self.backstage, backstage);
        std::mem::swap(&mut self.message, message);
        std::mem::swap(&mut self.toast, toast);
        std::mem::swap(&mut self.notice, notice);
        std::mem::swap(&mut self.grid, grid);
        std::mem::swap(&mut self.name_box, name_box);
        std::mem::swap(&mut self.keytips, keytips);
        std::mem::swap(&mut self.selected_object, selected_object);
        std::mem::swap(&mut self.tab_run, tab_run);
        std::mem::swap(&mut self.open_switch_windows, open_switch_windows);
        std::mem::swap(&mut self.ui.ribbon_tab, ribbon_tab);
        std::mem::swap(&mut self.session.mode, mode);
        std::mem::swap(&mut self.ui.keytip_invoke_next, keytip_invoke_next);
    }

    /// The window whose interface state is in the per-window fields.
    pub fn ui_window(&self) -> Option<u64> {
        self.ui_window
    }

    /// Brings the per-window fields in line with the engine's active window, after a command (or
    /// a script) switched windows: the previous window's state is parked, the active one's swapped
    /// in (a window never seen before starts fresh). A closed window's state is dropped.
    pub fn follow_active_window(&mut self) {
        let active = self.session.active_window();
        // A parked window can close without changing the current window, including on a
        // logic-only tick. Its UI and scroll state must go before the no-transition return.
        let session = &self.session;
        self.parked.retain(|w, _| session.window(*w).is_some());
        self.prune_sheet_views();
        // A window hidden while parked (from another window's View › Hide) isn't drawn either:
        // its waiting key tip request goes, as it does for a window hidden as it is left.
        let session = &self.session;
        for (id, parked) in &mut self.parked {
            if session.window(*id).is_some_and(|w| w.hidden) {
                parked.keytip_invoke_next = None;
            }
        }
        if active == self.ui_window {
            return;
        }
        let Some(previous) = self.ui_window else {
            // The first window: the fields are already its state.
            self.ui_window = active;
            return;
        };
        let mut outgoing = WindowUi::fresh(&self.grid, &self.ui.ribbon_tab);
        self.swap_window_ui(&mut outgoing);
        if let Some(window) = self.session.window(previous) {
            // A hidden window isn't drawn: a key tip request waiting for its next frame would
            // fire whenever it is shown again, so it goes.
            if window.hidden {
                outgoing.keytip_invoke_next = None;
            }
            self.parked.insert(previous, outgoing);
        }
        if let Some(mut incoming) = active.and_then(|w| self.parked.remove(&w)) {
            self.swap_window_ui(&mut incoming);
        }
        self.ui_window = active;
    }

    /// Activates window `id` (it becomes the most recently active) with its interface state.
    pub fn enter_window(&mut self, id: u64) {
        self.session.activate_window(id);
        self.follow_active_window();
    }

    /// Makes window `id` and its interface state current while it is drawn, without changing
    /// which window counts as most recently active.
    pub fn show_window(&mut self, id: u64) {
        self.session.show_window(id);
        self.follow_active_window();
    }

    /// Puts back a dialog that was taken out of window `home`'s fields while it ran. A command
    /// run from the dialog may have switched windows meanwhile; the dialog stays in the window it
    /// was opened in (dropped if that window closed).
    pub(crate) fn return_dialog(&mut self, home: Option<u64>, d: dialogs::Dialog) {
        if home == self.ui_window {
            self.dialog = Some(d);
        } else if let Some(w) = home.and_then(|h| self.parked.get_mut(&h))
            && w.dialog.is_none()
        {
            w.dialog = Some(d);
        }
    }

    /// The window that shows the dialog `name`: the current one or a parked one. A window that has
    /// closed shows nothing, even while its parked state is still kept.
    pub(crate) fn window_with_dialog(&self, name: &str) -> Option<u64> {
        if self.dialog.as_ref().is_some_and(|d| d.name == name) {
            return self.ui_window;
        }
        self.parked
            .iter()
            .filter(|(id, w)| w.dialog.as_ref().is_some_and(|d| d.name == name) && self.session.window(**id).is_some())
            .map(|(id, _)| *id)
            .min()
    }

    /// Whether a window other than the current one has a cell or text-box draft open.
    pub fn parked_window_with_draft(&self) -> Option<u64> {
        self.parked.iter().filter(|(_, w)| w.editor.is_some() || w.text_box_editor.is_some()).map(|(id, _)| *id).min()
    }
}

#[cfg(test)]
mod closed_view_state_tests {
    use super::*;
    use serde_json::json;

    fn sentinel() -> (SheetApp, egui::Context, u64, u64) {
        let mut session = gridcraft_engine::Session::new();
        session.new_workbook();
        let mut app = SheetApp::new(session, Default::default());
        app.ui.autosave = false;
        app.use_prepared_fonts();
        app.follow_active_window();
        app.view_mut().unwrap().scroll.y = 40.0;
        let window = app.session.active_window().unwrap();
        let doc = app.session.doc().unwrap().uid;
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::pinned_font_definitions());
        crate::theme::apply(&ctx, false);
        (app, ctx, window, doc)
    }

    fn target_cycle(app: &mut SheetApp, ctx: &egui::Context, sentinel: u64, sentinel_doc: u64) -> u64 {
        app.run("file.new", json!({})).unwrap();
        let target = app.session.active_window().unwrap();
        let target_doc = app.session.doc().unwrap().uid;
        app.run("range.setValues", json!({"range":"A1", "values":[[1],[10]]})).unwrap();
        app.run("home.conditionalFormat", json!({"range":"A1:A2", "rule":{"type":"colorScale"}})).unwrap();
        let mut output = ctx.run_ui(
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0))), ..Default::default() },
            |ui| crate::grid::show(app, ui),
        );
        // These are the actual two endpoint fills from the statistics-backed color scale.
        for color in [egui::Color32::from_rgb(0xF8, 0x69, 0x6B), egui::Color32::from_rgb(0x63, 0xBE, 0x7B)] {
            assert!(
                output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == color)),
                "CF endpoint was not painted: {color:?}"
            );
        }
        output.textures_delta.clear();
        drop(output);
        assert_eq!(app.grid.cf.key.unwrap().0, target_doc);
        app.view_mut().unwrap().scroll.y = 100.0;
        app.open_dialog("zoom", json!({}));
        assert!(app.dialog.is_some());
        app.run("edit.copy", json!({"range":"A1:A2"})).unwrap();
        app.run("edit.formatPainter", json!({})).unwrap();
        assert!(app.session.clipboard.is_some() && app.session.format_painter.is_some());
        app.enter_window(sentinel);
        assert_eq!(app.parked[&target].grid.cf.key.unwrap().0, target_doc);
        assert!(app.parked[&target].dialog.is_some());
        app.run("edit.clearClipboard", json!({})).unwrap();
        assert!(app.session.clipboard.is_none() && app.session.format_painter.is_none());
        app.session.hide_window(target);
        assert_eq!(app.session.active_window(), Some(sentinel));
        assert_eq!(app.ui_window(), Some(sentinel));
        let closed = app.run("window.close", json!({"window":target, "force":true})).unwrap();
        assert_eq!(closed["closed"], true);
        assert!(app.session.window(target).is_none());
        assert!(app.session.document_index(target_doc).is_none());
        // The hidden/no-paint path must prune even though engine and UI remain current.
        for _ in 0..2 {
            let _ = ctx.run_logic(&egui::RawInput::default(), |ctx| app.logic(ctx));
            assert_eq!(app.session.active_window(), Some(sentinel));
            assert_eq!(app.ui_window(), Some(sentinel));
            assert_eq!(app.session.doc().unwrap().uid, sentinel_doc);
        }
        target
    }

    #[test]
    fn m03a_closed_parked_target_releases_state_without_active_window_transition() {
        let (mut app, ctx, window, doc) = sentinel();
        let target = target_cycle(&mut app, &ctx, window, doc);
        assert!(!app.parked.contains_key(&target), "closed target retains populated CF cache and dialog");
        assert!(app.views.keys().all(|(w, _)| *w == window), "closed target retains scroll state");
        assert_eq!(app.view().scroll.y, 40.0);
    }

    fn sample(app: &SheetApp, phase: &str, cycle: usize, sentinel: u64, sentinel_doc: u64) {
        let target_cf = app.parked.values().filter(|w| w.grid.cf.key.is_some_and(|k| k.0 != sentinel_doc)).count();
        // Capacity times the inline key/value size is only a lower-bound storage estimate:
        // it excludes hash control bytes, allocator overhead and all nested allocations.
        let parked_inline = app.parked.capacity() * std::mem::size_of::<(u64, WindowUi)>();
        let view_inline =
            app.views.capacity() * app.views.iter().next().map_or(0, |(key, value)| std::mem::size_of_val(key) + std::mem::size_of_val(value));
        println!(
            "M03A_SAMPLE phase={phase} cycle={cycle} docs={} windows={} parked_len={} parked_capacity={} views_len={} views_capacity={} target_cf_keys={} target_cache_instances={} parked_inline_capacity_bytes={} views_inline_capacity_bytes={} sentinel_window={} sentinel_doc={} ui_window={:?} engine_window={:?}",
            app.session.documents().len(),
            app.session.windows().len(),
            app.parked.len(),
            app.parked.capacity(),
            app.views.len(),
            app.views.capacity(),
            target_cf,
            target_cf,
            parked_inline,
            view_inline,
            sentinel,
            sentinel_doc,
            app.ui_window(),
            app.session.active_window()
        );
    }

    #[test]
    fn m03a_collects_all_1000_cycles_before_asserting_closed_state_is_released() {
        let (mut app, ctx, window, doc) = sentinel();
        sample(&app, "before_warmup", 0, window, doc);
        for _ in 0..16 {
            target_cycle(&mut app, &ctx, window, doc);
        }
        sample(&app, "measured", 0, window, doc);
        for cycle in 1..=1000 {
            target_cycle(&mut app, &ctx, window, doc);
            if [100, 250, 500, 750, 1000].contains(&cycle) {
                sample(&app, "measured", cycle, window, doc);
            }
        }
        println!("M03A_COLLECTION_COMPLETE warmup=16 measured=1000");
        assert_eq!(app.session.documents().len(), 1);
        assert_eq!(app.session.windows().len(), 1);
        assert_eq!((app.parked.len(), app.views.len()), (0, 1), "all closed target UI/scroll state must be released");
    }
}
