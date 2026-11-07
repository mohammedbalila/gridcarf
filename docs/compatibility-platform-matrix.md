# Compatibility platform matrix

> **Last reviewed:** 2026-11-07 · **Last updated:** 2026-11-07 · **Change:** minor (V01 baseline)

This matrix reports executed checks, not the product's supported-platform list.
The source pin and commands are in the [baseline](compatibility-baseline.md).

| Platform / reference | Source build and checks | External application behavior |
|---|---|---|
| macOS 27.0.1 arm64, Apple M5, 16 GiB RAM | Headless suite, release CLI build and full 10-step gate passed (3,537 tests passed, zero failed, one existing ignored timing test); pinned optional font input | Excel unavailable: NOT TESTED |
| macOS Intel | NOT TESTED | Mac Excel NOT TESTED |
| Windows x64 / arm64 | NOT TESTED | Windows Microsoft 365 Excel build unpinned, NOT TESTED |
| Linux | NOT TESTED | NOT TESTED |
| BSD | NOT TESTED | NOT TESTED |
| Web / `wasm32-unknown-unknown` | Repository's 12-package WASM check passed in the full gate | Browser execution NOT TESTED |

An independent Excel check must pin the application version/build, platform,
locale, calculation settings and permitted corpus revision/hashes. It must open
the original and GridCraft-saved files, record repair prompts, recalculate, refresh
where applicable, save and reopen. Comparing only GridCraft to its own output
does not establish Excel compatibility. Windows Excel is the intended initial
full-feature reference; Mac Excel checks are limited to capabilities it supports.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-11-07 | minor | Record the executed macOS headless baseline and untested platforms/oracles |
