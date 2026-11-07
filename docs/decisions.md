# Decisions

## Account for allocations retained by undo history

The model estimates owned capacities, tree/hash node storage, nested payloads and
shared allocation identities. The engine seeds each accounting pass with the live
workbook and visits history newest first, charging a shared history-only allocation
to its newest owner once, including across sheets and nonadjacent snapshots.
Equal content in separately owned allocations still costs memory. Workbook metadata,
styles and their index, notes, features, spill maps, preserved content and cell values
all participate; this estimate is a retention policy, not a hard process-RSS ceiling.

The 128 MiB byte budget, 100-step cap and single oversized latest-step exception are
unchanged. Groups retain their existing protected boundaries. Saturated totals are
recomputed after eviction rather than subtracting from an overflowed sum.

Immutable cell bands cache allocation inventories to avoid revisiting every cell on
small edits. The private cache owns only accounting data, is excluded from equality
and JSON, starts empty on copy-on-write detach, and is invalidated before each band
mutation. Passes borrow inventories while their workbooks are alive; caches release
with their bands and their own retained capacity is included in the estimate.

## Share note storage between sheet snapshots

`Sheet::comments` is an `Arc<BTreeMap<CellRef, Comment>>`. Note edits use
`Sheet::comments_mut` to detach the map, so undo versions remain independent while
unrelated cell edits share the payload. Operations that cannot affect notes avoid
requesting mutable access. JSON keeps its existing A1-keyed object shape through
the cell-map serde helpers. This change is limited to notes; other sheet metadata
keeps its current storage.

## Bound repeated XLSX column expansion (#1022)

Limit each worksheet to 1,000,000 nondefault column assignments across all `<cols>` blocks.
This allows more than 60 full-grid passes while bounding repeated work on overlapping spans;
counting only the final map size would not limit that work. Charge each clipped span before
inserting it, after the existing default-record skip. Exceeding the limit returns the existing
`IoError::TooLarge`, which aborts the import instead of silently dropping a partially read sheet.
Keep the current last-wins semantics rather than introduce interval storage or normalization.

## Formula localization uses the existing interface preference

PR #98 builds on the persisted `UiState.language` introduced by #31 and extended by #104.
Español selects the Spanish function table and list/decimal separators; there is no competing
locale setting. Formula syntax without a supplied translation table falls back to English.
The editor translates to canonical English before engine commands, and translates back only
for display. API, clipboard and XLSX syntax remain stable. Existing names override aliases.

The revision is based directly on main after #31/#104 and Portuguese interface support (#82)
merged. Those locales retain their existing formula fallback; this change adds only Spanish.

## Preserve array evaluation when exporting formulas (#64)

GridCraft evaluates newly entered formulas with array semantics even when the final result occupies one cell. XLSX export marks actual spills and formulas whose results may differ under legacy implicit intersection as dynamic, using the spill range or a single-cell array reference. Legacy CSE array ranges keep their existing representation.

The exporter inspects the parsed formula without evaluating it. It distinguishes range-taking functions such as `SUM` from scalar arguments and operators: `SUM(A1:A3)` and `A1*2` stay ordinary, while `SUM(A1:A3*B1:B3)` and `SQRT(A1:A3)` retain array evaluation. Legacy array-aware contexts such as `SUMPRODUCT` do not need an extra marker. Explicit `@` scalarizes its result without hiding array-sensitive operations inside it.

Names are resolved using workbook/sheet scope, and structured references distinguish one table-row cell from a column. Literal `INDIRECT` and `INDEX` arguments permit tighter classification; unresolved reference dimensions are conservatively treated as potentially multiple cells. Name/AST traversal is bounded, with array preservation as the fallback. This is an export classifier, not a claim of exact Excel metadata parity for every function or user-defined function. It stays within the format crate's existing dependencies and does not change calculation or import behavior.

This is the writer policy for array-mode formulas. Imported legacy formulas follow the evaluation-mode decision below. See Microsoft's [Formula vs. Formula2 documentation](https://learn.microsoft.com/en-us/office/vba/excel/concepts/cells-and-ranges/range-formula-vs-formula2).

## Static cell pictures

Store immutable picture payloads in `Sheet::cell_pictures`, a sparse map keyed by `CellRef`,
shared through `Arc`. Ordinary cells keep their original size. Content replacement removes the
map entry; copy, fill, sort and structural edits explicitly transport it, and undo shares bytes.
Serialize the map as address/payload pairs so JSON does not require struct-valued object keys.
Keep the scalar `#VALUE!` fallback used by XLSX consumers that cannot interpret rich images.
This first implementation does not extend formula evaluation to rich values or implement
`IMAGE()`; references and text-only exports use the fallback.

Write picture value metadata and existing dynamic-array cell metadata into the same workbook
metadata part, with independent index tables. Resolve input parts through their relationships.
Support embedded PNG/JPEG files without fetching external resources. See
[cell pictures](cell-pictures.md) for the supported user workflow and current limits.

Picture metadata is optional during XLSX import: part-size limits, record limits and relationship
read errors produce warnings and retain scalar cell fallbacks. The generic package size limits
remain unchanged. Rebased on main before revision; ODS/XLSB import PRs are still unmerged, and
`Package::read_limited` uses the same implementation as the incoming XLSB PR #101 to avoid
duplicate helpers when that work lands.

The UI cache uses a 64 MiB texture budget with at most 4,096 sparse entries. Images used in the
current or preceding frame are protected from eviction. If the visible set exceeds the budget,
new images show the existing placeholder until capacity is available; header-based admission
avoids repeatedly decoding overflow images. More than 128 small visible pictures now keep stable
texture handles instead of cycling through the cache every frame.

## Ctrl+wheel zoom steps along its own ladder, not Excel's 15-point steps

A Ctrl+wheel notch moves the zoom to the next step on `grid::WHEEL_ZOOM_LADDER`: 10, 12, 15, 18,
21, 25, 30, 35, 42, 50, 60, 70, 85, 100, 120, 140, 170, 200, 240, 280, 340, 400 (four steps per
doubling, 17–25% apart). A step less than 5% away is skipped, except 100% and the 10% / 400%
limits, which are never skipped: 121% out goes to 100%, and 99% or 101% towards 100% goes to 100%.
The wheel's turning is counted in notches (egui eases each notch in over several frames, and
fine-grained wheels send parts of one); a notch steps once 60% of it has turned. With animations
on, the zoom then glides to the step over about a tenth of a second (as egui eases a scroll notch),
and a notch turned during a glide steps on from the step it's going to. With View › Animations off
it goes to the step at once. Whether the zoom glide should have a setting of its own is left to the
maintainers.

Before, each frame multiplied the whole-number zoom by egui's eased factor and rounded, so a notch
out never undid a notch in (100 → 122 → 101 → 123 → 102). Keeping the zoom unrounded fixed that
for a wheel with exact notches, but not for one that sends uneven parts of a notch.

Excel (observed: Microsoft 365, Windows) steps 15 points a notch: 100 → 115 → 130, and from 112%
to 115% or 100%. That's a large relative step at low zoom and a small one near 400%; equal
relative steps keep each notch feeling the same, and the doublings land on 25, 50, 100, 200 and
400%. The status bar's Zoom In / Zoom Out buttons and Ctrl+Alt+= / Ctrl+Alt+- keep their 10%
steps, so the wheel and the buttons now step differently. Whether they should share one ladder
(this one, Excel's, or another) is left to the maintainers.

## Equations are laid out by RaTeX, behind a guard

GridCraft lays equations out with RaTeX (`ratex-parser`, `ratex-layout`, `ratex-types`,
`ratex-font`, MIT), pinned exactly at 0.1.14. The gate that chose it, rather than an in-house
LaTeX subset:

- **Source scan.** The five crates hold about 31,700 lines. Outside tests and the bundled
  command-line tools there are 95 sites that can panic (`unwrap`, `expect`, `panic!`,
  `unreachable!`), 87 of them in the parser. Most take an argument the parser has already
  counted (`args.into_iter().next().unwrap()`) or compile a fixed regular expression. There is
  no `unsafe`, no clock (`Instant::now`, `SystemTime`), no threads, and no file or environment
  access. Indexing is not counted; the fuzz run covers it.
- **Fuzzing.** 70,000 random token strings and 50,000 mutations of valid equations: no panic,
  no input slower than 5 ms. Self-referencing and exponential macros stop at the parser's
  1,000-expansion limit. Nesting deeper than 32 is refused ("Recursion limit exceeded"), and
  the deepest accepted input needs under 256 KB of stack in a release build, well inside the
  smallest main-thread stack we run on (1 MB on Windows). Absurd sizes (`\rule{1e308em}{1em}`)
  lay out with huge dimensions, so the wrapper caps them.
- **wasm.** The crates build for `wasm32-unknown-unknown`.
- **Regular expressions.** `ratex-parser` pulls in `fancy-regex` (with `bit-set` 0.8 and a
  second `bit-vec`) for one module, its port of mhchem: 62 anchored patterns in
  `src/mhchem/data/patterns_regex.json` and a dozen in `src/mhchem/patterns.rs` tokenise the
  argument of `\ce{…}` and `\pu{…}`, and need lookahead, which the linear-time `regex` crate
  does not have (`regex` and `regex-lite` serve the parser's other, fixed patterns). Every
  pattern is compiled with `fancy-regex`'s default backtrack limit of 1,000,000 steps per
  match; past it the match is reported as no match, so the macro fails instead of hanging.
  The limit is not reachable from outside the crate. `gate.rs` times 38 sources of exactly
  4,096 bytes shaped to stress the parser and those patterns (nesting at the depth cap, runs of
  backslashes and braces, `(a+)+b`, wide and tall environments, chemistry such as `\ce{+-+-…}`,
  `\ce{111…}` and `\pu{kJ//…}`): the slowest takes 0.8 s in a debug build, with a 10 s bound
  per source.
- **Syntax.** Fractions, roots, scripts, big operators and limits, `\left`/`\middle`/`\right`,
  matrices (`matrix`, `pmatrix`, `bmatrix`, `vmatrix`, `cases`, `aligned`, `array`), accents,
  braces, arrows, `\mathbb`/`\mathcal`/`\mathfrak`, `\text`, colours and mhchem all lay out.

Because a stray panic is still possible, RaTeX runs only inside `gridcraft-math`, under
`catch_unwind`, with input capped at 4 KB and depth 32. Its output is converted to plain data and
cached on the shape; painting (egui and PDF) reads only that cache, so nothing in a paint path
can reach RaTeX. On the web build (`wasm32-unknown-unknown`) a panic aborts instead of unwinding,
so there the caps and the fuzzed input space are the protection. `crates/math/tests/gate.rs` re-runs a smaller, deterministic version of the
gate on every test run, so a version bump has to pass it. RaTeX positions glyphs with KaTeX
font metrics; GridCraft never bundles the KaTeX fonts, so glyphs are drawn with system fonts
and a glyph no font has is drawn as a `[?]` box.

## Preserve imported XLSX evaluation semantics (C01, 2026-11-07)

`Formula.eval_mode` separates calculation semantics from `Formula.dynamic`, which remains
an export marker. The additive JSON field is `"Array"` or `"Legacy"`; missing fields and
`Formula::new`/`from_expr` default to `Array`, preserving older GridCraft JSON and newly
entered formula behavior. Ordinary and shared XLSX formulas import as `Legacy`; dynamic
cell metadata or a CSE array record selects `Array`. Formula movement, fill, sort and
reference rewrites retain the mode; entering a replacement formula selects `Array`.

Each dirty formula uses its mode through `Evaluator::with_mode`. The default evaluator
used by direct evaluation, conditional formats and validation remains array-aware.
Legacy scalar arguments, arithmetic operands and final results use the existing `@`
intersection rules. Function registry masks preserve range parameters and the documented
multi-cell rejection of value parameters. Array-consuming contexts such as SUMPRODUCT,
INDEX's source and LAMBDA helpers keep nested array calculations. CSE/dynamic formulas
retain their existing array behavior. Export does not promote a legacy formula merely
because its expression contains a range; existing CSE and dynamic export markers retain
their roles.

Synthetic Session/XLSX tests cover SQRT at row 2, shared formulas, scalar and range
contexts, round trips, JSON compatibility and recalculation after edits. These bounded
checks do not establish complete legacy function parity; other file formats are unchanged.
Excel open/repair/recalculate and an external workbook corpus remain untested.

## Revision-scoped loss decisions (S02, 2026-11-07)

All document serialization commands (`file.save`, `file.saveAs`, `file.saveBytes`,
`file.exportCsv`, `file.exportHtml`) analyze known compatibility losses before encoding
or writing. Low-level `io` codecs remain policy-free for the checker and format tests;
PDF remains explicit print output. `file.analyzeSave {path, sheet?}` returns the open
document identity, revision, resolved path and typed issues. To proceed despite issues,
call the save command with `confirmedLosses` containing every reported code,
`expectedRevision` and `expectedDocument`. A stale revision or different document is
refused, including after recalculation, undo and changed import facts. Acknowledgement
applies to that call; it is not blanket permission for future saves or AutoSave.

Import issues remain on `DocState` across edits, undo and saves. Existing warning text
is preserved; definite omissions/static conversions are classified and unclassified
warnings remain unknown. The reader inventories unsupported package/workbook/sheet
relationships, workbook/worksheet extensions and table relationships. This is bounded
coverage, not arbitrary unknown-feature detection. After 1,000 distinct import issues,
an explicit unknown-risk record marks the incomplete inventory. Safe preservation notices stay notices.
Preserved reference-bearing content conservatively reports `reference.unadjusted` even
before edits: adjustment is unsupported, not evidence that references are corrupted.
Fully modelled controls do not receive that preserved-content warning.

Known destination differences include model slicers omitted by XLSX, VBA omitted by
non-macro XLSX and flat exports, opaque preserved content omitted by JSON, and flat-format
losses. JSON already preserves VBA as base64 and is not incorrectly labelled as dropping
it. HTML keeps floating pictures and renders charts as static SVG, but omits hyperlink
targets and conditional formatting. The UI lists definite
losses separately from compatibility risks and binds its confirmation to document,
revision and destination. Programmatic calls return `save.losses`/`save.stale` with the
analysis; MCP also exposes it as `structuredContent`. No encryption/trust implementation
or unknown-content recovery codec is introduced.
