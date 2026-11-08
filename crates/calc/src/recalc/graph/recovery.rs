//! Final-size construction; ordinary insert/remove/settle remain usable afterwards.
#[cfg(test)]
use super::{Area, CellRef, HashMap, RangeRef};
use super::{BTreeSet, Graph, HashSet, Intervals, Key, Node, WIDE};
use crate::recalc::recovery::{Budget, RecoveryCalcError, Result};
fn sort<T: Ord>(items: &mut Vec<T>, b: &mut Budget) -> Result<()> {
    b.work(items.len())?;
    b.work(1)?;
    items.sort_unstable();
    b.work(items.len())?;
    items.dedup();
    Ok(())
}
fn groups<T, K: Copy + Eq>(items: &[T], key: impl Fn(&T) -> K, b: &mut Budget) -> Result<usize> {
    let (mut last, mut count) = (None, 0);
    for item in items {
        b.work(1)?;
        let k = key(item);
        if last != Some(k) {
            count += 1;
            last = Some(k);
        }
    }
    Ok(count)
}
fn end<T, K: Eq>(items: &[T], start: usize, key: impl Fn(&T) -> K, b: &mut Budget) -> Result<usize> {
    let mut stop = start;
    while let Some(item) = items.get(stop) {
        b.work(1)?;
        if key(item) != key(&items[start]) {
            break;
        }
        stop += 1;
    }
    Ok(stop)
}
impl Graph {
    pub(in crate::recalc) fn from_recovery(nodes: Vec<(Key, Node)>, b: &mut Budget) -> Result<Self> {
        let (mut cells, mut cols, mut wide) = (Vec::new(), Vec::new(), Vec::new());
        let mut dynamic = 0;
        for (k, node) in &nodes {
            b.work(1)?;
            if node.dynamic {
                dynamic += 1;
            }
            for a in &node.areas {
                b.work(1)?;
                if a.range.is_single() {
                    b.index()?;
                    b.push(&mut cells, ((a.sheet, a.range.start), *k))?;
                } else if a.range.width() > WIDE {
                    b.index()?;
                    b.push(&mut wide, (a.sheet, a.range.start.row, a.range.end.row, a.range.start.col, a.range.end.col, *k))?;
                } else {
                    for c in a.range.start.col..=a.range.end.col {
                        b.index()?;
                        b.push(&mut cols, ((a.sheet, c), a.range.start.row, a.range.end.row, *k))?;
                    }
                }
            }
        }
        sort(&mut cells, b)?;
        sort(&mut cols, b)?;
        sort(&mut wide, b)?;
        let mut graph = Self::default();
        b.hash(&mut graph.nodes, nodes.len())?;
        b.set(&mut graph.dynamic, dynamic)?;
        let count = groups(&cells, |x| x.0, b)?;
        b.hash(&mut graph.cell_deps, count)?;
        let count = groups(&cols, |x| x.0, b)?;
        b.hash(&mut graph.col_deps, count)?;
        let count = groups(&wide, |x| x.0, b)?;
        b.hash(&mut graph.wide_deps, count)?;
        let mut start = 0;
        while start < cells.len() {
            let stop = end(&cells, start, |x| x.0, b)?;
            let mut set = HashSet::default();
            b.set(&mut set, stop - start)?;
            for (_, k) in &cells[start..stop] {
                b.spare(set.len(), set.capacity())?;
                set.insert(*k);
            }
            b.spare(graph.cell_deps.len(), graph.cell_deps.capacity())?;
            graph.cell_deps.insert(cells[start].0, set);
            start = stop;
        }
        let mut start = 0;
        while start < cols.len() {
            let stop = end(&cols, start, |x| x.0, b)?;
            let mut items = b.vector(stop - start)?;
            for (_, r0, r1, k) in &cols[start..stop] {
                b.push(&mut items, (*r0, *r1, *k))?;
            }
            let intervals = Intervals::from_recovery(items, b)?;
            b.spare(graph.col_deps.len(), graph.col_deps.capacity())?;
            graph.col_deps.insert(cols[start].0, intervals);
            start = stop;
        }
        let mut start = 0;
        while start < wide.len() {
            let stop = end(&wide, start, |x| x.0, b)?;
            let mut items = b.vector(stop - start)?;
            for (_, r0, r1, c0, c1, k) in &wide[start..stop] {
                b.push(&mut items, (*r0, *r1, (*c0, *c1, *k)))?;
            }
            let intervals = Intervals::from_recovery(items, b)?;
            b.spare(graph.wide_deps.len(), graph.wide_deps.capacity())?;
            graph.wide_deps.insert(wide[start].0, intervals);
            start = stop;
        }
        for (k, node) in nodes {
            b.work(1)?;
            if node.dynamic {
                b.spare(graph.dynamic.len(), graph.dynamic.capacity())?;
                graph.dynamic.insert(k);
            }
            b.spare(graph.nodes.len(), graph.nodes.capacity())?;
            graph.nodes.insert(k, node);
        }
        Ok(graph)
    }
    pub(in crate::recalc) fn recovery_dependents(&self, k: Key, b: &mut Budget, f: &mut impl FnMut(Key, &mut Budget) -> Result<()>) -> Result<()> {
        b.work(1)?;
        if let Some(set) = self.cell_deps.get(&k) {
            for dependent in set {
                b.offer()?;
                f(*dependent, b)?;
            }
        }
        b.work(1)?;
        if let Some(iv) = self.col_deps.get(&(k.0, k.1.col)) {
            iv.recovery_query(k.1.row, b, &mut |key, b| {
                b.offer()?;
                f(*key, b)
            })?;
        }
        b.work(1)?;
        if let Some(iv) = self.wide_deps.get(&k.0) {
            iv.recovery_query(k.1.row, b, &mut |(c0, c1, key), b| {
                b.offer()?; // Raw row hit: charge even if its columns miss or its target is non-Circ.
                if *c0 <= k.1.col && k.1.col <= *c1 {
                    f(*key, b)?;
                }
                Ok(())
            })?;
        }
        Ok(())
    }
}
impl<T: Copy + Ord> Intervals<T> {
    fn from_recovery(items: Vec<(u32, u32, T)>, b: &mut Budget) -> Result<Self> {
        let n = items.len();
        let size =
            if n == 0 { 0 } else { n.checked_next_power_of_two().and_then(|v| v.checked_mul(2)).ok_or(RecoveryCalcError::Limit("storage"))? };
        let mut max_end = b.vector(size)?;
        let mut dead = b.vector(n)?;
        for _ in 0..size {
            b.push(&mut max_end, 0)?;
        }
        for (i, it) in items.iter().enumerate() {
            b.work(1)?;
            max_end[size / 2 + i] = it.1;
            b.push(&mut dead, false)?;
        }
        for i in (1..size / 2).rev() {
            b.work(1)?;
            max_end[i] = max_end[2 * i].max(max_end[2 * i + 1]);
        }
        Ok(Self { items, dead, dead_count: 0, max_end, pending: BTreeSet::new() })
    }
    fn recovery_query(&self, row: u32, b: &mut Budget, f: &mut impl FnMut(&T, &mut Budget) -> Result<()>) -> Result<()> {
        for it in &self.pending {
            b.work(1)?;
            if it.0 <= row && it.1 >= row {
                f(&it.2, b)?;
            }
        }
        let (mut lo, mut hi) = (0, self.items.len());
        while lo < hi {
            b.work(1)?;
            let mid = lo + (hi - lo) / 2;
            if self.items[mid].0 <= row {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo > 0 {
            self.recovery_walk((1, 0, self.max_end.len() / 2), lo, row, b, f)?;
        }
        Ok(())
    }
    // The construction's I ceiling bounds this tree depth (<=15); no input-sized call stack.
    fn recovery_walk(
        &self,
        (node, lo, hi): (usize, usize, usize),
        p: usize,
        row: u32,
        b: &mut Budget,
        f: &mut impl FnMut(&T, &mut Budget) -> Result<()>,
    ) -> Result<()> {
        b.work(1)?;
        if lo >= p || self.max_end.get(node).is_none_or(|m| *m < row) {
            return Ok(());
        }
        if hi - lo <= 1 {
            if let (Some(it), Some(false)) = (self.items.get(lo), self.dead.get(lo))
                && it.1 >= row
            {
                f(&it.2, b)?;
            }
        } else {
            let mid = lo + (hi - lo) / 2;
            self.recovery_walk((2 * node, lo, mid), p, row, b, f)?;
            self.recovery_walk((2 * node + 1, mid, hi), p, row, b, f)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn area(width: u32) -> Area {
        Area { sheet: 0, range: RangeRef::new(CellRef::new(0, 0), CellRef::new(64, width - 1)) }
    }
    fn key(row: u32) -> Key {
        (0, CellRef::new(row, 100))
    }
    #[test]
    fn bulk_intervals_match_ordinary_then_support_ordinary_mutation() {
        for n in [0, 1, 3, 7, 14, 15, 65] {
            for width in [1, 64, 65] {
                let nodes: Vec<_> = (0..n).map(|i| (key(i), Node { areas: vec![area(width)], dynamic: i % 2 == 0 })).collect();
                let mut ordinary = Graph::default();
                for (k, node) in nodes.clone() {
                    ordinary.insert(k, node);
                }
                ordinary.settle();
                let mut b = Budget::new();
                let mut restored = Graph::from_recovery(nodes, &mut b).unwrap();
                for col in [0, width - 1, width] {
                    for row in [0, 64, 65] {
                        let c = CellRef::new(row, col);
                        let (mut a, mut z) = (vec![], vec![]);
                        ordinary.dependents(0, c, &mut a);
                        restored.dependents(0, c, &mut z);
                        a.sort_unstable();
                        z.sort_unstable();
                        assert_eq!(a, z, "{n}/{width}/{row}/{col}");
                    }
                }
                assert!(restored.unsettled_cols.is_empty() && restored.unsettled_wide.is_empty());
                for iv in restored.col_deps.values() {
                    assert!(iv.pending.is_empty());
                    assert_eq!(iv.items.capacity(), n as usize);
                    assert_eq!(iv.dead.capacity(), n as usize);
                }
                for iv in restored.wide_deps.values() {
                    assert!(iv.pending.is_empty());
                    assert_eq!(iv.items.capacity(), n as usize);
                    assert_eq!(iv.dead.capacity(), n as usize);
                }
                for i in 0..n {
                    restored.remove(key(i));
                }
                restored.settle();
                let mut out = vec![];
                restored.dependents(0, CellRef::default(), &mut out);
                assert!(out.is_empty());
                for i in 0..70 {
                    restored.insert(key(i), Node { areas: vec![area(width)], dynamic: false });
                }
                restored.settle();
                restored.dependents(0, CellRef::default(), &mut out);
                out.sort_unstable();
                assert_eq!(out, (0..70).map(key).collect::<Vec<_>>());
                println!("bulk n={n} width={width} requested_bytes={} requests={}", b.storage, b.requests);
            }
        }
    }
    #[test]
    fn raw_offers_charge_duplicates_non_circ_and_wide_column_misses_before_filter() {
        let a = Area { sheet: 0, range: RangeRef::cell(CellRef::default()) };
        let narrow = Area { sheet: 0, range: RangeRef::new(CellRef::default(), CellRef::new(1, 0)) };
        let wide = Area { sheet: 0, range: RangeRef::new(CellRef::new(0, 1), CellRef::new(0, 65)) };
        let graph = Graph::from_recovery(vec![(key(0), Node { areas: vec![a, narrow, wide], dynamic: false })], &mut Budget::new()).unwrap();
        let mut b = Budget::new();
        let mut hits = 0;
        graph
            .recovery_dependents((0, CellRef::default()), &mut b, &mut |_, _| {
                hits += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!((b.offers, hits), (3, 2)); // wide row offer counts despite col=0 miss
        let mut b = Budget::new();
        b.limits.offers = 2;
        let mut hits = 0;
        assert_eq!(
            graph.recovery_dependents((0, CellRef::default()), &mut b, &mut |_, _| {
                hits += 1;
                Ok(())
            }),
            Err(RecoveryCalcError::Limit("offers"))
        );
        assert_eq!((b.offers, hits), (2, 2));
        let mut b = Budget::new();
        b.offers = usize::MAX;
        assert!(graph.recovery_dependents((0, CellRef::default()), &mut b, &mut |_, _| panic!("offer before admission")).is_err());
    }
    #[test]
    fn final_empty_hash_reservations_cover_load_thresholds_without_growth() {
        for n in [0, 1, 3, 4, 7, 8, 14, 15, 28, 29, 8192] {
            let mut b = Budget::new();
            let mut map = HashMap::default();
            b.hash(&mut map, n).unwrap();
            let capacity = map.capacity();
            for i in 0..n {
                b.spare(map.len(), map.capacity()).unwrap();
                map.insert(i, i);
                assert_eq!(map.capacity(), capacity);
            }
            assert!(capacity >= n);
            // Derive buckets from the reservation's observed capacity, not the input count.
            let buckets = match capacity {
                0 => 0,
                3 => 4,
                7 => 8,
                other => other / 7 * 8,
            };
            let observed_payload_ceiling = if buckets == 0 { 0 } else { buckets * (std::mem::size_of::<(usize, usize)>() + 1) + 16 };
            assert_eq!(b.storage, observed_payload_ceiling);
            assert!(b.storage <= 4 * n * (std::mem::size_of::<(usize, usize)>() + 1) + 16);
            println!("fresh map n={n} actual_capacity={capacity} requested_payload_ceiling={}", b.storage);
        }
    }
}
