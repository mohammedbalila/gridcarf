# Compatibility baseline

> **Last reviewed:** 2026-11-07 · **Last updated:** 2026-11-09 · **Change:** minor (sealed recovery publication lifetime; earlier: typed sticky runtime origin; earlier: bounded detached recovery restore; earlier: bounded inert recovery codec foundation; earlier: F01 conditional-format statistics; M01 bounded note-history repair; C01 legacy formula repair; V01 baseline retained)

This baseline pins source `6a8eb4ff42cfa88b5bdd3eec13c6db3874d95214`, before
stabilization repairs. It covers four small synthetic cases, not general Excel
compatibility. V01 did not change production behavior; later repairs are recorded below. Recipes and input hashes live in
[`tools/compatibility/manifest.json`](../tools/compatibility/manifest.json);
[`tools/compatibility/README.md`](../tools/compatibility/README.md) explains regeneration.

## Build identity

The source workspace reports version 0.16.0, edition 2024, minimum Rust 1.95.
The earlier audit used a separately downloaded CLI reporting 0.17.0; that release
label does not identify this source build. The source baseline uses rustc 1.95.0
(`59807616e`, LLVM 22.1.2), cargo 1.95.0 (`f2d3ce0bd`), macOS 27.0.1 (26A434),
arm64 Apple M5, 10 logical CPUs, and 16 GiB RAM. The target is
`aarch64-apple-darwin`. `Cargo.lock` SHA-256 is
`2868f6f77405c857cf34d409a20509b4d4417e4184f68499f71a80b5b452ebf8`.
Dev/test debug info and incremental compilation were disabled in the isolated
build environment; release uses the committed release profile.

## Source suite

The pristine source headless command passed: **1,811 passed, zero failed, one
existing ignored timing test**, exit 0. The ignored case is
`recalc::tests::recalc_with_range_formulas_scales_linearly`, whose annotation
requires an explicit release timing run. The command includes the selected
packages' unit, integration and documentation tests:

```sh
rustc --version --verbose
cargo --version
cargo test -p gridcraft-core -p gridcraft-numfmt -p gridcraft-formula -p gridcraft-functions -p gridcraft-model -p gridcraft-calc -p gridcraft-xlsx -p gridcraft-engine -p gridcraft-cli
```

Cargo reports three existing unused patches (`egui_demo_lib`, `egui_inspection`,
`egui_system_font_provider`); no lockfile or dependency change was made to silence
them. The two added tests validate fixture inputs and pass independently. Known
defects are diagnostic observations, not green assertions that preserve defects.
Each repair must add and run its own desired-behavior regression.

The first full `cargo xtask ci` attempt passed formatting and Clippy, then stopped
on two UI missing-glyph tests: `interaction::column_autocomplete_folds_case_beyond_ascii`
and `interaction::language_switch_renders_the_ribbon_and_persists`. The optional
font build input was absent. Both tests passed after provisioning the existing
CI-pinned `storytold/craft-fonts` revision
`b076417b5880bd4154bd22f00695b1d28c6bea83` externally and setting
`CRAFT_FONTS_DIR` and `CRAFT_FONTS_REQUIRED=1`. All 14 manifest file hashes and OFL
licenses were checked. This was an environment prerequisite, not a source repair.
The missing `wasm32-unknown-unknown` standard library was installed into the isolated
Rust toolchain before the final gate attempt.

The final `cargo xtask ci` passed **all 10 steps**, exit 0: i18n-report, fmt,
Clippy, workspace tests, doctests, assets, layers, WASM, parity and keytips.
The completed test/doc-test summaries total **3,537 passed, zero failed, one
existing ignored timing test**. The WASM check covers the 12 packages selected by
the repository task, including `gridcraft-web`; it does not run a browser.
Existing non-fatal output includes unused Cargo patches, the font policy's
intentional Noto Sans CJK SC exclusion, and a WASM dead-code warning for
`Modified::At`/`Unknown`. No unrelated warning cleanup was attempted.

## Source diagnostic results

`cargo build --release -p gridcraft-cli` passed. The original audit harness was
rerun against that source binary in a fresh output directory. The Rust-generated
fixtures were also checked separately with the commands in the fixture README:

| Desired assertion | Source observation | Classification |
|---|---|---|
| Legacy B2 = 2 without spill | B2 = 1, B3 = 2, B4 = 3; `check --json` exits 1 with one numeric mismatch | Reproduced product failure |
| Slicer survives save/reopen | One slicer before save, zero after reopen | Reproduced product failure |
| What-if definition survives | Conversion exits 0, warns that data tables are unsupported and only their values were kept; `dataTable` is absent from the written worksheet | Reproduced product failure |
| Notes do not amplify single-cell history storage | Rust-generated notes peak RSS rises from 16,334,848 bytes at zero edits to 187,809,792 at 80 edits and 234,881,024 at 140 edits (100 retained undo steps); the empty control reaches 11,206,656 bytes at 140 edits | Reproduced memory amplification; diagnostic, no portable pass threshold |

The opt-in desired assertions for legacy output, checker success, slicer count and
what-if definition all failed as expected on the pinned source. These failures are
kept in local diagnostic evidence; no intentionally failing or ignored regression
tests were added to the default suite. The audit's Unicode probe
`=LEN(REPT(UNICHAR(128512),16384))` returned 16384 on the source binary, as on the
release binary; V01 does not interpret that as full Unicode compatibility.

## Expected behavior and evidence boundary

| Input | Desired behavior | Evidence source |
|---|---|---|
| Legacy B2 `SQRT(A1:A3)`, A1:A3 = 1,4,9, cached B2 = 2, no `cm` or array marker | B2 = 2 with no B3:B4 spill | Microsoft's [Formula vs Formula2](https://learn.microsoft.com/en-us/office/vba/excel/concepts/cells-and-ranges/range-formula-vs-formula2) documents intersection at the formula's row; the cache is synthetic |
| Table `SalesTable`, Region/Sales, East/10 and West/20, slicer at D1 | Preserve one slicer through save/reopen, or explicitly refuse an unsupported loss | Repository `docs/gaps.md` §3 and the input model; this is a persistence contract, not an Excel-repair result |
| B2 `<f t="dataTable" ref="B2:B3" r1="A1"/>`, cached outputs 4,6 | Preserve its definition and behavior, or make unsupported loss explicit before save | Public [CellFormula documentation](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.spreadsheet.cellformula?view=openxml-3.0.1) defines `dataTable`; complete recalculation parity is outside V01 |
| 64 notes, 32,767 ASCII bytes each; edit B1 up to 140 times | Retain notes and history without copying immutable note payload for every single-cell edit | Repository snapshot/history model and controlled empty-workbook comparison; RSS has no portable acceptance threshold here |

The input cache is a desired assertion, not an independent calculation oracle.
The minimal ZIPs deliberately omit styles; `no styles part` warnings are expected
input diagnostics.
Checking a round trip after unsupported input has already been discarded cannot
prove fidelity of the original package. Inspect input/output parts and feature
counts as well as checker output.

## Upstream coordination snapshot

Live GitHub `main` matched the pinned source on 2026-11-07. Existing legacy
reports [#410](https://github.com/storytold/gridcraft/issues/410) and
[#1636](https://github.com/storytold/gridcraft/issues/1636), what-if reports
[#2077](https://github.com/storytold/gridcraft/issues/2077) and
[#2889](https://github.com/storytold/gridcraft/issues/2889), and undo report
[#1227](https://github.com/storytold/gridcraft/issues/1227) and their comments were
read. Legacy PRs [#417](https://github.com/storytold/gridcraft/pull/417),
[#1660](https://github.com/storytold/gridcraft/pull/1660),
[#2677](https://github.com/storytold/gridcraft/pull/2677), and slicer PR
[#959](https://github.com/storytold/gridcraft/pull/959) were open, unmerged and
reported conflicts (`mergeable_state=dirty`). Recheck before implementing repairs;
this snapshot does not claim ownership or authorize duplicate fixes. No public
issue comments or remote changes were made.

## Post-baseline repair: legacy XLSX evaluation (C01)

Ordinary/shared XLSX formulas now carry a separate legacy evaluation mode. The synthetic
SQRT case returns B2 = 2 with B3:B4 empty, and saving/reopening keeps an ordinary formula.
Modern and CSE controls retain array evaluation. Session tests also cover range-taking
functions, SUMPRODUCT nested arrays, JSON defaults, shared formulas and dirty edits.
The baseline diagnostic table above remains the observation of its pinned pre-fix source.
This bounded repair does not claim full legacy parity or Excel oracle verification.

## Post-baseline repair: note-history storage (M01)

Sheet snapshots now share the note map through `Arc`; editing notes detaches it.
Regression tests retain 64 notes of 32,767 characters through 80 distinct B1 edits,
undo/redo, and 140 edits with 100 retained undo steps. They verify map and text
allocation identity, unchanged content, independent note edits, and sharing across
operations outside notes, including multi-area copy and cut/paste. Existing A1-keyed
JSON maps remain readable and keep the same wire shape.

Fresh release CLI processes on the baseline host used the same Rust-generated inputs
and 0, 20, 80 and 140 B1 edits, each followed by `history.list`:

| Edits | Plain peak RSS (bytes) | Notes peak RSS (bytes) | Undo depth |
|---|---:|---:|---:|
| 0 | 9,928,704 | 16,629,760 | 0 |
| 20 | 10,649,600 | 17,301,504 | 20 |
| 80 | 11,091,968 | 17,793,024 | 80 |
| 140 | 11,321,344 | 17,973,248 | 100 |

All eight processes exited 0. Notes at 80 and 140 edits stayed below the proposed
64 MiB host-specific target. RSS is a single-run diagnostic, not a portable allocation
bound; the deterministic sharing assertions establish that unrelated edits retain
one note map. Actual note edits may still clone that whole map. The pre-fix baseline
table above remains unchanged. No broader cell-store or metadata sharing claim is made.

## Recovery codec foundation

Runtime source origin is now a private sticky lattice independent of execution Trust: encrypted
origin dominates Unknown, which dominates Plain. Audited new/sample/decoded file entries establish
origin; generic construction/adoption and detached restore remain Unknown-or-worse. Payload-bound
clipboard and painter transfers carry origin even for passive formats/values, and external OS
text/HTML/picture paste plus append imports join Unknown before commit. OS text equality grants
no origin authority and can conservatively exclude a previously Plain document. Undo/rollback,
macro previews and Save As retain taint. See [runtime origin](recovery-codec.md#sticky-runtime-origin).
The engine now captures bounded, sealed recovery jobs and gates publication against lifecycle
changes and raw mutation. Native protection guarantees remain unverified. Windows runtime,
Windows final user-bundle and Excel checks remain untested.

The detached restore boundary now consumes a validated snapshot into an untrusted unsaved
`DocState`, preserving caches and inert context without evaluation. Its Calc graph has shared
node/depth/schedule/area/index/offer/work/name and requested-storage limits; any equation source
of at most 4,096 UTF-8 bytes refuses the entire candidate before layout. See the
[restore contract](recovery-codec.md#detached-restore-admission). Session can adopt the validated
copy as untrusted and unsaved. Activation still requires a UI restore coordinator and projection,
scheduling, and qualified native safe-root storage.

The [codec contract](recovery-codec.md) records 125 focused tests, nine public tests (overlapping
the ordinary suites), 2,061 ordinary engine/model/XLSX passes and 22 independently reviewed literal
files. A 180-process/36-point/14-family release campaign separates startup, fixtures and accepted/
refused operations; its RSS and counter limitations are explicit there. These synthetic codec
checks do not establish native restore, trust, durable storage, Windows runtime or Excel parity.
Host qualification remains blocked and untitled AutoSave recovery remains unavailable.

## Limitations

Excel is unavailable: **Excel open, repair prompts, recalculate, refresh and
save/reopen are NOT TESTED**. No Excel-authored or external corpus was used.
The [platform matrix](compatibility-platform-matrix.md) records the other
unverified targets. Local raw logs and generated binary workbooks stay outside
version control. No result here upgrades the repository's parity/readiness claims.

### Recovery publication lifetime (2026-11-09)

The engine now exposes sealed weak tickets and frozen borrowed jobs, cancels publication before
save/close/origin changes, and adopts validated recovery copies as untrusted unsaved documents.
Raw mutable host access conservatively joins Unknown. See the [capture contract](recovery-codec.md#sealed-live-capture-and-publication-lifetime).
Native/UI activation, Windows execution and Excel qualification remain separate; readiness is unchanged.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-11-09 | minor | Sealed recovery jobs and lifecycle gates; untrusted Session restore adoption; native/UI activation and external qualification remain separate |
| 2026-11-08 | minor | Add typed sticky origin at audited entries/transfers and conservative external ingress; live capture/native qualification remain incomplete |
| 2026-11-08 | minor | Add bounded detached restore with preserved caches, untrusted unsaved state and no equation layout; native activation/readiness remain unchanged |
| 2026-11-08 | minor | Document the bounded inert recovery codec and synthetic evidence; live capture, native restore/storage and external qualification remain separate, with no readiness increase |
| 2026-11-07 | minor | Record M01 note-map sharing, undo/redo regressions and fresh release RSS separately from the baseline |
| 2026-11-07 | minor | Record bounded C01 legacy XLSX repair separately from the pinned pre-fix diagnostics |
| 2026-11-07 | minor | Pin source/toolchain and bounded synthetic fixture recipes before repairs |

## F01 conditional-format statistics

Synthetic regressions exercise far-tail values beyond the old two-million rectangular visits,
overlapping/repeated ranges, effective spill values, and existing mixed-type/Unicode duplicate
comparisons. The union interpretation is inferred from public range scope, not observed in Excel.
Caches use document/revision/sheet identity without retaining workbook snapshots. Statistics have
explicit work, capacity and metadata limits; incomplete results are disclosed in a reserved grid
warning and checked PDF export refuses them before writing. The legacy infallible Rust renderer
retains content with one diagnostic page. See [the decision](decisions.md#complete-conditional-format-statistics-with-explicit-resource-limits)
for resource bounds and allocator limitations. Excel and Windows runtime oracle checks remain
untested; no readiness estimate is raised by these synthetic checks.
