//! Windows on open workbooks.
//!
//! Every open workbook has one or more windows (View › New Window adds another on the same
//! workbook). A window has its own selection, active sheet and per-sheet selections. The active
//! window's view lives in its [`DocState`] (`selection`, `sheet_selections`) and in the workbook's
//! active sheet ([`gridcraft_model::workbook::Workbook::active_sheet`]), where every command reads
//! it; each other window keeps its view in [`WindowState`] until it is activated. Swapping a view
//! in or out never copies the workbook or makes it dirty: the active sheet is set in place through
//! the shared `Arc`, and selections live outside the workbook.

use serde_json::{Value as Json, json};

use crate::selection_memory::{PreparedSelectionMemory, SelectionMemory, Transition};
use crate::{DocState, EngineError, Result, Selection, Session};

/// The view state one window keeps while another window on its workbook is showing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WindowView {
    pub selection: Selection,
    pub sheet: usize,
    /// Legacy name field retained for source compatibility; internal views use runtime UIDs.
    pub sheet_name: String,
    pub sheet_selections: Vec<Selection>,
}

/// One window on an open workbook.
#[derive(Clone, Debug)]
pub struct WindowState {
    /// Identifies the window for the rest of the session; never reused.
    pub id: u64,
    /// The [`DocState::uid`] of the workbook it shows.
    pub doc_uid: u64,
    /// View › Hide.
    pub hidden: bool,
    /// `None` while this window's view is the one in the document (the active window, or the
    /// workbook's most recently active window); exactly one window per workbook has `None`.
    view: Option<PairedView>,
    /// Activation order: higher was active more recently.
    last_active: u64,
}

/// Private snapshots keep payload, ownership and captured live identity together.
#[derive(Clone, Debug)]
pub(crate) struct PairedView {
    view: WindowView,
    memory: SelectionMemory,
    captured: Option<u64>,
    deferred: bool,
}
impl PairedView {
    pub(crate) fn capture(d: &DocState) -> Result<Self> {
        let plan = d.prepare_selection(&d.wb, Transition::Refresh)?;
        let mut pair = Self {
            view: WindowView {
                selection: d.selection.clone(),
                sheet: d.wb.active_sheet(),
                sheet_name: String::new(),
                sheet_selections: d.sheet_selections.clone(),
            },
            memory: SelectionMemory::default(),
            captured: None,
            deferred: false,
        };
        plan.apply(&mut pair.view.sheet_selections, &mut pair.memory, &mut pair.view.selection);
        pair.captured = pair.memory.active_uid();
        Ok(pair)
    }
    /// Exact existing snapshot clone after refresh, or mandatory post-publication deferral.
    pub(crate) fn snapshot(d: &DocState) -> Self {
        let captured = d.wb.active().map(|sh| sh.uid.get()).filter(|uid| d.wb.sheets.iter().filter(|sh| sh.uid.get() == *uid).take(2).count() == 1);
        Self {
            view: WindowView {
                selection: d.selection.clone(),
                sheet: d.wb.active_sheet(),
                sheet_name: String::new(),
                sheet_selections: d.sheet_selections.clone(),
            },
            memory: d.selection_memory.clone(),
            captured,
            deferred: d.selection_refresh_deferred,
        }
    }
    pub(crate) fn prepare(&self, d: &DocState) -> Result<(usize, PreparedSelectionMemory)> {
        let sheet = self
            .captured
            .and_then(|uid| d.wb.sheets.iter().position(|sh| sh.uid.get() == uid))
            .unwrap_or_else(|| d.wb.active().map_or(0, |_| d.wb.active_sheet()));
        Ok((
            sheet,
            self.memory.prepare_view(
                &self.view.sheet_selections,
                d,
                &self.view.selection,
                Transition::Restore { sheet, captured: self.captured },
                None,
            )?,
        ))
    }
    pub(crate) fn apply(mut self, d: &mut DocState, (sheet, plan): (usize, PreparedSelectionMemory)) {
        plan.apply(&mut self.view.sheet_selections, &mut self.memory, &mut self.view.selection);
        // Ambiguous UIDs retain the old positional view rule, without claiming identity.
        let sheet = if self.memory.active_uid().is_some() { sheet } else { self.view.sheet.min(d.wb.sheets.len().saturating_sub(1)) };
        d.wb.set_active_sheet(sheet);
        self.install(d);
        d.selection_refresh_deferred = false;
    }
    /// Saved calls this only after rollback has restored the original workbook topology.
    pub(crate) fn restore(self, d: &mut DocState) {
        let sheet = self
            .captured
            .and_then(|uid| d.wb.sheets.iter().position(|sh| sh.uid.get() == uid))
            .unwrap_or_else(|| self.view.sheet.min(d.wb.sheets.len().saturating_sub(1)));
        d.wb.set_active_sheet(sheet);
        self.install(d);
    }
    pub(crate) fn install(self, d: &mut DocState) {
        d.selection = self.view.selection;
        d.sheet_selections = self.view.sheet_selections;
        d.selection_memory = self.memory;
        d.selection_refresh_deferred = self.deferred;
    }
}

struct PreparedWindow {
    id: u64,
    doc: u64,
    plan: (usize, PreparedSelectionMemory),
    outgoing: Option<PairedView>,
}

/// Which window a command from a script or agent acts on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Target {
    /// The active window.
    #[default]
    Active,
    /// A window by [`WindowState::id`].
    Window(u64),
    /// A workbook by [`DocState::uid`]: its most recently active window.
    Doc(u64),
}

impl Target {
    /// Reads `window` or `doc` from a JSON object (control channel envelopes, MCP arguments).
    /// Neither present is [`Target::Active`]; a value that isn't an id is an error.
    pub fn from_json(p: &Json) -> Result<Target> {
        let id = |k: &str| -> Result<Option<u64>> {
            match p.get(k) {
                None | Some(Json::Null) => Ok(None),
                Some(v) => v.as_u64().map(Some).ok_or_else(|| EngineError::Other(format!("`{k}` must be a window or workbook id number"))),
            }
        };
        Ok(match (id("window")?, id("doc")?) {
            (Some(w), _) => Target::Window(w),
            (None, Some(d)) => Target::Doc(d),
            (None, None) => Target::Active,
        })
    }
}

/// What [`Session::leave_target`] restores.
#[derive(Debug)]
#[must_use]
pub struct TargetGuard {
    id: u64,
    back: Option<u64>,
}

impl Session {
    /// The open windows, in the order they were opened.
    pub fn windows(&self) -> &[WindowState] {
        &self.windows
    }

    /// The active window's id (`None` with no workbook open).
    pub fn active_window(&self) -> Option<u64> {
        self.active_window
    }

    pub fn window(&self, id: u64) -> Option<&WindowState> {
        self.windows.iter().find(|w| w.id == id)
    }

    /// A workbook's windows in opening order.
    fn windows_of(&self, doc: u64) -> impl Iterator<Item = &WindowState> {
        self.windows.iter().filter(move |w| w.doc_uid == doc)
    }

    /// The window title: the workbook's name when it has one window, "Name:1", "Name:2"… in
    /// opening order when it has several.
    pub fn window_title(&self, id: u64) -> Option<String> {
        let w = self.window(id)?;
        let d = self.docs.iter().find(|d| d.uid == w.doc_uid)?;
        let name = d.display_title();
        let mut n = 0;
        let mut at = 0;
        for x in self.windows_of(w.doc_uid) {
            n += 1;
            if x.id == id {
                at = n;
            }
        }
        Some(if n > 1 { format!("{name}:{at}") } else { name })
    }

    /// The most recently active window of a workbook, preferring one that isn't hidden.
    pub fn doc_window(&self, doc: u64) -> Option<u64> {
        self.windows_of(doc).max_by_key(|w| (!w.hidden, w.last_active)).map(|w| w.id)
    }

    /// The window whose view currently lives in the document, which can differ from the
    /// most recently activated or visible window after a host draws another window.
    pub fn document_view_window(&self, doc: u64) -> Option<u64> {
        self.windows_of(doc).find(|w| w.view.is_none()).map(|w| w.id)
    }

    /// Resolves a [`Target`] to a window id.
    pub fn target_window(&self, t: Target) -> Result<u64> {
        match t {
            Target::Active => self.active_window.ok_or(EngineError::NoDocument),
            Target::Window(id) => self.window(id).map(|w| w.id).ok_or_else(|| EngineError::Other(format!("no open window has id {id}"))),
            Target::Doc(uid) => self.doc_window(uid).ok_or_else(|| EngineError::Other(format!("no open workbook has id {uid}"))),
        }
    }

    pub(crate) fn push_window(&mut self, doc_uid: u64) -> u64 {
        self.next_window = self.next_window.saturating_add(1);
        let id = self.next_window;
        self.windows.push(WindowState { id, doc_uid, hidden: false, view: None, last_active: 0 });
        id
    }

    /// Makes a window active, swapping its view into its workbook. Unknown ids are ignored.
    pub fn activate_window(&mut self, id: u64) {
        self.enter_window(id, true);
    }

    /// Makes a window the one commands act on while the interface draws it, without counting as
    /// an activation: the most recently active window of each workbook stays the same.
    pub fn show_window(&mut self, id: u64) {
        self.enter_window(id, false);
    }

    fn enter_window(&mut self, id: u64, recent: bool) {
        if self.try_enter_window(id, recent).is_err() {
            log::warn!("Window view preparation failed.");
        }
    }

    pub(crate) fn try_set_active(&mut self, i: usize) -> Result<()> {
        self.try_set_active_view(i, None)
    }

    /// Prepare both independent owners before selecting a window or restoring a debugger pair.
    pub(crate) fn try_set_active_view(&mut self, i: usize, view: Option<PairedView>) -> Result<()> {
        let d = self.docs.get(i).ok_or(EngineError::NoDocument)?;
        let current = self.active == i && self.active_window.and_then(|w| self.window(w)).is_some_and(|w| w.doc_uid == d.uid);
        let window = if current {
            None
        } else {
            let id = self.doc_window(d.uid).ok_or(EngineError::NoDocument)?;
            Some(self.prepare_window(id)?.ok_or(EngineError::NoDocument)?)
        };
        let restore = view.as_ref().map(|view| view.prepare(d)).transpose()?;
        let switched = window.is_some();
        if let Some(window) = window {
            let id = window.id;
            self.apply_window(window);
            self.select_window(id, true);
        }
        if let (Some(view), Some(restore)) = (view, restore) {
            view.apply(&mut self.docs[i], restore);
        }
        if switched {
            self.cleanup_selection_memories();
        }
        Ok(())
    }

    fn prepare_window(&self, id: u64) -> Result<Option<PreparedWindow>> {
        let Some(w) = self.window(id) else { return Ok(None) };
        let d = self.docs.iter().find(|d| d.uid == w.doc_uid).ok_or(EngineError::NoDocument)?;
        let (plan, outgoing) = match &w.view {
            Some(view) => (view.prepare(d)?, Some(PairedView::capture(d)?)),
            None => ((d.wb.active_sheet(), d.prepare_selection(&d.wb, Transition::Refresh)?), None),
        };
        Ok(Some(PreparedWindow { id, doc: w.doc_uid, plan, outgoing }))
    }

    fn apply_window(&mut self, prepared: PreparedWindow) {
        let Some(di) = self.document_index(prepared.doc) else { return };
        let incoming = self.windows.iter_mut().find(|w| w.id == prepared.id).and_then(|w| w.view.take());
        if let Some(outgoing) = prepared.outgoing
            && let Some(w) = self.windows.iter_mut().find(|w| w.doc_uid == prepared.doc && w.id != prepared.id && w.view.is_none())
        {
            w.view = Some(outgoing);
        }
        let d = &mut self.docs[di];
        match incoming {
            Some(view) => view.apply(d, prepared.plan),
            None => {
                prepared.plan.1.apply(&mut d.sheet_selections, &mut d.selection_memory, &mut d.selection);
                d.selection_refresh_deferred = false;
            }
        }
    }

    fn select_window(&mut self, id: u64, recent: bool) {
        if let Some(w) = self.windows.iter_mut().find(|w| w.id == id) {
            if recent {
                self.activation = self.activation.saturating_add(1);
                w.last_active = self.activation;
            }
            self.active_window = Some(id);
            let doc = w.doc_uid;
            if let Some(di) = self.document_index(doc) {
                self.active = di;
            }
        }
    }

    pub(crate) fn try_enter_window(&mut self, id: u64, recent: bool) -> Result<()> {
        let Some(prepared) = self.prepare_window(id)? else { return Ok(()) };
        self.apply_window(prepared);
        self.select_window(id, recent);
        self.cleanup_selection_memories();
        Ok(())
    }

    /// Closing data is mandatory even when preparing the preferred view fails.
    pub(crate) fn prepare_document_close(&mut self, doc: u64) {
        if self.active_window.and_then(|id| self.window(id)).is_none_or(|w| w.doc_uid != doc) {
            return;
        }
        let next = self.windows.iter().filter(|w| w.doc_uid != doc).max_by_key(|w| (!w.hidden, w.last_active)).map(|w| w.id);
        if let Some(next) = next {
            match self.prepare_window(next) {
                Ok(Some(plan)) => {
                    self.apply_window(plan);
                    self.select_window(next, true);
                    return;
                }
                Ok(None) => {}
                Err(_) => {
                    let preferred = self.window(next).map(|w| w.doc_uid);
                    let live = self
                        .windows
                        .iter()
                        .filter(|w| w.doc_uid != doc && w.view.is_none())
                        .max_by_key(|w| (Some(w.doc_uid) == preferred, !w.hidden, w.last_active))
                        .map(|w| w.id);
                    for d in self.docs.iter_mut().filter(|d| d.uid != doc) {
                        d.selection_refresh_deferred = true;
                    }
                    for view in self.windows.iter_mut().filter(|w| w.doc_uid != doc).filter_map(|w| w.view.as_mut()) {
                        view.deferred = true;
                    }
                    log::warn!("Window view preparation failed.");
                    if let Some(id) = live {
                        self.select_window(id, true);
                        return;
                    }
                }
            }
        }
        self.active_window = None;
        self.active = 0;
    }

    pub(crate) fn prune_window_selections(&mut self, doc: u64, keep: &[u64], failed: bool) {
        let Some(d) = self.docs.iter().find(|d| d.uid == doc) else { return };
        for view in self.windows.iter_mut().filter(|w| w.doc_uid == doc).filter_map(|w| w.view.as_mut()) {
            let plan = if failed {
                Err(EngineError::Other("selection memory preparation failed".into()))
            } else {
                view.memory.prepare_view(&view.view.sheet_selections, d, &view.view.selection, Transition::Refresh, Some(keep))
            };
            match plan {
                Ok(plan) => {
                    // GC remaps only remembered slots. A parked live UID may return through Undo.
                    plan.apply(&mut view.view.sheet_selections, &mut view.memory, &mut view.view.selection);
                    view.deferred = false;
                }
                Err(_) => {
                    view.deferred = true;
                    log::warn!("Selection memory refresh deferred.");
                }
            }
        }
    }

    /// View › New Window: another window on the active window's workbook, showing what it shows.
    pub fn new_window(&mut self) -> Result<u64> {
        let current = self.active_window.ok_or(EngineError::NoDocument)?;
        let doc = self.window(current).map(|w| w.doc_uid).ok_or(EngineError::NoDocument)?;
        let d = self.doc()?;
        let refresh = d.prepare_selection(&d.wb, Transition::Refresh)?;
        let view = PairedView::capture(d)?;
        self.windows.try_reserve(1).map_err(|_| EngineError::Other("selection memory preparation failed".into()))?;
        let d = self.doc_mut()?;
        refresh.apply(&mut d.sheet_selections, &mut d.selection_memory, &mut d.selection);
        d.selection_refresh_deferred = false;
        // The new window takes over the live view (identical at this point); the old one keeps a copy.
        if let Some(w) = self.windows.iter_mut().find(|w| w.doc_uid == doc && w.view.is_none()) {
            w.view = Some(view);
        }
        let id = self.push_window(doc);
        self.select_window(id, true);
        self.cleanup_selection_memories();
        Ok(id)
    }

    /// Closes one window. The workbook stays open while it has another window; closing its last
    /// window closes the workbook (unsaved changes are the caller's to ask about).
    pub fn close_window(&mut self, id: u64) {
        if self.try_close_window(id).is_err() {
            log::warn!("Window view preparation failed.");
        }
    }

    pub(crate) fn try_close_window(&mut self, id: u64) -> Result<()> {
        let Some(w) = self.window(id) else { return Ok(()) };
        let doc = w.doc_uid;
        if self.windows_of(doc).count() <= 1 {
            if let Some(di) = self.document_index(doc) {
                self.close_document(di);
            }
            return Ok(());
        }
        let replacement = w.view.is_none().then(|| self.windows_of(doc).filter(|w| w.id != id).max_by_key(|w| w.last_active).map(|w| w.id)).flatten();
        let next = (self.active_window == Some(id))
            .then(|| self.windows.iter().filter(|w| w.id != id).max_by_key(|w| (!w.hidden, w.last_active)).map(|w| w.id))
            .flatten();
        // When the global destination is on this workbook, it is also its live replacement.
        let replacement =
            replacement.map(|replacement| next.filter(|next| self.window(*next).is_some_and(|w| w.doc_uid == doc)).unwrap_or(replacement));
        let first = replacement.map(|id| self.prepare_window(id)).transpose()?.flatten();
        let second = next.filter(|id| Some(*id) != replacement).map(|id| self.prepare_window(id)).transpose()?.flatten();
        if let Some(plan) = first {
            self.apply_window(plan);
        }
        if let Some(plan) = second {
            self.apply_window(plan);
        }
        self.windows.retain(|w| w.id != id);
        if let Some(next) = next {
            self.select_window(next, true);
        }
        self.cleanup_selection_memories();
        Ok(())
    }

    /// View › Hide on a window; the most recent visible window comes forward.
    pub fn hide_window(&mut self, id: u64) {
        if self.try_hide_window(id).is_err() {
            log::warn!("Window view preparation failed.");
        }
    }
    pub(crate) fn try_hide_window(&mut self, id: u64) -> Result<()> {
        if self.window(id).is_none() {
            return Ok(());
        }
        let next = (self.active_window == Some(id))
            .then(|| self.windows.iter().filter(|w| w.id != id && !w.hidden).max_by_key(|w| w.last_active).map(|w| w.id))
            .flatten();
        let prepared = next.map(|id| self.prepare_window(id)).transpose()?.flatten();
        if let Some(plan) = prepared {
            self.apply_window(plan);
        }
        if let Some(w) = self.windows.iter_mut().find(|w| w.id == id) {
            w.hidden = true;
        }
        if let Some(next) = next {
            self.select_window(next, true);
        }
        self.cleanup_selection_memories();
        Ok(())
    }

    /// View › Unhide: shows a hidden window and activates it.
    pub fn unhide_window(&mut self, id: u64) {
        if self.try_unhide_window(id).is_err() {
            log::warn!("Window view preparation failed.");
        }
    }
    pub(crate) fn try_unhide_window(&mut self, id: u64) -> Result<()> {
        let Some(plan) = self.prepare_window(id)? else { return Ok(()) };
        self.apply_window(plan);
        if let Some(w) = self.windows.iter_mut().find(|w| w.id == id) {
            w.hidden = false;
        }
        self.select_window(id, true);
        self.cleanup_selection_memories();
        Ok(())
    }

    /// The most recently hidden window.
    pub fn last_hidden_window(&self) -> Option<u64> {
        self.windows.iter().filter(|w| w.hidden).max_by_key(|w| w.last_active).map(|w| w.id)
    }

    /// The windows as scripts and the Switch Windows lists see them:
    /// `[{window, doc, index, title, dirty, active, hidden}]`, `index` being the workbook's
    /// position in [`Session::documents`].
    pub fn window_list(&self) -> Json {
        Json::Array(
            self.windows
                .iter()
                .filter_map(|w| {
                    let index = self.document_index(w.doc_uid)?;
                    let d = self.docs.get(index)?;
                    Some(json!({
                        "window": w.id,
                        "doc": w.doc_uid,
                        "index": index,
                        "title": self.window_title(w.id).unwrap_or_default(),
                        "dirty": d.is_dirty(),
                        "active": self.active_window == Some(w.id),
                        "hidden": w.hidden,
                    }))
                })
                .collect(),
        )
    }

    /// Runs `f` with the target window active, then brings back the window that was active, so a
    /// script acting on another window doesn't change what the user is looking at. When `f`
    /// itself switched windows (`window.activate`, `file.new`, a close), its choice stands.
    pub fn with_target<T>(&mut self, t: Target, f: impl FnOnce(&mut Session) -> T) -> Result<T> {
        let guard = self.enter_target(t)?;
        let r = f(self);
        self.leave_target(guard);
        Ok(r)
    }

    /// The first half of [`Session::with_target`], for callers whose closure can't borrow the
    /// session alone (the UI runs commands through its app): activates the target window and
    /// returns what [`Session::leave_target`] needs to bring the previous one back.
    pub fn enter_target(&mut self, t: Target) -> Result<Option<TargetGuard>> {
        if t == Target::Active {
            return Ok(None);
        }
        let id = self.target_window(t)?;
        let back = self.active_window;
        self.try_enter_window(id, true)?;
        Ok(Some(TargetGuard { id, back }))
    }

    pub fn leave_target(&mut self, guard: Option<TargetGuard>) {
        let Some(TargetGuard { id, back: Some(back) }) = guard else { return };
        if self.active_window == Some(id) && back != id && self.window(back).is_some() {
            self.activate_window(back);
        }
    }

    /// [`Session::execute`] on a target window.
    pub fn execute_in(&mut self, t: Target, id: &str, params: Json) -> Result<Json> {
        self.with_target(t, |s| s.execute(id, params))?
    }

    /// [`Session::execute_headless`] on a target window.
    pub fn execute_headless_in(&mut self, t: Target, id: &str, params: Json) -> Result<Json> {
        self.with_target(t, |s| s.execute_headless(id, params))?
    }
}

/// Commands that take `window`/`doc` as their own parameters rather than as a target.
pub fn is_window_command(id: &str) -> bool {
    id.starts_with("window.") || matches!(id, "view.newWindow" | "view.hideWindow" | "view.unhideWindow" | "view.switchWindows")
}

#[cfg(test)]
mod selection_tests;
