//! Whole-construction admission. Requested bytes are a conservative cumulative ledger:
//! requests are never refunded, including temporary buffers and old/new realloc overlap.
use std::mem::{align_of, size_of};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryCalcError {
    Limit(&'static str),
    Allocation,
}
impl std::fmt::Display for RecoveryCalcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Limit(reason) => write!(f, "recovery calculation limit ({reason})"),
            Self::Allocation => f.write_str("recovery calculation allocation"),
        }
    }
}
impl std::error::Error for RecoveryCalcError {}
pub(crate) type Result<T> = std::result::Result<T, RecoveryCalcError>;
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    pub(crate) nodes: usize,
    pub(crate) depth: usize,
    pub(crate) schedules: usize,
    pub(crate) areas: usize,
    pub(crate) indexes: usize,
    pub(crate) offers: usize,
    pub(crate) work: usize,
    pub(crate) names: usize,
    pub(crate) storage: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            nodes: 8192,
            depth: 8,
            schedules: 262144,
            areas: 16384,
            indexes: 16384,
            offers: 65536,
            work: 16777216,
            names: 16777216,
            storage: 67108864,
        }
    }
}
#[derive(Debug, Default)]
pub(crate) struct Budget {
    pub(crate) limits: Limits,
    pub(crate) nodes: usize,
    pub(crate) schedules: usize,
    pub(crate) areas: usize,
    pub(crate) indexes: usize,
    pub(crate) offers: usize,
    pub(crate) work: usize,
    pub(crate) names: usize,
    pub(crate) storage: usize,
    #[cfg(test)]
    pub(crate) requests: usize,
    #[cfg(test)]
    adjacency_capacity: usize,
    #[cfg(test)]
    pub(crate) refuse_request: Option<usize>,
}
fn add(value: &mut usize, n: usize, limit: usize, reason: &'static str) -> Result<()> {
    let next = value.checked_add(n).filter(|v| *v <= limit).ok_or(RecoveryCalcError::Limit(reason))?;
    *value = next;
    Ok(())
}
impl Budget {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn work(&mut self, n: usize) -> Result<()> {
        add(&mut self.work, n, self.limits.work, "work")
    }
    pub(crate) fn node(&mut self) -> Result<()> {
        self.work(1)?;
        add(&mut self.nodes, 1, self.limits.nodes, "nodes")
    }
    pub(crate) fn schedule(&mut self) -> Result<()> {
        self.work(1)?;
        add(&mut self.schedules, 1, self.limits.schedules, "schedules")
    }
    pub(crate) fn area(&mut self) -> Result<()> {
        self.work(1)?;
        add(&mut self.areas, 1, self.limits.areas, "areas")
    }
    pub(crate) fn index(&mut self) -> Result<()> {
        self.work(1)?;
        add(&mut self.indexes, 1, self.limits.indexes, "indexes")
    }
    pub(crate) fn offer(&mut self) -> Result<()> {
        self.work(1)?;
        add(&mut self.offers, 1, self.limits.offers, "offers")
    }
    pub(crate) fn text(&mut self, a: &str, b: &str) -> Result<()> {
        self.work(1)?;
        let n = a.len().checked_add(b.len()).ok_or(RecoveryCalcError::Limit("names"))?;
        add(&mut self.names, n, self.limits.names, "names")
    }
    fn request(&mut self, bytes: usize) -> Result<()> {
        self.work(1)?;
        add(&mut self.storage, bytes, self.limits.storage, "storage")?;
        #[cfg(test)]
        {
            self.requests += 1;
            if self.refuse_request == Some(self.requests) {
                return Err(RecoveryCalcError::Allocation);
            }
        }
        Ok(())
    }
    pub(crate) fn vector<T>(&mut self, count: usize) -> Result<Vec<T>> {
        let mut out = Vec::new();
        self.reserve(&mut out, count)?;
        Ok(out)
    }
    fn reserve<T>(&mut self, out: &mut Vec<T>, target: usize) -> Result<()> {
        if target <= out.capacity() {
            return Ok(());
        }
        let layout = std::alloc::Layout::array::<T>(target).map_err(|_| RecoveryCalcError::Limit("storage"))?;
        self.request(layout.size())?;
        out.try_reserve_exact(target - out.len()).map_err(|_| RecoveryCalcError::Allocation)?;
        // Global's requested payload is exact on the qualified toolchain. Excess allocator
        // residency is outside this ledger; never silently use an unexpected extra capacity.
        if out.capacity() != target {
            return Err(RecoveryCalcError::Allocation);
        }
        Ok(())
    }
    pub(crate) fn push<T>(&mut self, out: &mut Vec<T>, value: T) -> Result<()> {
        self.work(1)?;
        if out.len() == out.capacity() {
            let target = out.capacity().checked_mul(2).filter(|n| *n <= isize::MAX as usize).ok_or(RecoveryCalcError::Limit("storage"))?.max(1);
            self.reserve(out, target)?;
        }
        out.push(value);
        Ok(())
    }
    pub(crate) fn hash<K, V>(&mut self, map: &mut super::HashMap<K, V>, n: usize) -> Result<()>
    where
        K: Eq + std::hash::Hash,
    {
        self.hash_request::<(K, V)>(n)?;
        map.try_reserve(n).map_err(|_| RecoveryCalcError::Allocation)?;
        Ok(())
    }
    pub(crate) fn set<K>(&mut self, set: &mut super::HashSet<K>, n: usize) -> Result<()>
    where
        K: Eq + std::hash::Hash,
    {
        self.hash_request::<(K, ())>(n)?;
        set.try_reserve(n).map_err(|_| RecoveryCalcError::Allocation)?;
        Ok(())
    }
    fn hash_request<T>(&mut self, n: usize) -> Result<()> {
        if n == 0 {
            return Ok(());
        }
        let buckets = if n < 4 {
            4
        } else if n < 8 {
            8
        } else {
            n.checked_mul(8).and_then(|v| v.checked_div(7)).and_then(usize::checked_next_power_of_two).ok_or(RecoveryCalcError::Limit("storage"))?
        };
        // Conservative 16-byte control group covers the inspected generic and SIMD layouts.
        let alignment = align_of::<T>().max(16);
        let data = size_of::<T>().checked_mul(buckets).ok_or(RecoveryCalcError::Limit("storage"))?;
        let aligned = data.checked_add(alignment - 1).map(|v| v & !(alignment - 1)).ok_or(RecoveryCalcError::Limit("storage"))?;
        let bytes = aligned
            .checked_add(buckets)
            .and_then(|v| v.checked_add(16))
            .filter(|v| *v <= isize::MAX as usize)
            .ok_or(RecoveryCalcError::Limit("storage"))?;
        self.request(bytes)
    }
    pub(crate) fn spare(&mut self, len: usize, capacity: usize) -> Result<()> {
        self.work(1)?;
        if len >= capacity {
            return Err(RecoveryCalcError::Allocation);
        }
        Ok(())
    }
}

impl super::Calc {
    /// Build a bounded graph without evaluating. Caller supplies an immutable D3/model-validated
    /// workbook and conservatively precharged structural scan work. `scan_work` only debits the
    /// fixed work ceiling; this API neither validates codec provenance nor grants capture authority.
    pub fn from_recovery(wb: &gridcraft_model::Workbook, scan_work: usize) -> Result<Self> {
        let mut b = Budget::new();
        b.work(scan_work)?;
        build(wb, &mut b)
    }
}
#[cfg(test)]
mod tests;

fn build(wb: &gridcraft_model::Workbook, b: &mut Budget) -> Result<super::Calc> {
    use super::{Calc, Graph, Node};
    use gridcraft_core::{CellError, Value};
    let (mut nodes, mut circ) = (Vec::new(), Vec::new());
    for (si, sheet) in wb.sheets.iter().enumerate() {
        // Structural iterator costs were prepaid by the engine; each owned dispatch is extra.
        b.work(1)?;
        for (c, cell) in sheet.cells.iter() {
            b.work(1)?;
            if let Some(e) = cell.formula.as_ref().and_then(|f| f.expr.as_deref()) {
                b.node()?;
                let (areas, dynamic) = crate::eval::recovery_precedents(wb, si, e, b)?;
                for _ in &areas {
                    b.area()?;
                } // transfer into retained node, without copying
                b.push(&mut nodes, ((si, c), Node { areas, dynamic }))?;
                if cell.value == Value::Error(CellError::Circ) {
                    b.push(&mut circ, (si, c))?;
                }
            } else if cell.py.is_some() {
                b.node()?;
                b.push(&mut nodes, ((si, c), Node { areas: Vec::new(), dynamic: false }))?;
            }
        }
    }
    let graph = Graph::from_recovery(nodes, b)?;
    let circular = if wb.calc.iterative { Vec::new() } else { loops(&graph, &circ, b)? };
    Ok(Calc { graph, circular, ..Calc::default() })
}
fn filled<T: Copy>(n: usize, value: T, b: &mut Budget) -> Result<Vec<T>> {
    let mut out = b.vector(n)?;
    for _ in 0..n {
        b.push(&mut out, value)?;
    }
    Ok(out)
}
fn loops(graph: &super::Graph, circ: &[super::Key], b: &mut Budget) -> Result<Vec<super::Key>> {
    use super::HashMap;
    let n = circ.len();
    if n == 0 {
        return Ok(Vec::new());
    }
    let mut index = HashMap::default();
    b.hash(&mut index, n)?;
    for (i, k) in circ.iter().enumerate() {
        b.spare(index.len(), index.capacity())?;
        index.insert(*k, i);
    }
    let mut pairs = Vec::new();
    for (i, k) in circ.iter().enumerate() {
        b.work(1)?;
        graph.recovery_dependents(*k, b, &mut |key, b| {
            b.work(1)?;
            if let Some(j) = index.get(&key) {
                b.push(&mut pairs, (i, *j))?;
            }
            Ok(())
        })?;
    }
    b.work(pairs.len())?;
    b.work(1)?;
    pairs.sort_unstable();
    b.work(pairs.len())?;
    pairs.dedup();
    let mut edges = b.vector(n)?;
    let mut cursor = 0;
    for i in 0..n {
        b.work(1)?;
        let start = cursor;
        while let Some(pair) = pairs.get(cursor) {
            b.work(1)?;
            if pair.0 != i {
                break;
            }
            cursor += 1;
        }
        let mut row = b.vector(cursor - start)?;
        for (_, j) in &pairs[start..cursor] {
            b.push(&mut row, *j)?;
        }
        #[cfg(test)]
        {
            b.adjacency_capacity += row.capacity();
        }
        b.push(&mut edges, row)?;
    }
    let (comp, size) = components(&edges, b)?;
    let mut order = b.vector(n)?;
    for (i, (s, c)) in circ.iter().enumerate() {
        b.push(&mut order, (*s, c.row, c.col, i))?;
    }
    b.work(n)?;
    b.work(1)?;
    order.sort_unstable();
    let mut covered = filled(n, false, b)?;
    let mut out = b.vector(n)?;
    for (_, _, _, i) in order {
        b.work(1)?;
        // Indices originate from this candidate's dense index; component labels are <n.
        let c = comp[i];
        let mut on_loop = size[c] > 1;
        if !on_loop {
            for j in &edges[i] {
                b.work(1)?;
                if *j == i {
                    on_loop = true;
                    break;
                }
            }
        }
        if on_loop && !covered[c] {
            b.work(1)?;
            covered[c] = true;
            b.push(&mut out, circ[i])?;
        }
    }
    Ok(out)
}
fn components(edges: &[Vec<usize>], b: &mut Budget) -> Result<(Vec<usize>, Vec<usize>)> {
    let n = edges.len();
    let (mut low, mut num, mut comp, mut size) = (filled(n, 0, b)?, filled(n, usize::MAX, b)?, filled(n, usize::MAX, b)?, filled(n, 0, b)?);
    let (mut on_stack, mut stack, mut call) = (filled(n, false, b)?, b.vector(n)?, b.vector(n)?);
    let (mut next, mut comps) = (0, 0);
    for root in 0..n {
        b.work(1)?;
        if num[root] != usize::MAX {
            continue;
        }
        b.push(&mut call, (root, 0))?;
        num[root] = next;
        low[root] = next;
        next += 1;
        b.push(&mut stack, root)?;
        on_stack[root] = true;
        while !call.is_empty() {
            b.work(1)?;
            let Some(top) = call.last_mut() else {
                break;
            };
            let v = top.0;
            if let Some(&w) = edges[v].get(top.1) {
                b.work(1)?;
                top.1 += 1;
                if num[w] == usize::MAX {
                    num[w] = next;
                    low[w] = next;
                    next += 1;
                    b.push(&mut stack, w)?;
                    on_stack[w] = true;
                    b.push(&mut call, (w, 0))?;
                } else if on_stack[w] {
                    low[v] = low[v].min(num[w]);
                }
                continue;
            }
            call.pop();
            if let Some(&(u, _)) = call.last() {
                low[u] = low[u].min(low[v]);
            }
            if low[v] == num[v] {
                while !stack.is_empty() {
                    b.work(1)?;
                    let Some(w) = stack.pop() else {
                        break;
                    };
                    on_stack[w] = false;
                    comp[w] = comps;
                    size[comps] += 1;
                    if w == v {
                        break;
                    }
                }
                comps += 1;
            }
        }
    }
    Ok((comp, size))
}
