# PR stabilization repairs

> Last updated: 2026-11-08. Bounded fixes adapted from existing public GridCraft pull requests.
> These repairs do not establish full Excel compatibility.

This integration branch starts from the reviewed stabilization source containing legacy formula
evaluation modes, loss-aware saves, retained-history accounting, calculation/undo repairs and
native imported what-if definition preservation. Each repair keeps those contracts intact.

## Six implementation commits and verification

The six implementation commits below start from base
`b31400f431232f351c3d8e21365353b3a82e5111` and end at implementation head
`f674c107cdca0d70b25cb98e22c95feaa50a707c`. A later documentation publication
commit is outside this implementation range. Each row records the cumulative native
test totals from that commit's successful `cargo xtask ci` run on macOS; the focused
test commands and regression scope appear in the corresponding repair section below.

| Task | Repair / source PR | Local implementation commit | Native passed | Failed | Ignored |
|---|---|---|---:|---:|---:|
| PR01 | Negative fractional GCD/LCM arguments — #2072 | `a9fb70b4c31f746517c27989c4cc4827e2ca921c` | 3710 | 0 | 1 |
| PR02 | Shape typeface XML attributes — #2196 | `c977607fe25bd33751dfbe2311cb79663d59d253` | 3711 | 0 | 1 |
| PR03 | Borrow cached expressions — #2693 | `e3fe0f7959f3094b08196f0a0bc5be1a75a39735` | 3715 | 0 | 1 |
| PR04 | Bound MCP stdio input — #2326 | `98efa93b8f9166e231e0751e4d8cb2e9c7a4df56` | 3721 | 0 | 1 |
| PR05 | PivotTable accumulator budget — #2327 | `c70ed3c2cc1bafe09c8a48b618d824dec0d48e5c` | 3728 | 0 | 1 |
| PR06 | Stored calculated-column formula renames — #3391 | `f674c107cdca0d70b25cb98e22c95feaa50a707c` | 3733 | 0 | 1 |

All ten CI gates passed at each implementation commit, including compile checks for
all 12 shared WASM packages. The final implementation run recorded 3733 native tests
passed, zero failed and one existing calculation timing test ignored by default.

From a checkout containing these commits, reproduce the exact implementation diff with:

~~~sh
git diff --binary b31400f431232f351c3d8e21365353b3a82e5111..f674c107cdca0d70b25cb98e22c95feaa50a707c
~~~

This batch did not verify native Windows execution, live Excel observations or reopening,
GUI smoke behavior, process RSS/allocation/performance measurements, the extended release
200×200 campaign on this branch, or the combined source with the original session's
subsequent changes. The synthetic Rust and WASM checks do not establish those results.

## Negative fractional GCD and LCM arguments

[GCD/LCM repair source: PR #2072](https://github.com/storytold/gridcraft/pull/2072),
reviewed at 4e4bd75e31cd11db5c1773b34c80d253d97bb295.

Integer-argument validation now rejects negative fractional inputs before truncation. Previously
a value such as -0.5 became negative zero and was accepted. Positive fractions still truncate,
signed zero remains valid, and the existing finite and upper-bound restrictions remain.

Regression tests cover scalar arguments, arrays and references, invalid fractions after zero,
positive fractional inputs, signed zero, nonfinite values and the existing integer boundary.

Run the focused package checks with:

~~~sh
cargo test --locked -p gridcraft-functions gcd
cargo test --locked -p gridcraft-functions lcm
cargo test --locked -p gridcraft-functions
~~~

The integration workflow requires the project's complete gate before each commit:

~~~sh
cargo xtask ci
~~~

Tests use synthetic data. Live Excel observations, Windows execution and process-memory
measurements are separate verification activities; native Rust tests do not substitute for them.

## Shape typeface XML attributes

[Typeface repair source: PR #2196](https://github.com/storytold/gridcraft/pull/2196),
reviewed at 94eed2bd835ea0a991c6dc4fbb8f9c798ab6b2ed.

Drawing text now escapes its font name as an XML attribute. A quote in the font name
previously ended the typeface attribute, allowing the remaining text to become another
attribute or malformed XML. The regression checks emitted XML structure independently,
then verifies exact Unicode font, Arabic text and RTL state after save/reopen.

~~~sh
cargo test --locked -p gridcraft-xlsx tests::shape_text_rtl
cargo test --locked -p gridcraft-xlsx
~~~

## Borrow cached expressions while building dependencies

[Expression-borrowing source: PR #2693](https://github.com/storytold/gridcraft/pull/2693),
reviewed at 21504859c842d200bce71e3fd008c94fdc901f96 (also merged upstream).

Dependency graph construction and edited-formula updates now borrow the cached expression
through Formula::with_expr instead of copying its tree. Text-only formulas still parse on
request; malformed text retains the existing failure behavior. Existing calculation modes,
computed-criteria dependency rules, Python/spill handling and native what-if guards remain.

Regression tests check cached-tree identity and ownership, text-only/malformed behavior,
independent dependency/value expectations, formula replacement and stale-edge removal.
This establishes the absence of the cached-tree copy at these call sites, not a process-RSS
or timing improvement measurement.

~~~sh
cargo test --locked -p gridcraft-model -p gridcraft-calc
cargo test --locked -p gridcraft-engine --test it recalc_invariants
cargo test --locked -p gridcraft-engine --test it legacy_implicit_intersection
cargo test --locked -p gridcraft-engine --test it whatif_tables
~~~

## Bound MCP stdio input before accumulation

[Bounded reader source: PR #2326](https://github.com/storytold/gridcraft/pull/2326),
reviewed at e7cda6337b5ec39dacee91ec67075f69077c80b6.

The stdio server now caps each retained raw frame at the existing 16 MiB payload
limit plus two framing bytes, before accumulating input. Oversized frames drain
through their newline or EOF, produce one size error, and leave the next request
intact. Exact-limit payloads with LF, CRLF or EOF retain their existing behavior.

Excess raw whitespace padding is refused rather than buffered indefinitely.
Oversized invalid UTF-8 receives the size error first; ordinary invalid UTF-8
retains its parse error. This bounds retained input length; it does not measure
Vec capacity, process RSS, parser/backend allocations or drain time.

Six streaming/boundary regressions exercise the actual reader and server, including
EOF, invalid UTF-8, one-byte-over payloads and exactly one following ping response.

~~~sh
cargo test --locked -p gridcraft-mcp framing_
cargo test --locked -p gridcraft-mcp
~~~

## PivotTable accumulator budget includes width

[Accumulator-budget source: PR #2327](https://github.com/storytold/gridcraft/pull/2327),
reviewed at e3934e2ec3f1ceed721d08c843bf0ea48e1df4d0.

Aggregation retains the existing 5,000,000-group cap and adds a 10,000,000 total
accumulator cap, counting ordinary values and calculated-source accumulator columns.
Saturating multiplication prevents overflow from bypassing the budget; zero-width
reports retain the group limit. The existing error and post-record check stay in place.

Tests cover inclusive boundaries, wide reports, empty/zero-width input and overflow.
A small internal cardinality fixture checks the actual aggregation guard and its
calculated-source width without allocating millions of accumulators. It is not a
complete large report or a memory measurement. Per-record transient overshoot and
map/key overhead remain outside this count-based guard.

~~~sh
cargo test --locked -p gridcraft-engine pivot::
cargo test --locked -p gridcraft-engine
~~~

## Stored calculated-column formulas follow renames

[Calculated-column repair source: PR #3391](https://github.com/storytold/gridcraft/pull/3391),
reviewed at 645dc5c59b9c72022f2741a998efed76dc2ff25f.

Header and table renames now update TableColumn.formula, which is saved separately
from cell formulas as calculatedColumnFormula. The repair follows explicit structured
references across tables and sheets, both column-span endpoints, and local references
bound to their owning table and sheet. Bare names, LET locals and strings in stored
formulas keep their meaning; unmatched and unparseable stored text stays unchanged.

Rewrites are collected before mutation, so sheets with no matching references retain
their shared storage. Existing cell-formula evaluation modes and bare-name behavior
remain. Tests cover literal live/native XML/reopened formulas, Undo/Redo, legacy mode,
unrelated-sheet sharing, disjoint native what-if metadata and atomic refusal of
protected native formula-header rewrites. The repair does not establish universal
bare-table-name resolution or observed Excel reopen behavior.

~~~sh
cargo test --locked -p gridcraft-engine --lib tables::tests
cargo test --locked -p gridcraft-engine
cargo test --locked -p gridcraft-xlsx
~~~

## S02-B2: Clamp note anchors without signed overflow

Revision: 2026-11-07.

The note-box writer now saturates the addition of a cell coordinate and its signed
relative offset before applying the existing sheet-bound clamp. Constructed model
notes with extreme offsets therefore save at the appropriate sheet edge. Ordinary
negative offsets retain their meaning, and pixel offsets and point sizes are preserved.
This follows up the overflow finding on
[PR #1614](https://github.com/storytold/gridcraft/pull/1614#discussion_r4334289015).

Synthetic typed-model regressions exercise real XLSX writes at B2 for both axes,
row-only and column-only signed extremes. They inspect literal VML anchors, pixel
offsets and the 224.25 × 89.25 pt size, then reopen author and text. Ordinary negative
offsets and near-upper-bound coordinates provide controls. Clamped geometry is
normalized on reopen; the extreme relative inputs need not survive unchanged.

~~~sh
cargo test --locked -p gridcraft-xlsx note_box
cargo test --locked -p gridcraft-xlsx
cargo xtask ci
~~~

This is an arithmetic save repair. Native Windows, live Excel reopen, process RSS
and broad external-corpus fidelity remain untested by these synthetic checks.

## T01: Acquire exclusive AutoSave test fixtures

Revision: 2026-11-07.

The AutoSave integration fixtures and engine sandbox test now acquire their
synthetic parent directories with exclusive `create_dir`, retrying only existing
candidates. RAII cleans up only a successfully acquired parent. The sandbox
fixture also creates its workbook file exclusively and retries file collisions.
No production save behavior or script permission policy changes.

Directory-acquisition regressions retry both file and directory collisions. The
file-writer regression retries a regular-file collision; a separate directory-only
refusal control preserves its sentinel without requiring a platform-specific error
kind. Fresh candidates and owned cleanup are checked, and missing-parent errors
surface without trying another candidate. The direct-file regression reproduced
the old fixture write truncating sentinel bytes before exclusive creation was applied.
All collision setup and cleanup stays below an independently acquired parent.
The original AutoSave cases and sandbox denial, document/window, dirty-state and
journal assertions remain in place; fixture creation stays outside the sandbox.

~~~sh
cargo test --locked -p gridcraft-engine --lib autosave_tests
cargo test --locked -p gridcraft-ui-egui --test it autosave_failures
cargo test --locked -p gridcraft-engine --test it save_temp_ownership
cargo xtask ci
~~~

These are test-fixture ownership checks. Native Windows, live Excel, process RSS
and broad external-corpus fidelity remain untested.


## M03-A: Preserve sheet scroll identity and release closed-window state

Revision: 2026-11-08 (Asia/Dubai).

Scroll positions now follow the window and actual sheet UID, so inserting, moving,
deleting and restoring sheets cannot assign one sheet's scroll to another. Saving
maps those UIDs back to current sheet indices, including inactive AutoSave, while
preserving the document's live window, selection and guarded trace/conditional-format
revision updates. This adapts the narrow identity repair from
[PR #3507](https://github.com/storytold/gridcraft/pull/3507), commit
`1688b43ae225fc4f89879d57ebacba66c7908a5a`, around the existing document-scoped save path.

The existing window lifecycle seam now drops closed windows' parked UI and scroll
entries before returning for an unchanged active window. Hidden logic-only ticks
therefore release parked conditional-format statistics and dialogs even when the
surviving window remains current. Deleted-sheet positions remain available to Undo
until the window closes; broader history-reachability eviction is a separate task.

Regressions cover inserted/reordered sheets, delete-to-Undo and saved views,
independent windows, inactive/hidden AutoSave and stationary-window cleanup.
Ordinary active-cell storage and an empty session without a window are preservation
controls. An orphan active document without any window is not constructible through
the current public API and is not exercised; its defensive save path is unchanged.

A small synthetic collector paints actual two-cell color-scale output, parks a
dialog, positively checks clipboard and format-painter owners, explicitly clears
both, closes each target by ID, and runs two no-paint logic ticks. All 16 warm-up
and 1,000 measured cycles complete before the final lifetime assertion. Before the
repair, the last target remained parked and scroll entries grew to 1,017. After it,
each post-warm-up checkpoint retains one sentinel document/window/scroll entry,
zero parked entries and zero target conditional-format cache instances. Both maps
retain capacity for three entries. These are counts and inline-capacity estimates,
not exact heap or RSS measurements; internal conditional-cache capacity is not measured.

~~~sh
cargo test --locked -p gridcraft-ui-egui --test it scroll_position
cargo test --locked -p gridcraft-ui-egui --test it autosave_failures
cargo test --locked -p gridcraft-ui-egui --test it trace_navigation
cargo test --locked -p gridcraft-ui-egui --lib window_ui -- --nocapture
cargo test --locked -p gridcraft-ui-egui --lib grid::show::trace_arrow_tests
cargo xtask ci
~~~

The existing offscreen sales sample was rendered and visually inspected on macOS.
Full weak-payload and clipboard-retained lifetime controls, dead-cell textures,
history-reachable sheet eviction, release/RSS campaigns, native Windows and live
Excel observations remain separate gates. This bounded change does not complete
M03, V03, recovery or general platform/Excel compatibility.
