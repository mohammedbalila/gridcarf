//! Conditional formatting evaluation: what extra style, data bar, colour scale or icon a cell
//! gets.

use std::collections::HashMap;

use gridcraft_core::{CellRef, RangeRef, Value, compare};
use gridcraft_model::{CfOperator, CfRule, CfValueKind, Color, CondFormat, PatternType, Sheet, Style, Underline, Workbook, default_icon_thresholds};

/// The conditional look of one cell.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CfLook {
    /// Differential style (only non-default parts apply).
    pub style: Option<Style>,
    /// Fill from a colour scale (RGB).
    pub scale_fill: Option<[u8; 3]>,
    /// Data bar: fraction 0..=1 of the cell width, colour, gradient.
    pub bar: Option<(f32, [u8; 3], bool)>,
    /// Icon: set name and index (0 = lowest).
    pub icon: Option<(String, usize)>,
    pub hide_value: bool,
}

/// Why complete conditional-format statistics could not be evaluated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CfIncomplete {
    WorkLimit,
    MemoryLimit,
    AllocationFailed,
    InvalidNumber,
}

impl std::fmt::Display for CfIncomplete {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::WorkLimit => "Conditional formatting is incomplete: statistics work limit reached.",
            Self::MemoryLimit => "Conditional formatting is incomplete: statistics memory limit reached.",
            Self::AllocationFailed => "Conditional formatting is incomplete: statistics allocation failed.",
            Self::InvalidNumber => "Conditional formatting is incomplete: numeric statistics are not finite.",
        })
    }
}

impl std::error::Error for CfIncomplete {}

/// Statistics for one immutable sheet snapshot. A persistent caller must replace this cache
/// when its document, sheet or data/rule revision changes; it never owns workbook payloads.
#[derive(Default)]
pub struct CfCache {
    stats: HashMap<usize, Result<RuleStats, CfIncomplete>>,
    budget: StatsBudget,
    incomplete: Option<CfIncomplete>,
    exhausted: bool,
}

const MAX_STATS_METADATA: usize = 4096;

struct StatsBudget {
    work: usize,
    bytes: usize,
    work_limit: usize,
    byte_limit: usize,
}

impl Default for StatsBudget {
    fn default() -> Self {
        Self { work: 0, bytes: 0, work_limit: 64_000_000, byte_limit: 64 * 1024 * 1024 }
    }
}

impl StatsBudget {
    fn charge(&mut self, n: usize) -> Result<(), CfIncomplete> {
        self.work = self.work.saturating_add(n);
        if self.work > self.work_limit { Err(CfIncomplete::WorkLimit) } else { Ok(()) }
    }

    fn check_memory(&self, additional: usize) -> Result<(), CfIncomplete> {
        if additional > self.byte_limit.saturating_sub(self.bytes) { Err(CfIncomplete::MemoryLimit) } else { Ok(()) }
    }
}

// Account buckets, hash control bytes and fixed overhead conservatively, including spare
// capacity. This is an allocation estimate, not a process RSS or allocator-overhead guarantee.
fn hash_bytes<K, V>(capacity: usize) -> usize {
    if capacity == 0 {
        return 0;
    }
    capacity.checked_next_power_of_two().unwrap_or(usize::MAX).saturating_mul(std::mem::size_of::<(K, V)>().saturating_add(1)).saturating_add(32)
}

fn reserve_map<K: std::hash::Hash + Eq, V>(map: &mut HashMap<K, V>, budget: &mut StatsBudget) -> Result<(), CfIncomplete> {
    if map.len() < map.capacity() {
        return Ok(());
    }
    budget.charge(map.len())?;
    let old = hash_bytes::<K, V>(map.capacity());
    let target = map.capacity().max(4).saturating_mul(2);
    // Keep the old allocation charged during growth and allow for the table's bucket rounding.
    budget.check_memory(hash_bytes::<K, V>(target.saturating_mul(2)))?;
    map.try_reserve(target.saturating_sub(map.len())).map_err(|_| CfIncomplete::AllocationFailed)?;
    let new = hash_bytes::<K, V>(map.capacity());
    budget.check_memory(new.saturating_sub(old))?;
    budget.bytes = budget.bytes.saturating_add(new.saturating_sub(old));
    Ok(())
}

#[derive(Default, Clone)]
struct RuleStats {
    sorted: Vec<f64>,
    avg: f64,
    std: f64,
    counts: HashMap<String, usize>,
}

/// Rules that read no statistics never scan the sheet.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StatsNeed {
    Numbers,
    Average { deviation: bool },
    Counts,
}

fn stats_need(rule: &CfRule) -> Option<StatsNeed> {
    match rule {
        CfRule::Duplicate { .. } | CfRule::Unique { .. } => Some(StatsNeed::Counts),
        CfRule::AboveAverage { std_dev, .. } => Some(StatsNeed::Average { deviation: *std_dev != 0 }),
        CfRule::Top10 { .. } | CfRule::ColorScale { .. } | CfRule::DataBar { .. } | CfRule::IconSet { .. } => Some(StatsNeed::Numbers),
        _ => None,
    }
}

#[cfg(test)]
thread_local! {
    /// Sparse candidates visited on this thread; absent rectangular cells cost no scan work.
    static SCANNED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl RuleStats {
    fn build(sh: &Sheet, ranges: &[RangeRef], need: StatsNeed, budget: &mut StatsBudget) -> Result<Self, CfIncomplete> {
        if ranges.len() > MAX_STATS_METADATA {
            return Err(CfIncomplete::WorkLimit);
        }
        let mut bounds: Option<RangeRef> = None;
        for range in ranges {
            budget.charge(1)?;
            if range.start.row <= range.end.row && range.start.col <= range.end.col {
                bounds = Some(bounds.map_or(*range, |b| b.union(range)));
            }
        }
        let mut stats = Self::default();
        if let Some(bounds) = bounds {
            // iter_range hides work in populated rows with no selected columns. Charge every
            // stored candidate instead so even hostile sparse-column shapes have bounded work.
            for (c, _) in sh.cells.iter() {
                budget.charge(1)?;
                #[cfg(test)]
                SCANNED.with(|n| n.set(n.get().saturating_add(1)));
                if bounds.contains(c) {
                    stats.visit(sh, c, ranges, need, budget)?;
                }
            }
            for (&c, _) in sh.spill.range(bounds.start..=bounds.end) {
                budget.charge(1)?;
                #[cfg(test)]
                SCANNED.with(|n| n.set(n.get().saturating_add(1)));
                if bounds.contains(c) && sh.cells.get(c).is_none() {
                    stats.visit(sh, c, ranges, need, budget)?;
                }
            }
        }
        let len = stats.sorted.len();
        // Unstable sort is in-place with bounded stack; total_cmp remains valid for every f64.
        let levels = usize::BITS - len.saturating_sub(1).leading_zeros();
        budget.charge(len.saturating_mul(levels as usize).saturating_add(len.saturating_mul(2)))?;
        stats.sorted.sort_unstable_by(f64::total_cmp);
        // Only AboveAverage consumes moments. Overflow in unused moments must not reject
        // otherwise finite min/max, percentile or ranking results.
        if let StatsNeed::Average { deviation } = need {
            let n = len.max(1) as f64;
            stats.avg = stats.sorted.iter().sum::<f64>() / n;
            if !stats.avg.is_finite() {
                return Err(CfIncomplete::InvalidNumber);
            }
            if deviation {
                stats.std = (stats.sorted.iter().map(|x| (x - stats.avg).powi(2)).sum::<f64>() / n).sqrt();
                if !stats.std.is_finite() {
                    return Err(CfIncomplete::InvalidNumber);
                }
            }
        }
        Ok(stats)
    }

    fn visit(&mut self, sh: &Sheet, c: CellRef, ranges: &[RangeRef], need: StatsNeed, budget: &mut StatsBudget) -> Result<(), CfIncomplete> {
        budget.charge(1)?;
        let mut inside = false;
        for range in ranges {
            budget.charge(1)?;
            if range.contains(c) {
                inside = true;
                break;
            }
        }
        if !inside {
            return Ok(());
        }
        let Some(value) = sh.value_ref(c) else {
            return Ok(());
        };
        match need {
            StatsNeed::Numbers | StatsNeed::Average { .. } => {
                if let Value::Number(number) = value {
                    if !number.is_finite() {
                        return Err(CfIncomplete::InvalidNumber);
                    }
                    if self.sorted.len() == self.sorted.capacity() {
                        let old = self.sorted.capacity().saturating_mul(std::mem::size_of::<f64>());
                        let target = self.sorted.capacity().max(8).saturating_mul(2);
                        budget.check_memory(target.saturating_mul(std::mem::size_of::<f64>()))?;
                        self.sorted.try_reserve_exact(target.saturating_sub(self.sorted.len())).map_err(|_| CfIncomplete::AllocationFailed)?;
                        let new = self.sorted.capacity().saturating_mul(std::mem::size_of::<f64>());
                        budget.check_memory(new.saturating_sub(old))?;
                        budget.bytes = budget.bytes.saturating_add(new.saturating_sub(old));
                    }
                    self.sorted.push(*number);
                }
            }
            StatsNeed::Counts if !value.is_empty() => {
                let key = count_key(value, budget)?;
                // Keep the temporary string charged while the hash table may grow.
                budget.check_memory(key.capacity())?;
                budget.bytes = budget.bytes.saturating_add(key.capacity());
                if let Some(count) = self.counts.get_mut(&key) {
                    *count = count.saturating_add(1);
                    budget.bytes = budget.bytes.saturating_sub(key.capacity());
                } else {
                    if self.counts.len() == self.counts.capacity() {
                        // Rehashing revisits earlier strings. Long early keys can otherwise be
                        // hashed repeatedly without charging their bytes during table growth.
                        budget.charge(self.counts.len())?;
                        for old in self.counts.keys() {
                            budget.charge(old.len())?;
                        }
                    }
                    reserve_map(&mut self.counts, budget)?;
                    self.counts.insert(key, 1);
                }
            }
            StatsNeed::Counts => {}
        }
        Ok(())
    }
}

fn count_key(mut value: &Value, budget: &mut StatsBudget) -> Result<String, CfIncomplete> {
    let mut in_array = false;
    while let Value::Array(array) = value {
        budget.charge(1)?;
        let Some(first) = array.data.first() else {
            return Ok(String::new());
        };
        value = first;
        in_array = true;
    }
    // Value::display treats an array whose first scalar is an error as empty text.
    let owned;
    let text = match value {
        Value::Text(text) => text.as_ref(),
        Value::Bool(true) => "TRUE",
        Value::Bool(false) => "FALSE",
        Value::Error(error) if !in_array => error.as_str(),
        Value::Number(_) => {
            // number_to_text uses at most 30 fixed decimal places or a 15-digit exponent.
            // Allow all its bounded intermediate formatting strings before calling display.
            budget.check_memory(4096)?;
            budget.charge(64)?;
            owned = value.display();
            owned.as_str()
        }
        _ => "",
    };
    // Unicode lowercase maps each scalar to at most three scalars (12 UTF-8 bytes), and
    // String growth can double capacity. Include input/display and old/new output buffers.
    // str::to_lowercase preserves contextual sigma; char-by-char conversion would not.
    budget.charge(text.len().saturating_mul(4))?;
    budget.check_memory(text.len().saturating_mul(48).saturating_add(4096))?;
    let key = text.to_lowercase();
    budget.check_memory(key.capacity())?;
    Ok(key)
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (p / 100.0).clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    let a = sorted.get(lo).copied().unwrap_or(0.0);
    let b = sorted.get(hi).copied().unwrap_or(a);
    a + (b - a) * (rank - lo as f64)
}

fn threshold(wb: &Workbook, sheet: usize, at: CellRef, st: &RuleStats, k: &CfValueKind) -> f64 {
    let min = st.sorted.first().copied().unwrap_or(0.0);
    let max = st.sorted.last().copied().unwrap_or(0.0);
    match k {
        CfValueKind::Min => min,
        CfValueKind::Max => max,
        CfValueKind::Number(n) => *n,
        CfValueKind::Percent(p) => min + (max - min) * p / 100.0,
        CfValueKind::Percentile(p) => percentile(&st.sorted, *p),
        CfValueKind::Formula(f) => gridcraft_calc::evaluate(wb, sheet, at, f).to_number().unwrap_or(0.0),
    }
}

/// Preserve finite degenerate spans, but never use overflowed interpolation arithmetic.
fn geometry_fraction(n: f64, lo: f64, hi: f64, degenerate: f64) -> Option<f64> {
    if !n.is_finite() || !lo.is_finite() || !hi.is_finite() {
        return None;
    }
    if hi <= lo {
        return Some(degenerate);
    }
    let span = hi - lo;
    let offset = n - lo;
    let fraction = offset / span;
    (span.is_finite() && offset.is_finite() && fraction.is_finite()).then_some(fraction)
}

fn scale_color(n: f64, points: &[(f64, [u8; 3])]) -> Option<[u8; 3]> {
    if !n.is_finite() || points.iter().any(|(threshold, _)| !threshold.is_finite()) {
        return None;
    }
    Some(match points {
        [(a, ca), (b, cb)] => lerp(*ca, *cb, geometry_fraction(n, *a, *b, 0.0)?),
        [(a, ca), (m, cm), (b, cb)] => {
            if n <= *m {
                lerp(*ca, *cm, geometry_fraction(n, *a, *m, 0.0)?)
            } else {
                lerp(*cm, *cb, geometry_fraction(n, *m, *b, 1.0)?)
            }
        }
        _ => [255, 255, 255],
    })
}

fn bar_fraction(n: f64, lo: f64, hi: f64) -> Option<f64> {
    if !lo.is_finite() {
        return None;
    }
    geometry_fraction(n, lo.min(0.0), hi, 1.0).map(|fraction| fraction.clamp(0.0, 1.0))
}

fn lerp(a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        (a[0] as f64 + (b[0] as f64 - a[0] as f64) * t).round() as u8,
        (a[1] as f64 + (b[1] as f64 - a[1] as f64) * t).round() as u8,
        (a[2] as f64 + (b[2] as f64 - a[2] as f64) * t).round() as u8,
    ]
}

pub use gridcraft_model::icon_count;

struct RuleEval<'a> {
    wb: &'a Workbook,
    sheet: usize,
    cell: CellRef,
    first: CellRef,
    value: &'a Value,
    text: &'a str,
    stats: &'a RuleStats,
}

fn simple_rule_style<'a>(ctx: &RuleEval<'_>, rule: &'a CfRule) -> Option<&'a Style> {
    let RuleEval { wb, sheet, cell, first, value, text, stats } = ctx;
    match rule {
        CfRule::CellIs { op, a, b, style } => {
            let eval = |f: &str| gridcraft_calc::evaluate(wb, *sheet, *cell, &relative(f, *first, *cell));
            let av = eval(a);
            let matches = match op {
                CfOperator::Between | CfOperator::NotBetween => {
                    let bv = b.as_deref().map(eval).unwrap_or_default();
                    let (lo, hi) = if compare(&av, &bv) == std::cmp::Ordering::Greater { (bv, av) } else { (av, bv) };
                    let within = compare(value, &lo) != std::cmp::Ordering::Less && compare(value, &hi) != std::cmp::Ordering::Greater;
                    if *op == CfOperator::Between { within } else { !within }
                }
                CfOperator::Equal => compare(value, &av) == std::cmp::Ordering::Equal,
                CfOperator::NotEqual => compare(value, &av) != std::cmp::Ordering::Equal,
                CfOperator::Greater => compare(value, &av) == std::cmp::Ordering::Greater,
                CfOperator::Less => compare(value, &av) == std::cmp::Ordering::Less,
                CfOperator::GreaterOrEqual => compare(value, &av) != std::cmp::Ordering::Less,
                CfOperator::LessOrEqual => compare(value, &av) != std::cmp::Ordering::Greater,
            };
            (matches && !value.is_error()).then_some(style.as_ref())
        }
        CfRule::Expression { formula, style } => {
            gridcraft_calc::evaluate(wb, *sheet, *cell, &relative(formula, *first, *cell)).to_bool().unwrap_or(false).then_some(style.as_ref())
        }
        CfRule::ContainsText { text: needle, style } => {
            (!value.is_error() && text.to_lowercase().contains(&needle.to_lowercase())).then_some(style.as_ref())
        }
        CfRule::NotContainsText { text: needle, style } => {
            (value.is_error() || !text.to_lowercase().contains(&needle.to_lowercase())).then_some(style.as_ref())
        }
        CfRule::BeginsWith { text: needle, style } => {
            (!value.is_error() && text.to_lowercase().starts_with(&needle.to_lowercase())).then_some(style.as_ref())
        }
        CfRule::EndsWith { text: needle, style } => {
            (!value.is_error() && text.to_lowercase().ends_with(&needle.to_lowercase())).then_some(style.as_ref())
        }
        CfRule::Blanks { style } => (value.is_empty() || text.trim().is_empty()).then_some(style.as_ref()),
        CfRule::NoBlanks { style } => (!value.is_empty() && !text.trim().is_empty()).then_some(style.as_ref()),
        CfRule::Errors { style } => value.is_error().then_some(style.as_ref()),
        CfRule::NoErrors { style } => (!value.is_error()).then_some(style.as_ref()),
        CfRule::Duplicate { style } => {
            (!value.is_empty() && stats.counts.get(&text.to_lowercase()).copied().unwrap_or(0) > 1).then_some(style.as_ref())
        }
        CfRule::Unique { style } => {
            (!value.is_empty() && stats.counts.get(&text.to_lowercase()).copied().unwrap_or(0) == 1).then_some(style.as_ref())
        }
        _ => None,
    }
}

fn time_period_matches(wb: &Workbook, period: &str, serial: f64) -> bool {
    let today = gridcraft_core::date::serial_in_system(wb.date_system, gridcraft_calc::now_serial()).floor();
    let day = serial.floor();
    let date = |serial| gridcraft_core::date::datetime_from_serial(wb.date_system, serial);
    let week_start = today - date(today).map(|value| value.weekday as f64).unwrap_or(0.0);
    let (year, month) = date(today).map(|value| (value.year, value.month)).unwrap_or((2026, 1));
    let year_month = date(day).map(|value| (value.year, value.month));
    let month_matches = |offset: i32| {
        let shifted_month = month as i32 + offset;
        let shifted_year = year + (shifted_month - 1).div_euclid(12);
        let shifted_month = (shifted_month - 1).rem_euclid(12) + 1;
        year_month == Some((shifted_year, shifted_month as u32))
    };
    match period {
        "yesterday" => day == today - 1.0,
        "today" => day == today,
        "tomorrow" => day == today + 1.0,
        "last7Days" => day > today - 7.0 && day <= today,
        "lastWeek" => day >= week_start - 7.0 && day < week_start,
        "thisWeek" => day >= week_start && day < week_start + 7.0,
        "nextWeek" => day >= week_start + 7.0 && day < week_start + 14.0,
        "lastMonth" => month_matches(-1),
        "thisMonth" => month_matches(0),
        "nextMonth" => month_matches(1),
        _ => false,
    }
}

impl CfCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The first reason statistics are incomplete. Paint/export callers must disclose it;
    /// `look` omits affected rules instead of returning thresholds from partial statistics.
    pub fn incomplete(&self) -> Option<CfIncomplete> {
        self.incomplete
    }

    fn record_incomplete(&mut self, reason: CfIncomplete) {
        self.incomplete.get_or_insert(reason);
        self.exhausted |= reason != CfIncomplete::InvalidNumber;
    }

    fn statistics(&mut self, idx: usize, sh: &Sheet, cf: &CondFormat, need: StatsNeed) -> Option<&RuleStats> {
        if !self.stats.contains_key(&idx) {
            if self.exhausted {
                return None;
            }
            if self.stats.len() >= MAX_STATS_METADATA {
                self.record_incomplete(CfIncomplete::WorkLimit);
                return None;
            }
            if let Err(error) = reserve_map(&mut self.stats, &mut self.budget) {
                self.record_incomplete(error);
                return None;
            }
            let before = self.budget.bytes;
            let stats = RuleStats::build(sh, &cf.ranges, need, &mut self.budget);
            if let Err(error) = stats.as_ref() {
                self.record_incomplete(*error);
                // Partial builder payload has been dropped; consumed work is never refunded.
                self.budget.bytes = before;
            }
            self.stats.insert(idx, stats);
        }
        self.stats.get(&idx).and_then(|stats| stats.as_ref().ok())
    }

    /// Conditional look of `c`, or `None` when no rule applies. Check `incomplete` as well.
    pub fn look(&mut self, wb: &Workbook, sheet: usize, c: CellRef) -> Option<CfLook> {
        let sh = wb.sheet(sheet)?;
        if sh.cond_formats.is_empty() {
            return None;
        }
        let mut out = CfLook::default();
        let mut any = false;
        let mut rules: Vec<(usize, &CondFormat)> =
            sh.cond_formats.iter().enumerate().filter(|(_, cf)| cf.ranges.iter().any(|r| r.contains(c))).collect();
        rules.sort_by_key(|(_, cf)| cf.priority);
        let v = sh.value(c);
        let no_stats = RuleStats::default();
        for (idx, cf) in rules {
            let first = RangeRef::top_left(&cf.ranges).unwrap_or(c);
            let st: &RuleStats = match stats_need(&cf.rule) {
                Some(need) => match self.statistics(idx, sh, cf, need) {
                    Some(stats) => stats,
                    None => continue,
                },
                None => &no_stats,
            };
            let text = v.display();
            let context = RuleEval { wb, sheet, cell: c, first, value: &v, text: &text, stats: st };
            let mut invalid_geometry = false;
            let matched_style = simple_rule_style(&context, &cf.rule).or_else(|| match &cf.rule {
                CfRule::Top10 { bottom, percent, rank, style } => match v.as_f64() {
                    Some(n) if !st.sorted.is_empty() => {
                        let k = if *percent { ((st.sorted.len() as f64) * (*rank as f64) / 100.0).floor().max(1.0) as usize } else { *rank as usize };
                        let k = k.clamp(1, st.sorted.len());
                        let ok = if *bottom {
                            n <= st.sorted.get(k - 1).copied().unwrap_or(n)
                        } else {
                            n >= st.sorted.get(st.sorted.len() - k).copied().unwrap_or(n)
                        };
                        ok.then_some(style.as_ref())
                    }
                    _ => None,
                },
                CfRule::AboveAverage { below, equal, std_dev, style } => match v.as_f64() {
                    Some(n) => {
                        let thr = st.avg + if *below { -1.0 } else { 1.0 } * st.std * *std_dev as f64;
                        let ok = if *below { n < thr || (*equal && n == thr) } else { n > thr || (*equal && n == thr) };
                        ok.then_some(style.as_ref())
                    }
                    None => None,
                },
                CfRule::TimePeriod { period, style } => match v.as_f64() {
                    Some(n) if time_period_matches(wb, period, n) => Some(style.as_ref()),
                    _ => None,
                },
                CfRule::ColorScale { stops } => {
                    if let Some(n) = v.as_f64() {
                        let pts: Vec<(f64, [u8; 3])> = stops
                            .iter()
                            .map(|(k, col)| (threshold(wb, sheet, c, st, k), col.resolve(&wb.theme).unwrap_or([255, 255, 255])))
                            .collect();
                        let Some(rgb) = scale_color(n, &pts) else {
                            invalid_geometry = true;
                            return None;
                        };
                        if out.scale_fill.is_none() {
                            out.scale_fill = Some(rgb);
                            any = true;
                        }
                    }
                    None
                }
                CfRule::DataBar { min, max, color, gradient, show_value } => {
                    if let Some(n) = v.as_f64() {
                        let lo = threshold(wb, sheet, c, st, min);
                        let hi = threshold(wb, sheet, c, st, max);
                        let Some(frac) = bar_fraction(n, lo, hi) else {
                            invalid_geometry = true;
                            return None;
                        };
                        if out.bar.is_none() {
                            out.bar = Some((
                                (frac as f32) * 0.9 + 0.1 * (frac > 0.0) as u8 as f32,
                                color.resolve(&wb.theme).unwrap_or([0x63, 0x8E, 0xC6]),
                                *gradient,
                            ));
                            out.hide_value |= !show_value;
                            any = true;
                        }
                    }
                    None
                }
                CfRule::IconSet { set, thresholds, reverse, show_value } => {
                    if let Some(n) = v.as_f64() {
                        let k = icon_count(set);
                        // One threshold per icon, the first being the lower bound (as in the file);
                        // also accept the lower bound left out, else Excel's defaults.
                        let defaults;
                        let lower = match thresholds.len() {
                            n if n == k => thresholds.get(1..).unwrap_or_default(),
                            n if n + 1 == k => thresholds.as_slice(),
                            _ => {
                                defaults = default_icon_thresholds(set);
                                defaults.get(1..).unwrap_or_default()
                            }
                        };
                        let cuts: Vec<f64> = lower.iter().map(|t| threshold(wb, sheet, c, st, t)).collect();
                        if !n.is_finite() || cuts.iter().any(|cut| !cut.is_finite()) {
                            invalid_geometry = true;
                            return None;
                        }
                        let mut idx = cuts.iter().filter(|cut| n >= **cut).count();
                        if *reverse {
                            idx = k - 1 - idx.min(k - 1);
                        }
                        if out.icon.is_none() {
                            out.icon = Some((set.clone(), idx.min(k - 1)));
                            out.hide_value |= !show_value;
                            any = true;
                        }
                    }
                    None
                }
                _ => None,
            });
            if invalid_geometry {
                // Formula thresholds depend on the painted cell. Keep complete statistics
                // usable for other cells/rules while disclosing this failed evaluation.
                self.record_incomplete(CfIncomplete::InvalidNumber);
                continue;
            }
            if let Some(st) = matched_style {
                any = true;
                // Every matching rule applies; where two set the same part, the rule with the
                // higher priority (seen first) wins.
                match &mut out.style {
                    None => out.style = Some(st.clone()),
                    Some(prev) => layer_under(prev, st),
                }
                if cf.stop_if_true {
                    break;
                }
            }
        }
        any.then_some(out)
    }
}

/// Adds the parts `lower` sets that `style` leaves unset (differential styles: default parts
/// are unset).
fn layer_under(style: &mut Style, lower: &Style) {
    if style.fill.pattern == PatternType::None {
        style.fill = lower.fill;
    }
    if style.font.color == Color::Auto {
        style.font.color = lower.font.color;
    }
    style.font.bold |= lower.font.bold;
    style.font.italic |= lower.font.italic;
    style.font.strike |= lower.font.strike;
    if style.font.underline == Underline::None {
        style.font.underline = lower.font.underline;
    }
    let b = &mut style.borders;
    for (dst, src) in [
        (&mut b.left, lower.borders.left),
        (&mut b.right, lower.borders.right),
        (&mut b.top, lower.borders.top),
        (&mut b.bottom, lower.borders.bottom),
        (&mut b.diag_down, lower.borders.diag_down),
        (&mut b.diag_up, lower.borders.diag_up),
    ] {
        if dst.is_none() {
            *dst = src;
        }
    }
    if style.num_fmt.as_str() == "General" {
        style.num_fmt = lower.num_fmt.clone();
    }
}

/// Shifts a rule formula written for the top-left cell of the range to `c`.
fn relative(f: &str, first: CellRef, c: CellRef) -> String {
    if first == c {
        return f.to_string();
    }
    match gridcraft_formula::parse(f.trim_start_matches('=')) {
        Ok(e) => {
            gridcraft_formula::print(&gridcraft_formula::adjust::shift_relative(e, c.row as i64 - first.row as i64, c.col as i64 - first.col as i64))
        }
        Err(_) => f.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn bounded_book() -> Workbook {
        let mut wb = Workbook::new();
        let sh = wb.sheet_mut(0).unwrap();
        for row in 0..500 {
            sh.set_value(CellRef::new(row, 0), Value::Number(row as f64));
        }
        sh.cond_formats.push(CondFormat {
            ranges: vec![RangeRef::parse("A1:A500").unwrap()],
            rule: CfRule::ColorScale { stops: vec![(CfValueKind::Min, Color::rgb(0, 0, 0)), (CfValueKind::Max, Color::rgb(200, 200, 200))] },
            priority: 1,
            stop_if_true: false,
        });
        wb
    }

    #[test]
    fn statistics_refinement_charges_existing_text_when_hash_tables_grow() {
        let mut wb = Workbook::new();
        let sh = wb.sheet_mut(0).unwrap();
        for row in 0..32 {
            sh.set_value(CellRef::new(row, 0), Value::from(format!("{row:02}{}", "x".repeat(98))));
        }
        sh.cond_formats.push(CondFormat {
            ranges: vec![RangeRef::parse("A1:A32").unwrap()],
            rule: CfRule::Duplicate { style: Box::default() },
            priority: 1,
            stop_if_true: false,
        });
        let mut cache = CfCache::new();
        cache.budget.work_limit = 15_000;
        cache.look(&wb, 0, CellRef::default());
        assert_eq!(cache.incomplete(), Some(CfIncomplete::WorkLimit), "growth must charge rehashing prior keys, not just new strings");
    }

    #[test]
    fn statistics_refinement_invalid_numbers_do_not_exhaust_unrelated_rules() {
        let mut wb = bounded_book();
        let sh = wb.sheet_mut(0).unwrap();
        sh.cond_formats[0].ranges = vec![RangeRef::parse("A1:A2").unwrap()];
        sh.set_value(CellRef::new(1, 0), Value::Number(f64::NAN));
        sh.set_value(CellRef::new(0, 1), Value::Number(0.0));
        sh.set_value(CellRef::new(1, 1), Value::Number(10.0));
        sh.cond_formats.push(CondFormat { ranges: vec![RangeRef::parse("B1:B2").unwrap()], ..sh.cond_formats[0].clone() });
        let mut cache = CfCache::new();
        assert!(cache.look(&wb, 0, CellRef::default()).is_none());
        assert_eq!(cache.incomplete(), Some(CfIncomplete::InvalidNumber));
        assert_eq!(cache.look(&wb, 0, CellRef::new(1, 1)).and_then(|l| l.scale_fill), Some([200, 200, 200]));
        let work = cache.budget.work;
        assert!(cache.look(&wb, 0, CellRef::default()).is_none());
        assert_eq!(cache.budget.work, work, "invalid rule must retain its own failed result");
    }

    #[test]
    fn statistics_memory_limit_drops_partial_numbers_and_caches_failure() {
        let wb = bounded_book();
        let mut cache = CfCache::new();
        cache.budget.byte_limit = 4096;
        assert!(cache.look(&wb, 0, CellRef::default()).is_none());
        assert_eq!(cache.incomplete(), Some(CfIncomplete::MemoryLimit));
        let (bytes, work) = (cache.budget.bytes, cache.budget.work);
        assert_eq!(bytes, hash_bytes::<usize, Result<RuleStats, CfIncomplete>>(cache.stats.capacity()), "partial numbers released");
        assert!(cache.look(&wb, 0, CellRef::new(1, 0)).is_none());
        assert_eq!((cache.budget.bytes, cache.budget.work), (bytes, work), "failed builds are not retried per cell");
    }

    #[test]
    fn statistics_long_strings_are_bounded_and_prior_complete_rules_remain_usable() {
        let mut wb = bounded_book();
        let sh = wb.sheet_mut(0).unwrap();
        sh.set_value(CellRef::new(0, 1), Value::from("x".repeat(2000)));
        sh.cond_formats.push(CondFormat {
            ranges: vec![RangeRef::parse("B1").unwrap()],
            rule: CfRule::Duplicate { style: Box::default() },
            priority: 2,
            stop_if_true: false,
        });
        let mut cache = CfCache::new();
        cache.budget.byte_limit = 16 * 1024;
        let first = cache.look(&wb, 0, CellRef::default());
        assert!(first.is_some());
        assert!(cache.look(&wb, 0, CellRef::new(0, 1)).is_none());
        assert_eq!(cache.incomplete(), Some(CfIncomplete::MemoryLimit));
        let work = cache.budget.work;
        assert_eq!(cache.look(&wb, 0, CellRef::default()), first);
        assert_eq!(cache.budget.work, work);
        assert!(cache.budget.bytes <= cache.budget.byte_limit);
    }

    #[test]
    fn statistics_count_string_and_hash_capacities_together() {
        let mut wb = bounded_book();
        let sh = wb.sheet_mut(0).unwrap();
        for row in 0..500 {
            sh.set_value(CellRef::new(row, 0), Value::from(format!("key-{row}")));
        }
        sh.cond_formats[0].rule = CfRule::Duplicate { style: Box::default() };
        let mut cache = CfCache::new();
        cache.budget.byte_limit = 16 * 1024;
        assert!(cache.look(&wb, 0, CellRef::default()).is_none());
        assert_eq!(cache.incomplete(), Some(CfIncomplete::MemoryLimit));
        assert_eq!(cache.budget.bytes, hash_bytes::<usize, Result<RuleStats, CfIncomplete>>(cache.stats.capacity()));
    }

    #[test]
    fn statistics_work_limit_charges_candidates_outside_the_envelope() {
        let mut wb = bounded_book();
        let sh = wb.sheet_mut(0).unwrap();
        sh.cond_formats[0].ranges = vec![RangeRef::parse("A1").unwrap()];
        let mut cache = CfCache::new();
        cache.budget.work_limit = 10;
        assert!(cache.look(&wb, 0, CellRef::default()).is_none());
        assert_eq!(cache.incomplete(), Some(CfIncomplete::WorkLimit));
        let work = cache.budget.work;
        assert!(cache.look(&wb, 0, CellRef::default()).is_none());
        assert_eq!(cache.budget.work, work);
    }

    #[test]
    fn statistics_work_limit_charges_rejected_range_membership() {
        let mut wb = bounded_book();
        let sh = wb.sheet_mut(0).unwrap();
        let mut ranges = vec![RangeRef::parse("A500").unwrap(); 100];
        ranges.push(RangeRef::parse("A1").unwrap());
        sh.cond_formats[0].ranges = ranges;
        let mut cache = CfCache::new();
        cache.budget.work_limit = 150;
        assert!(cache.look(&wb, 0, CellRef::default()).is_none());
        assert_eq!(cache.incomplete(), Some(CfIncomplete::WorkLimit));
    }

    #[test]
    fn statistics_empty_and_reversed_ranges_do_not_enter_tree_range_queries() {
        let wb = bounded_book();
        let sh = wb.sheet(0).unwrap();
        let reversed = RangeRef { start: CellRef::new(9, 2), end: CellRef::default() };
        for ranges in [vec![], vec![reversed], vec![RangeRef::parse("Z1:Z3").unwrap()]] {
            let stats = RuleStats::build(sh, &ranges, StatsNeed::Numbers, &mut StatsBudget::default()).unwrap();
            assert!(stats.sorted.is_empty());
        }
        let stats = RuleStats::build(sh, &[reversed, RangeRef::parse("A1:A3").unwrap()], StatsNeed::Numbers, &mut StatsBudget::default()).unwrap();
        assert_eq!(stats.sorted, vec![0.0, 1.0, 2.0]);
    }

    #[test]
    fn statistics_metadata_exhaustion_is_bounded_and_does_not_discard_completed_entries() {
        let mut wb = bounded_book();
        let sh = wb.sheet_mut(0).unwrap();
        sh.cond_formats[0].ranges = vec![RangeRef::parse("A1").unwrap()];
        let first = sh.cond_formats[0].clone();
        sh.cond_formats = vec![first; MAX_STATS_METADATA + 1];
        let mut cache = CfCache::new();
        assert!(cache.look(&wb, 0, CellRef::default()).is_some());
        assert_eq!(cache.incomplete(), Some(CfIncomplete::WorkLimit));
        assert_eq!(cache.stats.len(), MAX_STATS_METADATA);
        let work = cache.budget.work;
        assert!(cache.look(&wb, 0, CellRef::default()).is_some());
        assert_eq!(cache.budget.work, work);
    }

    #[test]
    fn every_matching_rule_applies() {
        let mut s = crate::Session::new();
        s.new_workbook();
        s.execute("range.setValues", json!({"range": "A1", "values": [[200], [400], [510]]})).unwrap();
        let rule = |op: &str, value: &str, preset: &str| json!({"range": "A1:A3", "rule": {"type": "cellIs", "operator": op, "value": value, "preset": preset}});
        s.execute("home.conditionalFormat", rule("greaterThan", "300", "lightRedFill")).unwrap();
        s.execute("home.conditionalFormat", rule("greaterThan", "500", "redBorder")).unwrap();
        let d = s.doc().unwrap();
        let mut cache = CfCache::new();
        let mut look = |a: &str| cache.look(&d.wb, 0, CellRef::parse(a).unwrap()).and_then(|l| l.style);
        assert!(look("A1").is_none());
        let only_fill = look("A2").unwrap();
        assert_eq!(only_fill.fill.pattern, PatternType::Solid);
        assert!(only_fill.borders.top.is_none());
        let both = look("A3").unwrap();
        assert_eq!(both.fill, only_fill.fill);
        assert!(!both.borders.top.is_none());
    }

    #[test]
    fn cell_value_rules_on_blanks_and_errors() {
        let mut s = crate::Session::new();
        s.new_workbook();
        s.execute("range.setValues", json!({"range": "A1", "values": [[3, "=NA()"], [null, 7], ["=NA()", null], [" ", null], [7, null]]})).unwrap();
        let rule = |range: &str, r: serde_json::Value| json!({"range": range, "rule": r});
        s.execute("home.conditionalFormat", rule("A1:A5", json!({"type": "cellIs", "operator": "less", "value": "5", "preset": "lightRedFill"})))
            .unwrap();
        s.execute("home.conditionalFormat", rule("A1:A5", json!({"type": "noBlanks", "preset": "redBorder"}))).unwrap();
        s.execute("home.conditionalFormat", rule("B1:B2", json!({"type": "cellIs", "operator": "greater", "value": "5", "preset": "lightRedFill"})))
            .unwrap();
        let d = s.doc().unwrap();
        let mut cache = CfCache::new();
        let mut look = |a: &str| cache.look(&d.wb, 0, CellRef::parse(a).unwrap()).and_then(|l| l.style);
        let fill = |st: Option<Style>| st.is_some_and(|st| st.fill.pattern == PatternType::Solid);
        let border = |st: Option<Style>| st.is_some_and(|st| !st.borders.top.is_none());
        // Less than 5: 3 and the blank cell (as 0), not #N/A.
        assert_eq!([look("A1"), look("A2"), look("A3"), look("A5")].map(fill), [true, true, false, false]);
        // Greater than 5: 7, not #N/A.
        assert_eq!([look("B1"), look("B2")].map(fill), [false, true]);
        // No blanks: neither the empty cell nor the one holding a space.
        assert_eq!([look("A1"), look("A2"), look("A4")].map(border), [true, false, false]);
    }

    /// The statistics as every rule used to get them: one pass collecting both parts.
    fn all_stats_before(sh: &Sheet, ranges: &[RangeRef]) -> RuleStats {
        let mut nums = Vec::new();
        let mut counts = HashMap::new();
        for r in ranges {
            let Some(r) = sh.used_range().and_then(|u| r.intersection(&u)) else { continue };
            for c in r.iter() {
                let v = sh.value(c);
                if let Value::Number(n) = v {
                    nums.push(n);
                }
                if !v.is_empty() {
                    *counts.entry(v.display().to_lowercase()).or_insert(0) += 1;
                }
            }
        }
        nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = nums.len().max(1) as f64;
        let avg = nums.iter().sum::<f64>() / n;
        let std = (nums.iter().map(|x| (x - avg).powi(2)).sum::<f64>() / n).sqrt();
        RuleStats { sorted: nums, avg, std, counts }
    }

    #[test]
    fn needed_statistics_equal_the_full_ones() {
        let mut s = crate::Session::new();
        s.new_workbook();
        s.execute("range.setValues", json!({"range": "A1", "values": [[3, "x"], [7, "X"], [3, null], ["=NA()", 2.5], [" ", 9], [-1, "y"]]})).unwrap();
        let d = s.doc().unwrap();
        let sh = d.wb.sheet(0).unwrap();
        for ranges in ["A1:B6", "A1:A3", "B2:B9", "D1:E4", "A1:A2"].map(|r| vec![RangeRef::parse(r).unwrap()]) {
            let full = all_stats_before(sh, &ranges);
            let numbers = RuleStats::build(sh, &ranges, StatsNeed::Numbers, &mut StatsBudget::default()).unwrap();
            assert_eq!(numbers.sorted, full.sorted);
            let average = RuleStats::build(sh, &ranges, StatsNeed::Average { deviation: false }, &mut StatsBudget::default()).unwrap();
            assert_eq!((average.avg.to_bits(), average.std), (full.avg.to_bits(), 0.0));
            let deviation = RuleStats::build(sh, &ranges, StatsNeed::Average { deviation: true }, &mut StatsBudget::default()).unwrap();
            assert_eq!((deviation.avg.to_bits(), deviation.std.to_bits()), (full.avg.to_bits(), full.std.to_bits()));
            assert_eq!(RuleStats::build(sh, &ranges, StatsNeed::Counts, &mut StatsBudget::default()).unwrap().counts, full.counts);
        }
    }

    #[test]
    fn rules_that_read_statistics_still_match() {
        let mut s = crate::Session::new();
        s.new_workbook();
        s.execute("range.setValues", json!({"range": "A1", "values": [[1, "a"], [2, "A"], [3, "b"], [10, "c"], [2, null]]})).unwrap();
        let rule = |range: &str, r: serde_json::Value| json!({"range": range, "rule": r});
        s.execute("home.conditionalFormat", rule("B1:B5", json!({"type": "duplicate", "preset": "lightRedFill"}))).unwrap();
        s.execute("home.conditionalFormat", rule("A1:A5", json!({"type": "top10", "rank": 1, "preset": "redBorder"}))).unwrap();
        s.execute("home.conditionalFormat", rule("A1:A5", json!({"type": "colorScale"}))).unwrap();
        let d = s.doc().unwrap();
        let mut cache = CfCache::new();
        let mut look = |a: &str| cache.look(&d.wb, 0, CellRef::parse(a).unwrap());
        let fill = |l: Option<CfLook>| l.and_then(|l| l.style).is_some_and(|st| st.fill.pattern == PatternType::Solid);
        // Duplicates are case-insensitive texts: "a" and "A".
        assert_eq!(["B1", "B2", "B3", "B4", "B5"].map(|a| fill(look(a))), [true, true, false, false, false]);
        // The top value gets the border, the scale runs from the minimum to the maximum.
        assert!(look("A4").and_then(|l| l.style).is_some_and(|st| !st.borders.top.is_none()));
        assert!(look("A1").and_then(|l| l.style).is_none_or(|st| st.borders.top.is_none()));
        assert_eq!(look("A1").and_then(|l| l.scale_fill), Some([0xF8, 0x69, 0x6B]));
        assert_eq!(look("A4").and_then(|l| l.scale_fill), Some([0x63, 0xBE, 0x7B]));
    }

    /// Looking cells up does work in proportion to the cells asked for, not the rule's range: the
    /// statistics are only gathered for rules that read them, and once per rule.
    #[test]
    fn lookups_do_not_scan_the_range_for_rules_without_statistics() {
        let mut s = crate::Session::new();
        s.new_workbook();
        let rows: Vec<serde_json::Value> = (1..=10_000).map(|i| json!([i, i % 7])).collect();
        s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
        s.execute(
            "home.conditionalFormat",
            json!({"range": "A1:B10000", "rule": {"type": "expression", "formula": "=MOD(ROW(),2)=0", "preset": "lightRedFill"}}),
        )
        .unwrap();
        s.execute(
            "home.conditionalFormat",
            json!({"range": "A1:B10000", "rule": {"type": "cellIs", "operator": "greater", "value": "5", "preset": "redBorder"}}),
        )
        .unwrap();
        s.execute("home.conditionalFormat", json!({"range": "A1:B10000", "rule": {"type": "noBlanks", "preset": "redBorder"}})).unwrap();
        let d = s.doc().unwrap();
        let mut cache = CfCache::new();
        SCANNED.with(|n| n.set(0));
        for r in 1..=50 {
            for col in ["A", "B"] {
                cache.look(&d.wb, 0, CellRef::parse(&format!("{col}{r}")).unwrap());
            }
        }
        assert_eq!(SCANNED.with(|n| n.get()), 0);
        // A statistics rule scans stored candidates once, including the other column rejected
        // by its envelope. Repeated looks do not repeat those 20,000 candidate visits.
        s.execute("home.conditionalFormat", json!({"range": "A1:A10000", "rule": {"type": "top10", "preset": "redBorder"}})).unwrap();
        let d = s.doc().unwrap();
        let mut cache = CfCache::new();
        SCANNED.with(|n| n.set(0));
        for r in 1..=50 {
            cache.look(&d.wb, 0, CellRef::parse(&format!("A{r}")).unwrap());
        }
        assert_eq!(SCANNED.with(|n| n.get()), 20_000);
    }

    #[test]
    fn cell_value_operator_names() {
        let mut s = crate::Session::new();
        s.new_workbook();
        s.execute("range.setValues", json!({"range": "A1", "values": [[3], [7]]})).unwrap();
        let rule = |op: &str| json!({"range": "A1:A2", "rule": {"type": "cellIs", "operator": op, "value": "5", "preset": "lightRedFill"}});
        s.execute("home.conditionalFormat", rule("lessThan")).unwrap();
        assert!(s.execute("home.conditionalFormat", rule("smaller")).is_err());
        let d = s.doc().unwrap();
        let mut cache = CfCache::new();
        let mut look = |a: &str| cache.look(&d.wb, 0, CellRef::parse(a).unwrap()).and_then(|l| l.style).is_some();
        assert_eq!([look("A1"), look("A2")], [true, false]);
    }

    #[test]
    fn text_rules_skip_error_values() {
        let mut s = crate::Session::new();
        s.new_workbook();
        let values = json!([["=NA()", "=NA()", "=1/0", "=NA()"], ["abc", "#x", "x!", "abc"], ["xyz", "y", "y", "xyz"]]);
        s.execute("range.setValues", json!({"range": "A1", "values": values})).unwrap();
        let rule = |range: &str, kind: &str, text: &str| json!({"range": range, "rule": {"type": kind, "text": text, "preset": "lightRedFill"}});
        s.execute("home.conditionalFormat", rule("A1:A3", "containsText", "a")).unwrap();
        s.execute("home.conditionalFormat", rule("B1:B3", "beginsWith", "#")).unwrap();
        s.execute("home.conditionalFormat", rule("C1:C3", "endsWith", "!")).unwrap();
        s.execute("home.conditionalFormat", rule("D1:D3", "notContainsText", "a")).unwrap();
        let d = s.doc().unwrap();
        let mut cache = CfCache::new();
        let mut look = |a: &str| cache.look(&d.wb, 0, CellRef::parse(a).unwrap()).and_then(|l| l.style).is_some();
        // Contains "a": abc, never #N/A although its text holds an "a".
        assert_eq!([look("A1"), look("A2"), look("A3")], [false, true, false]);
        // Begins with "#": #x, never #N/A.
        assert_eq!([look("B1"), look("B2"), look("B3")], [false, true, false]);
        // Ends with "!": x!, never #DIV/0!.
        assert_eq!([look("C1"), look("C2"), look("C3")], [false, true, false]);
        // Does not contain "a": #N/A and xyz, not abc.
        assert_eq!([look("D1"), look("D2"), look("D3")], [true, false, true]);
    }
}
