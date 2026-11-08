//! Lexical budget guard followed by bounded structural preflight; typed recovery remains a later stage.

mod duplicates;
mod pictures;
mod scratch;

use super::context::{Context, KeyKind, Kind};
use crate::recovery::{RecoveryError, RecoveryLimits};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(super) struct AuditStats {
    pub(super) nodes: usize,
    pub(super) keys: usize,
    pub(super) decoded_bytes: usize,
    pub(super) max_string: usize,
    pub(super) number_bytes: usize,
    pub(super) max_number: usize,
    pub(super) max_depth: usize,
    pub(super) sheets: usize,
    pub(super) cells: usize,
    pub(super) parts: usize,
    pub(super) relationships: usize,
    pub(super) native_bytes: usize,
    pub(super) groups: usize,
}

#[derive(Clone, Copy)]
struct Frame {
    context: Context,
    child: Context,
    object: bool,
    state: u8,
    items: usize,
}
const EMPTY: Frame = Frame { context: Context::GENERAL, child: Context::GENERAL, object: false, state: 0, items: 0 };

// A long key is never truncated into a known label: only complete labels <=32 bytes match.
struct Label {
    bytes: [u8; 32],
    len: usize,
}
impl Label {
    fn text(&self) -> Option<&str> {
        self.bytes.get(..self.len).and_then(|bytes| std::str::from_utf8(bytes).ok())
    }
}

fn malformed(reason: &'static str) -> RecoveryError {
    RecoveryError::Malformed(reason)
}
pub(super) fn charge(value: &mut usize, amount: usize, limit: usize, reason: &'static str) -> Result<(), RecoveryError> {
    let next = value.checked_add(amount).filter(|n| *n <= limit).ok_or(RecoveryError::TooLarge(reason))?;
    *value = next;
    Ok(())
}

fn canonical_key(context: Context, label: &Label, raw: &[u8]) -> Result<(), RecoveryError> {
    match context.key_kind() {
        Some(KeyKind::Decimal) => {
            if !raw.iter().all(u8::is_ascii_digit) || (raw.starts_with(b"0") && raw.len() != 1) {
                return Err(RecoveryError::UnsupportedRepresentation("numeric_key"));
            }
            let valid = std::str::from_utf8(raw).ok().and_then(|text| text.parse::<u32>().ok()).is_some();
            if !valid {
                return Err(malformed("numeric_key"));
            }
        }
        Some(KeyKind::A1) => {
            // Every valid grid A1 fits the fixed label; long arbitrary General keys are unaffected.
            let text = label.text().ok_or_else(|| malformed("a1_key"))?;
            let split = text.bytes().position(|byte| byte.is_ascii_digit()).unwrap_or(text.len());
            let (column, row) = text.as_bytes().split_at(split);
            if column.is_empty()
                || !column.iter().all(u8::is_ascii_uppercase)
                || row.is_empty()
                || row[0] == b'0'
                || !row.iter().all(u8::is_ascii_digit)
            {
                return Err(RecoveryError::UnsupportedRepresentation("a1_key"));
            }
            if gridcraft_core::CellRef::parse(text).is_none() {
                return Err(malformed("a1_key"));
            }
        }
        None => {}
    }
    Ok(())
}

pub(super) fn lexical_audit(bytes: &[u8], limits: &RecoveryLimits) -> Result<AuditStats, RecoveryError> {
    let mut scanner = Scanner::new(bytes, limits);
    scanner.run()?;
    Ok(scanner.stats)
}

pub(super) fn item(context: Context, count: usize, stats: &mut AuditStats, limits: &RecoveryLimits) -> Result<(), RecoveryError> {
    use Kind::*;
    match context {
        Context::List(Sheet | KeptSheet) => charge(&mut stats.sheets, 1, limits.sheets, "sheets")?,
        Context::One(StoredCols | Spill) => charge(&mut stats.cells, 1, limits.cells, "cells")?,
        Context::List(Relationship) => {
            if count > limits.owner_relationships {
                return Err(RecoveryError::TooLarge("owner_relationships"));
            }
            charge(&mut stats.relationships, 1, limits.relationships, "relationships")?;
        }
        Context::List(U8 | U16) => {
            let width = if context == Context::List(U16) { 2 } else { 1 };
            if count.checked_mul(width).is_none_or(|n| n > limits.native_units) {
                return Err(RecoveryError::TooLarge("native_units"));
            }
            charge(&mut stats.native_bytes, width, usize::MAX, "native_units")?;
        }
        Context::List(Group) => charge(&mut stats.groups, 1, limits.nodes, "nodes")?,
        Context::One(NativePath) if count > 1 => return Err(malformed("native_path")),
        _ => {}
    }
    Ok(())
}

pub(super) fn string_bytes(
    length: &mut usize,
    amount: usize,
    context: Context,
    stats: &mut AuditStats,
    limits: &RecoveryLimits,
) -> Result<(), RecoveryError> {
    let (limit, reason) = match context {
        Context::One(Kind::Metadata) => (limits.metadata, "metadata"),
        Context::One(Kind::Xml) => (limits.xml, "xml"),
        _ => (limits.manifest, "strings"),
    };
    charge(length, amount, limit, reason)?;
    charge(&mut stats.decoded_bytes, amount, limits.manifest, "strings")
}

pub(super) fn preflight_manifest(bytes: &[u8], limits: &RecoveryLimits) -> Result<(), RecoveryError> {
    lexical_audit(bytes, limits)?;
    duplicates::audit(bytes, limits)?;
    Ok(())
}

struct Scanner<'a> {
    bytes: &'a [u8],
    limits: &'a RecoveryLimits,
    pos: usize,
    depth: usize,
    stack: [Frame; 64],
    stats: AuditStats,
}
impl<'a> Scanner<'a> {
    fn new(bytes: &'a [u8], limits: &'a RecoveryLimits) -> Self {
        Self { bytes, limits, pos: 0, depth: 0, stack: [EMPTY; 64], stats: AuditStats::default() }
    }
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }
    fn run(&mut self) -> Result<(), RecoveryError> {
        if self.bytes.len() > self.limits.manifest {
            return Err(RecoveryError::TooLarge("manifest"));
        }
        let mut root = false;
        loop {
            while matches!(self.peek(), Some(b' ' | b'\r' | b'\n' | b'\t')) {
                self.pos += 1;
            }
            let Some(byte) = self.peek() else { break };
            if self.depth == 0 {
                if root {
                    return Err(malformed("manifest_trailing"));
                }
                root = true;
                self.value(Context::ROOT)?;
                continue;
            }
            let top = self.depth - 1;
            let mut frame = self.stack[top];
            match (frame.object, frame.state) {
                (true, 0 | 1) => {
                    if byte == b'}' && frame.state == 0 {
                        self.close(frame)?;
                        continue;
                    }
                    if byte != b'"' {
                        return Err(malformed("manifest_key"));
                    }
                    charge(&mut self.stats.nodes, 1, self.limits.nodes, "nodes")?;
                    charge(&mut self.stats.keys, 1, self.limits.nodes, "nodes")?;
                    let start = self.pos;
                    let label = self.string(Context::GENERAL)?;
                    canonical_key(frame.context, &label, &self.bytes[start + 1..self.pos - 1])?;
                    frame.child = frame.context.field(label.text().unwrap_or(""));
                    if frame.context == Context::One(Kind::NativePath) && frame.child == Context::GENERAL {
                        return Err(malformed("native_path"));
                    }
                    frame.state = 2;
                }
                (true, 2) => {
                    if byte != b':' {
                        return Err(malformed("manifest_colon"));
                    }
                    self.pos += 1;
                    frame.state = 3;
                }
                (false, 0) | (_, 3) => {
                    if !frame.object && frame.state == 0 && byte == b']' {
                        self.close(frame)?;
                        continue;
                    }
                    let child = if frame.object { frame.child } else { frame.context.element(frame.items) };
                    self.item(&mut frame)?;
                    frame.state = 4;
                    self.stack[top] = frame;
                    self.value(child)?;
                    continue;
                }
                (_, 4) => {
                    if byte == if frame.object { b'}' } else { b']' } {
                        self.close(frame)?;
                        continue;
                    }
                    if byte != b',' {
                        return Err(malformed("manifest_comma"));
                    }
                    self.pos += 1;
                    frame.state = if frame.object { 1 } else { 3 };
                }
                _ => return Err(malformed("manifest_state")),
            }
            self.stack[top] = frame;
        }
        if !root || self.depth != 0 {
            return Err(malformed("manifest_syntax"));
        }
        Ok(())
    }
    fn close(&mut self, frame: Frame) -> Result<(), RecoveryError> {
        if frame.context == Context::One(Kind::NativePath) && frame.items != 1 {
            return Err(malformed("native_path"));
        }
        self.pos += 1;
        self.depth -= 1;
        Ok(())
    }
    fn item(&mut self, frame: &mut Frame) -> Result<(), RecoveryError> {
        charge(&mut frame.items, 1, self.limits.nodes, "nodes")?;
        item(frame.context, frame.items, &mut self.stats, self.limits)
    }
    fn value(&mut self, context: Context) -> Result<(), RecoveryError> {
        use Kind::*;
        let byte = self.peek().ok_or_else(|| malformed("manifest_value"))?;
        charge(&mut self.stats.nodes, 1, self.limits.nodes, "nodes")?;
        if matches!(context, Context::List(_)) && byte != b'[' {
            return Err(malformed("manifest_shape"));
        }
        if context == Context::One(NativePath) && !matches!(byte, b'{' | b'n') {
            return Err(malformed("native_path"));
        }
        if matches!(context, Context::One(U8 | U16 | Group)) && !byte.is_ascii_digit() {
            return Err(malformed("unsigned_scalar"));
        }
        if context == Context::One(Part) || (context == Context::One(OptionalPart) && byte != b'n') {
            charge(&mut self.stats.parts, 1, self.limits.parts, "parts")?;
        }
        match byte {
            b'{' | b'[' => {
                if self.depth >= self.limits.depth || self.depth == self.stack.len() {
                    return Err(RecoveryError::TooLarge("depth"));
                }
                self.stack[self.depth] = Frame { context, object: byte == b'{', ..EMPTY };
                self.depth += 1;
                self.stats.max_depth = self.stats.max_depth.max(self.depth);
                self.pos += 1;
            }
            b'"' => {
                self.string(context)?;
            }
            b'-' | b'0'..=b'9' => self.number(context)?,
            b'n' => self.literal(b"null")?,
            b't' => self.literal(b"true")?,
            b'f' => self.literal(b"false")?,
            _ => return Err(malformed("manifest_value")),
        }
        Ok(())
    }
    fn literal(&mut self, literal: &[u8]) -> Result<(), RecoveryError> {
        if !self.bytes.get(self.pos..).is_some_and(|bytes| bytes.starts_with(literal)) {
            return Err(malformed("manifest_literal"));
        }
        self.pos += literal.len();
        Ok(())
    }
    fn append(&mut self, label: &mut Label, bytes: &[u8], context: Context) -> Result<(), RecoveryError> {
        let start = label.len;
        string_bytes(&mut label.len, bytes.len(), context, &mut self.stats, self.limits)?;
        for (i, byte) in bytes.iter().enumerate() {
            if let Some(slot) = label.bytes.get_mut(start + i) {
                *slot = *byte;
            }
        }
        Ok(())
    }
    fn string(&mut self, context: Context) -> Result<Label, RecoveryError> {
        let mut label = Label { bytes: [0; 32], len: 0 };
        self.pos += 1;
        loop {
            let byte = self.peek().ok_or_else(|| malformed("manifest_string"))?;
            if byte == b'"' {
                self.pos += 1;
                self.stats.max_string = self.stats.max_string.max(label.len);
                return Ok(label);
            }
            if byte == b'\\' {
                self.pos += 1;
                let escape = self.peek().ok_or_else(|| malformed("manifest_escape"))?;
                self.pos += 1;
                let scalar = match escape {
                    b'"' | b'\\' | b'/' => escape as char,
                    b'b' => '\u{8}',
                    b'f' => '\u{c}',
                    b'n' => '\n',
                    b'r' => '\r',
                    b't' => '\t',
                    b'u' => {
                        let first = self.hex4()?;
                        let code = if (0xd800..=0xdbff).contains(&first) {
                            if !self.bytes.get(self.pos..).is_some_and(|bytes| bytes.starts_with(b"\\u")) {
                                return Err(malformed("manifest_surrogate"));
                            }
                            self.pos += 2;
                            let second = self.hex4()?;
                            if !(0xdc00..=0xdfff).contains(&second) {
                                return Err(malformed("manifest_surrogate"));
                            }
                            0x10000 + (u32::from(first) - 0xd800) * 1024 + u32::from(second) - 0xdc00
                        } else {
                            u32::from(first)
                        };
                        char::from_u32(code).ok_or_else(|| malformed("manifest_surrogate"))?
                    }
                    _ => return Err(malformed("manifest_escape")),
                };
                let mut encoded = [0; 4];
                self.append(&mut label, scalar.encode_utf8(&mut encoded).as_bytes(), context)?;
            } else {
                if byte < 32 {
                    return Err(malformed("manifest_control"));
                }
                let width = match byte {
                    0..=0x7f => 1,
                    0xc2..=0xdf => 2,
                    0xe0..=0xef => 3,
                    0xf0..=0xf4 => 4,
                    _ => return Err(malformed("manifest_utf8")),
                };
                let bytes = self.bytes.get(self.pos..).and_then(|bytes| bytes.get(..width)).ok_or_else(|| malformed("manifest_utf8"))?;
                std::str::from_utf8(bytes).map_err(|_| malformed("manifest_utf8"))?;
                self.append(&mut label, bytes, context)?;
                self.pos += width;
            }
        }
    }
    fn hex4(&mut self) -> Result<u16, RecoveryError> {
        let bytes = self.bytes.get(self.pos..).and_then(|bytes| bytes.get(..4)).ok_or_else(|| malformed("manifest_escape"))?;
        let mut value = 0u16;
        for byte in bytes {
            let digit = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => return Err(malformed("manifest_escape")),
            };
            value = value * 16 + u16::from(digit);
        }
        self.pos += 4;
        Ok(value)
    }
    fn digits(&mut self) -> Result<(), RecoveryError> {
        let start = self.pos;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.pos == start {
            return Err(malformed("manifest_number"));
        }
        Ok(())
    }
    fn number(&mut self, context: Context) -> Result<(), RecoveryError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        if self.peek() == Some(b'0') {
            self.pos += 1;
        } else {
            self.digits()?;
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            self.digits()?;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            self.digits()?;
        }
        let length = self.pos - start;
        charge(&mut self.stats.number_bytes, length, self.limits.manifest, "manifest")?;
        self.stats.max_number = self.stats.max_number.max(length);
        let bound = match context {
            Context::One(Kind::U8) => Some(255),
            Context::One(Kind::U16) => Some(65535),
            Context::One(Kind::Group) => Some(u32::MAX),
            _ => None,
        };
        if let Some(bound) = bound {
            let value = std::str::from_utf8(&self.bytes[start..self.pos]).ok().and_then(|text| text.parse::<u32>().ok());
            if value.is_none_or(|value| value > bound) {
                return Err(malformed("unsigned_scalar"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(super) fn measure(bytes: &[u8], limits: &RecoveryLimits) -> (Result<(), RecoveryError>, serde_json::Value) {
    let mut scanner = Scanner::new(bytes, limits);
    let result = scanner.run();
    let mut metrics = serde_json::json!({"lexical":scanner.stats,"structural_entries":0,"structural":null});
    if let Err(error) = result {
        return (Err(error), metrics);
    }
    metrics["structural_entries"] = serde_json::json!(1);
    match duplicates::audit(bytes, limits) {
        Ok(stats) => {
            metrics["structural"] = serde_json::to_value(stats).unwrap();
            (Ok(()), metrics)
        }
        Err(error) => (Err(error), metrics),
    }
}
