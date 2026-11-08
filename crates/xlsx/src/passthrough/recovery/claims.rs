//! Effect-free output-name planning. Selection occupancy and emitted identities are separate.
use super::super::write::{ClaimName, claim_with};
use super::{Checked, Error};
use gridcraft_model::{Preserved, PreservedPart, Workbook};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::hash::Hasher;

pub(crate) struct Work {
    pub scratch: usize,
    pub events: usize,
    pub bytes: usize,
}
impl Default for Work {
    fn default() -> Self {
        Self { scratch: 16 * 1024 * 1024, events: 4_000_000, bytes: 256 * 1024 * 1024 }
    }
}
impl Work {
    pub fn touch(&mut self, events: usize, bytes: usize) -> Checked<()> {
        self.events = self.events.checked_sub(events).ok_or(Error::WorkLimit)?;
        self.bytes = self.bytes.checked_sub(bytes).ok_or(Error::WorkLimit)?;
        Ok(())
    }
    pub(crate) fn allocate(&mut self, n: usize) -> Checked<()> {
        self.scratch = self.scratch.checked_sub(n).ok_or(Error::WorkLimit)?;
        Ok(())
    }
    pub(crate) fn release(&mut self, n: usize) {
        self.scratch = self.scratch.saturating_add(n);
    }
    pub fn push<T>(&mut self, v: &mut Vec<T>, value: T) -> Checked<()> {
        self.touch(1, 0)?;
        // All callers append monotonically; coordinates are deduplicated only after their
        // last push. Logical requests follow powers of two, independent of allocator rounding.
        if v.is_empty() || (v.len() >= 4 && v.len().is_power_of_two()) {
            let old = v.len().checked_mul(std::mem::size_of::<T>()).ok_or(Error::WorkLimit)?;
            let cap = v.len().checked_mul(2).ok_or(Error::WorkLimit)?.max(4);
            let bytes = cap.checked_mul(std::mem::size_of::<T>()).ok_or(Error::WorkLimit)?;
            self.allocate(bytes)?;
            v.try_reserve_exact(cap - v.len()).map_err(|_| Error::Allocation)?;
            self.release(old);
        }
        v.push(value);
        Ok(())
    }
    fn string(&mut self, recipe: ClaimName<'_>) -> Checked<String> {
        let n = recipe.byte_len().filter(|n| *n <= super::MAX_TEXT + 14).ok_or(Error::WorkLimit)?;
        self.touch(1, n)?;
        self.allocate(n)?;
        let mut s = String::new();
        s.try_reserve_exact(n).map_err(|_| Error::Allocation)?;
        write!(s, "{recipe}").map_err(|_| Error::Allocation)?;
        Ok(s)
    }
}
fn hash(name: &str, w: &mut Work) -> Checked<u64> {
    w.touch(1, name.len())?;
    let mut h = std::hash::DefaultHasher::new();
    for b in name.bytes() {
        h.write_u8(b.to_ascii_lowercase());
    }
    Ok(h.finish())
}
fn control(name: &str) -> Option<u32> {
    let s = ascii_prefix(name, "xl/ctrlprops/ctrlprop")?;
    let s = ascii_suffix(s, ".xml")?;
    number(s)
}
fn number(s: &str) -> Option<u32> {
    if s.is_empty() || s.starts_with('0') || !s.bytes().all(|b| b.is_ascii_digit()) { None } else { s.parse().ok() }
}
fn ascii_prefix<'a>(s: &'a str, p: &str) -> Option<&'a str> {
    s.get(..p.len()).filter(|s| s.eq_ignore_ascii_case(p))?;
    s.get(p.len()..)
}
fn ascii_suffix<'a>(s: &'a str, p: &str) -> Option<&'a str> {
    let at = s.len().checked_sub(p.len())?;
    s.get(at..).filter(|s| s.eq_ignore_ascii_case(p))?;
    s.get(..at)
}
struct Emitted<'a> {
    name: String,
    data: Option<&'a [u8]>,
    sheet: usize,
}
struct State<'a> {
    work: Work,
    suffix: u32,
    controls: u32,
    coordinates: Vec<u32>,
    active: Vec<bool>,
    used: HashMap<u64, Vec<usize>>,
    emitted: Vec<Emitted<'a>>,
}
impl<'a> State<'a> {
    fn new(wb: &'a Workbook, mut work: Work) -> Checked<Self> {
        work.touch(wb.sheets.len(), 0)?;
        let mut coordinates = Vec::new();
        for p in wb.preserved.iter().chain(wb.sheets.iter().filter_map(|s| s.preserved.as_ref())) {
            for q in p.parts.iter().chain(p.vml.iter()) {
                work.touch(1, q.name.len())?;
                if let Some(n) = control(&q.name) {
                    work.push(&mut coordinates, n)?;
                }
            }
        }
        fallible_sort(&mut coordinates, &mut work, |a, b, w| {
            w.touch(1, 0)?;
            Ok(a.cmp(b))
        })?;
        work.touch(coordinates.len(), 0)?;
        coordinates.dedup();
        let mut active = Vec::new();
        work.allocate(coordinates.len())?;
        active.try_reserve_exact(coordinates.len()).map_err(|_| Error::Allocation)?;
        active.resize(coordinates.len(), false);
        let mut used = HashMap::new();
        work.allocate(super::LIMITS.parts.checked_mul(std::mem::size_of::<(u64, Vec<usize>)>()).ok_or(Error::WorkLimit)?)?;
        used.try_reserve(super::LIMITS.parts).map_err(|_| Error::Allocation)?;
        Ok(Self { work, suffix: 0, controls: 0, coordinates, active, used, emitted: Vec::new() })
    }
    fn emit(&mut self, name: String, data: Option<&'a [u8]>, sheet: usize, rels: bool) -> Checked<usize> {
        let index = self.emitted.len();
        let sidecar = if rels {
            let (dir, file) = crate::package::split_dir(&name);
            let len = name.len().checked_add(11).ok_or(Error::WorkLimit)?;
            self.work.allocate(len)?;
            self.work.touch(1, len)?;
            let mut s = String::new();
            s.try_reserve_exact(len).map_err(|_| Error::Allocation)?;
            if !dir.is_empty() {
                s.push_str(dir);
                s.push('/');
            }
            s.push_str("_rels/");
            s.push_str(file);
            s.push_str(".rels");
            Some(s)
        } else {
            None
        };
        self.work.push(&mut self.emitted, Emitted { name, data, sheet })?;
        if let Some(name) = sidecar {
            self.work.push(&mut self.emitted, Emitted { name, data: None, sheet })?;
        }
        Ok(index)
    }
    fn claim(&mut self, q: &'a PreservedPart, sheet: usize) -> Checked<()> {
        let mut suffix = self.suffix;
        claim_with(&q.name, &mut suffix, |recipe| {
            let name = self.work.string(recipe)?;
            let key = hash(&name, &mut self.work)?;
            let mut occupied = control(&name).is_some_and(|n| n <= self.controls);
            if let Some(bucket) = self.used.get(&key) {
                for &i in bucket {
                    self.work.touch(1, name.len().saturating_add(self.emitted[i].name.len()))?;
                    if self.emitted[i].name.eq_ignore_ascii_case(&name) {
                        occupied = true;
                        break;
                    }
                }
            }
            if occupied {
                let requested = name.len();
                drop(name);
                self.work.release(requested);
                if matches!(recipe, ClaimName::Renamed { number: u32::MAX, .. }) {
                    return Err(Error::WorkLimit);
                }
                return Ok((false, ()));
            }
            if let Some(n) = control(&name) {
                self.work.touch((usize::BITS - self.coordinates.len().leading_zeros()) as usize, 0)?;
                if let Ok(i) = self.coordinates.binary_search(&n) {
                    self.active[i] = true;
                }
            }
            let i = self.emit(name, Some(&q.data), sheet, !q.rels.is_empty())?;
            let bucket = self.used.entry(key).or_default();
            self.work.push(bucket, i)?;
            Ok((true, ()))
        })?;
        self.suffix = suffix;
        Ok(())
    }
    fn reserve_controls(&mut self, count: usize) -> Checked<()> {
        self.work.touch(1, 0)?;
        let mut end = self.controls.checked_add(u32::try_from(count).map_err(|_| Error::WorkLimit)?).ok_or(Error::WorkLimit)?;
        self.work.touch((usize::BITS - self.coordinates.len().leading_zeros()) as usize, 0)?;
        for (i, &n) in self.coordinates.iter().enumerate().skip(self.coordinates.partition_point(|n| *n <= self.controls)) {
            self.work.touch(1, 0)?;
            if n > end {
                break;
            }
            if self.active[i] {
                end = end.checked_add(1).ok_or(Error::WorkLimit)?;
            }
        }
        self.controls = end;
        Ok(())
    }
    fn owner(&mut self, p: Option<&'a Preserved>, sheet: usize, controls: usize, merge: bool, vml_number: u32) -> Checked<()> {
        if let Some(p) = p {
            for q in &p.parts {
                self.work.touch(1, 0)?;
                self.claim(q, sheet)?;
            }
        }
        self.reserve_controls(controls)?;
        if let Some(q) = p.and_then(|p| p.vml.as_ref()) {
            if merge {
                let n = vml_number;
                let len = 26usize.checked_add(10).ok_or(Error::WorkLimit)?;
                self.work.allocate(len)?;
                self.work.touch(1, len)?;
                let mut name = String::new();
                name.try_reserve_exact(len).map_err(|_| Error::Allocation)?;
                write!(name, "xl/drawings/vmlDrawing{n}.vml").map_err(|_| Error::Allocation)?;
                self.emit(name, None, sheet, !q.rels.is_empty())?;
            } else {
                self.claim(q, sheet)?;
            }
        }
        Ok(())
    }
}

pub(crate) fn validate(wb: &Workbook) -> Checked<()> {
    validate_with(wb, Work::default())
}
pub(crate) fn validate_with(wb: &Workbook, work: Work) -> Checked<()> {
    let mut s = State::new(wb, work)?;
    s.owner(wb.preserved.as_deref(), 0, 0, false, 0)?;
    let mut vml = 0u32;
    for (i, sh) in wb.sheets.iter().enumerate() {
        s.work.touch(1, 0)?;
        let merge = !sh.comments.is_empty() || !sh.controls.is_empty();
        if merge {
            vml = vml.checked_add(1).ok_or(Error::WorkLimit)?;
        }
        s.owner(sh.preserved.as_deref(), i + 1, sh.controls.len(), merge, vml)?;
    }
    let mut order = Vec::new();
    for i in 0..s.emitted.len() {
        s.work.push(&mut order, i)?;
    }
    fallible_sort(&mut order, &mut s.work, |&a, &b, w| {
        let (a, b) = (&s.emitted[a].name, &s.emitted[b].name);
        w.touch(1, a.len() + b.len())?;
        Ok(super::ascii_cmp(a, b))
    })?;
    for pair in order.windows(2) {
        let (a, b) = (&s.emitted[pair[0]].name, &s.emitted[pair[1]].name);
        s.work.touch(1, a.len() + b.len())?;
        if a.eq_ignore_ascii_case(b) {
            return Err(Error::OutputName);
        }
    }
    let mut native = NativeFacts {
        wb,
        emitted: &s.emitted,
        coordinates: &s.coordinates,
        active: &s.active,
        controls: s.controls,
        floating: None,
        pictures: None,
        cache: Vec::new(),
    };
    for &i in &order {
        let a = &s.emitted[i].name;
        let (mut lo, mut hi) = (0, order.len());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let b = &s.emitted[order[mid]].name;
            s.work.touch(1, a.len() + b.len())?;
            if b.bytes().map(|b| b.to_ascii_lowercase()).cmp(a.bytes().map(|b| b.to_ascii_lowercase()).chain(std::iter::once(b'/'))).is_lt() {
                lo = mid + 1
            } else {
                hi = mid
            }
        }
        if let Some(&j) = order.get(lo) {
            let b = &s.emitted[j].name;
            s.work.touch(1, a.len() + b.len())?;
            if b.len() > a.len() && b.get(..a.len()).is_some_and(|p| p.eq_ignore_ascii_case(a)) && b.as_bytes().get(a.len()) == Some(&b'/') {
                return Err(Error::OutputName);
            }
        }
        for (at, _) in a.match_indices('/').take(4).chain(std::iter::once((a.len(), ""))) {
            let prefix = &a[..at];
            if native.exists(prefix, &mut s.work)? {
                return Err(Error::OutputName);
            }
        }
    }
    Ok(())
}

fn fallible_sort<T>(v: &mut [T], work: &mut Work, mut compare: impl FnMut(&T, &T, &mut Work) -> Checked<std::cmp::Ordering>) -> Checked<()> {
    fn sift<T>(
        v: &mut [T],
        mut root: usize,
        end: usize,
        w: &mut Work,
        c: &mut impl FnMut(&T, &T, &mut Work) -> Checked<std::cmp::Ordering>,
    ) -> Checked<()> {
        while root < end / 2 {
            let mut child = root * 2 + 1;
            if child + 1 < end && c(&v[child], &v[child + 1], w)?.is_lt() {
                child += 1;
            }
            if !c(&v[root], &v[child], w)?.is_lt() {
                break;
            }
            v.swap(root, child);
            root = child;
        }
        Ok(())
    }
    for i in (0..v.len() / 2).rev() {
        sift(v, i, v.len(), work, &mut compare)?;
    }
    for end in (1..v.len()).rev() {
        v.swap(0, end);
        sift(v, 0, end, work, &mut compare)?;
    }
    Ok(())
}

struct NativeFacts<'a, 'b> {
    wb: &'a Workbook,
    emitted: &'b [Emitted<'a>],
    coordinates: &'b [u32],
    active: &'b [bool],
    controls: u32,
    floating: Option<Vec<(u32, &'static str)>>,
    pictures: Option<Vec<(&'a gridcraft_model::CellPicture, &'static str)>>,
    cache: Vec<(&'b str, bool)>,
}
impl<'a, 'b> NativeFacts<'a, 'b> {
    fn exists(&mut self, name: &'b str, w: &mut Work) -> Checked<bool> {
        // Queries are bounded and duplicate queries reuse exact decisions without new traversal.
        for &(key, value) in &self.cache {
            w.touch(1, key.len() + name.len())?;
            if key.eq_ignore_ascii_case(name) {
                return Ok(value);
            }
        }
        let value = self.fact(name, w)?;
        w.push(&mut self.cache, (name, value))?;
        Ok(value)
    }
    fn fact(&mut self, name: &str, w: &mut Work) -> Checked<bool> {
        w.touch(1, name.len())?;
        if [
            "xl/styles.xml",
            "xl/sharedStrings.xml",
            "xl/theme/theme1.xml",
            "xl/workbook.xml",
            "xl/_rels/workbook.xml.rels",
            "docProps/core.xml",
            "docProps/app.xml",
            "_rels/.rels",
            "[Content_Types].xml",
        ]
        .iter()
        .any(|n| n.eq_ignore_ascii_case(name))
        {
            return Ok(true);
        }
        if let Some(n) = control(name) {
            w.touch((usize::BITS - self.coordinates.len().leading_zeros()) as usize, 0)?;
            return Ok(n <= self.controls && self.coordinates.binary_search(&n).ok().is_none_or(|i| !self.active[i]));
        }
        if let Some(n) = indexed(name, "xl/worksheets/sheet", ".xml") {
            return Ok(n as usize <= self.wb.sheets.len());
        }
        if let Some(n) = indexed(name, "xl/worksheets/_rels/sheet", ".xml.rels") {
            return self.sheet_rels(n, w);
        }
        if let Some(n) = indexed(name, "xl/drawings/vmlDrawing", ".vml") {
            let mut count = 0;
            for s in &self.wb.sheets {
                w.touch(1, 0)?;
                if !s.comments.is_empty() || !s.controls.is_empty() {
                    count += 1;
                    if count == n {
                        // Merged VML and its sidecar already have exact emitted identities.
                        // Only a wholly native drawing still needs this separate oracle.
                        return Ok(s.preserved.as_deref().is_none_or(|p| p.vml.is_none()));
                    }
                }
            }
            return Ok(false);
        }
        for (prefix, suffix, rels) in [("xl/drawings/drawing", ".xml", false), ("xl/drawings/_rels/drawing", ".xml.rels", true)] {
            if let Some(n) = indexed(name, prefix, suffix) {
                let mut count = 0;
                for s in &self.wb.sheets {
                    w.touch(1, 0)?;
                    if !s.images.is_empty() || !s.charts.is_empty() || !s.shapes.is_empty() {
                        count += 1;
                        if count == n {
                            if !rels {
                                return Ok(true);
                            }
                            if !s.charts.is_empty() {
                                return Ok(true);
                            }
                            for i in &s.images {
                                w.touch(1, 0)?;
                                if !i.data.is_empty() {
                                    return Ok(true);
                                }
                            }
                            return Ok(false);
                        }
                    }
                }
                return Ok(false);
            }
        }
        for (prefix, suffix, extended) in [
            ("xl/charts/chart", ".xml", false),
            ("xl/charts/chartEx", ".xml", true),
            ("xl/charts/style", ".xml", true),
            ("xl/charts/colors", ".xml", true),
            ("xl/charts/_rels/chartEx", ".xml.rels", true),
        ] {
            if let Some(n) = indexed(name, prefix, suffix) {
                let mut before = 0u64;
                for s in &self.wb.sheets {
                    w.touch(1, 0)?;
                    let after = before.checked_add(s.charts.len() as u64).ok_or(Error::WorkLimit)?;
                    if u64::from(n) > before && u64::from(n) <= after {
                        return Ok(s
                            .charts
                            .get((u64::from(n) - before - 1) as usize)
                            .is_some_and(|c| crate::chartex::is_chartex(c.kind) == extended));
                    }
                    before = after;
                }
                return Ok(false);
            }
        }
        if let Some(n) = indexed(name, "xl/tables/table", ".xml") {
            let mut count = 0u32;
            for s in &self.wb.sheets {
                w.touch(1, 0)?;
                for t in &s.tables {
                    w.touch(1, 0)?;
                    if crate::sheet_write::table_is_written(t).ok_or(Error::Owner)? {
                        count = count.checked_add(1).ok_or(Error::WorkLimit)?;
                        if count == n {
                            return Ok(true);
                        }
                    }
                }
            }
            return Ok(false);
        }
        for (prefix, suffix, records) in [
            ("xl/pivotTables/pivotTable", ".xml", false),
            ("xl/pivotTables/_rels/pivotTable", ".xml.rels", false),
            ("xl/pivotCache/pivotCacheDefinition", ".xml", false),
            ("xl/pivotCache/pivotCacheRecords", ".xml", true),
            ("xl/pivotCache/_rels/pivotCacheDefinition", ".xml.rels", true),
        ] {
            if let Some(n) = indexed(name, prefix, suffix) {
                let mut count = 0u32;
                for (si, s) in self.wb.sheets.iter().enumerate() {
                    w.touch(1, 0)?;
                    for pt in &s.pivots {
                        w.touch(1, 0)?;
                        let fact =
                            crate::pivot::PivotWriter::recovery_name_facts(self.wb, si, pt, records, &mut |events, bytes| w.touch(events, bytes))?;
                        if let Some(has_records) = fact {
                            count = count.checked_add(1).ok_or(Error::WorkLimit)?;
                            if count == n {
                                return Ok(!records || has_records);
                            }
                        }
                    }
                }
                return Ok(false);
            }
        }
        for (prefix, suffix, threaded) in [("xl/comments", ".xml", false), ("xl/threadedComments/threadedComment", ".xml", true)] {
            if let Some(n) = indexed(name, prefix, suffix) {
                let mut count = 0u32;
                for s in &self.wb.sheets {
                    w.touch(1, 0)?;
                    if !s.comments.is_empty() {
                        count += 1;
                        if count == n {
                            if !threaded {
                                return Ok(true);
                            }
                            for c in s.comments.values() {
                                w.touch(1, 0)?;
                                if c.threaded {
                                    return Ok(true);
                                }
                            }
                            return Ok(false);
                        }
                    }
                }
                return Ok(false);
            }
        }
        if name.eq_ignore_ascii_case("xl/persons/person.xml") {
            for s in &self.wb.sheets {
                w.touch(1, 0)?;
                for c in s.comments.values() {
                    w.touch(1, 0)?;
                    if c.threaded {
                        return Ok(true);
                    }
                }
            }
            return Ok(false);
        }
        if name.eq_ignore_ascii_case("xl/vbaProject.bin") {
            return Ok(self.wb.vba_project.is_some());
        }
        if name.eq_ignore_ascii_case("xl/featurePropertyBag/featurePropertyBag.xml") {
            for (_, st) in self.wb.styles.iter() {
                w.touch(1, st.num_fmt.as_str().len())?;
                if st.num_fmt.as_str() == "checkbox" {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        if name.eq_ignore_ascii_case("xl/python.xml") {
            return native_cells(self.wb, true, false, w).map(|v| v.0);
        }
        if name.eq_ignore_ascii_case("xl/metadata.xml") {
            let mut any = false;
            for s in &self.wb.sheets {
                w.touch(1, 0)?;
                if !s.cell_pictures.is_empty() {
                    any = true;
                    break;
                }
            }
            if any {
                self.rich_pictures(w)?;
                return Ok(true);
            }
            return native_cells(self.wb, false, true, w).map(|v| v.1);
        }
        if [
            "xl/richData/rdrichvalue.xml",
            "xl/richData/rdrichvaluestructure.xml",
            "xl/richData/richValueRel.xml",
            "xl/richData/_rels/richValueRel.xml.rels",
        ]
        .iter()
        .any(|n| n.eq_ignore_ascii_case(name))
        {
            return self.rich_pictures(w).map(|p| !p.is_empty());
        }
        if let Some((n, ext)) = media_index(name, "xl/media/cellImage") {
            let pictures = self.rich_pictures(w)?;
            return Ok(pictures.get(n as usize - 1).is_some_and(|(_, e)| e.eq_ignore_ascii_case(ext)));
        }
        if let Some((n, ext)) = media_index(name, "xl/media/image") {
            let floating = self.floating(w)?;
            for &(number, e) in floating {
                w.touch(1, e.len() + ext.len())?;
                if number == n && e.eq_ignore_ascii_case(ext) {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        Ok(false)
    }
    fn sheet_rels(&mut self, n: u32, w: &mut Work) -> Checked<bool> {
        let Some(s) = self.wb.sheets.get(n as usize - 1) else { return Ok(false) };
        w.touch(1, 0)?;
        if s.preserved.as_deref().is_some_and(|p| !p.rels.is_empty() || p.vml.is_some())
            || !s.comments.is_empty()
            || !s.controls.is_empty()
            || !s.images.is_empty()
            || !s.charts.is_empty()
            || !s.shapes.is_empty()
        {
            return Ok(true);
        }
        for t in &s.tables {
            w.touch(1, 0)?;
            if crate::sheet_write::table_is_written(t).ok_or(Error::Owner)? {
                return Ok(true);
            }
        }
        for p in &s.pivots {
            w.touch(1, 0)?;
            if crate::pivot::PivotWriter::recovery_name_facts(self.wb, n as usize - 1, p, false, &mut |e, b| w.touch(e, b))?.is_some() {
                return Ok(true);
            }
        }
        for h in s.hyperlinks.values() {
            w.touch(1, h.target.len())?;
            let t = h.target.trim();
            if !t.is_empty()
                && (h.external
                    || t.contains("://")
                    || ascii_prefix(t, "mailto:").is_some()
                    || ascii_prefix(t, "file:").is_some()
                    || t.starts_with("\\\\"))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn rich_pictures(&mut self, w: &mut Work) -> Checked<&[(&gridcraft_model::CellPicture, &'static str)]> {
        if self.pictures.is_none() {
            let mut pictures: Vec<(&'a gridcraft_model::CellPicture, &'static str)> = Vec::new();
            let mut total = 0usize;
            for s in &self.wb.sheets {
                w.touch(1, 0)?;
                for p in s.cell_pictures.values() {
                    w.touch(1, 0)?;
                    let mut seen = false;
                    for &(old, _) in &pictures {
                        w.touch(1, 0)?;
                        if std::ptr::eq(old, p.as_ref()) {
                            seen = true;
                            break;
                        }
                    }
                    if seen {
                        continue;
                    }
                    if pictures.len() >= crate::richdata::MAX_RECORDS || p.data.len() as u64 > crate::richdata::MAX_IMAGE {
                        return Err(Error::WorkLimit);
                    }
                    total = total.checked_add(p.data.len()).filter(|n| *n <= crate::richdata::MAX_IMAGES_TOTAL).ok_or(Error::WorkLimit)?;
                    w.touch(1, p.data.len())?;
                    let (_, ext) = crate::richdata::image_type(&p.data).ok_or(Error::UnsupportedNative)?;
                    w.push(&mut pictures, (p.as_ref(), ext))?;
                }
            }
            self.pictures = Some(pictures);
        }
        Ok(self.pictures.as_deref().unwrap_or(&[]))
    }
    fn floating(&mut self, w: &mut Work) -> Checked<&[(u32, &'static str)]> {
        if self.floating.is_none() {
            let mut records: Vec<(&[u8], &'static str, u64)> = Vec::new();
            let mut names = Vec::new();
            let mut counter = 0u32;
            let mut cursor = 0;
            for phase in 0..=self.wb.sheets.len() {
                w.touch(1, 0)?;
                while let Some(p) = self.emitted.get(cursor) {
                    w.touch(1, 0)?;
                    if p.sheet != phase {
                        break;
                    }
                    cursor += 1;
                    w.touch(1, p.name.len())?;
                    let Some(data) = p.data else {
                        continue;
                    };
                    for ext in ["png", "jpeg", "gif", "bmp", "tiff", "svg", "emf", "wmf", "webp"] {
                        w.touch(1, p.name.len().min("xl/media/image".len() + ext.len() + 1))?;
                        if crate::write::reusable_image_part(&p.name, &[], ext, &[]) {
                            w.touch(1, data.len())?;
                            let mut h = std::hash::DefaultHasher::new();
                            h.write(data);
                            w.push(&mut records, (data, ext, h.finish()))?;
                            break;
                        }
                    }
                }
                if phase == 0 {
                    continue;
                }
                let s = &self.wb.sheets[phase - 1];
                for image in &s.images {
                    w.touch(1, image.mime.len().saturating_add(image.data.len().min(10)))?;
                    if image.data.is_empty() {
                        continue;
                    }
                    let ext = crate::drawing::image_ext(&image.mime, &image.data);
                    w.touch(1, image.data.len())?;
                    let mut h = std::hash::DefaultHasher::new();
                    h.write(&image.data);
                    let key = h.finish();
                    let mut found = false;
                    for &(data, e, k) in &records {
                        w.touch(1, 0)?;
                        if k == key && e == ext && data.len() == image.data.len() {
                            w.touch(1, data.len().saturating_add(image.data.len()))?;
                            if data == image.data.as_slice() {
                                found = true;
                                break;
                            }
                        }
                    }
                    if !found {
                        counter = counter.checked_add(1).ok_or(Error::WorkLimit)?;
                        w.push(&mut names, (counter, ext))?;
                        w.push(&mut records, (image.data.as_slice(), ext, key))?;
                    }
                }
            }
            let requested = if records.is_empty() { 0 } else { records.len().next_power_of_two().max(4) };
            let capacity = requested * std::mem::size_of::<(&[u8], &str, u64)>();
            drop(records);
            w.release(capacity);
            self.floating = Some(names);
        }
        Ok(self.floating.as_deref().unwrap_or(&[]))
    }
}
fn indexed(name: &str, prefix: &str, suffix: &str) -> Option<u32> {
    number(ascii_suffix(ascii_prefix(name, prefix)?, suffix)?)
}
fn media_index<'a>(name: &'a str, prefix: &str) -> Option<(u32, &'a str)> {
    let s = ascii_prefix(name, prefix)?;
    let (n, e) = s.split_once('.')?;
    Some((number(n)?, e))
}
fn native_cells(wb: &Workbook, py: bool, dynamic: bool, w: &mut Work) -> Checked<(bool, bool)> {
    use crate::array_formula::{AnalysisCharge, AnalysisError};
    use crate::sheet_write::NativeCellRequest;
    crate::sheet_write::native_cell_facts(wb, py, dynamic, |r| match r {
        NativeCellRequest::Charge(e, b) => {
            w.touch(e, b)?;
            Ok(false)
        }
        NativeCellRequest::Table(t) => crate::sheet_write::table_is_written(t).ok_or(Error::Owner),
        NativeCellRequest::Array(si, at, f) => crate::array_formula::needs_array_checked(wb, si, at, f, &mut |r| match r {
            AnalysisCharge::Work(e, b) => w.touch(e, b).map_err(|_| AnalysisError::Limit),
            AnalysisCharge::Allocate(n) => w.allocate(n).map_err(|_| AnalysisError::Limit),
            AnalysisCharge::Release(n) => {
                w.release(n);
                Ok(())
            }
        })
        .map_err(|e| match e {
            AnalysisError::Limit => Error::WorkLimit,
            AnalysisError::Allocation => Error::Allocation,
        }),
    })
}
