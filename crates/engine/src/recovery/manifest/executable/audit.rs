use super::{Caps, Macro, RecoveryError, RecoveryLimits, Stats, Step, StepKind, charge, count};
use serde_json::Value as Json;

#[derive(Clone, Copy)]
pub(super) enum Role {
    General,
    Macro,
    Steps(usize),
    Step(usize),
    Inputs,
    Input,
    Permissions,
    Columns,
    Output,
    Name,
    InputName,
    Text,
    Reference,
    Shortcut,
    Id,
    IfValue,
}
enum Children<'a> {
    Array(std::slice::Iter<'a, Json>),
    Object(serde_json::map::Iter<'a>),
}
struct Frame<'a> {
    value: &'a Json,
    role: Role,
    children: Children<'a>,
}
#[derive(Clone, Copy)]
struct Steps<'a> {
    list: &'a [Step],
    pos: usize,
    other: Option<&'a [Step]>,
}
pub(super) struct Audit<'a> {
    limits: &'a RecoveryLimits,
    caps: &'a Caps,
    pub(super) nodes: usize,
    pub(super) keys: usize,
    pub(super) bytes: usize,
    pub(super) depth: usize,
    pub(super) steps: usize,
    pub(super) counts: Stats,
}
fn text(value: &str, limit: usize) -> Result<(), RecoveryError> {
    if value.chars().take(limit + 1).count() > limit {
        return Err(RecoveryError::TooLarge("macro_text"));
    }
    Ok(())
}
fn id(value: &str) -> Result<(), RecoveryError> {
    if value.len() > gridcraft_model::MAX_MACRO_NAME {
        return Err(RecoveryError::TooLarge("macro_id"));
    }
    Ok(())
}
impl<'a> Audit<'a> {
    pub(super) fn new(limits: &'a RecoveryLimits, caps: &'a Caps) -> Self {
        Self { limits, caps, nodes: 0, keys: 0, bytes: 0, depth: 0, steps: 0, counts: Stats::default() }
    }
    pub(super) fn measure<T: serde::Serialize + ?Sized>(&mut self, value: &T, chars: usize) -> Result<(), RecoveryError> {
        self.counts.count_passes += 1;
        let measured = count::measure(value, self.limits.manifest, chars)?;
        self.counts.max_count_bytes = self.counts.max_count_bytes.max(measured.bytes);
        self.counts.max_count_chars = self.counts.max_count_chars.max(measured.chars);
        Ok(())
    }
    fn string(&mut self, value: &str) -> Result<(), RecoveryError> {
        charge(&mut self.bytes, value.len(), self.limits.manifest, "strings")
    }
    fn scalar(&self, value: &Json, role: Role) -> Result<(), RecoveryError> {
        if let Some(value) = value.as_str() {
            match role {
                Role::Name => text(value.trim(), gridcraft_model::MAX_MACRO_NAME)?,
                Role::InputName => text(value, gridcraft_model::MAX_MACRO_NAME)?,
                Role::Text => text(value, gridcraft_model::MAX_MACRO_TEXT)?,
                Role::Reference => text(value, gridcraft_model::MAX_MACRO_REF)?,
                Role::Shortcut => text(value, gridcraft_model::MAX_MACRO_SHORTCUT)?,
                Role::Id => id(value)?,
                _ => {}
            }
        }
        Ok(())
    }
    fn field(parent: &Json, role: Role, key: &str) -> Role {
        match (role, key) {
            (Role::Macro, "steps") => Role::Steps(1),
            (Role::Macro, "inputs") => Role::Inputs,
            (Role::Macro, "permissions") => Role::Permissions,
            (Role::Macro, "output") => Role::Output,
            (Role::Macro, "name") => Role::Name,
            (Role::Macro, "description") => Role::Text,
            (Role::Macro, "shortcut") => Role::Shortcut,
            (Role::Step(level), "body" | "then" | "else") => Role::Steps(level + 1),
            (Role::Step(_), "value") if parent.get("type").and_then(Json::as_str) == Some("if") => Role::IfValue,
            (Role::Step(_), "note" | "message") => Role::Text,
            (Role::Step(_), "over" | "cell") => Role::Reference,
            (Role::Step(_), "id") => Role::Id,
            (Role::Input, "name") => Role::InputName,
            (Role::Input, "colTypes") => Role::Columns,
            (Role::Input, "defaultRef") | (Role::Output, "anchor") => Role::Reference,
            _ => Role::General,
        }
    }
    fn element(&mut self, role: Role) -> Result<Role, RecoveryError> {
        Ok(match role {
            Role::Steps(level) => {
                charge(&mut self.steps, 1, self.caps.steps.min(gridcraft_model::MAX_MACRO_STEPS), "macro_steps")?;
                Role::Step(level)
            }
            Role::Inputs => Role::Input,
            Role::Permissions => Role::Id,
            _ => Role::General,
        })
    }
    pub(super) fn json(&mut self, value: &Json, role: Role) -> Result<(), RecoveryError> {
        let mut stack: [Option<Frame<'_>>; 64] = std::array::from_fn(|_| None);
        let mut depth = 0;
        let mut next = Some((value, role));
        loop {
            if let Some((value, role)) = next.take() {
                charge(&mut self.nodes, 1, self.limits.nodes, "nodes")?;
                self.scalar(value, role)?;
                if let Some(text) = value.as_str() {
                    self.string(text)?;
                }
                if let Role::Step(level) = role
                    && value.as_object().is_some_and(|o| ["body", "then", "else"].iter().any(|key| o.contains_key(*key)))
                    && level > self.caps.controls.min(gridcraft_model::MAX_MACRO_DEPTH)
                {
                    return Err(RecoveryError::TooLarge("macro_depth"));
                }
                let children = match value {
                    Json::Array(values) => {
                        let cap = match role {
                            Role::Inputs => 64,
                            Role::Permissions => gridcraft_model::MAX_MACRO_PERMISSIONS,
                            Role::Columns => gridcraft_model::MAX_MACRO_COLUMNS,
                            _ => usize::MAX,
                        };
                        if values.len() > cap {
                            return Err(RecoveryError::TooLarge("macro_items"));
                        }
                        Some(Children::Array(values.iter()))
                    }
                    Json::Object(values) => Some(Children::Object(values.iter())),
                    _ => None,
                };
                if let Some(children) = children {
                    if depth >= self.limits.depth.min(64) {
                        return Err(RecoveryError::TooLarge("depth"));
                    }
                    stack[depth] = Some(Frame { value, role, children });
                    depth += 1;
                    self.depth = self.depth.max(depth);
                } else if matches!(role, Role::IfValue) {
                    self.measure(value, self.caps.if_chars.min(gridcraft_model::MAX_MACRO_TEXT))?;
                }
            }
            if depth == 0 {
                break;
            }
            let frame = stack[depth - 1].as_mut().ok_or(RecoveryError::Malformed("macro_audit"))?;
            next = match &mut frame.children {
                Children::Array(values) => values.next().map(|v| self.element(frame.role).map(|r| (v, r))).transpose()?,
                Children::Object(values) => {
                    if let Some((key, value)) = values.next() {
                        charge(&mut self.nodes, 1, self.limits.nodes, "nodes")?;
                        charge(&mut self.keys, 1, self.limits.nodes, "nodes")?;
                        self.string(key)?;
                        Some((value, Self::field(frame.value, frame.role, key)))
                    } else {
                        None
                    }
                }
            };
            if next.is_none() {
                let frame = stack[depth - 1].take().ok_or(RecoveryError::Malformed("macro_audit"))?;
                depth -= 1;
                if matches!(frame.role, Role::IfValue) {
                    self.measure(frame.value, self.caps.if_chars.min(gridcraft_model::MAX_MACRO_TEXT))?;
                }
            }
        }
        Ok(())
    }
    pub(super) fn typed(&mut self, value: &Macro) -> Result<(), RecoveryError> {
        text(value.name.trim(), gridcraft_model::MAX_MACRO_NAME)?;
        text(&value.description, gridcraft_model::MAX_MACRO_TEXT)?;
        if let Some(value) = &value.shortcut {
            text(value, gridcraft_model::MAX_MACRO_SHORTCUT)?;
        }
        if let Some(value) = &value.output.anchor {
            text(value, gridcraft_model::MAX_MACRO_REF)?;
        }
        if value.inputs.len() > 64 || value.permissions.len() > gridcraft_model::MAX_MACRO_PERMISSIONS {
            return Err(RecoveryError::TooLarge("macro_items"));
        }
        for permission in &value.permissions {
            id(permission)?;
        }
        for input in &value.inputs {
            text(&input.name, gridcraft_model::MAX_MACRO_NAME)?;
            if let Some(value) = &input.default_ref {
                text(value, gridcraft_model::MAX_MACRO_REF)?;
            }
            let maximum = if input.cols[1] == 0 {
                gridcraft_model::MAX_MACRO_COLUMNS
            } else {
                (input.cols[1] as usize).min(gridcraft_model::MAX_MACRO_COLUMNS)
            };
            if input.col_types.len() > maximum {
                return Err(RecoveryError::TooLarge("macro_columns"));
            }
        }
        let empty = Steps { list: &[], pos: 0, other: None };
        let mut stack = [empty; 5];
        stack[0].list = &value.steps;
        let mut depth = 1;
        while depth > 0 {
            let frame = &mut stack[depth - 1];
            if frame.pos == frame.list.len() {
                if let Some(other) = frame.other.take() {
                    frame.list = other;
                    frame.pos = 0;
                    continue;
                }
                depth -= 1;
                continue;
            }
            let step = &frame.list[frame.pos];
            frame.pos += 1;
            charge(&mut self.steps, 1, self.caps.steps.min(gridcraft_model::MAX_MACRO_STEPS), "macro_steps")?;
            text(&step.note, gridcraft_model::MAX_MACRO_TEXT)?;
            let children = match &step.kind {
                StepKind::Command { id: command, params } => {
                    id(command)?;
                    self.json(params, Role::General)?;
                    None
                }
                StepKind::Stop { message, .. } => {
                    text(message, gridcraft_model::MAX_MACRO_TEXT)?;
                    None
                }
                StepKind::Repeat { over, body, .. } => {
                    text(over, gridcraft_model::MAX_MACRO_REF)?;
                    Some((body.as_slice(), None))
                }
                StepKind::If { cell, value, then, otherwise, .. } => {
                    text(cell, gridcraft_model::MAX_MACRO_REF)?;
                    self.json(value, Role::General)?;
                    self.measure(value, self.caps.if_chars.min(gridcraft_model::MAX_MACRO_TEXT))?;
                    Some((then.as_slice(), Some(otherwise.as_slice())))
                }
            };
            if let Some((list, other)) = children {
                if depth > self.caps.controls.min(gridcraft_model::MAX_MACRO_DEPTH) {
                    return Err(RecoveryError::TooLarge("macro_depth"));
                }
                stack[depth] = Steps { list, pos: 0, other };
                depth += 1;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
