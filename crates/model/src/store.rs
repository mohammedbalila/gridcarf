//! Copy-on-write sparse cell storage.
//!
//! Cells live in bands of 64 rows, each band behind an `Arc`. Cloning a store (an undo snapshot)
//! copies only the band pointers; editing a cell copies just its band.

use std::collections::BTreeMap;
use std::sync::Arc;

use gridcraft_core::{CellRef, RangeRef};
use serde::{Deserialize, Serialize};

use crate::cell::Cell;

const BAND: u32 = 64;

fn has_value(cell: &Cell) -> bool {
    cell.has_content()
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Band {
    /// row → (col → cell)
    rows: BTreeMap<u32, BTreeMap<u32, Cell>>,
    #[serde(skip)]
    accounting: crate::retained::BandAccounting,
}

impl Band {
    fn rows_mut(&mut self) -> &mut BTreeMap<u32, BTreeMap<u32, Cell>> {
        self.accounting.0.take();
        &mut self.rows
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CellStore {
    bands: BTreeMap<u32, Arc<Band>>,
    len: usize,
}

impl CellStore {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn get(&self, c: CellRef) -> Option<&Cell> {
        self.bands.get(&(c.row / BAND))?.rows.get(&c.row)?.get(&c.col)
    }
    /// Mutable access (copies the band if shared).
    pub fn get_mut(&mut self, c: CellRef) -> Option<&mut Cell> {
        let band = self.bands.get_mut(&(c.row / BAND))?;
        if !band.rows.get(&c.row).is_some_and(|r| r.contains_key(&c.col)) {
            return None;
        }
        Arc::make_mut(band).rows_mut().get_mut(&c.row)?.get_mut(&c.col)
    }
    /// Stores a cell; a default (empty, unstyled) cell is removed instead.
    pub fn set(&mut self, c: CellRef, cell: Cell) {
        if cell.is_blank() {
            self.remove(c);
            return;
        }
        let band = Arc::make_mut(self.bands.entry(c.row / BAND).or_default());
        if band.rows_mut().entry(c.row).or_default().insert(c.col, cell).is_none() {
            self.len += 1;
        }
    }
    /// Puts every cell in the band its row says and recounts the length — serialized band keys
    /// and the cached count are derived state, and the ones a file carries are not trusted. Cells
    /// outside the sheet grid are dropped; returns how many (#1008).
    pub fn rebuild(&mut self) -> usize {
        let mut out = CellStore::new();
        let mut dropped = 0;
        for (_, band) in std::mem::take(&mut self.bands) {
            for (row, cols) in band.rows.iter() {
                for (col, cell) in cols.iter() {
                    let at = CellRef::new(*row, *col);
                    if at.is_valid() {
                        out.set(at, cell.clone());
                    } else {
                        dropped += 1;
                    }
                }
            }
        }
        *self = out;
        dropped
    }

    pub fn remove(&mut self, c: CellRef) -> Option<Cell> {
        let key = c.row / BAND;
        let band = self.bands.get_mut(&key)?;
        if !band.rows.get(&c.row).is_some_and(|r| r.contains_key(&c.col)) {
            return None;
        }
        let b = Arc::make_mut(band);
        let rows = b.rows_mut();
        let row = rows.get_mut(&c.row)?;
        let old = row.remove(&c.col);
        if row.is_empty() {
            rows.remove(&c.row);
        }
        if rows.is_empty() {
            self.bands.remove(&key);
        }
        if old.is_some() {
            self.len -= 1;
        }
        old
    }
    /// Cells in `r`, row-major. Efficient for sparse data: skips empty bands and rows.
    pub fn iter_range(&self, r: RangeRef) -> impl Iterator<Item = (CellRef, &Cell)> + '_ {
        let (b0, b1) = (r.start.row / BAND, r.end.row / BAND);
        self.bands.range(b0..=b1).flat_map(move |(_, band)| {
            band.rows
                .range(r.start.row..=r.end.row)
                .flat_map(move |(row, cols)| cols.range(r.start.col..=r.end.col).map(move |(col, cell)| (CellRef::new(*row, *col), cell)))
        })
    }
    pub fn iter(&self) -> impl Iterator<Item = (CellRef, &Cell)> + '_ {
        self.bands
            .values()
            .flat_map(|band| band.rows.iter().flat_map(|(row, cols)| cols.iter().map(move |(col, cell)| (CellRef::new(*row, *col), cell))))
    }
    /// Cells of one row in `c0..=c1`.
    pub fn row(&self, row: u32, c0: u32, c1: u32) -> impl Iterator<Item = (u32, &Cell)> + '_ {
        self.bands.get(&(row / BAND)).and_then(|b| b.rows.get(&row)).into_iter().flat_map(move |cols| cols.range(c0..=c1).map(|(c, cell)| (*c, cell)))
    }
    /// Smallest range containing every stored cell, `None` when empty.
    pub fn used_range(&self) -> Option<RangeRef> {
        let first_band = self.bands.values().next()?;
        let last_band = self.bands.values().next_back()?;
        let r0 = *first_band.rows.keys().next()?;
        let r1 = *last_band.rows.keys().next_back()?;
        let mut c0 = u32::MAX;
        let mut c1 = 0;
        for band in self.bands.values() {
            for cols in band.rows.values() {
                if let (Some(a), Some(b)) = (cols.keys().next(), cols.keys().next_back()) {
                    c0 = c0.min(*a);
                    c1 = c1.max(*b);
                }
            }
        }
        if c0 == u32::MAX {
            return None;
        }
        Some(RangeRef::new(CellRef::new(r0, c0), CellRef::new(r1, c1)))
    }
    /// Last stored row in column `col` at or above `below_or_at` (for Ctrl+arrow navigation).
    pub fn has(&self, c: CellRef) -> bool {
        self.get(c).is_some_and(has_value)
    }
    /// Last row (inclusive, capped at `cap`) of the maximal run of contiguous rows at or after
    /// `from.row` that store a cell in `from.col` with a non-empty *value* in `from.col`. `None`
    /// when `from` itself doesn't qualify.
    ///
    /// Deliberately narrower than `has`/`get` (which also count a formula with no cached value
    /// yet, for Ctrl+arrow navigation): this matches `Sheet::value_ref`, whose callers test the
    /// `Value` it returns with `!v.is_empty()` regardless of whether a formula produced it — a
    /// formula cell with an empty cached value (`=""`, or not yet recalculated) is "empty" to
    /// them. A stored cell with no value and no formula (for example a formatted spill child) does
    /// fall through to `spill` in `value_ref`, so `last_contiguous_nonempty` asks `value_ref`
    /// about the row where this run stops.
    ///
    /// Walks each band's row map with `BTreeMap::range` (one in-order pass per band) instead of
    /// one `get` per row, so a long, densely-populated run costs roughly one tree-traversal step
    /// per row rather than one binary search per row.
    pub fn contiguous_value_run(&self, from: CellRef, cap: u32) -> Option<u32> {
        let mut last = None;
        let mut expect = from.row;
        while expect <= cap {
            let band_key = expect / BAND;
            let Some(band) = self.bands.get(&band_key) else { break };
            let band_hi = ((band_key + 1) * BAND - 1).min(cap);
            let mut advanced = false;
            for (r, cols) in band.rows.range(expect..=band_hi) {
                if *r != expect || cols.get(&from.col).is_none_or(|c| c.value.is_empty()) {
                    return last;
                }
                last = Some(expect);
                expect += 1;
                advanced = true;
            }
            if !advanced {
                break;
            }
        }
        last
    }
    /// Positions whose cells differ between `self` and `other` (stored on one side only, or not
    /// equal). Bands both stores share are skipped, so comparing an edited copy with the store
    /// it was cloned from only visits the bands the edit touched.
    pub fn diff(&self, other: &CellStore) -> Vec<CellRef> {
        let empty = Band::default();
        let mut keys: Vec<u32> = self.bands.keys().chain(other.bands.keys()).copied().collect();
        keys.sort_unstable();
        keys.dedup();
        let mut out = Vec::new();
        for k in keys {
            let (a, b) = (self.bands.get(&k), other.bands.get(&k));
            if let (Some(a), Some(b)) = (a, b)
                && Arc::ptr_eq(a, b)
            {
                continue;
            }
            let (a, b) = (a.map_or(&empty, |x| x), b.map_or(&empty, |x| x));
            for (row, cols) in &a.rows {
                for (col, cell) in cols {
                    if b.rows.get(row).and_then(|r| r.get(col)) != Some(cell) {
                        out.push(CellRef::new(*row, *col));
                    }
                }
            }
            for (row, cols) in &b.rows {
                for col in cols.keys() {
                    if !a.rows.get(row).is_some_and(|r| r.contains_key(col)) {
                        out.push(CellRef::new(*row, *col));
                    }
                }
            }
        }
        out
    }
    /// Estimated allocations this store retains beyond `newer`. For a whole history,
    /// use one [`crate::RetainedBytes`] pass to deduplicate across sheets and snapshots.
    pub fn retained_bytes_against(&self, newer: &CellStore) -> usize {
        crate::retained::RetainedBytes::against(self, newer)
    }
    /// Removes every cell in `r` and returns them.
    pub fn take_range(&mut self, r: RangeRef) -> Vec<(CellRef, Cell)> {
        let keys: Vec<CellRef> = self.iter_range(r).map(|(c, _)| c).collect();
        keys.into_iter().filter_map(|c| self.remove(c).map(|cell| (c, cell))).collect()
    }
    /// Rows strictly greater than or equal to `from` moved by `delta` (positive = down). Rows that
    /// would land before `from`... callers delete the vacated block first.
    pub fn shift_rows(&mut self, from: u32, delta: i64) {
        let moved: Vec<(CellRef, Cell)> = {
            let keys: Vec<CellRef> = self.iter().filter(|(c, _)| c.row >= from).map(|(c, _)| c).collect();
            keys.into_iter().filter_map(|c| self.remove(c).map(|cell| (c, cell))).collect()
        };
        for (c, cell) in moved {
            let nr = c.row as i64 + delta;
            if (0..gridcraft_core::MAX_ROWS as i64).contains(&nr) {
                self.set(CellRef::new(nr as u32, c.col), cell);
            }
        }
    }
    pub fn shift_cols(&mut self, from: u32, delta: i64) {
        let keys: Vec<CellRef> = self.iter().filter(|(c, _)| c.col >= from).map(|(c, _)| c).collect();
        let moved: Vec<(CellRef, Cell)> = keys.into_iter().filter_map(|c| self.remove(c).map(|cell| (c, cell))).collect();
        for (c, cell) in moved {
            let nc = c.col as i64 + delta;
            if (0..gridcraft_core::MAX_COLS as i64).contains(&nc) {
                self.set(CellRef::new(c.row, nc as u32), cell);
            }
        }
    }
    /// Shifts cells within rows `r0..=r1` and columns `>= from` horizontally (Insert/Delete Cells
    /// with shift right/left).
    pub fn shift_cols_in_rows(&mut self, r0: u32, r1: u32, from: u32, delta: i64) {
        let range = RangeRef::new(CellRef::new(r0, from), CellRef::new(r1, gridcraft_core::MAX_COLS - 1));
        let moved = self.take_range(range);
        for (c, cell) in moved {
            let nc = c.col as i64 + delta;
            if (0..gridcraft_core::MAX_COLS as i64).contains(&nc) {
                self.set(CellRef::new(c.row, nc as u32), cell);
            }
        }
    }
    /// Shifts cells within columns `c0..=c1` and rows `>= from` vertically.
    pub fn shift_rows_in_cols(&mut self, c0: u32, c1: u32, from: u32, delta: i64) {
        let range = RangeRef::new(CellRef::new(from, c0), CellRef::new(gridcraft_core::MAX_ROWS - 1, c1));
        let moved = self.take_range(range);
        for (c, cell) in moved {
            let nr = c.row as i64 + delta;
            if (0..gridcraft_core::MAX_ROWS as i64).contains(&nr) {
                self.set(CellRef::new(nr as u32, c.col), cell);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridcraft_core::Value;

    fn c(r: u32, col: u32) -> CellRef {
        CellRef::new(r, col)
    }

    #[test]
    fn set_get_remove() -> Result<(), Box<dyn std::error::Error>> {
        let mut s = CellStore::new();
        s.set(c(0, 0), Cell::value(1.0.into()));
        s.set(c(100, 5), Cell::value("x".into()));
        assert_eq!(s.len(), 2);
        assert_eq!(s.get(c(100, 5)).map(|x| x.value.clone()), Some(Value::from("x")));
        assert_eq!(s.used_range().ok_or("none")?.a1(), "A1:F101");
        s.set(c(0, 0), Cell::default());
        assert_eq!(s.len(), 1);
        assert!(s.remove(c(100, 5)).is_some());
        assert!(s.is_empty());
        assert!(s.used_range().is_none());
        Ok(())
    }

    #[test]
    fn cow_snapshots() -> Result<(), Box<dyn std::error::Error>> {
        let mut s = CellStore::new();
        for r in 0..1000 {
            s.set(c(r, 0), Cell::value((r as f64).into()));
        }
        let snap = s.clone();
        s.set(c(5, 0), Cell::value(99.0.into()));
        assert_eq!(snap.get(c(5, 0)).ok_or("none")?.value, Value::Number(5.0));
        assert_eq!(s.get(c(5, 0)).ok_or("none")?.value, Value::Number(99.0));
        Ok(())
    }

    #[test]
    fn diff_visits_changed_bands() {
        let mut s = CellStore::new();
        for r in 0..1000 {
            s.set(c(r, 0), Cell::value((r as f64).into()));
        }
        let snap = s.clone();
        assert!(s.diff(&snap).is_empty());
        s.set(c(5, 0), Cell::value(99.0.into()));
        s.set(c(700, 3), Cell::value("new".into()));
        s.remove(c(900, 0));
        assert_eq!(snap.diff(&s), [c(5, 0), c(700, 3), c(900, 0)]);
    }

    #[test]
    fn range_iteration_order() -> Result<(), Box<dyn std::error::Error>> {
        let mut s = CellStore::new();
        s.set(c(70, 2), Cell::value(3.0.into()));
        s.set(c(1, 1), Cell::value(1.0.into()));
        s.set(c(1, 3), Cell::value(2.0.into()));
        s.set(c(500, 0), Cell::value(4.0.into()));
        let v: Vec<f64> =
            s.iter_range(RangeRef::parse("A1:D100").ok_or("none")?).map(|(_, x)| x.value.as_f64()).collect::<Option<_>>().ok_or("none")?;
        assert_eq!(v, [1.0, 2.0, 3.0]);
        Ok(())
    }

    #[test]
    fn shifting() {
        let mut s = CellStore::new();
        s.set(c(2, 0), Cell::value(1.0.into()));
        s.set(c(5, 0), Cell::value(2.0.into()));
        s.shift_rows(3, 2);
        assert!(s.get(c(7, 0)).is_some());
        assert!(s.get(c(2, 0)).is_some());
        s.shift_cols(0, 1);
        assert!(s.get(c(7, 1)).is_some());
    }
}

impl crate::retained::HeapBytes for CellStore {
    fn heap_bytes<'a>(&'a self, c: &mut crate::RetainedBytes<'a>) {
        let Self { bands, len: _ } = self;
        c.visit(bands);
    }
}
#[cfg(test)]
thread_local! { static BAND_ESTIMATE_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

impl crate::retained::HeapBytes for Band {
    fn heap_bytes<'a>(&'a self, c: &mut crate::RetainedBytes<'a>) {
        let summary = self.accounting.0.get_or_init(|| {
            #[cfg(test)]
            BAND_ESTIMATE_VISITS.with(|n| n.set(n.get() + 1));
            crate::retained::AllocationSummary::new(&self.rows)
        });
        c.band(summary);
    }
}

#[cfg(test)]
mod retained_cache_tests {
    use super::*;
    use gridcraft_core::Value;

    #[test]
    fn cached_identities_release_bands_and_shared_payload_after_last_store() {
        let mut store = CellStore::new();
        store.set(CellRef::new(0, 0), Cell::value(Value::from("cached payload")));
        store.set(CellRef::new(64, 0), Cell::value(Value::from("other band")));
        assert!(store.retained_bytes_against(&CellStore::new()) > 0);
        let bands: Vec<_> = store.bands.values().map(Arc::downgrade).collect();
        assert!(store.bands.values().all(|band| band.accounting.0.get().unwrap().storage_bytes() > 0));
        let Value::Text(text) = &store.get(CellRef::new(0, 0)).unwrap().value else { panic!("missing text") };
        let payload = Arc::downgrade(text);
        let snapshot = store.clone();
        drop(store);
        assert!(bands.iter().all(|band| band.strong_count() == 1));
        assert!(payload.upgrade().is_some(), "the second store still owns its cached bands");
        drop(snapshot);
        assert!(bands.iter().all(|band| band.upgrade().is_none()));
        assert!(payload.upgrade().is_none(), "cached integer identities must not own payloads");
    }

    #[test]
    fn unchanged_band_accounting_reuses_its_allocation_inventory() {
        let mut store = CellStore::new();
        store.set(CellRef::new(0, 0), Cell::value(Value::from("payload")));
        let empty = CellStore::new();
        BAND_ESTIMATE_VISITS.with(|n| n.set(0));
        let first = store.retained_bytes_against(&empty);
        let second = store.retained_bytes_against(&empty);
        assert_eq!(first, second);
        assert_eq!(BAND_ESTIMATE_VISITS.with(|n| n.get()), 1, "unchanged bands must not walk every cell on each pass");
    }

    #[test]
    fn cached_inventory_storage_is_charged_and_cow_clones_start_empty() {
        let mut store = CellStore::new();
        store.set(CellRef::new(0, 0), Cell::value(Value::from("payload")));
        let empty = CellStore::new();
        let bytes = store.retained_bytes_against(&empty);
        let band = store.bands.values().next().unwrap();
        let summary = band.accounting.0.get().unwrap();
        assert!(summary.storage_bytes() >= 2 * std::mem::size_of::<usize>());
        assert!(bytes >= summary.storage_bytes() + std::mem::size_of::<Band>());
        let cloned = (**band).clone();
        assert!(cloned.accounting.0.get().is_none());
        assert_eq!(*band.as_ref(), cloned);
        let owner = Arc::downgrade(band);
        drop(store);
        assert!(owner.upgrade().is_none(), "inventory must not keep the owning band alive");
    }

    #[test]
    fn mutation_rebuilds_inventory_and_keeps_snapshot_json_and_equality() {
        let mut store = CellStore::new();
        store.set(CellRef::new(0, 0), Cell::value(Value::from("x".repeat(32 * 1024).as_str())));
        store.set(CellRef::new(1, 0), Cell::value(Value::Number(1.0)));
        let empty = CellStore::new();
        let json = serde_json::to_string(&store).unwrap();
        let baseline = store.retained_bytes_against(&empty);
        let snapshot = store.clone();
        assert_eq!(serde_json::to_string(&store).unwrap(), json);
        assert_eq!(store, snapshot);
        store.get_mut(CellRef::new(0, 0)).unwrap().value = Value::from("x".repeat(64 * 1024).as_str());
        assert!(store.retained_bytes_against(&empty) >= baseline + 32 * 1024);
        assert_eq!(snapshot.retained_bytes_against(&empty), baseline);
        store.remove(CellRef::new(0, 0));
        assert!(store.retained_bytes_against(&empty) < 32 * 1024);
        store.set(CellRef::new(2, 0), Cell::value(Value::from("x".repeat(64 * 1024).as_str())));
        assert!(store.retained_bytes_against(&empty) >= 64 * 1024);
        store.shift_rows(1, 128);
        assert!(store.retained_bytes_against(&empty) >= 64 * 1024);
        store.rebuild();
        assert!(store.retained_bytes_against(&empty) >= 64 * 1024);
    }
}
