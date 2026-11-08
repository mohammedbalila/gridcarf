# Typed recovery model validation

Last updated: 2026-11-08.

`gridcraft_model::validate_recovery_model(&Workbook) -> Result<(), RecoveryModelError>`
checks the stored typed model without modifying it. It neither installs a recovered
workbook nor authorizes recovery activation. The public errors and their payload
`RecoveryRepresentation` are finite and contain no document strings, paths,
formulas, code, JSON, or other user content.

The entry point checks every sheet's raw cell-store buckets and cached count
before iteration or fallback access. It then validates workbook/sheet structure,
styles, cell values and ownership, Python code/results, spills, feature regions,
objects, and print/protection/phonetic metadata. The exceptional atomic active-sheet
selection is sampled once by the structural check; capture coherence is the
caller's responsibility. No repair, cloning, recalculation, parsing, reference
resolution, cache rebuild, layout, image decoding, password verification, macro
loading, or serialization occurs.

## Checked content and retained representations

Coordinates and ordered ranges must fit the grid. Scalar numbers must be finite;
arrays have positive checked dimensions, exact data length, and scalar leaves.
Style IDs must refer to actual stored styles; colors and rotation use their
modeled domains. Font sizes are finite-only, including signed, zero and negative
values. Stored line sizes and object extents must be finite and nonnegative;
finite signed offsets and extreme sheet geometry are retained.

Raw stores must have consistent buckets, placement and counts, with no stored
default blank. Spill owners, ranges, aggregate cardinality and exact cached point
ownership must agree. Empty spilled values, stale/manual results and cells that
shadow a spill cache are retained. The validator does not compare cached values
with recalculated or visible results.

Sheet, scoped defined-name, named-style, table, table-column, pivot and calculated
pivot-field identities follow their actual lookup namespaces. Calculated-field
keys use trimmed ASCII case comparison; other ASCII namespaces preserve whitespace.
Phonetic base-text identity is exact bytes. Floating object IDs share one namespace;
pivot IDs have their own namespace; table IDs remain inert. Merge rectangles and
table rectangles are separately disjoint; different families may overlap.

Tables, what-if definitions, conditional formatting, validation, pivots, drawings,
controls, notes and rich content receive their stored-coordinate, numeric, style,
identity and active-control checks. External, missing, unparseable and inactive
references remain unchanged. Pivot output caches need not agree with current
anchors or source data. Full signed note offsets, unknown image MIME/bytes,
unparseable equation/ink text and derived equation caches remain unchanged.

Print margins must be finite/nonnegative, print areas and title axes valid and
ordered, and each manual break strictly inside its own axis. Break order and
duplicates, all header/footer strings, unknown paper names, bounded scale values
and automatic zero fit dimensions remain unchanged. No page grid is constructed.
Protection flags, legacy hashes and all iterated hash strings/spin counts are
retained without KDF or decoding, including unknown algorithms. Edit ranges must
have valid endpoints/order; empty, overlapping and duplicate definitions and
opaque security descriptors remain unchanged. This grants no editing permission.

Phonetic runs use UTF-16 code-unit offsets: `start < end <= base length`.
Run order/overlap, offsets inside surrogate pairs, empty readings, unknown
properties, properties without runs and arbitrary font indices are retained.
The phonetic font index is not a cell StyleId. Ordinary file writers may have
separate fidelity limitations; this check does not authorize dropping data.

Python code is checked against the existing encoded 8,192-character file-formula
limit, counting prefix/syntax, doubled quotes, 255-character chunks and separators
without encoding or allocating. Exact code, CRLF, object/stale modes and cached
result metadata remain unchanged. Result values use the scalar/array checks and
the stricter 1,000,000 output-cell limit. No Python execution or result helper is
called and no requirement equates output with code, cell value or spill cache.

## Errors

`Storage`, `Coordinate`, `Range`, `Style`, `Value`, `Array`, `CellOwnership`,
`Spill`, `Workbook`, `Geometry`, `Table`, `Filter`, `WhatIf`, `Pivot`, `Control`,
`Print`, `Phonetic` and `Python` report a malformed typed family. `Capacity`
reports checked capacity overflow or failure to reserve validation scratch.
`UnsupportedRepresentation(AmbiguousIdentity)` and
`UnsupportedRepresentation(OverlappingRectangles)` report operational ambiguity.
These errors describe validation failures; they do not quote offending content.
The first encountered failure is returned in the entry ordering above. No stable
priority between multiple defects within a family is promised.

## Cost and ownership boundaries

The validator borrows content and uses checked capacities and fallible reservations
for identity records, rectangle events and spill scratch. It does not expand
rectangles into cells. Identity sorting costs include compared string bytes;
UTF-16 counting visits base-text bytes. Value arrays are visited per occurrence,
even when shared. This is deterministic work/scratch accounting, not an RSS bound
or immunity to process/allocator failure. The already-decoded model and allocations
made before this function remain outside its control.

For R spill owners and V cached entries, checked `sum(area - 1) == V` precedes
scratch allocation and column traversal. At most `2*(V+R)` column slots are visited.
Spill scratch is O(R + MAX_COLS), with O(R log R + V + MAX_COLS) sweep work plus
owner lookups. Each merge/table sweep reserves 2R row events, uses a fixed 256-word
(2 KiB) bitset, and costs O(R log R + 256R), independent of rectangle area.
Other identity indexes reserve one borrowed record per actual entry; comparisons
include common byte prefixes. Python counting stops when the encoded limit is
exceeded. Other leaves scan actual stored collections and array leaves.

Success explicitly excludes raw JSON duplicate-key/alias detection and predecode
byte/node/depth/base64 budgets; opaque package/XML/relationship graph validity;
executable adoption and consent; host metadata, provenance, sidecar completeness;
and context-dependent screen/page projection. It does not certify ordinary save
fidelity or safely installing every accepted value in an unchecked UI consumer.
Engine, codec, capture and UI owners must enforce those separate gates before
native recovery activation and during later view/zoom/mode changes.

Macros, actions, pending/future records, arbitrary JSON, VBA bytes, source paths
and preserved package content remain owner-deferred. Engine budgets must precede
`Macro::check_limits`, `step_count`, serialization or loaders. Model success is
not evidence those operations are safe or trusted. Derived AST, style indexes,
raw-store accounting and equation caches remain untouched; private implementation
fields are inventoried at their existing owner boundaries, not recursively
compile-drift guarded by the public-field inventory.

## Verification

Synthetic public-API tests cover populated families, accepted neighbors, hostile
single-field changes, variants, sharing/bit preservation and late errors. Existing
leaf and raw-store owner tests remain complementary. Run the focused model tests,
complete model suite and formatting check in the repository's configured build
environment:

```sh
cargo test --locked -p gridcraft-model --lib recovery
cargo test --locked -p gridcraft-model --lib
cargo fmt --all -- --check
```

The repository's complete `cargo xtask ci` gate, independent model review and
engine/codec/UI integration verification remain necessary before release. These
commands do not constitute Windows runtime or native recovery qualification.

## Revision history

| Date | Change |
| --- | --- |
| 2026-11-08 | Document the complete read-only typed validator and its separate owner gates. |
