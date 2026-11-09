//! Fallible bounded ownership of live metadata, after Session eligibility checks.
use super::{Metadata, NativePath, RecoveryError, RecoveryId, RecoveryLimits, RecoveryProvenance, Selection};
use crate::DocState;

struct Budget {
    bytes: usize,
    nodes: usize,
    limits: RecoveryLimits,
}
impl Budget {
    fn reserve<T>(&mut self, n: usize) -> Result<Vec<T>, RecoveryError> {
        let bytes = n.checked_mul(std::mem::size_of::<T>()).ok_or(RecoveryError::TooLarge("metadata"))?;
        self.bytes = self.bytes.checked_add(bytes).filter(|v| *v <= self.limits.manifest).ok_or(RecoveryError::TooLarge("metadata"))?;
        let mut values = Vec::new();
        values.try_reserve_exact(n).map_err(|_| RecoveryError::TooLarge("metadata_allocation"))?;
        Ok(values)
    }
    fn nodes(&mut self, n: usize) -> Result<(), RecoveryError> {
        self.nodes = self.nodes.checked_add(n).filter(|v| *v <= self.limits.nodes).ok_or(RecoveryError::TooLarge("nodes"))?;
        Ok(())
    }
    fn text(&mut self, value: &str) -> Result<String, RecoveryError> {
        if value.len() > self.limits.metadata {
            return Err(RecoveryError::TooLarge("metadata"));
        }
        let bytes = self.reserve::<u8>(value.len())?;
        let mut text = String::from_utf8(bytes).map_err(|_| RecoveryError::Malformed("metadata"))?;
        text.push_str(value);
        Ok(text)
    }
    fn selection(&mut self, s: &Selection) -> Result<Selection, RecoveryError> {
        self.nodes(s.ranges.len())?;
        let mut ranges = self.reserve(s.ranges.len())?;
        ranges.extend_from_slice(&s.ranges);
        Ok(Selection { active: s.active, anchor: s.anchor, ranges })
    }
    fn path(&mut self, path: &str) -> Result<NativePath, RecoveryError> {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let units = std::path::Path::new(path).as_os_str().as_bytes();
            if units.len() > self.limits.native_units {
                return Err(RecoveryError::TooLarge("native_units"));
            }
            let mut out = self.reserve(units.len())?;
            out.extend_from_slice(units);
            Ok(NativePath::Unix(out))
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            let units = std::path::Path::new(path).as_os_str();
            let count = units.encode_wide().count();
            if count.checked_mul(2).is_none_or(|n| n > self.limits.native_units) {
                return Err(RecoveryError::TooLarge("native_units"));
            }
            let mut out = self.reserve(count)?;
            out.extend(units.encode_wide());
            Ok(NativePath::Windows(out))
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = path;
            Err(RecoveryError::UnsupportedRepresentation("native_path_platform"))
        }
    }
}
impl Metadata {
    pub(in crate::recovery) fn captured_revision(&self) -> u64 {
        self.captured_revision
    }
    pub(in crate::recovery) fn capture(doc: &DocState, id: RecoveryId, sequence: u64) -> Result<Self, RecoveryError> {
        #[cfg(test)]
        super::CAPTURE_WORK.with(|work| {
            let (metadata, serialization) = work.get();
            work.set((metadata + 1, serialization));
        });
        let active = doc.wb.active_sheet();
        if active >= doc.wb.sheets.len() {
            return Err(RecoveryError::Malformed("active_sheet"));
        }
        let mut budget = Budget { bytes: 0, nodes: 0, limits: RecoveryLimits::default() };
        let title = budget.text(&doc.title)?;
        let original_path_hint = doc.path.as_deref().map(|p| budget.path(p)).transpose()?;
        budget.nodes(doc.import_issues().len())?;
        let mut import_issues = budget.reserve(doc.import_issues().len())?;
        for issue in doc.import_issues() {
            import_issues.push(gridcraft_model::CompatibilityIssue {
                code: budget.text(&issue.code)?,
                feature: budget.text(&issue.feature)?,
                location: issue.location.as_deref().map(|s| budget.text(s)).transpose()?,
                effect: issue.effect,
                editability: issue.editability,
                description: budget.text(&issue.description)?,
            });
        }
        let selection = budget.selection(&doc.selection)?;
        budget.nodes(doc.sheet_selections.len())?;
        let mut sheet_selections = budget.reserve(doc.sheet_selections.len())?;
        for selection in &doc.sheet_selections {
            sheet_selections.push(budget.selection(selection)?);
        }
        Ok(Self {
            title,
            original_path_hint,
            file_kind: doc.content_kind.map(Into::into),
            dirty_at_capture: doc.is_dirty(),
            import_issues,
            selection,
            sheet_selections,
            captured_revision: doc.revision,
            id,
            sequence,
            provenance: RecoveryProvenance::Plain,
            frozen_active_sheet: Some(active),
        })
    }
}
