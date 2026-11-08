//! Actual writer and comparison primitives; complete semantic preparation belongs to E3.
use super::{ManifestRef, Metadata, RecoveryError, RecoveryLimits, Workbook, observe_manifest};
use serde::Serialize;
use std::io::{self, Write};

#[derive(Debug, Default)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(super) struct Growth {
    calls: usize,
    peak: usize,
    capacity_peak: usize,
    largest: usize,
}
pub(super) struct Encoded {
    pub(super) bytes: Vec<u8>,
    #[cfg(test)]
    pub(super) growth: Growth,
}
struct Bounded {
    bytes: Vec<u8>,
    limit: usize,
    requested: usize,
    growth: Growth,
    refusal: Option<RecoveryError>,
}
impl Bounded {
    fn new(limit: usize) -> Self {
        Self { bytes: Vec::new(), limit, requested: 0, growth: Growth::default(), refusal: None }
    }
    fn append(&mut self, bytes: &[u8]) -> Result<(), RecoveryError> {
        let large = || RecoveryError::TooLarge("manifest");
        let required = self.bytes.len().checked_add(bytes.len()).filter(|n| *n <= self.limit).ok_or_else(large)?;
        if required > self.requested {
            let desired = required.max(self.requested.checked_mul(2).unwrap_or(self.limit).min(self.limit));
            if desired > isize::MAX as usize {
                return Err(large());
            }
            let peak = self.requested.checked_add(desired).ok_or_else(large)?;
            let old_capacity = self.bytes.capacity();
            self.growth.calls += 1;
            self.growth.peak = self.growth.peak.max(peak);
            self.growth.largest = self.growth.largest.max(desired);
            self.bytes.try_reserve_exact(desired - self.bytes.len()).map_err(|_| RecoveryError::TooLarge("manifest_allocation"))?;
            self.growth.capacity_peak = self.growth.capacity_peak.max(old_capacity.saturating_add(self.bytes.capacity()));
            self.requested = desired;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
}
fn refusal() -> io::Error {
    io::Error::other("recovery writer refusal")
}
impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.refusal.is_some() {
            return Err(refusal());
        }
        match self.append(bytes) {
            Ok(()) => Ok(bytes.len()),
            Err(error) => {
                self.refusal = Some(error);
                Err(refusal())
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn writer_error(error: serde_json::Error) -> RecoveryError {
    error.io_error_kind().map(RecoveryError::Io).unwrap_or(RecoveryError::Malformed("manifest_writer"))
}
pub(super) fn encode<T: Serialize + ?Sized>(value: &T, limit: usize) -> Result<Encoded, RecoveryError> {
    let mut writer = Bounded::new(limit);
    let result = serde_json::to_writer(&mut writer, value);
    if let Some(error) = writer.refusal {
        return Err(error);
    }
    result.map_err(writer_error)?;
    Ok(Encoded {
        bytes: writer.bytes,
        #[cfg(test)]
        growth: writer.growth,
    })
}
struct Compare<'a> {
    expected: &'a [u8],
    offset: usize,
    refusal: Option<RecoveryError>,
}
impl Write for Compare<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.refusal.is_some() {
            return Err(refusal());
        }
        let end = self.offset.checked_add(bytes.len());
        if end.and_then(|end| self.expected.get(self.offset..end)) != Some(bytes) {
            self.refusal = Some(RecoveryError::UnsupportedRepresentation("manifest_writer_form"));
            return Err(refusal());
        }
        self.offset = end.ok_or_else(refusal)?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(super) fn compare<T: Serialize + ?Sized>(value: &T, expected: &[u8]) -> Result<(), RecoveryError> {
    let mut writer = Compare { expected, offset: 0, refusal: None };
    let result = serde_json::to_writer(&mut writer, value);
    if let Some(error) = writer.refusal {
        return Err(error);
    }
    result.map_err(writer_error)?;
    if writer.offset != expected.len() {
        return Err(RecoveryError::UnsupportedRepresentation("manifest_writer_form"));
    }
    Ok(())
}
pub(super) fn manifest_bytes(workbook: &Workbook, metadata: &Metadata, limits: &RecoveryLimits) -> Result<Encoded, RecoveryError> {
    observe_manifest(workbook, metadata, limits, false)?;
    encode(&ManifestRef::new(workbook, metadata, limits)?, limits.manifest)
}
pub(super) fn compare_manifest(workbook: &Workbook, metadata: &Metadata, original: &[u8], limits: &RecoveryLimits) -> Result<(), RecoveryError> {
    if original.len() > limits.manifest {
        return Err(RecoveryError::TooLarge("manifest"));
    }
    observe_manifest(workbook, metadata, limits, false)?;
    compare(&ManifestRef::new(workbook, metadata, limits)?, original)
}

#[cfg(test)]
mod tests;
