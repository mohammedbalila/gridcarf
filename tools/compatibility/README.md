# Compatibility fixtures

Version 1 is a bounded, synthetic baseline, not an Excel certification corpus. The
[manifest](manifest.json) pins the source, input recipes, hashes and expected behavior.
The [baseline report](../../docs/compatibility-baseline.md) separates source results
from the earlier release-binary audit. See the [platform matrix](../../docs/compatibility-platform-matrix.md)
for checks that have not run.

## Generate inputs

Use Rust 1.95 or later (edition 2024). From the repository root, choose a fresh output
directory outside the repository; the opt-in generators refuse to replace files:

```sh
export GRIDCRAFT_COMPAT_FIXTURE_DIR="$(mktemp -d)"
cargo test -p gridcraft-xlsx tests::compatibility_fixtures::synthetic_baseline_inputs_are_bounded -- --exact
cargo test -p gridcraft-engine --test it compatibility_fixtures::synthetic_slicer_input_is_bounded -- --exact
```

The first test reuses `crates/xlsx/src/tests.rs::minimal` and emits
`legacy-intersection.xlsx`, `whatif.xlsx`, `notes.xlsx`, and `plain.xlsx`. The second
uses engine commands to emit `slicer.xlsx` and the original `slicer-before.json`.
Without the environment variable, these tests validate only their synthetic inputs
in memory. They do not assert that a known defect is correct or suppress a failing
regression test. Repairs own their desired-behavior regression tests.

## Run diagnostics

```sh
cargo build --release -p gridcraft-cli
cargo run --release -p gridcraft-cli -- check "$GRIDCRAFT_COMPAT_FIXTURE_DIR/legacy-intersection.xlsx" --json
cargo run --release -p gridcraft-cli -- cat "$GRIDCRAFT_COMPAT_FIXTURE_DIR/legacy-intersection.xlsx" --range A1:B4 --csv
cargo run --release -p gridcraft-cli -- convert "$GRIDCRAFT_COMPAT_FIXTURE_DIR/whatif.xlsx" "$GRIDCRAFT_COMPAT_FIXTURE_DIR/whatif-roundtrip.xlsx"
cargo run --release -p gridcraft-cli -- convert "$GRIDCRAFT_COMPAT_FIXTURE_DIR/slicer.xlsx" "$GRIDCRAFT_COMPAT_FIXTURE_DIR/slicer-after.json"
```

Expected legacy behavior: B2 is 2, with no spill into B3:B4. The cached value is
synthetic, derived from the documented implicit-intersection rule; it is not an
independent Excel oracle. Inspect the what-if output's worksheet XML for the
`dataTable` formula and its `ref`/`r1` attributes. Compare slicers in the before/after
JSON; creation alone does not prove persistence. A checker reporting equality after
import does not prove unsupported input content survived import.

For history profiling, run `gridcraft-cli run --in <notes-or-plain.xlsx> --script
<edits.jsonl>` in a fresh process, under the platform's peak-RSS tool. The script is
0, 20, 80 or 140 `cell.set` commands on B1 with inputs `1` through the edit count,
followed by `history.list`. Compare notes against the empty control at the same
edit count. Record build profile, peak-RSS units and undo depth. RSS is diagnostic;
there is no portable pass threshold in this baseline. The notes payload is exactly
64 × 32,767 ASCII bytes (2,097,088 bytes). No workbook exceeds 64 notes or 140 edits.

## Hashes and provenance

Generated workbooks are original code-built inputs, not files authored by Excel.
Do not commit their binaries, copied fonts, screenshots or outside corpora. Whole
ZIP hashes identify one run; different ZIP metadata, writer timestamps or compression
versions can change them. For comparisons, also record a canonical package hash:
sort ZIP entries by name, and SHA-256 the concatenation of each UTF-8 name, a NUL
byte, the uncompressed content, and another NUL byte. No parts are normalized.

Record exact commands, tool versions, source SHA, lockfile hash, OS/CPU/RAM, exit
codes, warnings, desired assertions and raw outputs before changing production code.
External oracle checks require a separately pinned, permitted corpus and an Excel
installation; until those exist their status is **NOT TESTED**.

## Full source gate

Run `cargo xtask ci` before committing. The workspace's UI glyph tests need the
existing optional font build input: set `CRAFT_FONTS_DIR` to a separate checkout of
[`storytold/craft-fonts`](https://github.com/storytold/craft-fonts) at
`b076417b5880bd4154bd22f00695b1d28c6bea83` (the revision in the repository CI
configuration), and `CRAFT_FONTS_REQUIRED=1` for acceptance. Install
`wasm32-unknown-unknown` in the selected Rust toolchain for the WASM gate. Keep font
files outside this repository; do not alter glyph tests to fit a missing input.
