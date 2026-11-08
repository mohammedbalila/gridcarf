//! Runtime UID labels beside the view's sole persistent selection payload.

use crate::{EngineError, Result, Selection};
use gridcraft_core::{CellRef, RangeRef};
use gridcraft_model::Workbook;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SelectionMemory {
    /// Current prefix, then dormant identities. Remaining payload entries are anonymous.
    labels: Vec<u64>,
    current: Vec<u64>,
    /// None marks ambiguous/unadopted layout; a changed length resets slot ownership.
    published_len: Option<usize>,
    active: Option<u64>,
}

pub(crate) enum Transition<'a> {
    Refresh,
    Restore { sheet: usize, captured: Option<u64> },
    Navigate(usize),
    Publish(&'a Selection),
}

pub(crate) struct PreparedSelectionMemory {
    owner: SelectionMemory,
    output: Option<Vec<Selection>>,
    moves: Vec<Option<usize>>,
    live: Option<Selection>,
}

#[derive(Default)]
struct Preparation<'a> {
    keep: Option<&'a [u64]>,
    #[cfg(test)]
    remaining: Option<usize>,
}

fn preparation_error() -> EngineError {
    EngineError::Other("selection memory preparation failed".into())
}

impl Preparation<'_> {
    fn vector<T>(&mut self, len: usize) -> Result<Vec<T>> {
        let mut result = Vec::new();
        if len != 0 {
            #[cfg(test)]
            if let Some(remaining) = &mut self.remaining {
                *remaining = remaining.checked_sub(1).ok_or_else(preparation_error)?;
            }
            result.try_reserve_exact(len).map_err(|_| preparation_error())?;
        }
        Ok(result)
    }
    fn copy<T: Copy>(&mut self, values: &[T]) -> Result<Vec<T>> {
        let mut result = self.vector(values.len())?;
        result.extend_from_slice(values);
        Ok(result)
    }
    fn selection(&mut self, selection: &Selection) -> Result<Selection> {
        Ok(Selection { active: selection.active, anchor: selection.anchor, ranges: self.copy(&selection.ranges)? })
    }
    fn default_selection(&mut self) -> Result<Selection> {
        let cell = CellRef::default();
        let mut ranges = self.vector(1)?;
        ranges.push(RangeRef::cell(cell));
        Ok(Selection { active: cell, anchor: cell, ranges })
    }
    fn order(&mut self, wb: &Workbook) -> Result<Vec<u64>> {
        let mut order = self.vector(wb.sheets.len())?;
        order.extend(wb.sheets.iter().map(|sh| sh.uid.get()));
        Ok(order)
    }
    fn index(&mut self, ids: &[u64]) -> Result<Vec<(u64, usize)>> {
        let mut sorted = self.vector(ids.len())?;
        sorted.extend(ids.iter().copied().enumerate().map(|(i, uid)| (uid, i)));
        sorted.sort_unstable_by_key(|entry| entry.0);
        Ok(sorted)
    }
}

fn find(sorted: &[(u64, usize)], uid: u64) -> Option<usize> {
    sorted.binary_search_by_key(&uid, |entry| entry.0).ok().and_then(|i| sorted.get(i)).map(|entry| entry.1)
}

fn unique(sorted: &[(u64, usize)]) -> bool {
    sorted.windows(2).all(|pair| pair.first().map(|entry| entry.0) != pair.last().map(|entry| entry.0))
}

impl SelectionMemory {
    pub(crate) fn active_uid(&self) -> Option<u64> {
        self.active
    }

    pub(crate) fn prepare_view(
        &self,
        values: &[Selection],
        d: &crate::DocState,
        live: &Selection,
        transition: Transition<'_>,
        keep: Option<&[u64]>,
    ) -> Result<PreparedSelectionMemory> {
        self.prepare_with(
            values,
            &d.wb,
            &d.wb,
            live,
            transition,
            &mut Preparation {
                keep,
                #[cfg(test)]
                remaining: d.selection_preparation_budget,
            },
        )
    }
    pub(crate) fn prepare(
        &self,
        values: &[Selection],
        before: &Workbook,
        after: &Workbook,
        live: &Selection,
        transition: Transition<'_>,
    ) -> Result<PreparedSelectionMemory> {
        self.prepare_with(values, before, after, live, transition, &mut Preparation::default())
    }

    #[cfg(test)]
    pub(crate) fn prepare_with_budget(
        &self,
        values: &[Selection],
        before: &Workbook,
        after: &Workbook,
        live: &Selection,
        transition: Transition<'_>,
        budget: usize,
    ) -> Result<PreparedSelectionMemory> {
        self.prepare_with(values, before, after, live, transition, &mut Preparation { keep: None, remaining: Some(budget) })
    }

    /// Caller supplies only borrowed numeric identities, after temporary restoration roots die.
    pub(crate) fn prepare_prune(&self, values: &[Selection], wb: &Workbook, live: &Selection, keep: &[u64]) -> Result<PreparedSelectionMemory> {
        self.prepare_with(
            values,
            wb,
            wb,
            live,
            Transition::Refresh,
            &mut Preparation {
                keep: Some(keep),
                #[cfg(test)]
                remaining: None,
            },
        )
    }

    fn prepare_with(
        &self,
        values: &[Selection],
        before: &Workbook,
        after: &Workbook,
        live: &Selection,
        transition: Transition<'_>,
        budget: &mut Preparation<'_>,
    ) -> Result<PreparedSelectionMemory> {
        let before_order = budget.order(before)?;
        let current = budget.order(after)?;
        let before_index = budget.index(&before_order)?;
        let current_index = budget.index(&current)?;
        if !unique(&before_index) || !unique(&current_index) {
            return Self::positional(values, before, after, live, transition, budget);
        }
        let reset = self.published_len != Some(values.len());
        let old_labels = if reset { before_order.get(..values.len().min(before_order.len())).unwrap_or_default() } else { &self.labels };
        let old_index = budget.index(old_labels)?;
        let actual = match transition {
            Transition::Restore { captured, .. } => captured,
            _ => before_order.get(before.active_sheet()).copied(),
        };
        let target = match transition {
            Transition::Navigate(i) => current.get(i).copied().or(actual),
            Transition::Restore { sheet, .. } => current.get(sheet).copied(),
            _ => current.get(after.active_sheet()).copied(),
        };
        let save_outgoing = match transition {
            Transition::Refresh => false,
            Transition::Restore { .. } => target != actual,
            Transition::Navigate(i) => i < current.len() && target != actual,
            Transition::Publish(_) => true,
        };
        let outgoing = actual.filter(|_| save_outgoing);
        let mut keep = budget.copy(budget.keep.unwrap_or_default())?;
        keep.sort_unstable();
        let pruning = budget.keep.is_some();
        let retained = |uid| !pruning || keep.binary_search(&uid).is_ok();
        let old_order = if reset { &before_order } else { &self.current };
        let prune = old_labels.iter().any(|uid| find(&current_index, *uid).is_none() && !retained(*uid));
        if (matches!(transition, Transition::Refresh) || matches!(transition, Transition::Restore { .. }) && target.is_some() && target == actual)
            && *old_order == current
            && !prune
        {
            return Ok(PreparedSelectionMemory {
                owner: Self { labels: budget.copy(old_labels)?, current, published_len: Some(values.len()), active: target },
                output: None,
                moves: Vec::new(),
                live: None,
            });
        }
        // Reserve the union's upper bound once; never insert/remove payload repeatedly.
        let maximum =
            current.len().checked_add(old_labels.len()).and_then(|n| n.checked_add(usize::from(outgoing.is_some()))).ok_or_else(preparation_error)?;
        let mut labels = budget.vector(maximum)?;
        labels.extend_from_slice(&current);
        labels.extend(old_labels.iter().copied().filter(|uid| find(&current_index, *uid).is_none() && retained(*uid)));
        if let Some(uid) = outgoing
            && find(&current_index, uid).is_none()
            && find(&old_index, uid).is_none()
            && retained(uid)
        {
            labels.push(uid);
        }
        let anonymous = values.len().checked_sub(old_labels.len()).ok_or_else(preparation_error)?;
        let length = labels.len().checked_add(anonymous).ok_or_else(preparation_error)?;
        let mut output = budget.vector(length)?;
        // Placeholders own no ranges. Every published slot is filled below or moved at apply.
        output.resize_with(length, || Selection { active: CellRef::default(), anchor: CellRef::default(), ranges: Vec::new() });
        let mut moves = budget.vector(values.len())?;
        moves.resize(values.len(), None);
        for (destination, uid) in labels.iter().copied().enumerate() {
            let slot = output.get_mut(destination).ok_or_else(preparation_error)?;
            if Some(uid) == target
                && let Transition::Publish(chosen) = transition
            {
                *slot = budget.selection(chosen)?;
            } else if Some(uid) == outgoing {
                *slot = budget.selection(live)?;
            } else if let Some(source) = find(&old_index, uid) {
                *moves.get_mut(source).ok_or_else(preparation_error)? = Some(destination);
            } else {
                *slot = budget.default_selection()?;
            }
        }
        for (offset, source) in (old_labels.len()..values.len()).enumerate() {
            *moves.get_mut(source).ok_or_else(preparation_error)? = Some(labels.len().checked_add(offset).ok_or_else(preparation_error)?);
        }
        let next_live = match transition {
            Transition::Publish(chosen) => Some(budget.selection(chosen)?),
            Transition::Navigate(i) if i < current.len() && target != actual => {
                let uid = current.get(i).copied().ok_or_else(preparation_error)?;
                Some(if let Some(source) = find(&old_index, uid) {
                    budget.selection(values.get(source).ok_or_else(preparation_error)?)?
                } else {
                    budget.selection(output.get(i).ok_or_else(preparation_error)?)?
                })
            }
            Transition::Restore { .. } if target != actual || target.is_none() => Some(match target.and_then(|uid| find(&old_index, uid)) {
                Some(source) => budget.selection(values.get(source).ok_or_else(preparation_error)?)?,
                None => budget.default_selection()?,
            }),
            _ => None,
        };
        Ok(PreparedSelectionMemory {
            owner: Self { labels, current, published_len: Some(length), active: target },
            output: Some(output),
            moves,
            live: next_live,
        })
    }

    fn positional(
        values: &[Selection],
        before: &Workbook,
        after: &Workbook,
        live: &Selection,
        transition: Transition<'_>,
        budget: &mut Preparation<'_>,
    ) -> Result<PreparedSelectionMemory> {
        let mut plan = PreparedSelectionMemory { owner: Self::default(), output: None, moves: Vec::new(), live: None };
        match transition {
            Transition::Navigate(i) if i < after.sheets.len() && i != before.active_sheet() => {
                let length = values.len().max(after.sheets.len());
                let mut output = budget.vector(length)?;
                let mut moves = budget.vector(values.len())?;
                moves.extend((0..values.len()).map(Some));
                for slot in 0..length {
                    output.push(if slot == before.active_sheet() {
                        budget.selection(live)?
                    } else if slot >= values.len() {
                        budget.default_selection()?
                    } else {
                        Selection { active: CellRef::default(), anchor: CellRef::default(), ranges: Vec::new() }
                    });
                }
                if let Some(source) = moves.get_mut(before.active_sheet()) {
                    *source = None;
                }
                plan.live = Some(if let Some(target) = values.get(i) { budget.selection(target)? } else { budget.default_selection()? });
                plan.output = Some(output);
                plan.moves = moves;
            }
            Transition::Publish(chosen) => plan.live = Some(budget.selection(chosen)?),
            _ => {}
        }
        Ok(plan)
    }
}

impl PreparedSelectionMemory {
    /// Discard an unapplied plan to stage its already-prepared live selection in a Ctx.
    pub(crate) fn into_selection(self) -> Option<Selection> {
        self.live
    }

    /// All reservations and selection copies finished in prepare. Stored ranges move once.
    /// Values and layout must remain the exact prepared inputs until apply; callers must not
    /// edit or replace them between preparation and publication.
    pub(crate) fn apply(self, values: &mut Vec<Selection>, owner: &mut SelectionMemory, live: &mut Selection) {
        if let Some(mut output) = self.output {
            for (value, destination) in values.drain(..).zip(self.moves) {
                if let Some(slot) = destination.and_then(|i| output.get_mut(i)) {
                    *slot = value;
                }
            }
            *values = output;
        }
        *owner = self.owner;
        if let Some(selection) = self.live {
            *live = selection;
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod command_tests;

impl crate::DocState {
    pub(crate) fn prepare_selection(&self, after: &Workbook, transition: Transition<'_>) -> Result<PreparedSelectionMemory> {
        #[cfg(test)]
        {
            self.selection_memory.prepare_with(
                &self.sheet_selections,
                &self.wb,
                after,
                &self.selection,
                transition,
                &mut Preparation { keep: None, remaining: self.selection_preparation_budget },
            )
        }
        #[cfg(not(test))]
        self.selection_memory.prepare(&self.sheet_selections, &self.wb, after, &self.selection, transition)
    }

    pub(crate) fn prepare_selection_prune(&self, keep: &[u64]) -> Result<PreparedSelectionMemory> {
        #[cfg(test)]
        {
            self.selection_memory.prepare_with(
                &self.sheet_selections,
                &self.wb,
                &self.wb,
                &self.selection,
                Transition::Refresh,
                &mut Preparation { keep: Some(keep), remaining: self.selection_preparation_budget },
            )
        }
        #[cfg(not(test))]
        self.selection_memory.prepare_prune(&self.sheet_selections, &self.wb, &self.selection, keep)
    }

    pub(crate) fn refresh_selection_memory(&mut self) -> Result<()> {
        let plan = self.prepare_selection(&self.wb, Transition::Refresh)?;
        plan.apply(&mut self.sheet_selections, &mut self.selection_memory, &mut self.selection);
        self.selection_refresh_deferred = false;
        Ok(())
    }

    pub(crate) fn restore_selection_memory(&mut self) {
        if self.refresh_selection_memory().is_err() {
            self.selection_refresh_deferred = true;
            log::warn!("Selection memory refresh deferred.");
        }
    }
}

#[cfg(test)]
mod restore_tests;
