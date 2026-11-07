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
