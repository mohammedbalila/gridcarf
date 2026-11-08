# Runtime selection memory

Updated: 2026-11-08

Each document view owns one `Vec<Selection>`. A selection includes its active cell,
anchor, and all ranges. Private integer labels associate remembered selections with
numeric runtime sheet UIDs. Names, ordinals, file IDs (including zero and duplicate
file IDs), pointer addresses, and sheet-content equality are not identity.

The vector has a current-sheet-order prefix, followed by known dormant UIDs that
real restoration roots can still restore, followed by anonymous legacy entries.
Whenever a dormant or anonymous suffix follows the prefix, the prefix is complete.
A newly created or copied UID receives its own default selection or the command's
explicit anchor, never an old ordinal's value or an anonymous entry.

## Public edits and supported boundaries

Initial adoption preserves empty, short, full, and long vectors exactly. Existing
prefix entries bind to current unique UIDs; surplus entries remain anonymous,
including entries equal to the default selection. Metadata adoption alone does not
pad, trim, reorder, or rewrite the active slot.

Same-length public replacements, permutations, and edits modify the published
slots. An observed length change adopts the actual current prefix and treats the
actual remainder as anonymous. A topology-only change instead retains the old
labels and remaps by UID. An untracked active-sheet/topology refresh keeps the
caller's live selection for the actual active UID, without guessing an outgoing
write. Navigating to the same UID keeps the live selection; another existing UID
restores its remembered selection, or the default if absent.

Commands, direct group completion, and successful window lifecycle operations are
supported reconciliation boundaries. Consumers such as RefEdit can read inactive
selections from the current prefix after a successful boundary. Arbitrary raw
reads immediately after public mutation have no coherence guarantee.

Duplicate injected runtime UIDs retain positional, unadopted behavior and preserve
all payload. They do not trigger UID repair or identity-based pruning. No identity
or bound claim applies until a later unique-layout adoption succeeds.

## Windows, debugger, and dry runs

Parked windows and debugger/dry-run snapshots carry their exact vector together
with matching private labels, captured active UID, and deferred-refresh state.
Internal transfer is not an external vector reset. A window whose captured UID
still exists restores its exact live selection, even after rename or reordering.
If it vanished, activation chooses the workbook's valid current sheet, otherwise
the first sheet, otherwise no sheet and a default live selection. The target uses
its own remembered selection. The vanished live selection is remembered only for
its captured UID while that UID remains restorable. Parked cleanup never eagerly
changes the captured live UID merely because another window temporarily deleted
its sheet; Undo before reactivation can still restore that original live view.

Fallible preparation precedes reversible view/flag/stamp/window changes. Result
callers receive an error; legacy void callers log a fixed diagnostic and preserve
state. Closing a document remains mandatory: if the preferred surviving view
cannot be prepared, a surviving already-live window is selected without remapping,
preferring the intended document. Closed documents, windows, groups, and their
debugger are released. No closed content is resurrected.

Open groups capture the numeric active UID beside their existing saved selection.
Rollback restores that identity before reconciliation, even when view-only navigation
changed the active scalar through the shared workbook.

Workbook/history restoration after rollback or failure takes priority over new
view reconciliation. If that reconciliation fails, payload and old UID labels
remain paired, a private deferred flag is set, and cleanup is retried later. There
is no current-prefix or bound claim while deferred. Debugger capture after an
already-published step or opened start group follows the same rule and retains an
exact current snapshot. Its captured identity comes from the actual active UID
when that UID occurs once (a linear scan); this alone does not establish global
UID uniqueness. Normal Restore preparation still checks the whole topology.
Before another debugger step, fallible Restore completes before taking its machine.
Dry-run snapshots retain one exact live pair for every original document and restore
those pairs before the original window ownership. Other documents' selection payload
and metadata are temporarily cloned once per snapshot nesting level, within the
existing Clone allocation limits. Snapshot lifetimes defer pruning; debugger comparison
workbooks are not restoration roots.

## Retention and allocation limits

Cleanup borrows current, saved, Undo, Redo, open-group and captured-redo workbook
roots and collects only numeric UIDs. For a reconciled view in a stable session,
with fixed anonymous count `L` and distinct actually restorable identities `U`,
remembered payload length is at most `L + U`. The ordinary insert/select/switch/Undo
cycle with two restorable identities is at most `L + 2`. The public regression runs
1,016 cycles and 4,064 commands. External resets, duplicate injection, and repeated
recovery/adoption are outside this fixed-`L` claim.

Outer capacity can retain a legitimate high-water allocation. Account separately
for outer vector capacity, nested range capacities, and metadata capacities. These
are container measurements, not RSS, process heap, GPU/native retention, or a
history-budget-wide bound. Owned preparation uses fallible reservations and a
private per-document test budget. Existing intentional `Clone` operations and
allocations inside dependencies retain their existing allocation-failure limits;
this is not a universal OOM guarantee.

## Recovery contract

Recovery v1 capture and decode preserve the **entire ordered vector** exactly,
including dormant-known and anonymous entries. Runtime UIDs and labels are not
serialized. Codec capture/decode does not adopt, normalize, pad, trim, or reorder.
Later runtime adoption can classify a recovered tail as anonymous, changing `L`;
there is no universal bound across repeated exact capture/recovery cycles.

A future runtime recovery capture must successfully reconcile its selected view
before freezing metadata or owning/submitting a job. Failed refresh must prevent
that capture rather than serialize a stale indexed prefix. This bookkeeping does
not activate persistence, establish native provenance, qualify encrypted content,
or replace sealed recovery authority checks.

## Revision history

| Date | Change |
|---|---|
| 2026-11-08 | Restore all original live document pairs after previews and the captured group selection UID after view-only rollback. |
| 2026-11-08 | Document runtime UID ownership, paired views, deferred refresh, conditional retention bounds, and exact recovery separation. |
