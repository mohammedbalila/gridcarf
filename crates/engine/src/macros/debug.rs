//! The macro debugger: a run that stops between steps.
//!
//! `macro.debug` starts a run that pauses before its first step, then steps, continues to the next
//! breakpoint, runs to a chosen step, or stops. The run lives on the session between calls (one
//! action per call) inside an undo group that spans them; Stop keeps the changes as one undo step
//! or rolls them back. While a macro is being debugged its workbook is locked: only the
//! debugger, queries and the commands in [`LOOKS_AROUND`] run there, and everything else, saving
//! included, is refused ("Finish or stop the macro first"). Other workbooks stay editable. The
//! debugger runs on its own, never as a step of a script or macro, and is never recorded.

use std::collections::BTreeSet;
use std::sync::Arc;

use gridcraft_model::Workbook;
use serde_json::{Value as Json, json};

use super::run::{Machine, StepError, rolled_back};
use crate::{EngineError, GroupToken, Result, Session};

/// The most breakpoints one call sets.
pub const MAX_BREAKPOINTS: usize = 10_000;
/// The most entries `changed` lists (ranges and sheet notes), as the footprint lists ranges.
const MAX_LISTED: usize = 500;

/// Commands that run on the debugged workbook between steps, besides `macro.debug` and queries.
/// They only look around (the selection, the active sheet, Go To, Find, zoom, copying), switch to
/// another window, close the workbook (which ends the run) or stop the run (Stop Action). The
/// debugger puts the run's active sheet and selection back before its next step runs.
pub const LOOKS_AROUND: &[&str] = &[
    "selection.set",
    "selection.move",
    "selection.next",
    "selection.row",
    "selection.column",
    "selection.currentRegion",
    "edit.selectAll",
    "sheet.activate",
    "sheet.next",
    "sheet.previous",
    "edit.goTo",
    "edit.goToSpecial",
    "edit.find",
    "edit.copy",
    "view.zoom",
    "view.zoom100",
    "view.zoomToSelection",
    "window.activate",
    "file.close",
    "automate.stopAction",
];

/// Whether command `id` (a query when `query`) runs on a workbook the debugger locks.
pub fn runs_while_locked(id: &str, query: bool) -> bool {
    // `macro.check` is a query, but its dry run runs the macro's steps, which the lock refuses: it
    // waits too, rather than report the lock as the failure of a step.
    (query && id != "macro.check") || id == "macro.debug" || LOOKS_AROUND.contains(&id)
}

/// A macro being debugged.
pub struct Debugger {
    pub name: String,
    pub doc: u64,
    /// Out of the session only while a step runs.
    machine: Option<Machine>,
    token: Option<GroupToken>,
    /// Breakpoints set for this run (step paths), on top of the steps' own.
    pub breakpoints: BTreeSet<String>,
    /// Only `breakpoints` stop the run; the steps' own flags are ignored (an editor that shows
    /// its own breakpoints sends them all, so clearing one takes effect at once).
    pub only_listed: bool,
    /// The workbook when the run started, for "changed so far".
    start: Arc<Workbook>,
    /// Every step path in the macro, for checking `runTo` targets and breakpoints.
    paths: BTreeSet<String>,
    /// The workbooks the run has touched (by uid). Steps run on the macro's own workbook and the
    /// policy refuses window and file commands, so this is the macro's workbook unless a command
    /// reaches another one some other way; edits of these wait until the run ends.
    touched: BTreeSet<u64>,
    /// Steps the footprint's dry runs executed before the run (they count towards the run's
    /// budget, not towards `executed`).
    base: u64,
    /// The conflict rule that shifted the data at the start (`shiftDown`, `shiftRight`).
    shifted: Option<&'static str>,
    /// Where the run left its workbook after the last action.
    view: Option<View>,
}

type View = crate::windows::PairedView;

impl Debugger {
    /// Whether edits of the workbook with this uid wait for the run to end.
    pub fn locks(&self, doc: u64) -> bool {
        self.touched.contains(&doc)
    }

    /// Whether one of the run's steps is running (its machine is out of the session).
    pub fn stepping(&self) -> bool {
        self.machine.is_none()
    }

    /// The run's own steps so far.
    fn executed(&self) -> u64 {
        self.machine.as_ref().map_or(0, |m| m.executed.saturating_sub(self.base))
    }
}

/// Each open workbook's uid and its current version, to see which ones a step changes.
fn versions(s: &Session) -> Vec<(u64, Arc<Workbook>)> {
    s.docs.iter().map(|d| (d.uid, d.wb.clone())).collect()
}

/// Adds to the run's touched log the workbooks that changed since `before`.
fn note_touched(s: &mut Session, before: &[(u64, Arc<Workbook>)]) {
    let changed: Vec<u64> =
        s.docs.iter().filter(|d| before.iter().any(|(uid, wb)| *uid == d.uid && !Arc::ptr_eq(wb, &d.wb))).map(|d| d.uid).collect();
    if let Some(dbg) = s.macro_debug.as_mut() {
        dbg.touched.extend(changed);
    }
}

/// Remembers where the run left its workbook: the active sheet and the selections.
fn remember_view(s: &mut Session) {
    let Some(uid) = s.macro_debug.as_ref().map(|d| d.doc) else { return };
    let view = s.docs.iter_mut().find(|d| d.uid == uid).map(|d| {
        // The step/start group is already published: retain its exact pair on failed refresh.
        d.restore_selection_memory();
        View::snapshot(d)
    });
    if let Some(dbg) = s.macro_debug.as_mut() {
        dbg.view = view;
    }
}

/// Puts back the active sheet and the selections where the run left them, so the next step runs
/// where `macro.run` would run it, however the person looked around meanwhile.
fn put_back_view(s: &mut Session) -> Result<()> {
    let Some((uid, Some(view))) = s.macro_debug.as_ref().map(|d| (d.doc, d.view.clone())) else { return Ok(()) };
    let Some(i) = s.document_index(uid) else { return Ok(()) };
    // The persisted step limit terminates without activation; the per-action clock resets later.
    if s.macro_debug.as_ref().and_then(|d| d.machine.as_ref()).is_some_and(|m| m.executed >= super::run::MAX_EXECUTED_STEPS) {
        let plan = view.prepare(&s.docs[i])?;
        view.apply(&mut s.docs[i], plan);
        return Ok(());
    }
    s.try_set_active_view(i, Some(view))
}

/// The message for edits refused while a macro is being debugged.
pub const BUSY: &str = "Finish or stop the macro first";

/// Ends a debugger whose undo group is no longer open (something other than the debugger ended
/// or rolled it back): its workbook is not locked any more. Returns the macro's name when it did.
pub(crate) fn drop_if_ended(s: &mut Session) -> Option<String> {
    let d = s.macro_debug.as_ref()?;
    if d.stepping() || d.token.as_ref().is_some_and(|t| s.group_is_open(t)) {
        return None;
    }
    s.macro_debug.take().map(|d| d.name)
}

/// After Stop Action rolled back the undo groups of the workbook with uid `doc`: ends the
/// debugger if its run was there. Returns the macro's name and the files the run wrote, which stay.
pub(crate) fn end_stopped(s: &mut Session, doc: u64) -> Option<(String, Vec<String>)> {
    if !s.macro_debug.as_ref().is_some_and(|d| d.doc == doc && !d.stepping()) {
        return None;
    }
    let d = s.macro_debug.take()?;
    Some((d.name, d.machine.map(|m| m.wrote_files).unwrap_or_default()))
}

/// Why an action finds the run already ended: its undo group was closed outside the debugger.
fn already_ended(name: &str) -> String {
    format!("Macro '{name}' was no longer being debugged: its undo group had been closed outside the debugger, so nothing was kept or rolled back.")
}

enum Mode {
    Step,
    Continue,
    RunTo(String),
}

const ACTIONS: [&str; 6] = ["start", "step", "continue", "runTo", "stop", "state"];

/// `macro.debug {name?, inputs?, action: start|step|continue|runTo|stop|state, step?, keep?,
/// breakpoints?, breakpointsOnly?}`.
pub fn debug(s: &mut Session, p: &Json) -> Result<Json> {
    // A step of a script or macro that started the debugger would leave it behind when the
    // script fails and rolls back its groups, the debugger's among them.
    if s.depth() > 1 {
        return Err(EngineError::Disabled("macro.debug".into(), "The macro debugger runs on its own, not as a step of a script or macro.".into()));
    }
    let ended = drop_if_ended(s);
    let action = match p.get("action") {
        None | Some(Json::Null) if s.macro_debug.is_some() || ended.is_some() => "state".to_string(),
        None | Some(Json::Null) => "start".to_string(),
        Some(Json::String(a)) => a.clone(),
        // `{"action": {"runTo": {"step": "3"}}}`
        Some(Json::Object(o)) if o.len() == 1 => o.keys().next().cloned().unwrap_or_default(),
        Some(_) => return Err(bad("`action` must be start, step, continue, runTo, stop or state")),
    };
    if !ACTIONS.contains(&action.as_str()) {
        return Err(bad(format!("action must be start, step, continue, runTo, stop or state, not '{action}'")));
    }
    if let Some(name) = ended
        && action != "start"
    {
        return Err(EngineError::Other(already_ended(&name)));
    }
    let only = bool_param(p, "breakpointsOnly")?;
    if action == "start" {
        return start(s, p, only.unwrap_or(false));
    }
    let keep = bool_param(p, "keep")?;
    let d = s.macro_debug.as_ref().ok_or_else(not_debugging)?;
    let target = if action == "runTo" {
        let target = p
            .get("step")
            .or_else(|| p.pointer("/action/runTo/step"))
            .and_then(path_of)
            .ok_or_else(|| bad("`runTo` needs `step`: the path of the step to stop at"))?;
        if !d.paths.contains(&target) {
            return Err(bad(format!("macro '{}' has no step {target}", d.name)));
        }
        Some(target)
    } else {
        None
    };
    let breakpoints = p.get("breakpoints").filter(|v| !v.is_null()).map(|v| paths_param(v, &d.paths, &d.name)).transpose()?;
    if let Some(d) = s.macro_debug.as_mut() {
        if let Some(bps) = breakpoints {
            d.breakpoints = bps;
        }
        if let Some(only) = only {
            d.only_listed = only;
        }
    }
    match (action.as_str(), target) {
        ("step", _) => advance(s, Mode::Step),
        ("continue", _) => advance(s, Mode::Continue),
        ("runTo", Some(target)) => advance(s, Mode::RunTo(target)),
        ("stop", _) => stop(s, keep.unwrap_or(false)),
        _ => state(s),
    }
}

fn bad(msg: impl Into<String>) -> EngineError {
    crate::cmd::bad("macro.debug", msg)
}

fn not_debugging() -> EngineError {
    EngineError::Other("No macro is being debugged. Start one with action: start.".into())
}

/// `true`, `false`, or not given (`null` counts as not given); anything else is refused.
fn bool_param(p: &Json, key: &str) -> Result<Option<bool>> {
    match p.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(bad(format!("`{key}` must be true or false"))),
    }
}

/// A step path from JSON: `"2.1"` or `3`.
fn path_of(v: &Json) -> Option<String> {
    match v {
        Json::String(t) if !t.trim().is_empty() => Some(t.trim().to_string()),
        Json::Number(n) => n.as_u64().map(|n| n.to_string()),
        _ => None,
    }
}

/// The `breakpoints` list: step paths of macro `name` (one of `paths`), at most
/// [`MAX_BREAKPOINTS`].
fn paths_param(v: &Json, paths: &BTreeSet<String>, name: &str) -> Result<BTreeSet<String>> {
    let list = v.as_array().ok_or_else(|| bad("`breakpoints` must be a list of step paths"))?;
    if list.len() > MAX_BREAKPOINTS {
        return Err(bad(format!("`breakpoints` lists {} steps; at most {MAX_BREAKPOINTS} can be set", list.len())));
    }
    let mut out = BTreeSet::new();
    for x in list {
        let path = path_of(x).ok_or_else(|| bad("a breakpoint is a step path such as \"3\" or \"2.1\""))?;
        if !paths.contains(&path) {
            return Err(bad(format!("macro '{name}' has no step {path} for a breakpoint")));
        }
        out.insert(path);
    }
    Ok(out)
}

fn start(s: &mut Session, p: &Json, only_listed: bool) -> Result<Json> {
    if let Some(d) = &s.macro_debug {
        return Err(EngineError::Other(format!("Macro '{}' is being debugged. Stop it first.", d.name)));
    }
    let name = p.get("name").and_then(Json::as_str).filter(|n| !n.trim().is_empty()).ok_or_else(|| bad("missing `name`"))?;
    let b = super::run::begin(s, name, p)?;
    let paths = all_paths(&b.mac.steps, "");
    let breakpoints = match p.get("breakpoints").filter(|v| !v.is_null()) {
        Some(v) => paths_param(v, &paths, &b.mac.name)?,
        None => BTreeSet::new(),
    };
    let start_wb = s.doc()?.wb.clone();
    let token = s.group_begin(&format!("Run Macro: {}", b.mac.name))?;
    // The run's machine: unless the rule is overwrite, it checks each step for writes over data.
    let (mut machine, shifted) = match super::run::make_way(s, &b) {
        Ok(made) => made,
        Err(e) => {
            let _ = s.group_rollback(token);
            return Err(e);
        }
    };
    // Each action has the time limit to itself; the dry runs just had theirs.
    machine.restart_clock();
    let base = machine.executed;
    s.macro_debug = Some(Debugger {
        name: b.mac.name.clone(),
        doc: b.prepared.doc,
        machine: Some(machine),
        token: Some(token),
        breakpoints,
        only_listed,
        start: start_wb,
        paths,
        touched: BTreeSet::from([b.prepared.doc]),
        base,
        shifted: shifted.then(|| b.rule.name()),
        view: None,
    });
    if s.macro_debug.as_ref().and_then(|d| d.machine.as_ref()).is_some_and(Machine::is_done) {
        return finish(s, None);
    }
    remember_view(s);
    state(s)
}

fn all_paths(steps: &[gridcraft_model::Step], prefix: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (i, st) in steps.iter().enumerate() {
        let path = if prefix.is_empty() { (i + 1).to_string() } else { format!("{prefix}.{}", i + 1) };
        match &st.kind {
            gridcraft_model::StepKind::Repeat { body, .. } => out.extend(all_paths(body, &path)),
            gridcraft_model::StepKind::If { then, otherwise, .. } => {
                out.extend(all_paths(then, &format!("{path}.then")));
                out.extend(all_paths(otherwise, &format!("{path}.else")));
            }
            _ => {}
        }
        out.insert(path);
    }
    out
}

/// Runs one step (`Step`), or steps until a breakpoint, the target step or the end.
fn advance(s: &mut Session, mode: Mode) -> Result<Json> {
    put_back_view(s)?;
    let d = s.macro_debug.as_mut().ok_or_else(not_debugging)?;
    let (name, base) = (d.name.clone(), d.base);
    let breakpoints = d.breakpoints.clone();
    let only_listed = d.only_listed;
    let mut machine = d.machine.take().ok_or_else(not_debugging)?;
    machine.restart_clock();
    let before = versions(s);
    let mut first = true;
    let mut hit = false;
    let mut outcome = Ok(());
    let mut preparation_error = None;
    let executed_before = machine.executed;
    while !machine.is_done() {
        if !first {
            let Some((path, st)) = machine.current() else { break };
            if matches!(mode, Mode::Step) {
                break;
            }
            if (st.breakpoint && !only_listed) || breakpoints.contains(&path) {
                hit = true;
                break;
            }
            if let Mode::RunTo(target) = &mode
                && *target == path
            {
                break;
            }
        }
        if let Err(e) = machine.prepare_activation(s) {
            preparation_error = Some(e);
            break;
        }
        first = false;
        outcome = machine.step(s);
        if outcome.is_err() {
            break;
        }
    }
    let published = machine.executed != executed_before;
    let done = machine.is_done();
    match s.macro_debug.as_mut() {
        Some(d) => d.machine = Some(machine),
        // The macro.debug call panicked and `recover` has already rolled the run back.
        None => {
            return match outcome {
                Err(e) => Ok(failed(&name, machine.executed.saturating_sub(base), &e)),
                Ok(()) => Err(not_debugging()),
            };
        }
    }
    if let Some(error) = preparation_error {
        if published {
            note_touched(s, &before);
            remember_view(s);
        }
        return Err(error);
    }
    note_touched(s, &before);
    if let Err(e) = outcome {
        return finish(s, Some(e));
    }
    if done {
        return finish(s, None);
    }
    remember_view(s);
    let mut out = state(s)?;
    out["atBreakpoint"] = json!(hit);
    Ok(out)
}

/// What every answer about the run's end or its state adds: the data shift at the start and the
/// files the run wrote (they stay, whatever happens to the workbook).
fn add_notes(out: &mut Json, d: &Debugger, wrote_files: &[String]) {
    if let Some(rule) = d.shifted {
        out["shifted"] = json!(rule);
    }
    if !wrote_files.is_empty() {
        out["wroteFiles"] = json!(wrote_files);
    }
}

/// Ends the run after its last step (kept as one undo step, unless a Stop step asked for a
/// rollback) or a failed step (rolled back).
fn finish(s: &mut Session, error: Option<StepError>) -> Result<Json> {
    let Some(mut d) = s.macro_debug.take() else { return Err(not_debugging()) };
    let (changed, cells) = changed(s, &d);
    let executed = d.executed();
    let machine = d.machine.take();
    let stopped = machine.as_ref().and_then(|m| m.stopped.clone());
    let wrote = machine.map(|m| m.wrote_files).unwrap_or_default();
    let rollback = error.is_some() || matches!(stopped, Some((_, true)));
    let ended = match d.token.take() {
        Some(token) if rollback => s.group_rollback(token).is_ok(),
        Some(token) => s.group_end(token).is_ok(),
        None => false,
    };
    if !ended {
        let step = error.map(|e| format!("{} ", e.describe())).unwrap_or_default();
        return Err(EngineError::Other(format!("{step}{}", already_ended(&d.name))));
    }
    if let Some(e) = &error {
        return Ok(failed(&d.name, executed, e));
    }
    let mut out = json!({"macro": d.name, "executed": executed});
    match &stopped {
        Some((msg, rolled)) => {
            out["state"] = json!("stopped");
            out["stopped"] = json!(msg);
            out["kept"] = json!(!rolled);
            if *rolled {
                out["message"] = json!(rolled_back(&wrote));
            }
        }
        None => out["state"] = json!("finished"),
    }
    if !rollback {
        out["changed"] = json!(changed);
        out["changedCells"] = json!(cells);
    }
    add_notes(&mut out, &d, &wrote);
    Ok(out)
}

/// The result of a run that a failed step rolled back.
fn failed(name: &str, executed: u64, e: &StepError) -> Json {
    let mut out = json!({
        "macro": name,
        "executed": executed,
        "state": "failed",
        "error": serde_json::to_value(e).unwrap_or(Json::Null),
        "message": format!("{} {}", e.describe(), rolled_back(&e.wrote_files)),
    });
    if !e.wrote_files.is_empty() {
        out["wroteFiles"] = json!(e.wrote_files);
    }
    out
}

/// Stops the run: keeps what it did as one undo step, or rolls it back.
fn stop(s: &mut Session, keep: bool) -> Result<Json> {
    let Some(mut d) = s.macro_debug.take() else { return Err(not_debugging()) };
    let (changed, cells) = changed(s, &d);
    let ended = match d.token.take() {
        Some(token) if keep => s.group_end(token).is_ok(),
        Some(token) => s.group_rollback(token).is_ok(),
        None => false,
    };
    if !ended {
        return Err(EngineError::Other(already_ended(&d.name)));
    }
    let wrote = d.machine.as_ref().map(|m| m.wrote_files.clone()).unwrap_or_default();
    let mut out = json!({"state": "stopped", "macro": d.name, "kept": keep, "executed": d.executed()});
    if keep {
        out["changed"] = json!(changed);
        out["changedCells"] = json!(cells);
    } else {
        out["message"] = json!(rolled_back(&wrote));
    }
    add_notes(&mut out, &d, &wrote);
    Ok(out)
}

/// Where the run is: the next step with its parameters as they would run, the step that just
/// ran, and what the run has changed in its workbook so far.
fn state(s: &mut Session) -> Result<Json> {
    let d = s.macro_debug.as_ref().ok_or_else(not_debugging)?;
    let machine = d.machine.as_ref().ok_or_else(not_debugging)?;
    let wb = s.docs.iter().find(|x| x.uid == d.doc).map(|x| x.wb.clone()).ok_or(EngineError::NoDocument)?;
    let (changed, cells) = changed(s, d);
    let mut out = json!({
        "state": "paused",
        "macro": d.name,
        "current": machine.current_json(&wb),
        "lastParams": machine.last_params,
        "executed": d.executed(),
        "breakpoints": d.breakpoints,
        "changed": changed,
        "changedCells": cells,
    });
    add_notes(&mut out, d, &machine.wrote_files);
    Ok(out)
}

/// What the run changed in its workbook since it started: cells as `Sheet!A1:B2` ranges and the
/// sheets it added, deleted or renamed (at most [`MAX_LISTED`] entries), and how many cells
/// changed on the sheets the workbook had at the start.
fn changed(s: &Session, d: &Debugger) -> (Vec<String>, usize) {
    let Some(now) = s.docs.iter().find(|x| x.uid == d.doc).map(|x| x.wb.clone()) else { return (vec![], 0) };
    let (pairs, added) = super::run::pair_sheets(&d.start, &now);
    let mut out = Vec::new();
    let mut count = 0usize;
    for (bi, ai) in pairs {
        let Some(before) = d.start.sheets.get(bi) else { continue };
        let Some(after) = ai.and_then(|a| now.sheets.get(a)) else {
            out.push(format!("{} (deleted sheet)", before.name));
            continue;
        };
        if before.name != after.name {
            out.push(format!("{} (renamed from {})", after.name, before.name));
        }
        if Arc::ptr_eq(before, after) {
            continue;
        }
        let mut cells = before.cells.diff(&after.cells);
        count = count.saturating_add(cells.len());
        if out.len() < MAX_LISTED {
            let q = super::resolve::quote_sheet(&after.name);
            out.extend(super::run::ranges(&mut cells).into_iter().map(|r| format!("{q}!{r}")));
        }
    }
    out.extend(added.into_iter().filter_map(|a| now.sheets.get(a)).map(|sh| format!("{} (new sheet)", sh.name)));
    out.truncate(MAX_LISTED);
    (out, count)
}

/// After the `macro.debug` call itself panicked: if the run lost its place, roll it back so its
/// undo group doesn't stay open.
pub(crate) fn recover(s: &mut Session) {
    if s.macro_debug.as_ref().is_some_and(Debugger::stepping)
        && let Some(mut d) = s.macro_debug.take()
        && let Some(token) = d.token.take()
    {
        let _ = s.group_rollback(token);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;

    #[test]
    fn a_run_that_lost_its_place_is_rolled_back() {
        let mut s = Session::new();
        s.new_workbook();
        let m = crate::macros::parse_macro(&json!({"name": "M", "output": {"onConflict": "overwrite"}, "steps": [
            {"command": "cell.set", "params": {"cell": "A1", "input": "x"}},
            {"command": "cell.set", "params": {"cell": "A2", "input": "y"}}
        ]}))
        .unwrap();
        let d = s.doc_mut().unwrap();
        let mut wb = (*d.wb).clone();
        wb.macros.push(m);
        d.wb = std::sync::Arc::new(wb);
        let start = s.doc().unwrap().wb.clone();
        s.execute("macro.debug", json!({"name": "M"})).unwrap();
        s.execute("macro.debug", json!({"action": "step"})).unwrap();
        // As if a step had panicked with the machine out of the session.
        s.macro_debug.as_mut().unwrap().machine = None;
        super::recover(&mut s);
        assert!(s.macro_debug.is_none());
        assert!(std::sync::Arc::ptr_eq(&s.doc().unwrap().wb, &start));
        assert!(!s.in_group(s.doc().unwrap().uid));
        s.execute("cell.set", json!({"cell": "B1", "input": "free"})).unwrap();
    }

    #[test]
    fn a_step_that_panics_reports_the_step_failure() {
        crate::register_test_command("test.debugStepPanics", |_, _| panic!("boom"));
        let mut s = Session::new();
        s.new_workbook();
        let m = crate::macros::parse_macro(&json!({"name": "M", "output": {"onConflict": "overwrite"}, "steps": [
            {"command": "cell.set", "params": {"cell": "A1", "input": "x"}},
            {"command": "test.debugStepPanics", "params": {}}
        ]}))
        .unwrap();
        let d = s.doc_mut().unwrap();
        let mut wb = (*d.wb).clone();
        wb.macros.push(m);
        d.wb = std::sync::Arc::new(wb);
        let start = s.doc().unwrap().wb.clone();
        s.execute("macro.debug", json!({"name": "M"})).unwrap();
        s.execute("macro.debug", json!({"action": "step"})).unwrap();
        let r = s.execute("macro.debug", json!({"action": "step"})).unwrap();
        assert_eq!(r["state"], "failed", "{r}");
        assert_eq!(r["macro"], "M");
        assert_eq!(r["error"]["step"], "2");
        assert!(r["message"].as_str().unwrap().contains("boom"), "{r}");
        assert!(s.macro_debug.is_none());
        assert!(std::sync::Arc::ptr_eq(&s.doc().unwrap().wb, &start));
        assert!(!s.in_group(s.doc().unwrap().uid));
    }

    #[test]
    fn a_workbook_in_the_touched_log_waits_until_the_run_ends() {
        let mut s = Session::new();
        s.new_workbook();
        let m = crate::macros::parse_macro(&json!({"name": "M", "output": {"onConflict": "overwrite"}, "steps": [
            {"command": "cell.set", "params": {"cell": "A1", "input": "x"}},
            {"command": "cell.set", "params": {"cell": "A2", "input": "y"}}
        ]}))
        .unwrap();
        let d = s.doc_mut().unwrap();
        let mut wb = (*d.wb).clone();
        wb.macros.push(m);
        d.wb = std::sync::Arc::new(wb);
        s.new_workbook();
        let other = s.doc().unwrap().uid;
        s.set_active(0);
        s.execute("macro.debug", json!({"name": "M"})).unwrap();
        assert!(!s.macro_debug.as_ref().unwrap().locks(other));
        // As if a step had changed the other workbook too.
        let before = super::versions(&s);
        let d = s.docs.iter_mut().find(|d| d.uid == other).unwrap();
        let mut wb = (*d.wb).clone();
        wb.props.title = "changed by the macro".into();
        d.wb = std::sync::Arc::new(wb);
        super::note_touched(&mut s, &before);
        assert!(s.macro_debug.as_ref().unwrap().locks(other));
        s.set_active(1);
        assert!(s.execute("cell.set", json!({"cell": "B1", "input": "no"})).is_err());
        s.execute("macro.debug", json!({"action": "stop", "keep": true})).unwrap();
        s.execute("cell.set", json!({"cell": "B1", "input": "free"})).unwrap();
    }

    #[test]
    fn edits_run_by_another_command_wait_too() {
        let mut s = Session::new();
        s.new_workbook();
        let m = crate::macros::parse_macro(&json!({"name": "M", "output": {"onConflict": "overwrite"}, "steps": [
            {"command": "cell.set", "params": {"cell": "A1", "input": "x"}},
            {"command": "cell.set", "params": {"cell": "A2", "input": "y"}}
        ]}))
        .unwrap();
        let d = s.doc_mut().unwrap();
        let mut wb = (*d.wb).clone();
        wb.macros.push(m);
        d.wb = std::sync::Arc::new(wb);
        s.execute("automate.recordActions", json!({})).unwrap();
        s.execute("cell.set", json!({"cell": "C1", "input": "1"})).unwrap();
        s.execute("macro.debug", json!({"name": "M"})).unwrap();
        s.execute("macro.debug", json!({"action": "step"})).unwrap();
        // Record Actions changes nothing itself; stopping it saves the script with a nested
        // command, which would land in the run's undo group.
        let e = s.execute("automate.recordActions", json!({"stop": true, "name": "S"})).unwrap_err().to_string();
        assert!(e.contains(super::BUSY), "{e}");
        assert!(s.execute("automate.allScripts", json!({})).unwrap().as_array().unwrap().is_empty());
        // The run's own steps still go ahead.
        let r = s.execute("macro.debug", json!({"action": "step"})).unwrap();
        assert_eq!(r["executed"], 2, "{r}");
        assert!(s.macro_debug.is_none());
        // The recording was kept: stopping now saves it.
        let saved = s.execute("automate.recordActions", json!({"stop": true, "name": "S"})).unwrap();
        assert_eq!(saved["saved"], "S", "{saved}");
        let scripts = s.execute("automate.allScripts", json!({})).unwrap();
        assert_eq!(scripts[0]["name"], "S", "{scripts}");
    }
    #[test]
    fn debug_restore_uses_captured_uid_after_external_rename_and_move() {
        let mut s = Session::new();
        s.new_workbook();
        s.execute("home.insertSheet", json!({"name":"B"})).unwrap();
        let mac = crate::macros::parse_macro(&json!({"name":"Pair", "output":{"onConflict":"overwrite"}, "steps":[
            {"command":"cell.set", "params":{"input":"first"}}, {"command":"cell.set", "params":{"cell":"D4", "input":"second"}}
        ]}))
        .unwrap();
        let d = s.doc_mut().unwrap();
        std::sync::Arc::make_mut(&mut d.wb).macros.push(mac);
        d.selection = crate::Selection::at(gridcraft_core::CellRef::new(6, 1));
        let b = d.wb.sheets[1].uid.get();
        s.execute("macro.debug", json!({"name":"Pair"})).unwrap();
        s.execute("sheet.activate", json!({"sheet":0})).unwrap();
        let wb = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().wb);
        std::sync::Arc::make_mut(&mut wb.sheets[1]).name = "Moved".into();
        wb.sheets.swap(0, 1);
        wb.set_active_sheet(1);
        s.execute("macro.debug", json!({"action":"step"})).unwrap();
        let d = s.doc().unwrap();
        assert_eq!(d.wb.active().unwrap().uid.get(), b);
        assert_eq!(d.wb.sheets[0].value(gridcraft_core::CellRef::new(6, 1)), gridcraft_core::Value::text("first"));
    }

    #[test]
    fn post_step_refresh_failure_keeps_published_step_and_retry_runs_once() {
        crate::register_test_command("test.debugCaptureFailure", |s, _| {
            s.execute("cell.set", json!({"cell":"B1", "input":"first"}))?;
            let d = s.doc_mut()?;
            d.wb.set_active_sheet(0);
            d.selection = crate::Selection::at(gridcraft_core::CellRef::new(8, 2));
            d.selection_preparation_budget = Some(0);
            Ok(json!({"published":true}))
        });
        let mut s = Session::new();
        s.new_workbook();
        s.execute("home.insertSheet", json!({"name":"B"})).unwrap();
        let mac = crate::macros::parse_macro(&json!({"name":"Deferred", "output":{"onConflict":"overwrite"}, "steps":[
            {"command":"test.debugCaptureFailure"}, {"command":"cell.set", "params":{"cell":"A2", "input":"second"}}
        ]}))
        .unwrap();
        std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().wb).macros.push(mac);
        let undo_before = s.doc().unwrap().undo.len();
        s.execute("macro.debug", json!({"name":"Deferred"})).unwrap();
        let result = s.execute("macro.debug", json!({"action":"step"})).unwrap();
        assert_eq!(result["executed"], 1);
        assert_eq!(result["changedCells"], 1);
        assert_eq!(s.doc().unwrap().undo.len(), undo_before);
        assert!(s.macro_debug.as_ref().unwrap().touched.contains(&s.doc().unwrap().uid));
        assert_eq!(
            format!("{:?}", s.macro_debug.as_ref().unwrap().view.as_ref().unwrap()),
            format!("{:?}", crate::windows::PairedView::snapshot(s.doc().unwrap()))
        );
        assert_eq!(s.doc().unwrap().wb.sheets[1].value(gridcraft_core::CellRef::new(0, 1)), gridcraft_core::Value::text("first"));
        assert!(s.doc().unwrap().selection_refresh_deferred);
        assert!(s.in_group(s.doc().unwrap().uid));
        let before = (s.doc().unwrap().wb.clone(), s.doc().unwrap().selection.clone(), s.doc().unwrap().sheet_selections.clone());
        assert!(super::advance(&mut s, super::Mode::Step).is_err());
        assert_eq!(s.macro_debug.as_ref().unwrap().executed(), 1);
        assert!(std::sync::Arc::ptr_eq(&before.0, &s.doc().unwrap().wb));
        assert_eq!((&s.doc().unwrap().selection, &s.doc().unwrap().sheet_selections), (&before.1, &before.2));
        s.doc_mut().unwrap().selection_preparation_budget = None;
        let result = s.execute("macro.debug", json!({"action":"step"})).unwrap();
        assert_eq!(result["executed"], 2);
        assert_eq!(result["state"], "finished");
        assert_eq!(result["changedCells"], 2);
        assert_eq!(s.doc().unwrap().undo.len(), undo_before + 1);
        assert_eq!(s.doc().unwrap().wb.sheets[0].value(gridcraft_core::CellRef::new(1, 0)), gridcraft_core::Value::text("second"));
        assert!(s.macro_debug.is_none());
    }
    #[test]
    fn start_snapshot_failure_keeps_open_run_and_exact_deferred_pair() {
        let mut s = Session::new();
        s.new_workbook();
        let mac = crate::macros::parse_macro(
            &json!({"name":"Start", "output":{"onConflict":"overwrite"}, "steps":[{"command":"cell.set", "params":{"cell":"A1", "input":"once"}}]}),
        )
        .unwrap();
        std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().wb).macros.push(mac);
        s.execute("macro.debug", json!({"name":"Start"})).unwrap();
        assert_eq!(s.macro_debug.as_ref().unwrap().executed(), 0);
        s.doc_mut().unwrap().selection_preparation_budget = Some(0);
        // Exercise the same mandatory capture used immediately after start opens its group.
        super::remember_view(&mut s);
        assert!(s.in_group(s.doc().unwrap().uid));
        assert!(s.doc().unwrap().selection_refresh_deferred);
        assert_eq!(
            format!("{:?}", s.macro_debug.as_ref().unwrap().view.as_ref().unwrap()),
            format!("{:?}", crate::windows::PairedView::snapshot(s.doc().unwrap()))
        );
        assert!(super::advance(&mut s, super::Mode::Continue).is_err());
        assert_eq!(s.macro_debug.as_ref().unwrap().executed(), 0);
        s.doc_mut().unwrap().selection_preparation_budget = None;
        assert_eq!(s.execute("macro.debug", json!({"action":"continue"})).unwrap()["executed"], 1);
    }

    #[test]
    fn debugger_keep_rollback_failure_and_external_end_keep_view_ownership() {
        for mode in ["keep", "rollback", "failure", "external"] {
            let mut s = Session::new();
            s.new_workbook();
            s.execute("home.insertSheet", json!({"name":"B"})).unwrap();
            let initial = crate::Selection::at(gridcraft_core::CellRef::new(6, 1));
            s.doc_mut().unwrap().selection = initial.clone();
            let mac = crate::macros::parse_macro(&json!({"name":"Modes", "output":{"onConflict":"overwrite"}, "steps":[
                {"command":"selection.set", "params":{"range":"C9:D10,F12:G13", "active":"G13"}},
                {"command":"cell.set", "params":{"cell":"?", "input":"bad"}}
            ]}))
            .unwrap();
            std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().wb).macros.push(mac);
            s.execute("macro.debug", json!({"name":"Modes"})).unwrap();
            s.execute("sheet.activate", json!({"sheet":0})).unwrap();
            s.execute("macro.debug", json!({"action":"step"})).unwrap();
            let selected = s.doc().unwrap().selection.clone();
            assert_eq!(selected.ranges.len(), 2);
            s.execute("sheet.activate", json!({"sheet":0})).unwrap();
            match mode {
                "keep" => {
                    s.execute("macro.debug", json!({"action":"stop", "keep":true})).unwrap();
                }
                "rollback" => {
                    s.execute("macro.debug", json!({"action":"stop"})).unwrap();
                }
                "failure" => {
                    assert_eq!(s.execute("macro.debug", json!({"action":"continue"})).unwrap()["state"], "failed");
                }
                _ => {
                    let token = s.macro_debug.as_mut().unwrap().token.take().unwrap();
                    s.group_end(token).unwrap();
                    assert!(s.execute("macro.debug", json!({"action":"state"})).is_err());
                }
            }
            assert!(s.macro_debug.is_none());
            if matches!(mode, "rollback" | "failure") {
                assert_eq!(s.doc().unwrap().selection, initial);
            } else {
                s.execute("sheet.activate", json!({"sheet":1})).unwrap();
                assert_eq!(s.doc().unwrap().selection, selected);
            }
        }
    }
    fn activation_views(s: &Session) -> String {
        format!(
            "{:?}|{:?}",
            (s.active, s.active_window, s.activation, &s.windows, s.groups.len(), s.next_group, s.group_epoch, s.journal.len()),
            s.docs.iter().map(|d| (super::View::snapshot(d), &d.edits, &d.touched, d.undo.len(), d.redo.len())).collect::<Vec<_>>()
        )
    }

    #[test]
    fn activation_failure_stages_parked_window_and_debug_pair_before_either_changes() {
        for fail_window in [true, false] {
            let mut s = Session::new();
            s.new_workbook();
            let long = vec![crate::Selection::at(gridcraft_core::CellRef::new(70, 4)); 24];
            if fail_window {
                s.doc_mut().unwrap().sheet_selections = long.clone();
            }
            let parked = super::View::capture(s.doc().unwrap()).unwrap();
            let a = s.active_window().unwrap();
            let b = s.new_window().unwrap();
            s.activate_window(a);
            s.show_window(b);
            s.doc_mut().unwrap().sheet_selections = if fail_window { Vec::new() } else { long };
            let mac = crate::macros::parse_macro(&json!({"name":"Entry", "output":{"onConflict":"overwrite"}, "steps":[
                {"command":"cell.set", "params":{"cell":"A1", "input":"once"}},
                {"command":"cell.set", "params":{"cell":"A2", "input":"twice"}}
            ]}))
            .unwrap();
            std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().wb).macros.push(mac);
            s.execute("macro.debug", json!({"name":"Entry"})).unwrap();
            if !fail_window {
                s.doc_mut().unwrap().sheet_selections.clear();
                s.doc_mut().unwrap().refresh_selection_memory().unwrap();
            }
            s.new_workbook();
            let view = s.macro_debug.as_ref().unwrap().view.clone().unwrap();
            let d = &mut s.docs[0];
            let budget = (0..128)
                .find(|n| {
                    d.selection_preparation_budget = Some(*n);
                    d.prepare_selection(&d.wb, crate::selection_memory::Transition::Refresh).is_ok()
                        && view.prepare(d).is_ok() == fail_window
                        && parked.prepare(d).is_err() == fail_window
                })
                .unwrap();
            d.selection_preparation_budget = Some(budget);
            s.docs[1].sheet_selections = vec![crate::Selection::default(); 3];
            s.docs[1].selection_refresh_deferred = true;
            let before = activation_views(&s);
            let machine = format!("{:?}", s.macro_debug.as_ref().unwrap().machine);
            assert!(s.execute("macro.debug", json!({"action":"step"})).is_err(), "budget {budget}, fail_window={fail_window}");
            assert_eq!(activation_views(&s), before);
            assert_eq!(format!("{:?}", s.macro_debug.as_ref().unwrap().machine), machine);
            assert!(s.docs.iter().all(|d| d.wb.sheets[0].cells.is_empty()));
            s.docs[0].selection_preparation_budget = None;
            assert_eq!(s.execute("macro.debug", json!({"action":"step"})).unwrap()["executed"], 1);
            assert_eq!(s.execute("macro.debug", json!({"action":"continue"})).unwrap()["executed"], 2);
            assert_eq!(s.docs[0].wb.sheets[0].value(gridcraft_core::CellRef::new(0, 0)), gridcraft_core::Value::text("once"));
            assert!(s.docs[1].wb.sheets[0].cells.is_empty());
        }
    }

    #[test]
    fn activation_failure_during_continue_keeps_published_prefix_and_retry_position() {
        let mut s = Session::new();
        s.new_workbook();
        s.doc_mut().unwrap().sheet_selections = vec![crate::Selection::default(); 24];
        let parked = super::View::capture(s.doc().unwrap()).unwrap();
        let a = s.active_window().unwrap();
        let b = s.new_window().unwrap();
        s.activate_window(a);
        s.show_window(b);
        s.doc_mut().unwrap().sheet_selections.clear();
        s.new_workbook();
        let other = s.active_window().unwrap();
        s.show_window(b);
        let mac = crate::macros::parse_macro(&json!({"name":"Prefix", "output":{"onConflict":"overwrite"}, "steps":[
            {"command":"view.switchWindows", "params":{"window":other}},
            {"command":"cell.set", "params":{"cell":"A2", "input":"retry"}}
        ]}))
        .unwrap();
        std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().wb).macros.push(mac);
        s.execute("macro.debug", json!({"name":"Prefix"})).unwrap();
        let view = s.macro_debug.as_ref().unwrap().view.clone().unwrap();
        let d = &mut s.docs[0];
        let budget = (0..128)
            .find(|n| {
                d.selection_preparation_budget = Some(*n);
                view.prepare(d).is_ok()
                    && d.prepare_selection(&d.wb, crate::selection_memory::Transition::Refresh).is_ok()
                    && parked.prepare(d).is_err()
            })
            .unwrap();
        assert_eq!(budget, 4);
        d.selection_preparation_budget = Some(budget);
        assert!(s.execute("macro.debug", json!({"action":"continue"})).is_err());
        let dbg = s.macro_debug.as_ref().unwrap();
        assert_eq!(dbg.executed(), 1);
        assert_eq!(dbg.machine.as_ref().unwrap().current().unwrap().0, "2");
        assert_eq!(s.active_window(), Some(other), "the first real step published in this continue call");
        assert!(dbg.touched.contains(&s.docs[0].uid));
        assert!(s.in_group(s.docs[0].uid));
        assert_eq!(format!("{:?}", dbg.view.as_ref().unwrap()), format!("{:?}", super::View::snapshot(&s.docs[0])));
        assert!(s.docs.iter().all(|d| d.wb.sheets[0].cells.is_empty()));
        assert_eq!(super::state(&mut s).unwrap()["changedCells"], 0);
        assert!(s.docs[1].edits.is_none() && s.docs[1].touched.is_none());
        s.docs[0].selection_preparation_budget = None;
        let result = s.execute("macro.debug", json!({"action":"continue"})).unwrap();
        assert_eq!(result["state"], "finished");
        assert_eq!(result["executed"], 2);
        assert_eq!(result["changedCells"], 1);
        assert_eq!(s.docs[0].wb.sheets[0].value(gridcraft_core::CellRef::new(1, 0)), gridcraft_core::Value::text("retry"));
        assert!(s.docs[1].wb.sheets[0].cells.is_empty());
    }

    #[test]
    fn exhausted_debugger_entry_keeps_terminal_rollback_before_window_preflight() {
        let mut s = Session::new();
        s.new_workbook();
        let mac = crate::macros::parse_macro(&json!({"name":"Terminal", "output":{"onConflict":"overwrite"}, "steps":[
            {"command":"cell.set", "params":{"cell":"A1", "input":"rollback"}},
            {"command":"cell.set", "params":{"cell":"A2", "input":"never"}}
        ]}))
        .unwrap();
        std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().wb).macros.push(mac);
        let original = s.doc().unwrap().wb.clone();
        let selection = s.doc().unwrap().selection.clone();
        let history = (s.doc().unwrap().undo.len(), s.doc().unwrap().redo.len());
        s.execute("macro.debug", json!({"name":"Terminal"})).unwrap();
        assert_eq!(s.execute("macro.debug", json!({"action":"step"})).unwrap()["executed"], 1);
        assert_eq!(s.docs[0].wb.sheets[0].value(gridcraft_core::CellRef::new(0, 0)), gridcraft_core::Value::text("rollback"));
        s.doc_mut().unwrap().sheet_selections.clear();
        super::remember_view(&mut s);
        s.doc_mut().unwrap().sheet_selections = vec![crate::Selection::default(); 24];
        let parked = super::View::capture(s.doc().unwrap()).unwrap();
        let a = s.active_window().unwrap();
        let b = s.new_window().unwrap();
        s.activate_window(a);
        s.show_window(b);
        s.doc_mut().unwrap().sheet_selections.clear();
        s.new_workbook();
        let view = s.macro_debug.as_ref().unwrap().view.clone().unwrap();
        let d = &mut s.docs[0];
        let budget = (0..128)
            .find(|n| {
                d.selection_preparation_budget = Some(*n);
                view.prepare(d).is_ok()
                    && d.prepare_selection(&d.wb, crate::selection_memory::Transition::Refresh).is_ok()
                    && parked.prepare(d).is_err()
            })
            .unwrap();
        d.selection_preparation_budget = Some(budget);
        s.cleanup_selection_memories();
        let dbg = s.macro_debug.as_mut().unwrap();
        let base = dbg.base;
        let machine = dbg.machine.as_mut().unwrap();
        assert_eq!(machine.current().unwrap().0, "2");
        machine.executed = crate::macros::run::MAX_EXECUTED_STEPS;
        assert!(s.in_group(s.docs[0].uid));
        let windows = format!("{:?}", (s.active, s.active_window, s.activation, &s.windows));
        let other = format!("{:?}", (super::View::snapshot(&s.docs[1]), &s.docs[1].edits, &s.docs[1].touched));
        let result = s.execute("macro.debug", json!({"action":"step"}));
        assert!(result.is_ok(), "terminal step limit must survive parked-window refusal at budget {budget}: {result:?}");
        let result = result.unwrap();
        assert_eq!(result["state"], "failed");
        assert_eq!(result["executed"], crate::macros::run::MAX_EXECUTED_STEPS + 1 - base);
        assert_eq!(result["error"]["step"], "2");
        assert!(result["message"].as_str().unwrap().contains("ran more than"));
        assert!(s.macro_debug.is_none());
        assert!(!s.in_group(s.docs[0].uid));
        assert!(s.groups.is_empty());
        assert!(std::sync::Arc::ptr_eq(&s.docs[0].wb, &original));
        assert_eq!(s.docs[0].selection, selection);
        assert_eq!((s.docs[0].undo.len(), s.docs[0].redo.len()), history);
        assert!(s.docs.iter().all(|d| d.wb.sheets[0].cells.is_empty()));
        assert_eq!(format!("{:?}", (s.active, s.active_window, s.activation, &s.windows)), windows);
        assert_eq!(format!("{:?}", (super::View::snapshot(&s.docs[1]), &s.docs[1].edits, &s.docs[1].touched)), other);
    }
}

#[cfg(test)]
mod final_e2_tests {
    use super::*;
    use crate::Selection;
    use gridcraft_core::{CellRef, RangeRef};

    fn selected(row: u32) -> Selection {
        Selection {
            active: CellRef::new(row + 1, 5),
            anchor: CellRef::new(row, 1),
            ranges: vec![RangeRef::cell(CellRef::new(row, 1)), RangeRef::cell(CellRef::new(row + 1, 5))],
        }
    }
    fn view_only(mode: &str) {
        let mut s = Session::new();
        s.new_workbook();
        s.execute("home.insertSheet", json!({"name":"B"})).unwrap();
        let mac = crate::macros::parse_macro(&json!({"name":"ViewOnly", "output":{"onConflict":"overwrite"}, "steps":[
            {"command":"selection.set", "params":{"range":"C9:D10,F12:G13", "active":"G13"}},
            {"command":"cell.set", "params":{"cell":"?", "input":"bad"}}
        ]}))
        .unwrap();
        let d = s.doc_mut().unwrap();
        Arc::make_mut(&mut d.wb).macros.push(mac);
        d.selection = selected(6);
        d.sheet_selections = vec![selected(2), selected(4), selected(90), Selection::default()];
        d.refresh_selection_memory().unwrap();
        // Settle history accounting after the fixture installs its macro directly.
        s.execute("sheet.activate", json!({"sheet":1})).unwrap();
        let d = s.doc().unwrap();
        let (a, b) = (d.wb.sheets[0].uid.get(), d.wb.sheets[1].uid.get());
        let wb = d.wb.clone();
        let history = format!("{:?}", (&d.undo, &d.redo));
        s.execute("macro.debug", json!({"name":"ViewOnly"})).unwrap();
        assert_eq!(s.doc().unwrap().wb.active().unwrap().uid.get(), b);
        assert_eq!(s.doc().unwrap().selection, selected(6));
        s.execute("sheet.activate", json!({"sheet":0})).unwrap();
        assert_eq!(s.doc().unwrap().wb.active().unwrap().uid.get(), a);
        assert_eq!(s.doc().unwrap().selection, selected(2));
        s.execute("macro.debug", json!({"action":"step"})).unwrap();
        let stepped = Selection {
            active: CellRef::parse("G13").unwrap(),
            anchor: CellRef::parse("F12").unwrap(),
            ranges: vec![RangeRef::parse("C9:D10").unwrap(), RangeRef::parse("F12:G13").unwrap()],
        };
        assert_eq!(s.doc().unwrap().wb.active().unwrap().uid.get(), b);
        assert_eq!(s.doc().unwrap().selection, stepped);
        s.execute("sheet.activate", json!({"sheet":0})).unwrap();
        assert!(Arc::ptr_eq(&wb, &s.doc().unwrap().wb), "view-only prefix must share the captured workbook");
        assert_eq!(s.doc().unwrap().wb.active().unwrap().uid.get(), a);
        match mode {
            "rollback" => {
                s.execute("macro.debug", json!({"action":"stop"})).unwrap();
            }
            "failure" => {
                assert_eq!(s.execute("macro.debug", json!({"action":"continue"})).unwrap()["state"], "failed");
            }
            "keep" => {
                s.execute("macro.debug", json!({"action":"stop", "keep":true})).unwrap();
            }
            _ => {
                let token = s.macro_debug.as_mut().unwrap().token.take().unwrap();
                assert!(!s.group_end(token).unwrap());
                assert!(s.execute("macro.debug", json!({"action":"state"})).is_err());
            }
        }
        let rolled = matches!(mode, "rollback" | "failure");
        assert_eq!(s.doc().unwrap().wb.active().unwrap().uid.get(), if rolled { b } else { a }, "{mode}: selection owner UID");
        assert_eq!(s.doc().unwrap().selection, if rolled { selected(6) } else { selected(2) });
        assert!(s.macro_debug.is_none() && s.groups.is_empty());
        assert!(Arc::ptr_eq(&wb, &s.doc().unwrap().wb));
        assert_eq!(format!("{:?}", (&s.doc().unwrap().undo, &s.doc().unwrap().redo)), history);
        assert!(s.doc().unwrap().wb.sheets.iter().all(|sh| sh.cells.is_empty()));
        s.execute("sheet.activate", json!({"sheet":0})).unwrap();
        assert_eq!(s.doc().unwrap().wb.active().unwrap().uid.get(), a);
        assert_eq!(s.doc().unwrap().selection, selected(2));
        s.execute("sheet.activate", json!({"sheet":1})).unwrap();
        assert_eq!(s.doc().unwrap().wb.active().unwrap().uid.get(), b);
        assert_eq!(s.doc().unwrap().selection, if rolled { selected(6) } else { stepped });
        assert_eq!(&s.doc().unwrap().sheet_selections[2..], &[selected(90), Selection::default()]);
    }
    #[test]
    fn e2_final_debugger_view_only_stop_restores_captured_uid_and_selection() {
        view_only("rollback");
    }
    #[test]
    fn e2_final_debugger_view_only_failure_keep_and_external_end() {
        for mode in ["failure", "keep", "external"] {
            view_only(mode);
        }
    }
}
