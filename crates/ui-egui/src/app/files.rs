use super::{Json, Notice, NoticeKind, OpenWindow, SheetApp, clean_error, json};

/// Session-local per-document scheduling and result; never confused with recovery protection.
#[derive(Default)]
pub(crate) struct AutoSaveState {
    path: Option<String>,
    last_attempt: Option<f64>,
    failure: Option<String>,
}

impl SheetApp {
    pub fn open_path(&mut self, path: &str) {
        match self.run("file.open", json!({"path": path})) {
            Ok(r) => self.after_open(Some(path), &r),
            // A password-protected workbook: ask for the password; the dialog opens it.
            Err(e) if e.contains(gridcraft_engine::io::PASSWORD_REQUIRED) => self.open_dialog("openPassword", json!({"path": path})),
            Err(e) => self.message = Some(("GridCraft".into(), clean_error(&e))),
        }
        self.after_engine();
    }

    /// What follows a successful `file.open`: the workbook shows (not the Backstage), the Recent
    /// list (for a file on disk) and the open warnings notice. Shared by File › Open, the
    /// password dialog and the control channel.
    pub fn after_open(&mut self, path: Option<&str>, r: &Json) {
        self.backstage = None;
        if let Some(path) = path {
            self.ui.recent.retain(|p| p != path);
            self.ui.recent.insert(0, path.to_string());
            self.ui.recent.truncate(20);
        }
        self.show_open_warnings(r);
    }

    /// Turns the `warnings` a `file.open` result carries into an info-bar notice, choosing the
    /// most important one (a preserved VBA project outranks, say, a skipped optional part).
    /// Kept deliberately generic: any open-time warning can reach the user this way.
    pub fn show_open_warnings(&mut self, r: &Json) {
        // The opened workbook's window shows them, however it was opened (a script may have run
        // `file.open` on the session directly).
        self.follow_active_window();
        let Some(list) = r.get("warnings").and_then(Json::as_array) else {
            self.notice = None;
            return;
        };
        let msgs: Vec<&str> = list.iter().filter_map(Json::as_str).collect();
        if msgs.is_empty() {
            // A clean open clears any notice left over from the previous document.
            self.notice = None;
            return;
        }
        let vba: Vec<&str> = msgs.iter().copied().filter(|m| m.to_lowercase().contains("macro") || m.contains("VBA")).collect();
        if !vba.is_empty() {
            // The reader says "dropped" when the VBA project was missing or unreadable; never promise to keep it then.
            let dropped = vba.iter().any(|m| m.contains("dropped"));
            let text = if dropped {
                "This workbook's VBA macros could not be loaded and were dropped, so they will not be in the saved file. Keep the original file."
            } else {
                "This workbook contains VBA macros. GridCraft keeps them when you save as .xlsm, but does not run them."
            };
            self.notice = Some(Notice { text: text.into(), kind: NoticeKind::Warning, action: Some(("What this means".into(), "macros".into())) });
        } else {
            let text = if msgs.len() == 1 {
                msgs[0].to_string()
            } else {
                format!("{} items in this file were handled differently. First: {}", msgs.len(), msgs[0])
            };
            self.notice = Some(Notice { text, kind: NoticeKind::Info, action: None });
        }
    }

    /// Only menu invocations ask for a loss decision. Script calls keep the structured error.
    pub(crate) fn save_or_explain(&mut self, id: &str, params: Json) -> bool {
        match self.run(id, params.clone()) {
            Ok(result) => {
                if id == "file.saveBytes" {
                    self.deliver_saved_bytes(&result);
                }
                true
            }
            Err(error) => match serde_json::from_str::<gridcraft_engine::save::SaveFailure>(&error) {
                Ok(failure) => {
                    let analysis = failure.analysis;
                    let mut confirmed = if params.is_object() { params } else { json!({}) };
                    // Bind both identity and revision: two open workbooks may have the same revision.
                    confirmed["expectedDocument"] = json!(analysis.document);
                    confirmed["expectedRevision"] = json!(analysis.revision);
                    confirmed["confirmedLosses"] = json!(analysis.issues.iter().map(|i| &i.code).collect::<Vec<_>>());
                    if id != "file.saveBytes" {
                        confirmed["path"] = json!(analysis.path);
                    }
                    self.open_dialog("saveLosses", json!({"command": id, "params": confirmed, "analysis": analysis}));
                    true
                }
                Err(_) => {
                    self.alert(error);
                    false
                }
            },
        }
    }

    pub(crate) fn deliver_saved_bytes(&self, result: &Json) {
        if let Some(download) = &self.services.download
            && let Some(bytes) = result.get("base64").and_then(Json::as_str).and_then(gridcraft_engine::io::base64_decode)
        {
            download(result.get("name").and_then(Json::as_str).unwrap_or("Book.xlsx"), &bytes);
        }
    }

    /// The open windows in order, for the Switch Windows lists.
    pub fn open_windows(&self) -> Vec<OpenWindow> {
        let active = self.session.active_window();
        self.session
            .windows()
            .iter()
            .filter_map(|w| {
                let d = self.session.documents().iter().find(|d| d.uid == w.doc_uid)?;
                let title = self.session.window_title(w.id)?;
                Some(OpenWindow { window: w.id, doc: d.uid, title, dirty: d.is_dirty(), active: active == Some(w.id) })
            })
            .collect()
    }

    /// Opens files that arrived asynchronously (web file picker, dropped files).
    pub(super) fn poll_async_opens(&mut self) {
        let arrived: Vec<(String, Vec<u8>)> = self
            .services
            .inbox
            .as_ref()
            .map(|i| std::mem::take(&mut *i.lock().unwrap_or_else(std::sync::PoisonError::into_inner)))
            .unwrap_or_default();
        for (name, bytes) in arrived {
            let b64 = gridcraft_engine::io::base64_encode(&bytes);
            match self.run("file.open", json!({"name": &name, "base64": &b64})) {
                // A file picked from File › Open (web) arrives here: show it, not the Backstage.
                Ok(r) => {
                    self.backstage = None;
                    self.show_open_warnings(&r);
                }
                Err(e) if e.contains(gridcraft_engine::io::PASSWORD_REQUIRED) => {
                    self.backstage = None;
                    self.open_dialog("openPassword", json!({"name": name, "base64": b64}));
                }
                Err(e) => self.message = Some(("GridCraft".into(), clean_error(&e))),
            }
            self.after_engine();
        }
    }

    /// Save every eligible dirty document synchronously, at most once per two seconds per
    /// document. A failed write keeps its warning until a successful save, close or clean undo.
    pub(super) fn autosave(&mut self, ctx: &egui::Context) {
        self.autosave_states.retain(|uid, _| self.session.documents().iter().any(|d| d.uid == *uid));
        let docs: Vec<_> = self.session.documents().iter().map(|d| (d.uid, d.path.clone(), d.is_dirty())).collect();
        let mut pending = false;
        for (uid, path, dirty) in docs {
            let state = self.autosave_states.entry(uid).or_default();
            if state.path != path {
                *state = AutoSaveState { path: path.clone(), ..Default::default() };
            }
            if !dirty {
                state.failure = None;
                continue;
            }
            if !self.ui.autosave {
                continue;
            }
            let Some(path) = path else {
                state.failure = Some("no save destination. Recovery is not available yet; choose Save As.".into());
                continue;
            };
            if gridcraft_engine::io::FileKind::from_path(&path) != Some(gridcraft_engine::io::FileKind::Xlsx) {
                continue;
            }
            pending = true;
            let now = crate::monotonic_ms();
            if state.last_attempt.is_some_and(|last| now - last < 2000.0) || self.document_has_draft(uid) {
                continue;
            }
            if self.session.active().is_some_and(|d| d.uid == uid) {
                self.store_view_state();
            } else {
                self.store_document_view_state(uid);
            }
            let result = self.session.autosave_document(uid);
            let failure = result.err().map(|error| {
                if matches!(error, gridcraft_engine::EngineError::SaveBlocked(_)) {
                    "saving needs your decision about content loss or compatibility. Choose Save to review it.".into()
                } else {
                    clean_error(&error.to_string())
                }
            });
            let state = self.autosave_states.entry(uid).or_default();
            state.last_attempt = Some(crate::monotonic_ms());
            state.failure = failure;
        }
        if pending {
            // Hidden logic ticks also retry failures; there is never a retry for every paint.
            ctx.request_repaint_after(std::time::Duration::from_secs(2));
        }
    }

    /// Prefer the visible document's result, then the first other failed document in open order.
    /// This warning is independent of dismissible open notices and survives window switches.
    pub(crate) fn autosave_notice(&self) -> Option<Notice> {
        let active = self.session.active().map(|d| d.uid);
        let failed = |d: &&gridcraft_engine::DocState| self.autosave_states.get(&d.uid).is_some_and(|s| s.failure.is_some());
        let d = self
            .session
            .documents()
            .iter()
            .filter(|d| Some(d.uid) == active)
            .find(failed)
            .or_else(|| self.session.documents().iter().find(failed))?;
        let failure = self.autosave_states.get(&d.uid)?.failure.as_ref()?;
        Some(Notice { text: format!("AutoSave paused for {}: {failure}", d.display_title()), kind: NoticeKind::Warning, action: None })
    }

    /// Flip AutoSave (title-bar switch / Options ▸ Save). Persisted with the other UI
    /// preferences, so it stays as the user left it across restarts (#1901).
    pub fn toggle_autosave(&mut self) {
        self.ui.autosave = !self.ui.autosave;
    }
}
