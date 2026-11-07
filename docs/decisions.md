# Decisions

## Complete conditional-format statistics with explicit resource limits

Statistics count each effective cell address once across the rule's Applies To ranges,
including spills according to `Sheet::value_ref`. This union interpretation follows the
public range scope; it is not an observed Excel oracle. Existing lowercase-display duplicate
comparison, including contextual Unicode lowercase, stays unchanged.

Each window retains only its current sheet's statistics, keyed by document identity, revision
and actual sheet identity. Proven view transitions rebase matching current/parked caches;
content edits, rules, recalculation and history changes invalidate them. Caches own no workbook
or sheet snapshots.

Statistics are limited to 64 million charged work units, 64 MiB of accounted retained/build
capacity, 4096 cached rules and 4096 input ranges per rule. Sparse stored candidates, rejected
candidates, range membership, strings and sorting all consume budget. Scanning stored cells
directly avoids uncharged row traversal hidden by range iterators; a small rule in a very large
unrelated sheet can therefore exhaust the work budget. Collection capacities, string keys,
growth and lowercase temporaries are accounted conservatively. This is not a process-RSS or
global OOM guarantee: standard whole-string lowercase/number formatting remains bounded but
does not expose fallible allocation. Existing non-statistical predicate/render work is separate.

An incomplete build drops its partial payload and retains a typed reason; it never supplies
partial thresholds. Resource exhaustion stops further builds while previously complete entries
remain usable. Invalid numeric rules fail individually, allowing unrelated statistics to finish.
String-table growth charges rehashing earlier keys as well as processing new text. The grid reserves a visible
warning row. Print/export commands use the additive checked PDF renderer and return an error
before writing when required statistics are incomplete. The existing infallible Rust renderer
keeps its tuple signature and content, adding exactly one diagnostic page if needed (at most
2001 pages for the normally capped 2000-page layouts). No command success schema changes.

## Imported what-if tables are preserved as static content

Keep native XLSX data-table definitions separately from executable formulas, with their caches
in ordinary cells. Optional native flags retain absence as well as explicit false. The static
compatibility issue remains after preservation: outside precedent edits can leave results stale.
Direct result/header/input changes and address-invalidating structure are refused at the existing
transactional edit boundary. Exact recorded reference rewrites and sheet renames may adjust
formula headers while native coordinates stay fixed. Native text remains verbatim; structural
edits require a pure-adjustment proof that its references stay unchanged, and unparseable text
conservatively prevents such edits. The actual sheet-copy owner checks native text before giving
it a new owner: local/empty text can copy, but source-qualified or unparseable text is refused.
Whole-sheet reorder/delete retain owner semantics. Result ranges
also block dynamic spills, including empty cached cells. This does not enable TABLE evaluation.

## Undoable invocations own their nested history

Outside the existing undo-group boundaries, a changed undoable invocation retains
one step from its captured before state to its result. Nested edits, undo and redo
are part of that action: completion restores the captured undo history before
appending the owner's step. Ordinary errors still keep partial workbook effects;
this is not a new rollback boundary. Arc publication remains the change policy.

Intentionally nonundoable dispatchers retain their children's undoable steps.
Split remains such a dispatcher for one freeze/unfreeze step. Original-document
ownership, group labels, history budgets, script permissions, recording and
journaling retain their existing rules. Successful inner commands and their
wrapper can still both appear in the journal; raw replay of toggles can therefore
diverge. Correcting journal/replay semantics is a separate change.

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
before edits: adjustment is unsupported in general, not evidence that references are corrupted.
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

## Preserved-reference edits (S05, 2026-11-07)

The XLSX crate prepares supported reference changes or returns an error against an unpublished
engine candidate. It receives existing typed coordinate edits and pending auto-height rows;
all validation and byte patches finish before mutable calculation, touched/edit logs, selection
or workbook publication. Sheet identities use numeric `SheetUid::get()`, since `SheetUid`
equality deliberately ignores identity in workbook content comparisons. XML stays opaque
to the model and engine. Copying a sheet with preserved content refuses explicitly.

Initial positive support is a complete static checkbox graph: unique relationships, parts and
shape IDs, consistent single-cell links in `formControlPr@fmlaLink`, VML `FmlaLink`, and every
present worksheet `controlPr@linkedCell`. It accepts zero-offset, matching VML and worksheet
markers with unchanged owner grid metrics. Owner coordinate edits, cross-sheet moves into
the owner and unproved geometry changes refuse; VML movement flags are not interpreted.
Pending auto-height at or above the bottom marker refuses, except custom-height rows that
the existing auto-height routine skips. Rows below all markers can still auto-fit. Sheet
rename and edits on another sheet can adjust the links while leaving geometry bytes intact.

A private streaming namespace/span scanner validates only these known paths and a separate
`singleXmlCell@r` same-owner mapping handler. It uses existing part/fragment/depth/node limits,
additional per-tag/name/value bounds, and nonoverlapping bounded byte patches. Captured
fragments retain inherited namespaces, including an absent default;
explicit child namespace declarations retain their original bindings. Unsupported
children, formula fields, ambiguous ownership, nonzero offsets, `AlternateContent`, DTDs,
CDATA and entity-bearing VML link text refuse invalidating operations. Attribute values
can use XML entities; quoted sheet names in plain VML text remain supported. Mapping
deletions or cross-owner moves refuse. This is not a generic XML rewrite or query refresh.

Unknown surviving workbook/package graphs can refer to any sheet. Address edits and
sheet, defined-name or table identity/range/column-name changes therefore require proof
for every surviving graph; unknown maps, connections and query tables conservatively veto
such changes. There is no invented query-table `ref` attribute. Safe unrelated value/style/view
edits keep original bytes; deleting a whole owner can drop its graph when every surviving graph
remains valid. The existing conservative `reference.unadjusted` save warning remains. No executable
content policy changes, new dependencies or native Excel compatibility claims accompany this
synthetic-schema coverage.
