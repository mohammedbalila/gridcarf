//! Framing only: neither a parsed envelope nor a frame plan certifies recovery semantics.
use super::{FrameRef, RecoveryError, RecoveryLimits};
use crate::recovery::{RECOVERY_FRAMES, RECOVERY_HEADER, RECOVERY_MAGIC, RECOVERY_VERSION};
use std::io::{self, Write};

fn malformed(reason: &'static str) -> RecoveryError {
    RecoveryError::Malformed(reason)
}
fn large() -> RecoveryError {
    RecoveryError::TooLarge("envelope")
}
fn part(length: usize, limits: &RecoveryLimits) -> Result<(), RecoveryError> {
    if length > limits.part_bytes {
        return Err(RecoveryError::TooLarge("part_bytes"));
    }
    Ok(())
}
fn count(count: usize, limits: &RecoveryLimits) -> Result<(), RecoveryError> {
    if count > limits.parts.min(RECOVERY_FRAMES) {
        return Err(RecoveryError::TooLarge("parts"));
    }
    Ok(())
}
fn prefix(bytes: &[u8]) -> Result<u64, RecoveryError> {
    Ok(u64::from_le_bytes(bytes.get(..8).ok_or_else(|| malformed("frame_length"))?.try_into().map_err(|_| malformed("frame_length"))?))
}

#[derive(Clone, Copy)]
pub(super) struct Envelope<'a> {
    pub(super) manifest: &'a [u8],
    body: &'a [u8],
}
impl<'a> Envelope<'a> {
    pub(super) fn parse(bytes: &'a [u8], limits: &RecoveryLimits) -> Result<Self, RecoveryError> {
        if bytes.len() > limits.envelope {
            return Err(large());
        }
        let header = bytes.get(..RECOVERY_HEADER).ok_or_else(|| malformed("envelope_header"))?;
        if &header[..8] != RECOVERY_MAGIC {
            return Err(malformed("envelope_header"));
        }
        let version = u32::from_le_bytes(header[8..12].try_into().map_err(|_| malformed("envelope_header"))?);
        if version != RECOVERY_VERSION {
            return Err(RecoveryError::UnsupportedVersion(version));
        }
        let total = usize::try_from(prefix(&header[12..20])?).map_err(|_| large())?;
        if total > limits.envelope {
            return Err(large());
        }
        if total != bytes.len() {
            return Err(malformed("envelope_length"));
        }
        let length = usize::try_from(prefix(&header[20..28])?).map_err(|_| RecoveryError::TooLarge("manifest"))?;
        if length > limits.manifest {
            return Err(RecoveryError::TooLarge("manifest"));
        }
        let end = RECOVERY_HEADER.checked_add(length).ok_or_else(large)?;
        let manifest = bytes.get(RECOVERY_HEADER..end).ok_or_else(|| malformed("envelope_length"))?;
        Ok(Self { manifest, body: &bytes[end..] })
    }
    pub(super) fn plan<'r, I>(self, refs: I, limits: &RecoveryLimits) -> Result<FramePlan<'a>, RecoveryError>
    where
        I: Iterator<Item = &'r FrameRef> + Clone,
    {
        let count = scan(self.body, refs.clone(), limits, |_| Ok(()))?;
        // Complete reference/prefix/body/EOF validation precedes this sole plan reservation.
        let mut plan = FramePlan::reserved(count)?;
        scan(self.body, refs, limits, |slice| plan.push(slice, count))?;
        if plan.payloads.len() != count {
            return Err(malformed("frame_count"));
        }
        Ok(plan)
    }
}
fn scan<'a, 'r, I>(
    body: &'a [u8],
    refs: I,
    limits: &RecoveryLimits,
    mut visit: impl FnMut(&'a [u8]) -> Result<(), RecoveryError>,
) -> Result<usize, RecoveryError>
where
    I: Iterator<Item = &'r FrameRef>,
{
    let (mut cursor, mut seen) = (0usize, 0usize);
    for reference in refs {
        count(seen + 1, limits)?;
        if reference.frame as usize != seen {
            return Err(malformed("frame_reference"));
        }
        let length = usize::try_from(reference.length).map_err(|_| RecoveryError::TooLarge("part_bytes"))?;
        part(length, limits)?;
        let bytes = body.get(cursor..).ok_or_else(|| malformed("frame_length"))?;
        if prefix(bytes)? != reference.length {
            return Err(malformed("frame_length"));
        }
        cursor = cursor.checked_add(8).ok_or_else(large)?;
        let end = cursor.checked_add(length).ok_or_else(large)?;
        visit(body.get(cursor..end).ok_or_else(|| malformed("frame_body"))?)?;
        cursor = end;
        seen += 1;
    }
    if cursor != body.len() {
        return Err(malformed("frame_trailing"));
    }
    Ok(seen)
}

pub(super) struct FramePlan<'a> {
    payloads: Vec<&'a [u8]>,
    #[cfg(test)]
    requested: usize,
    #[cfg(test)]
    capacity: usize,
}
impl<'a> FramePlan<'a> {
    fn reserved(count: usize) -> Result<Self, RecoveryError> {
        count.checked_mul(std::mem::size_of::<&[u8]>()).filter(|n| *n <= isize::MAX as usize).ok_or_else(large)?;
        let mut payloads = Vec::new();
        payloads.try_reserve_exact(count).map_err(|_| RecoveryError::TooLarge("frame_plan"))?;
        Ok(Self {
            #[cfg(test)]
            requested: count * std::mem::size_of::<&[u8]>(),
            #[cfg(test)]
            capacity: payloads.capacity() * std::mem::size_of::<&[u8]>(),
            payloads,
        })
    }
    fn push(&mut self, slice: &'a [u8], count: usize) -> Result<(), RecoveryError> {
        if self.payloads.len() >= count {
            return Err(malformed("frame_count"));
        }
        self.payloads.push(slice);
        Ok(())
    }
    pub(super) fn payload(&self, index: usize) -> Option<&'a [u8]> {
        self.payloads.get(index).copied()
    }
    pub(super) fn compare_payloads<'b>(&self, mut source: impl Iterator<Item = &'b [u8]>) -> Result<(), RecoveryError> {
        for original in &self.payloads {
            if source.next() != Some(*original) {
                return Err(RecoveryError::UnsupportedRepresentation("frame_payload"));
            }
        }
        if source.next().is_some() {
            return Err(RecoveryError::UnsupportedRepresentation("frame_payload"));
        }
        Ok(())
    }
}
fn total_length(manifest: usize, lengths: impl Iterator<Item = usize>, limits: &RecoveryLimits) -> Result<(usize, usize), RecoveryError> {
    if manifest > limits.manifest {
        return Err(RecoveryError::TooLarge("manifest"));
    }
    let mut total = RECOVERY_HEADER.checked_add(manifest).ok_or_else(large)?;
    let mut seen = 0;
    if total > limits.envelope {
        return Err(large());
    }
    for length in lengths {
        seen += 1;
        count(seen, limits)?;
        part(length, limits)?;
        total = total.checked_add(8).and_then(|n| n.checked_add(length)).ok_or_else(large)?;
        if total > limits.envelope {
            return Err(large());
        }
    }
    Ok((total, seen))
}

pub(in crate::recovery) struct WriteParts<'a> {
    plan: FramePlan<'a>,
    header: [u8; RECOVERY_HEADER],
    length: usize,
}
impl<'a> WriteParts<'a> {
    pub(in crate::recovery) fn new<I: Iterator<Item = &'a [u8]> + Clone>(
        manifest: usize,
        source: I,
        limits: &RecoveryLimits,
    ) -> Result<Self, RecoveryError> {
        let (length, count) = total_length(manifest, source.clone().map(<[u8]>::len), limits)?;
        let mut plan = FramePlan::reserved(count)?;
        for payload in source {
            plan.push(payload, count)?;
        }
        if plan.payloads.len() != count {
            return Err(malformed("frame_count"));
        }
        let mut header = [0; RECOVERY_HEADER];
        header[..8].copy_from_slice(RECOVERY_MAGIC);
        header[8..12].copy_from_slice(&RECOVERY_VERSION.to_le_bytes());
        header[12..20].copy_from_slice(&u64::try_from(length).map_err(|_| large())?.to_le_bytes());
        header[20..28].copy_from_slice(&u64::try_from(manifest).map_err(|_| large())?.to_le_bytes());
        Ok(Self { plan, header, length })
    }
    pub(in crate::recovery) fn len(&self) -> usize {
        self.length
    }
    pub(in crate::recovery) fn write_to(&self, manifest: &[u8], sink: &mut impl Write) -> Result<(), RecoveryError> {
        let mut sink = Remaining::new(sink, self.length);
        let result = (|| {
            sink.write_all(&self.header)?;
            sink.write_all(manifest)?;
            for payload in &self.plan.payloads {
                sink.write_all(&(payload.len() as u64).to_le_bytes())?;
                sink.write_all(payload)?;
            }
            Ok::<_, io::Error>(())
        })();
        if let Some(error) = sink.refusal.take() {
            return Err(error);
        }
        result.map_err(|error| RecoveryError::Io(error.kind()))?;
        sink.finish()
    }
}
#[cfg(test)]
pub(super) struct Framing<'m, 'a> {
    manifest: &'m [u8],
    parts: WriteParts<'a>,
}
#[cfg(test)]
impl<'m, 'a> Framing<'m, 'a> {
    pub(super) fn new<I: Iterator<Item = &'a [u8]> + Clone>(manifest: &'m [u8], source: I, limits: &RecoveryLimits) -> Result<Self, RecoveryError> {
        Ok(Self { manifest, parts: WriteParts::new(manifest.len(), source, limits)? })
    }
    pub(super) fn len(&self) -> usize {
        self.parts.len()
    }
    pub(super) fn write_to(&self, sink: &mut impl Write) -> Result<(), RecoveryError> {
        self.parts.write_to(self.manifest, sink)
    }
}

struct Remaining<'a, W> {
    sink: &'a mut W,
    left: usize,
    refusal: Option<RecoveryError>,
}
impl<'a, W: Write> Remaining<'a, W> {
    fn new(sink: &'a mut W, left: usize) -> Self {
        Self { sink, left, refusal: None }
    }
    fn finish(self) -> Result<(), RecoveryError> {
        if let Some(error) = self.refusal {
            return Err(error);
        }
        if self.left != 0 {
            return Err(malformed("envelope_length"));
        }
        Ok(())
    }
}
impl<W: Write> Write for Remaining<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.refusal.is_some() || bytes.len() > self.left {
            if self.refusal.is_none() {
                self.refusal = Some(malformed("envelope_length"));
            }
            return Err(io::Error::other("recovery framing refusal"));
        }
        let count = self.sink.write(bytes)?;
        if count > bytes.len() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        self.left -= count;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.sink.flush()
    }
}

#[cfg(test)]
mod tests;
