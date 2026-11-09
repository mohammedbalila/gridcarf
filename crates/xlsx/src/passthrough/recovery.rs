//! Effect-free validation of bounded, writer-representable preserved XLSX graphs.

use std::cmp::Ordering;
use std::collections::VecDeque;

use gridcraft_model::{Preserved, PreservedPart, PreservedRel, Workbook};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Finite recovery refusal reasons. No paths, XML or relationship payloads are retained.
/// `Unsupported*` denotes an excluded representation; `*Limit`, `TextLimit`, `ByteLimit`
/// and `Allocation` denote bounded-work/storage refusal. Other variants denote integrity errors.
pub enum RecoveryPreservedError {
    Slot,
    Xml,
    UnsupportedXml,
    XmlLimit,
    Vml,
    VmlLimit,
    ChartSheet,
    OutputName,
    WorkLimit,
    UnsupportedNative,
    PartName,
    UnsupportedPartName,
    TextLimit,
    Allocation,
    DuplicatePart,
    DuplicateRelationship,
    Relationship,
    Owner,
    MissingTarget,
    UnreachablePart,
    ChainLimit,
    PartLimit,
    RelationshipLimit,
    ByteLimit,
    SheetLimit,
}

pub(crate) use RecoveryPreservedError as Error;
pub(crate) type Checked<T> = Result<T, Error>;
pub(crate) const MAX_TEXT: usize = 32 * 1024;

fn ucschar(c: char) -> bool {
    matches!(c as u32, 0xA0..=0xD7FF | 0xF900..=0xFDCF | 0xFDF0..=0xFFEF
        | 0x10000..=0x1FFFD | 0x20000..=0x2FFFD | 0x30000..=0x3FFFD
        | 0x40000..=0x4FFFD | 0x50000..=0x5FFFD | 0x60000..=0x6FFFD
        | 0x70000..=0x7FFFD | 0x80000..=0x8FFFD | 0x90000..=0x9FFFD
        | 0xA0000..=0xAFFFD | 0xB0000..=0xBFFFD | 0xC0000..=0xCFFFD
        | 0xD0000..=0xDFFFD | 0xE1000..=0xEFFFD)
}

fn iunreserved(c: char) -> bool {
    c.is_ascii_alphanumeric() || "-._~".contains(c) || ucschar(c)
}

/// Model-relative OPC spelling; the returned writer identity never decodes or folds Unicode.
pub(crate) fn part_key(name: &str) -> Checked<String> {
    part_name(name)?;
    let mut key = String::new();
    key.try_reserve_exact(name.len()).map_err(|_| Error::Allocation)?;
    key.push_str(name);
    key.make_ascii_lowercase();
    Ok(key)
}

fn part_name(name: &str) -> Checked<()> {
    if name.len() > MAX_TEXT {
        return Err(Error::TextLimit);
    }
    if name.is_empty() || name.starts_with('/') || name.contains('\\') {
        return Err(Error::PartName);
    }
    for segment in name.split('/') {
        if segment.is_empty() || segment.ends_with('.') {
            return Err(Error::PartName);
        }
        let mut chars = segment.chars();
        while let Some(c) = chars.next() {
            if c == '%' {
                if !chars.next().is_some_and(|c| c.is_ascii_hexdigit()) || !chars.next().is_some_and(|c| c.is_ascii_hexdigit()) {
                    return Err(Error::PartName);
                }
            } else if !iunreserved(c) && !"!$&'()*+,;=:@".contains(c) {
                return Err(Error::PartName);
            }
        }
    }
    check_percent(name)?;
    if let Some((scheme, _)) = name.split('/').next().and_then(|s| s.split_once(':'))
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    {
        return Err(Error::UnsupportedPartName);
    }
    let mut segments = name.rsplit('/');
    if segments.next().is_some_and(|s| s.get(s.len().saturating_sub(5)..).is_some_and(|s| s.eq_ignore_ascii_case(".rels")))
        && segments.next().is_some_and(|s| s.eq_ignore_ascii_case("_rels"))
    {
        return Err(Error::PartName);
    }
    Ok(())
}

fn check_percent(name: &str) -> Checked<()> {
    let bytes = name.as_bytes();
    let octet = |at: usize| {
        bytes.get(at..at + 3).filter(|s| s[0] == b'%').and_then(|s| std::str::from_utf8(&s[1..]).ok()).and_then(|s| u8::from_str_radix(s, 16).ok())
    };
    let mut at = 0;
    while at < bytes.len() {
        if bytes.get(at) != Some(&b'%') {
            at += 1;
            continue;
        }
        let first = octet(at).ok_or(Error::PartName)?;
        if first.is_ascii() {
            if iunreserved(char::from(first)) || matches!(first, b'/' | b'\\') {
                return Err(Error::PartName);
            }
            if first.is_ascii_control() {
                return Err(Error::UnsupportedPartName);
            }
            at += 3;
        } else {
            let count = match first {
                0xC2..=0xDF => 2,
                0xE0..=0xEF => 3,
                0xF0..=0xF4 => 4,
                _ => return Err(Error::UnsupportedPartName),
            };
            let mut scalar = [0u8; 4];
            for (offset, byte) in scalar.iter_mut().take(count).enumerate() {
                *byte = octet(at + offset * 3).ok_or(Error::UnsupportedPartName)?;
            }
            if std::str::from_utf8(&scalar[..count]).ok().and_then(|s| s.chars().next()).is_some_and(iunreserved) {
                return Err(Error::PartName);
            }
            return Err(Error::UnsupportedPartName);
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) struct Limits {
    pub bytes: usize,
    pub part_bytes: usize,
    pub parts: usize,
    pub rels: usize,
    pub total_rels: usize,
    pub sheets: usize,
    pub text: usize,
    pub fragment: usize,
}

pub(crate) const LIMITS: Limits = Limits {
    bytes: super::MAX_KEPT_TOTAL,
    part_bytes: super::MAX_KEPT_PART as usize,
    parts: super::MAX_KEPT_PARTS,
    rels: super::MAX_RELS,
    total_rels: 100_000,
    sheets: 1024,
    text: MAX_TEXT,
    fragment: super::MAX_FRAGMENT,
};

fn text(value: &str, cap: usize, nonempty: bool) -> Checked<()> {
    if value.len() > cap {
        return Err(Error::TextLimit);
    }
    if (nonempty && value.is_empty())
        || !value.chars().all(|c| matches!(c as u32, 9 | 10 | 13 | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF))
    {
        return Err(Error::Relationship);
    }
    Ok(())
}

/// Raw live-model guards precede index scratch; no model clone or graph deduplication.
pub(crate) fn preflight(wb: &Workbook, limits: Limits) -> Checked<()> {
    if wb.sheets.len() > limits.sheets {
        return Err(Error::SheetLimit);
    }
    let (mut bytes, mut parts, mut rels, mut sheets) = (0, 0, 0, wb.sheets.len());
    for (p, workbook) in wb.preserved.iter().map(|p| (p, true)).chain(wb.sheets.iter().filter_map(|s| s.preserved.as_ref()).map(|p| (p, false))) {
        if (workbook && p.vml.is_some()) || (!workbook && (!p.package_rels.is_empty() || !p.sheets.is_empty())) {
            return Err(Error::Owner);
        }
        if p.vml.as_ref().is_some_and(|q| q.data.len() > super::MAX_META as usize) {
            return Err(Error::ByteLimit);
        }
        charge(&mut sheets, p.sheets.len(), limits.sheets, Error::SheetLimit)?;
        charge(&mut parts, p.parts.len(), limits.parts, Error::PartLimit)?;
        charge(&mut parts, usize::from(p.vml.is_some()), limits.parts, Error::PartLimit)?;
        if p.elements.len() > 5 {
            return Err(Error::Owner);
        }
        for list in std::iter::once(&p.package_rels).chain(std::iter::once(&p.rels)).chain(p.parts.iter().chain(p.vml.iter()).map(|q| &q.rels)) {
            if list.len() > limits.rels {
                return Err(Error::RelationshipLimit);
            }
            charge(&mut rels, list.len(), limits.total_rels, Error::RelationshipLimit)?;
            for r in list {
                for s in [&r.id, &r.rel_type, &r.target] {
                    text(s, limits.text, true)?;
                }
            }
        }
        for q in p.parts.iter().chain(p.vml.iter()) {
            if q.data.len() > limits.part_bytes {
                return Err(Error::ByteLimit);
            }
            charge(&mut bytes, q.data.len(), limits.bytes, Error::ByteLimit)?;
            text(&q.name, limits.text, true)?;
            text(&q.content_type, limits.text, true)?;
        }
        for e in &p.elements {
            text(&e.slot, limits.text, true)?;
            if e.xml.len() > limits.fragment {
                return Err(Error::TextLimit);
            }
        }
        for s in &p.sheets {
            text(&s.name, limits.text, true)?;
            text(&s.rel_id, limits.text, true)?;
        }
        owner_slots(p, workbook)?;
    }
    Ok(())
}

fn owner_slots(p: &Preserved, workbook: bool) -> Checked<()> {
    let slots: &[&str] =
        if workbook { &["externalReferences", "fileVersion", "ext"] } else { &["pageSetup", "legacyDrawingHF", "picture", "oleObjects", "controls"] };
    if p.elements.len() > slots.len() {
        return Err(Error::Owner);
    }
    let mut used = 0u8;
    for e in &p.elements {
        let bit = 1 << slots.iter().position(|s| *s == e.slot).ok_or(Error::Owner)?;
        if used & bit != 0 {
            return Err(Error::Owner);
        }
        used |= bit;
    }
    if p.package_rels.iter().any(|r| !super::is_kind(r, super::PACKAGE_ROOTS)) {
        return Err(Error::Owner);
    }
    for r in &p.rels {
        let allowed = if workbook {
            super::is_kind(r, super::WORKBOOK_ROOTS) || super::is_kind(r, &["chartsheet", "externalLink"])
        } else {
            super::is_kind(r, super::SHEET_ROOTS)
                || super::is_kind(r, super::SHEET_ELEMENT_KINDS)
                || (super::is_kind(r, &["vmlDrawing"]) && p.elements.iter().any(|e| e.slot == "legacyDrawingHF"))
        };
        if !allowed {
            return Err(Error::Owner);
        }
    }
    Ok(())
}

pub(crate) fn charge(total: &mut usize, amount: usize, cap: usize, error: Error) -> Checked<()> {
    *total = total.checked_add(amount).filter(|n| *n <= cap).ok_or(error)?;
    Ok(())
}

pub(crate) fn reserve<T>(items: &mut Vec<T>, count: usize) -> Checked<()> {
    items.try_reserve_exact(count).map_err(|_| Error::Allocation)
}

fn rel_index(rels: &[PreservedRel]) -> Checked<Vec<&PreservedRel>> {
    if rels.len() > super::MAX_RELS {
        return Err(Error::RelationshipLimit);
    }
    for r in rels {
        for s in [&r.id, &r.rel_type, &r.target] {
            text(s, MAX_TEXT, true)?;
        }
    }
    let mut result = Vec::new();
    reserve(&mut result, rels.len())?;
    result.extend(rels);
    result.sort_unstable_by_key(|r| &r.id);
    if result.windows(2).any(|w| w[0].id == w[1].id) {
        return Err(Error::DuplicateRelationship);
    }
    Ok(result)
}

pub(crate) struct Part<'a> {
    key: String,
    pub part: &'a PreservedPart,
    pub rels: Vec<&'a PreservedRel>,
}

pub(crate) struct OwnerIndex<'a> {
    pub parts: Vec<Part<'a>>,
    // Sorted lookup indices; traversal roots must retain their original stored order.
    pub rels: Vec<&'a PreservedRel>,
    pub package: Vec<&'a PreservedRel>,
    pub vml_rels: Vec<&'a PreservedRel>,
}

impl<'a> OwnerIndex<'a> {
    /// Build one local closure after the complete workbook's raw preflight.
    pub fn new(p: &'a Preserved) -> Checked<Self> {
        if p.parts.len().checked_add(usize::from(p.vml.is_some())).is_none_or(|n| n > super::MAX_KEPT_PARTS) {
            return Err(Error::PartLimit);
        }
        let mut parts = Vec::new();
        reserve(&mut parts, p.parts.len())?;
        for q in &p.parts {
            parts.push(Part { key: part_key(&q.name)?, part: q, rels: rel_index(&q.rels)? });
        }
        parts.sort_unstable_by(|a, b| a.key.cmp(&b.key));
        let vml_key = p.vml.as_ref().map(|q| part_key(&q.name)).transpose()?;
        let mut keys = Vec::new();
        reserve(&mut keys, parts.len() + usize::from(vml_key.is_some()))?;
        keys.extend(parts.iter().map(|q| q.key.as_str()));
        keys.extend(vml_key.as_deref());
        keys.sort_unstable();
        if keys.windows(2).any(|w| w[0] == w[1]) {
            return Err(Error::DuplicatePart);
        }
        for key in &keys {
            // Search for key + '/' without allocating it or rescanning every path prefix.
            let at = keys.partition_point(|candidate| candidate.bytes().cmp(key.bytes().chain(std::iter::once(b'/'))).is_lt());
            if keys.get(at).is_some_and(|candidate| candidate.strip_prefix(key).is_some_and(|tail| tail.starts_with('/'))) {
                return Err(Error::DuplicatePart);
            }
        }
        Ok(Self {
            parts,
            rels: rel_index(&p.rels)?,
            package: rel_index(&p.package_rels)?,
            vml_rels: rel_index(p.vml.as_ref().map_or(&[], |q| q.rels.as_slice()))?,
        })
    }

    pub fn part(&self, name: &str) -> Option<&Part<'a>> {
        self.parts.binary_search_by(|p| ascii_cmp(&p.key, name)).ok().and_then(|i| self.parts.get(i))
    }
}

fn ascii_cmp(a: &str, b: &str) -> Ordering {
    a.bytes().map(|b| b.to_ascii_lowercase()).cmp(b.bytes().map(|b| b.to_ascii_lowercase()))
}

#[derive(Clone, Copy)]
#[repr(u8)]
pub(crate) enum Allowance {
    Ordinary = 1,
    Chart = 2,
    HeaderVml = 4,
}

impl OwnerIndex<'_> {
    /// Caller supplies ordered roots obtained from owner slots, including separate VML edges.
    /// This private graph result does not establish XML/slot/writer representability.
    pub fn walk<'r>(&self, roots: impl IntoIterator<Item = (&'r PreservedRel, Allowance)>) -> Checked<Vec<u8>> {
        let mut visited = Vec::new();
        reserve(&mut visited, self.parts.len())?;
        visited.resize(self.parts.len(), 0);
        let mut queue = VecDeque::new();
        queue.try_reserve_exact(self.parts.len()).map_err(|_| Error::Allocation)?;
        for (root, allowance) in roots {
            self.enqueue(root, 0, allowance, &mut visited, &mut queue)?;
            while let Some((index, depth, allowance)) = queue.pop_front() {
                let part = self.parts.get(index).ok_or(Error::MissingTarget)?;
                for rel in &part.rels {
                    self.enqueue(rel, depth.checked_add(1).ok_or(Error::ChainLimit)?, allowance, &mut visited, &mut queue)?;
                }
            }
        }
        if visited.contains(&0) {
            return Err(Error::UnreachablePart);
        }
        Ok(visited)
    }

    fn enqueue(
        &self,
        rel: &PreservedRel,
        depth: usize,
        allowance: Allowance,
        visited: &mut [u8],
        queue: &mut VecDeque<(usize, usize, Allowance)>,
    ) -> Checked<()> {
        if rel.external {
            return Ok(());
        }
        let kind = super::kind(&rel.rel_type);
        let permitted = match allowance {
            Allowance::Ordinary => false,
            Allowance::Chart => super::CHART_SHEET_GRAPH.iter().any(|k| k.eq_ignore_ascii_case(kind)),
            Allowance::HeaderVml => kind.eq_ignore_ascii_case("vmlDrawing"),
        };
        if !permitted && super::REWRITTEN.iter().any(|k| k.eq_ignore_ascii_case(kind)) {
            return Err(Error::Relationship);
        }
        part_name(&rel.target)?;
        let index = self.parts.binary_search_by(|p| ascii_cmp(&p.key, &rel.target)).map_err(|_| Error::MissingTarget)?;
        let state = visited.get_mut(index).ok_or(Error::MissingTarget)?;
        let bit = allowance as u8;
        if *state & bit == 0 {
            if depth > super::MAX_CHAIN {
                return Err(Error::ChainLimit);
            }
            *state |= bit;
            queue.push_back((index, depth, allowance));
        }
        Ok(())
    }
}

pub(crate) mod claims;

pub(crate) use super::references::xml as scanner;

pub(crate) mod xml;

/// Validates preserved owner closures and their actual output identities for `write_xlsx`.
/// Borrows the workbook without mutation, path access, output serialization or link resolution.
/// This checks the supported bounded XML representation and writer-sensitive references,
/// slots, VML and chartsheets; it does not claim full OOXML/XSD or modeled-feature validation.
pub fn validate_recovery_preserved(wb: &Workbook) -> Result<(), RecoveryPreservedError> {
    if wb.preserved.iter().chain(wb.sheets.iter().filter_map(|sheet| sheet.preserved.as_ref())).any(|owner| owner.native_analysis.is_some()) {
        return Err(Error::UnsupportedNative);
    }
    preflight(wb, LIMITS)?;
    chart_entries(wb)?;
    if let Some(p) = wb.preserved.as_deref() {
        let index = OwnerIndex::new(p)?;
        let roots = xml::owner_roots(p, &index, true, 0, false)?;
        let visited = index.walk(roots.ordered)?;
        xml::parts(wb, p, &index, &visited)?;
    }
    let mut header = 0u64;
    let mut blocks = 0u32;
    for sh in &wb.sheets {
        let shapes = sh.comments.len().checked_add(sh.controls.len()).ok_or(Error::VmlLimit)?;
        if let Some(p) = sh.preserved.as_deref() {
            let index = OwnerIndex::new(p)?;
            let roots = xml::owner_roots(p, &index, false, shapes, !sh.controls.is_empty())?;
            let visited = index.walk(roots.ordered)?;
            header = header.max(xml::parts(wb, p, &index, &visited)?);
            if let Some(base) = roots.shape_base {
                base.checked_add(shapes as u64).ok_or(Error::VmlLimit)?;
            }
        }
    }
    let header = u32::try_from(header).map_err(|_| Error::VmlLimit)?;
    for sh in &wb.sheets {
        let shapes = sh.comments.len().checked_add(sh.controls.len()).ok_or(Error::VmlLimit)?;
        if shapes == 0 {
            continue;
        }
        let count = (shapes as u64).checked_add(1).ok_or(Error::VmlLimit)?.div_ceil(1024).max(1);
        let count = u32::try_from(count).map_err(|_| Error::VmlLimit)?;
        header.checked_add(blocks).and_then(|n| n.checked_add(1)).and_then(|n| n.checked_add(count)).ok_or(Error::VmlLimit)?;
        blocks = blocks.checked_add(count).ok_or(Error::VmlLimit)?;
    }
    claims::validate(wb)
}
fn chart_entries(wb: &Workbook) -> Checked<()> {
    let Some(p) = wb.preserved.as_deref() else { return Ok(()) };
    let mut names: Vec<&str> = Vec::new();
    let mut ids = Vec::new();
    reserve(&mut ids, wb.sheets.len() + p.sheets.len())?;
    for sh in &wb.sheets {
        if sh.file_id != 0 {
            ids.push(sh.file_id);
        }
    }
    ids.sort_unstable();
    ids.dedup();
    let mut before = 0;
    for sh in &p.sheets {
        if sh.before < before
            || sh.before > wb.sheets.len()
            || sh.sheet_id == 0
            || ids.binary_search(&sh.sheet_id).is_ok()
            || sh.name.is_empty()
            || sh.name.chars().count() > 31
            || sh.name.contains([':', '\\', '/', '?', '*', '[', ']'])
        {
            return Err(Error::ChartSheet);
        }
        before = sh.before;
        let same = |a: &str, b: &str| a.chars().take(31).map(|c| c.to_ascii_lowercase()).eq(b.chars().take(31).map(|c| c.to_ascii_lowercase()));
        if wb.sheets.iter().any(|s| same(&s.name, &sh.name)) || names.iter().any(|s| same(s, &sh.name)) {
            return Err(Error::ChartSheet);
        }
        reserve(&mut names, 1)?;
        names.push(&sh.name);
        ids.push(sh.sheet_id);
        ids.sort_unstable();
    }
    Ok(())
}

#[cfg(test)]
pub(crate) use super::write::visit_sheet_names;

#[cfg(test)]
pub(crate) use validate_recovery_preserved as check_all;
