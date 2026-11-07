# PR stabilization repairs

> Last updated: 2026-11-07. Bounded fixes adapted from existing public GridCraft pull requests.
> These repairs do not establish full Excel compatibility.

This integration branch starts from the reviewed stabilization source containing legacy formula
evaluation modes, loss-aware saves, retained-history accounting, calculation/undo repairs and
native imported what-if definition preservation. Each repair keeps those contracts intact.

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
