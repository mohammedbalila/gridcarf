# Recovery codec

> **Last reviewed:** 2026-11-08 · **Last updated:** 2026-11-08 · **Change:** minor (bounded detached restore; native recovery remains inactive)

The engine's recovery codec encodes and validates a supported, lossless subset of modeled
workbook state plus preserved XLSX graphs. It is an internal foundation, not an active recovery
feature, a general interchange format or an increase in product readiness. Ordinary AutoSave
still cannot recover untitled workbooks and asks for Save As. Host qualification is blocked;
Windows runtime is **NOT_TESTED** and no Excel oracle has run.

## Public boundary

[`gridcraft_engine::recovery`](../crates/engine/src/recovery.rs) exports:

| Item | Current interface |
|---|---|
| `decode(&[u8])` | Complete fixed-limit validation returning `Result<ValidatedRecovery, RecoveryError>` |
| `RecoveryCapture<'a>` | Private borrowed fields; public `prepare()`, with no public general constructor |
| `PreparedRecovery<'a>` | Private frozen manifest and payload-reference plan; `len()`, `is_empty()`, `write_to(&mut dyn Write)` |
| `ValidatedRecovery` | Private owned state; recorded `id()`, `sequence()`, `captured_revision()`, consumed by `restore` |
| `restore(ValidatedRecovery)` | `Result<DocState, RecoveryError>`; detached untrusted unsaved copy, no Session installation |
| `RecoveryId`, `RecoveryProvenance` | Inert recorded data; IDs parse/serialize as 32 lowercase hexadecimal digits |
| `RecoveryError` | `UnsupportedVersion`, `Malformed`, `TooLarge`, `UnsupportedRepresentation`, `Ineligible`, `Io` |

Validation does not restore a Session or admit Calc dependencies, dynamic ranges, equations or
UI projections. Live policy-aware capture/CaptureGuard ownership, runtime provenance, storage
and trust remain separate gates. Untrusted bytes labelled `Plain` never grant runtime Plain
provenance; recorded Unknown/EncryptedOrigin and persisted encryption-origin facts refuse.
There is no public arbitrary capture constructor, limits override or workbook/part extraction
getter on the validated type. The separate restore boundary admits a complete detached document.

Errors retain finite variants/static reasons, version numbers, provenance or sink ErrorKind,
without attached input, path, helper/serde text or OS messages. `Display`/`Error` are implemented.
Validated Debug contains only inert ID/sequence/revision and worksheet count, not workbook content.
Preparation completes rejecting semantic work before writing. Prepared writes stream frozen
bytes without new serialization, Workbook traversal or semantic work; sink failures remain
ErrorKind. Neither the format nor successful writing authenticates data or guarantees atomic
publication, durable storage, directory persistence, confidentiality or power-cut recovery.
No native recovery store, scheduler or activation is enabled by this codec.

## Version 1 bytes and ordering

All header integers are little endian; lengths count bytes.

| Offset | Width | Value |
|---|---:|---|
| 0 | 8 | ASCII `GCRCV001` |
| 8 | 4 | version `1` (`u32`) |
| 12 | 8 | total envelope length (`u64`), exactly the input length |
| 20 | 8 | manifest length (`u64`) |
| 28 | declared length | UTF-8 JSON manifest |
| after manifest | repeated | `u64` payload length followed by exactly that many opaque bytes |

The manifest root is ordered `workbook`, `metadata`, `sidecars`. Sidecars contain `workbook`
then `sheets`. Workbook sidecar fields are `source_dir`, `source_path`, `macros_unread`,
`cell_picture_groups`, `preserved`; sheet records are `ordinal`, `file_id`, `spill`,
`spill_ranges`, `preserved`, in workbook sheet order. The sidecar count and dense ordinals must
cover every sheet. Required nullable fields must appear as null rather than disappear; required
empty lists/maps remain present. None and an explicitly present empty preserved owner differ.

A preserved record orders `package_rels`, `rels`, `parts`, `elements`, `vml`, `sheets`.
Opaque payload descriptors contain zero-based `frame` and byte `length`. Occurrences follow
workbook preserved owner, then each sheet preserved owner in workbook order; within an owner,
parts in stored order then optional VML. Empty bodies still have a length prefix and consume
an occurrence. Shared or equal opaque bodies are not deduplicated. Descriptors, every prefix,
last body and exact EOF are validated before copying payloads or constructing the borrowed plan.
See [DTOs and borrowed projections](../crates/engine/src/recovery/manifest.rs) and
[framing](../crates/engine/src/recovery/manifest/frame.rs).

Acceptance is stricter than generic JSON parsing: decoded duplicate keys, noncanonical base64,
numeric/A1 map aliases, malformed picture pairs and invalid field shapes refuse. Added wrappers
reject unknown/missing fields. After complete model/graph validation and derived reconstruction,
a streamed final comparison requires exactly the writer's manifest and payload form. Changed
field order, number spelling, omitted defaults, escapes or ignored nested fields can therefore
refuse even when a generic parser considers them equivalent. There is no generic JSON rewrite,
repair, ordinary import normalization or partial acceptance fallback.

## Preserved meaning and identity

The supported subset keeps scalar bits including negative zero, ordered style duplicates,
formula text/mode/flags/caches, Python code/results/staleness, names/future records, selections
and selection memory, file IDs, spill data and preserved owner/relationship/part metadata.
Only detached derived cell-store accounting, style lookup and parseable formula ASTs are rebuilt;
no calculation or macro/Python/formula execution occurs. Unparseable formula text and its cache
remain data. Strict macro adoption rejects trimming/default insertion/ignored-field loss and
wrong future channels; ordinary macro/Python repair helpers are not used to make recovery pass.
Unsupported or nonrepresentable content refuses instead of silently discarding it.

Cell-picture sharing uses dense first-encounter group IDs across workbook sheet order and ordered
cell coordinates. Equal but separately allocated pictures remain distinct; group reconstruction
checks full picture equality before sharing and before preserved-graph/final-writer validation.
Pointers and runtime SheetUid identity are not serialized; decoded sheets have fresh runtime IDs.
Opaque occurrence payloads remain separate even when source Arcs shared their bytes.

Source paths use required tagged native representations: Unix byte arrays or Windows UTF-16
code-unit arrays, without lossy UTF-8 conversion. Actual `source_dir`/`source_path` values in the
other platform's native representation refuse as `native_path_platform`. `original_path_hint`
is inert metadata and can retain the other representation, including non-Unicode units.
Recorded IDs, paths, file kinds, dirty flags and revisions confer no runtime authority.

The authoritative field inventory is in [independent field assertions](../crates/engine/src/recovery/manifest/complete/tests/goldens/fields.rs),
[typed source fixtures](../crates/engine/src/recovery/manifest/complete/tests/goldens/source.rs),
[literal contracts](../crates/engine/src/recovery/manifest/complete/tests/goldens.rs) and the actual
[model validator](../crates/model/src/recovery_validation.rs)/[preserved validator](../crates/xlsx/src/passthrough/recovery.rs).
The codec is not a promise to preserve arbitrary original XLSX packages or establish Excel parity.

## Fixed bounds

The private [RecoveryLimits defaults](../crates/engine/src/recovery.rs) apply independently at
raw lexical/structural, borrowed serializer and complete decode boundaries; they are not a
process heap/RSS cap. Counts charge actual emitted occurrences rather than Arc deduplication.

| Quantity | Limit |
|---|---:|
| Whole envelope / manifest | 256 MiB / 64 MiB |
| One opaque body / body occurrences | 64 MiB / 4,096 |
| Owner relationships / total relationships | 10,000 / 100,000 |
| One XML fragment | 4 MiB |
| Sheets / stored-cell-plus-spill occurrences | 1,024 / 1,000,000 |
| JSON values plus object keys / container depth | 4,000,000 / 64 |
| Selected metadata strings / native path storage | 32 KiB / 32 KiB (Windows units charge two bytes each) |
| Aggregate decoded strings / encoded binary strings | Each bounded by the manifest budget |
| Aggregate decoded inline binary | Envelope budget; expanded base64 is checked before serialization |

Existing stricter helpers still apply. [Macros](../crates/model/src/macros.rs) allow 1,000 records,
10,000 Steps per macro, control nesting four, 64 inputs, 255-character names, 4,096-character
text/If values, 1,024-character references, 64-character shortcuts, 256 permissions and 16,384
column types; the executable-item ceiling is 4 Mi characters. Arbitrary future/action JSON is
still bounded as General context; familiar-looking keys do not acquire another field's quota.
[Python metadata checks](../crates/model/src/recovery_validation/metadata.rs) require the existing
8,192-character encoded PY formula and at most 1,000,000 output-array cells, without running Python
or rebuilding its output. The ordinary formula parser's existing limits constrain AST rebuilding,
not lossless storage of otherwise valid unparseable formula text.

Preserved-graph validation also enforces the [existing limits](../crates/xlsx/src/passthrough.rs):
256 MiB kept bytes, 64 MiB per part, 4,096 parts, 10,000 owner/100,000 total relationships,
1,024 kept sheets, 32 KiB text, 4 MiB fragments, 16 MiB VML and relationship-chain depth 16.
Its [output-name work ledger](../crates/xlsx/src/passthrough/recovery/claims.rs) has 16 MiB scratch,
4,000,000 events and 256 MiB byte work; the linked graph/XML scanner's stricter work/shape limits
also remain authoritative. Checked sizes and fallible recovery-owned reservations do not make
serde, Arc, maps, AST/XML/helper allocations universally fallible or prevent allocator abort.

## Verification and limitations

The source checkpoint has 125 focused recovery tests and nine public integration tests; those
are contained in the ordinary engine suites, not extra counts added to the 2,061 ordinary
engine-library/integration/model/XLSX passes. Twenty-two independently reviewed literal files
cover modeled fields, owners, picture identity, native paths, kinds and hostile boundaries.
[Public hostile tests](../crates/engine/tests/it/recovery_snapshots.rs) and
[private tests](../crates/engine/src/recovery/manifest/complete/tests.rs) assert actual finite refusals.

These existing commands exercise the focused/public and ordinary suites:

```sh
cargo test --locked --offline -p gridcraft-engine --lib recovery::
cargo test --locked --offline -p gridcraft-engine --test it recovery_snapshots::
cargo test --locked --offline -p gridcraft-engine --lib --tests
cargo test --locked --offline -p gridcraft-model -p gridcraft-xlsx
```

The [deterministic measurement test](../crates/engine/src/recovery/manifest/measurement.rs) runs
accepted/refused regressions by default. A release libtest campaign measured 180 fresh processes,
36 matched points and 14 families: raw keys/nodes, escaped arrays, typed picture/VBA serde,
strict macros, sparse styled/formula rebuilds, complete decode, sheet/cell/style/depth boundaries,
minimal complete shape and shared/unshared payload occurrences. Every point has separate startup,
accepted/refused fixture and operation processes. The minimal complete shape measured 3,176
manifest bytes, 3,204 total bytes and 415 emitted nodes; 64/128/256-node probes are raw-stage cases.

The largest accepted operation peak was 13.125 MiB on macOS arm64/Rust 1.95, for the small
synthetic 256-cell complete case, with a 10.859 MiB fixture baseline. These observations include
libtest, fixture creation, lexical inventory, reporting and allocator overhead; compilation was
not timed as decode RSS. Requested sizes and observed capacities are separate from process RSS;
old/new capacity sums do not prove simultaneous allocations. General encoded-string-token sums,
failed structural-audit scratch peaks and true simultaneous heap totals remain uninstrumented;
all process fixtures had zero physical spills. There is no production-maximum RSS, global leak
freedom, universal OOM or external-oracle claim. Host safe-root qualification is blocked, Windows
runtime remains NOT_TESTED, Excel validation is unavailable, and native recovery remains inactive.

## Detached restore admission

`restore` preserves cached values, spills, formula evaluation modes, opaque modeled content,
typed import facts, content kind and the entire selection vector (including its anonymous tail).
It uses a bounded, parser-free Calc constructor with ordinary precedent semantics and bulk
indexes; it never evaluates or performs an ordinary graph rebuild. Fresh live document/sheet
identities and live revision 1 are distinct from the inert recorded ID/revision accessors.
Trust is always `FromFile`, script consent is false even without scripts, path is `None`, and
undo/redo/unlocks start empty. All recorded context is retained behind a private shared owner
whose Debug is redacted. The original path hint is never converted, including foreign native
representations; workbook source paths remain independent.

All limits below are inclusive and shared across the whole restore, including nested/revisited
expressions, all sheets, index families and adjacency rows. Refusal aborts the owned candidate.

| Resource | Limit |
|---|---:|
| Graph nodes N | 8,192 |
| Active precedent invocations D (root = 1) | 8 |
| AST schedules C (including revisits) | 262,144 |
| Area emissions/copies/transfers A | 16,384 |
| Index attempts I (before deduplication) | 16,384 |
| Circular dependent offers E (before deduplication/filtering, including wide-column misses) | 65,536 |
| Recovery-owned work/calls W | 16,777,216 |
| Full string comparison/normalization operand bytes B | 16,777,216 |
| Calc-owned cumulative requested dynamic payload | 67,108,864 bytes |

The storage ledger conservatively retains every request, even after temporary storage is freed,
and includes old/new vector growth overlap. Empty final-size hash reservations and bounded
fixed-tuple unstable sorts avoid populated rehashes and input-sized sort scratch. Hash payload
estimates use the actual compiled sizes/alignment and conservative control-group space; vector
requests check the resulting capacity. Standard-library collision/sort tails remain finite but
are not charged as individual CPU instructions. The qualified Rust 1.95 implementation is not
an all-future-toolchain, latency or RSS guarantee. Allocator residency/rounding, native stack,
Workbook/codec/AST storage, fixed cold function-registry initialization and Arc headers are outside
this Calc requested-payload ceiling; fallible requests do not establish universal OOM recovery.
Long unknown function names may exhaust the comparison-byte budget earlier than graph limits.

Structural scans use verified `CellStore.len` after codec/model validation: S is prepaid before
sheet-header inventory, then `3S + 3Cstore + 2T` for Calc/equation/no-stale-refresh passes. Selection
identity preparation additionally prepays at most `10S + 4` owned visits/calls. These initial
charges only consume the same fixed W ceiling passed to `Calc::from_recovery`; they cannot raise
limits or establish validation/provenance for arbitrary external Workbook callers.

Any equation source of 0 through 4,096 UTF-8 bytes rejects the entire candidate before refresh.
Only sources above 4,096 bytes use the existing shared `SourceTooLong` no-layout cache response;
the exact source is retained. UI projection admission remains outstanding.

A separate recovery dirty reason survives saved-pointer/view housekeeping. Only the existing
successful real XLSX/JSON document-save branch clears it, after existing loss/write/sync guards
and publication succeed. Save As cancellation, saveBytes, exports, failures and snapshot writing
do not clear it. This does not extend ordinary save durability or activate native recovery.

Verification uses synthetic public decode/restore and real Save commands, bounded collector/
graph tests, and paired model layout-call-site instrumentation with engine equation admission.
Windows native execution and Excel oracle verification remain **NOT_TESTED**; no new release/RSS
campaign or native storage activation is claimed.
