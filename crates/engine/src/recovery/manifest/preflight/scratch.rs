use super::{RecoveryError, RecoveryLimits, malformed};
use std::{cmp::Ordering, mem::size_of};

#[derive(Clone, Copy, Debug, Default)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(super) struct Requests {
    pub(super) current: usize,
    pub(super) peak: usize,
    pub(super) actual_current: usize,
    pub(super) actual_peak: usize,
    pub(super) calls: usize,
    pub(super) largest: usize,
}

pub(super) fn reserve<T>(
    values: &mut Vec<T>,
    requested: &mut usize,
    required: usize,
    limit: usize,
    stats: &mut Requests,
) -> Result<(), RecoveryError> {
    let large = || RecoveryError::TooLarge("scratch");
    if required > limit {
        return Err(large());
    }
    if required <= *requested {
        return Ok(());
    }
    let desired = required.max(requested.checked_mul(2).unwrap_or(limit).min(limit));
    let bytes = desired.checked_mul(size_of::<T>()).filter(|n| *n <= isize::MAX as usize).ok_or_else(large)?;
    let old = requested.checked_mul(size_of::<T>()).ok_or_else(large)?;
    let peak = stats.current.checked_add(bytes).ok_or_else(large)?;
    let old_actual = values.capacity().checked_mul(size_of::<T>()).ok_or_else(large)?;
    stats.calls += 1;
    stats.peak = stats.peak.max(peak);
    stats.largest = stats.largest.max(bytes);
    // try_reserve_exact takes additional elements relative to length, not capacity.
    values.try_reserve_exact(desired - values.len()).map_err(|_| large())?;
    let actual = values.capacity() * size_of::<T>();
    stats.actual_peak = stats.actual_peak.max(stats.actual_current.saturating_add(actual));
    stats.actual_current = stats.actual_current - old_actual + actual;
    stats.current = stats.current - old + bytes;
    *requested = desired;
    Ok(())
}

pub(super) fn release<T>(values: &Vec<T>, requested: usize, stats: &mut Requests) {
    stats.current -= requested * size_of::<T>();
    stats.actual_current -= values.capacity() * size_of::<T>();
}

pub(super) fn sort<T: Copy>(
    values: &mut [T],
    merge: &mut Vec<T>,
    requested: &mut usize,
    limit: usize,
    stats: &mut Requests,
    mut compare: impl FnMut(&T, &T) -> Ordering,
) -> Result<(), RecoveryError> {
    let n = values.len();
    if n < 2 {
        return Ok(());
    }
    reserve(merge, requested, n, limit, stats)?;
    merge.resize(n, values[0]);
    let mut width = 1;
    while width < n {
        for start in (0..n).step_by(width.saturating_mul(2)) {
            let mid = start.saturating_add(width).min(n);
            let end = mid.saturating_add(width).min(n);
            let (mut left, mut right) = (start, mid);
            for slot in &mut merge[start..end] {
                let take_left = left < mid && (right == end || compare(&values[left], &values[right]) != Ordering::Greater);
                *slot = if take_left {
                    let v = values[left];
                    left += 1;
                    v
                } else {
                    let v = values[right];
                    right += 1;
                    v
                };
            }
        }
        values.copy_from_slice(&merge[..n]);
        width = width.saturating_mul(2);
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Span {
    offset: u32,
    len: u32,
}
#[derive(Default)]
pub(super) struct Keys {
    bytes: Vec<u8>,
    spans: Vec<Span>,
    merge: Vec<Span>,
    requested: [usize; 3],
    pub(super) peak: usize,
    pub(super) comparisons: usize,
}
impl Keys {
    pub(super) fn mark(&self) -> (usize, usize) {
        (self.bytes.len(), self.spans.len())
    }
    #[cfg(test)]
    pub(super) fn live(&self) -> (usize, usize) {
        self.mark()
    }
    pub(super) fn push(&mut self, key: &str, limits: &RecoveryLimits, stats: &mut Requests) -> Result<(), RecoveryError> {
        let end = self
            .bytes
            .len()
            .checked_add(key.len())
            .filter(|n| *n <= limits.manifest && *n <= u32::MAX as usize)
            .ok_or(RecoveryError::TooLarge("key_bytes"))?;
        let count = self.spans.len().checked_add(1).filter(|n| *n <= limits.nodes).ok_or(RecoveryError::TooLarge("nodes"))?;
        reserve(&mut self.bytes, &mut self.requested[0], end, limits.manifest.min(u32::MAX as usize), stats)?;
        reserve(&mut self.spans, &mut self.requested[1], count, limits.nodes, stats)?;
        self.spans.push(Span { offset: self.bytes.len() as u32, len: key.len() as u32 });
        self.bytes.extend_from_slice(key.as_bytes());
        self.peak = self.peak.max(end);
        Ok(())
    }
    pub(super) fn check(&mut self, mark: (usize, usize), limits: &RecoveryLimits, stats: &mut Requests) -> Result<(), RecoveryError> {
        let bytes = &self.bytes;
        let slice = |span: &Span| &bytes[span.offset as usize..span.offset as usize + span.len as usize];
        let comparisons = &mut self.comparisons;
        sort(&mut self.spans[mark.1..], &mut self.merge, &mut self.requested[2], limits.nodes, stats, |a, b| {
            *comparisons += 1;
            slice(a).cmp(slice(b))
        })?;
        if self.spans[mark.1..].windows(2).any(|pair| slice(&pair[0]) == slice(&pair[1])) {
            return Err(malformed("duplicate_key"));
        }
        Ok(())
    }
    pub(super) fn release(&mut self, mark: (usize, usize)) {
        self.bytes.truncate(mark.0);
        self.spans.truncate(mark.1);
    }
    pub(super) fn close(self, stats: &mut Requests) {
        release(&self.bytes, self.requested[0], stats);
        release(&self.spans, self.requested[1], stats);
        release(&self.merge, self.requested[2], stats);
    }
}

#[cfg(test)]
mod tests;
