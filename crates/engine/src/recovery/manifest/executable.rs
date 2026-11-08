//! Readonly executable checks; staged macros are raw data, never recovery/provenance authority.
mod audit;
mod count;
use super::{Metadata, RecoveryError, RecoveryLimits, Workbook, observe_manifest, preflight::charge, write};
use gridcraft_model::{Macro, Step, StepKind};
use serde_json::Value as Json;

struct Caps {
    steps: usize,
    controls: usize,
    item_chars: usize,
    if_chars: usize,
}
impl Default for Caps {
    fn default() -> Self {
        Self {
            steps: gridcraft_model::MAX_MACRO_STEPS,
            controls: gridcraft_model::MAX_MACRO_DEPTH,
            item_chars: crate::cmd::stash::MAX_ITEM_CHARS,
            if_chars: gridcraft_model::MAX_MACRO_TEXT,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(super) struct Stats {
    observer_calls: usize,
    audit_nodes: usize,
    audit_keys: usize,
    audit_bytes: usize,
    audit_depth: usize,
    step_peak: usize,
    parse_entries: usize,
    check_entries: usize,
    deserialize_entries: usize,
    value_entries: usize,
    count_passes: usize,
    max_count_bytes: usize,
    max_count_chars: usize,
    canonical_buffer: usize,
    canonical_capacity: usize,
    namespace_request: usize,
    namespace_capacity: usize,
    stage_request: usize,
    stage_capacity: usize,
}
pub(super) struct Staged {
    pub(super) adopted: Option<Vec<Macro>>,
    #[cfg(test)]
    pub(super) stats: Stats,
}
fn unsupported(reason: &'static str) -> RecoveryError {
    RecoveryError::UnsupportedRepresentation(reason)
}
fn newer(value: &Json) -> bool {
    value.get("v").and_then(Json::as_u64).is_some_and(|v| v > u64::from(gridcraft_model::MACRO_VERSION))
}
fn item_chars(caps: &Caps) -> usize {
    caps.item_chars.min(crate::cmd::stash::MAX_ITEM_CHARS)
}
fn reserve<T>(values: &mut Vec<T>, count: usize) -> Result<usize, RecoveryError> {
    let bytes =
        count.checked_mul(std::mem::size_of::<T>()).filter(|n| *n <= isize::MAX as usize).ok_or(RecoveryError::TooLarge("macro_allocation"))?;
    values.try_reserve_exact(count).map_err(|_| RecoveryError::TooLarge("macro_allocation"))?;
    Ok(bytes)
}
fn measured<T: serde::Serialize + ?Sized>(value: &T, limits: &RecoveryLimits, caps: &Caps, stats: &mut Stats) -> Result<(), RecoveryError> {
    stats.count_passes += 1;
    let result = count::measure(value, limits.manifest, item_chars(caps))?;
    stats.max_count_bytes = stats.max_count_bytes.max(result.bytes);
    stats.max_count_chars = stats.max_count_chars.max(result.chars);
    Ok(())
}
fn value(value: &Macro, stats: &mut Stats) -> Result<Json, RecoveryError> {
    stats.value_entries += 1;
    serde_json::to_value(value).map_err(|_| unsupported("macro_adoption"))
}
fn equal(original: &Json, adopted: &Macro, limits: &RecoveryLimits, stats: &mut Stats) -> Result<(), RecoveryError> {
    let candidate = value(adopted, stats)?;
    // Both are Json BTree maps here: compare actual scalar writer forms without struct/map order confusion.
    let encoded = write::encode(&candidate, limits.manifest)?;
    stats.canonical_buffer = stats.canonical_buffer.max(encoded.bytes.len());
    stats.canonical_capacity = stats.canonical_capacity.max(encoded.bytes.capacity());
    write::compare(original, &encoded.bytes).map_err(|_| unsupported("macro_adoption"))
}
fn audit_all(workbook: &Workbook, limits: &RecoveryLimits, caps: &Caps, stats: &mut Stats) -> Result<(), RecoveryError> {
    let mut audit = audit::Audit::new(limits, caps);
    let result = (|| {
        for macro_ in &workbook.macros {
            audit.steps = 0;
            audit.typed(macro_)?;
            stats.step_peak = stats.step_peak.max(audit.steps);
            audit.measure(macro_, item_chars(caps))?;
        }
        for pending in &workbook.macros_read {
            audit.steps = 0;
            audit.json(pending, if newer(pending) { audit::Role::General } else { audit::Role::Macro })?;
            stats.step_peak = stats.step_peak.max(audit.steps);
            audit.measure(pending, item_chars(caps))?;
        }
        for future in workbook.macros_unread.iter() {
            audit.json(future, audit::Role::General)?;
            audit.measure(future, item_chars(caps))?;
        }
        for sheet in &workbook.sheets {
            for control in &sheet.controls {
                if let Some(gridcraft_model::ControlAction::Command { params, .. }) = &control.action {
                    audit.json(params, audit::Role::General)?;
                }
            }
        }
        Ok(())
    })();
    stats.audit_nodes = audit.nodes;
    stats.audit_keys = audit.keys;
    stats.audit_bytes = audit.bytes;
    stats.audit_depth = audit.depth;
    stats.step_peak = stats.step_peak.max(audit.steps);
    stats.count_passes += audit.counts.count_passes;
    stats.max_count_bytes = stats.max_count_bytes.max(audit.counts.max_count_bytes);
    stats.max_count_chars = stats.max_count_chars.max(audit.counts.max_count_chars);
    result
}
#[derive(Clone, Copy)]
struct Name<'a> {
    text: &'a str,
    typed: bool,
}
fn compare_names(a: &str, b: &str) -> std::cmp::Ordering {
    a.trim().bytes().map(|b| b.to_ascii_lowercase()).cmp(b.trim().bytes().map(|b| b.to_ascii_lowercase()))
}
fn namespace(typed: &[Macro], future: &[Json], stats: &mut Stats) -> Result<(), RecoveryError> {
    let mut names = Vec::new();
    stats.namespace_request = reserve::<Name<'_>>(&mut names, typed.len() + future.len())?;
    for value in typed {
        names.push(Name { text: &value.name, typed: true });
    }
    // kept_name reserves every actual string name after trim; no future schema/name-validity rule.
    for value in future {
        if let Some(text) = value.get("name").and_then(Json::as_str) {
            names.push(Name { text, typed: false });
        }
    }
    stats.namespace_capacity = names.capacity() * std::mem::size_of::<Name<'_>>();
    names.sort_unstable_by(|a, b| compare_names(a.text, b.text));
    if names.windows(2).any(|pair| compare_names(pair[0].text, pair[1].text).is_eq() && (pair[0].typed || pair[1].typed)) {
        return Err(unsupported("macro_namespace"));
    }
    Ok(())
}
fn run(
    workbook: &Workbook,
    metadata: &Metadata,
    limits: &RecoveryLimits,
    decode: bool,
    caps: &Caps,
    stats: &mut Stats,
) -> Result<Staged, RecoveryError> {
    stats.observer_calls += 1;
    observe_manifest(workbook, metadata, limits, decode)?;
    let current = if workbook.macros_read.is_empty() { workbook.macros.len() } else { workbook.macros_read.len() };
    if current.checked_add(workbook.macros_unread.len()).is_none_or(|n| n > gridcraft_model::MAX_MACROS) {
        return Err(RecoveryError::TooLarge("macros"));
    }
    audit_all(workbook, limits, caps, stats)?;
    for future in workbook.macros_unread.iter() {
        if !newer(future) {
            return Err(unsupported("macro_future"));
        }
    }
    let mut adopted = None;
    if !workbook.macros_read.is_empty() {
        let mut stage = Vec::new();
        stats.stage_request = reserve(&mut stage, workbook.macros_read.len())?;
        stats.stage_capacity = stage.capacity() * std::mem::size_of::<Macro>();
        for original in &workbook.macros_read {
            if newer(original) {
                return Err(unsupported("macro_channel"));
            }
            // Ordinary derived deserialization is staged only after bounded raw audits. No macro helper yet.
            stats.deserialize_entries += 1;
            let decoded: Macro = serde_json::from_value(original.clone()).map_err(|_| unsupported("macro_adoption"))?;
            let mut audit = audit::Audit::new(limits, caps);
            let result = audit.typed(&decoded);
            stats.count_passes += audit.counts.count_passes;
            stats.max_count_bytes = stats.max_count_bytes.max(audit.counts.max_count_bytes);
            stats.max_count_chars = stats.max_count_chars.max(audit.counts.max_count_chars);
            result?;
            measured(&decoded, limits, caps, stats)?;
            // Refuse inserted defaults/ignored fields before parse_macro's allocating check_size.
            equal(original, &decoded, limits, stats)?;
            drop(decoded);
            stats.parse_entries += 1;
            let parsed = crate::macros::parse_macro(original).map_err(|_| unsupported("macro_adoption"))?;
            equal(original, &parsed, limits, stats)?;
            stage.push(parsed);
        }
        namespace(&stage, &workbook.macros_unread, stats)?;
        adopted = Some(stage);
    } else {
        for macro_ in &workbook.macros {
            stats.check_entries += 1;
            crate::macros::check(macro_).map_err(|_| unsupported("macro_adoption"))?;
            let original = value(macro_, stats)?;
            stats.parse_entries += 1;
            let parsed = crate::macros::parse_macro(&original).map_err(|_| unsupported("macro_adoption"))?;
            equal(&original, &parsed, limits, stats)?;
        }
        namespace(&workbook.macros, &workbook.macros_unread, stats)?;
    }
    Ok(Staged {
        adopted,
        #[cfg(test)]
        stats: *stats,
    })
}
pub(super) fn check(workbook: &Workbook, metadata: &Metadata, limits: &RecoveryLimits, decode: bool) -> Result<Staged, RecoveryError> {
    run(workbook, metadata, limits, decode, &Caps::default(), &mut Stats::default())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(super) fn measure(workbook: &Workbook, metadata: &Metadata, limits: &RecoveryLimits, steps: usize) -> (Result<(), RecoveryError>, Json) {
    let mut stats = Stats::default();
    let result = run(workbook, metadata, limits, true, &Caps { steps, ..Caps::default() }, &mut stats);
    (result.map(|_| ()), serde_json::to_value(stats).unwrap())
}
