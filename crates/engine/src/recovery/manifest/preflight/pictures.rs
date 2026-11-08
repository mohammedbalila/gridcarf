use super::{
    RecoveryError, RecoveryLimits, malformed,
    scratch::{self, Requests},
};
use gridcraft_core::CellRef;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Role {
    Plain,
    Pair,
    Coordinate,
    Row,
    Col,
    Object,
}
impl Role {
    pub(super) fn field(self, key: &str) -> Self {
        match (self, key) {
            (Self::Coordinate, "row") => Self::Row,
            (Self::Coordinate, "col") => Self::Col,
            _ => Self::Plain,
        }
    }
    pub(super) fn element(self, index: usize) -> Self {
        match (self, index) {
            (Self::Pair, 0) => Self::Coordinate,
            (Self::Pair, 1) => Self::Object,
            _ => Self::Plain,
        }
    }
}
pub(super) fn coordinate(row: Option<u32>, col: Option<u32>) -> Result<CellRef, RecoveryError> {
    let (row, col) = row.zip(col).ok_or_else(|| malformed("picture_coordinate"))?;
    if row >= 1_048_576 || col >= 16_384 {
        return Err(malformed("picture_coordinate"));
    }
    Ok(CellRef::new(row, col))
}

#[derive(Default)]
pub(super) struct Coordinates {
    values: Vec<CellRef>,
    merge: Vec<CellRef>,
    requested: [usize; 2],
}
impl Coordinates {
    pub(super) fn push(&mut self, value: CellRef, limits: &RecoveryLimits, stats: &mut Requests) -> Result<(), RecoveryError> {
        // Minimal raw pairs need not have the node count of a complete source Image.
        let count = self.values.len().checked_add(1).ok_or(RecoveryError::TooLarge("nodes"))?;
        scratch::reserve(&mut self.values, &mut self.requested[0], count, limits.nodes, stats)?;
        self.values.push(value);
        Ok(())
    }
    pub(super) fn check(&mut self, limits: &RecoveryLimits, stats: &mut Requests) -> Result<(), RecoveryError> {
        scratch::sort(&mut self.values, &mut self.merge, &mut self.requested[1], limits.nodes, stats, Ord::cmp)?;
        if self.values.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(malformed("picture_coordinate"));
        }
        Ok(())
    }
    pub(super) fn release(self, stats: &mut Requests) {
        scratch::release(&self.values, self.requested[0], stats);
        scratch::release(&self.merge, self.requested[1], stats);
    }
}

#[cfg(test)]
mod tests;
