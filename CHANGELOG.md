# Changelog

All notable changes to pycc-next. Format: [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

## [0.13.0] - 2026-11-13

Compile little, compile well, continued -- the functions a profile selects now call each
other natively inside a region, and a parameter is specialized for its dominant observed
type. tomli's `loads()` is back above its whole-module build (**1.81x / 1.56x** CPython,
x86_64 / aarch64, against 1.70x / 1.49x before v0.12's switch and 1.42x / 1.31x at
v0.12.0), though mypyc's build of tomli is still faster than ours (1.33x / 1.35x).
pypng's five-filter read is back at **13.94x / 16.98x** (2.02x / 1.95x at v0.12.0), its
PngSuite read at 2.72x / 2.88x, still below the 3.10x / 3.69x it ran before the switch.
pydantic's `model_json_schema()` holds at **1.24x / 1.22x** and the whole process at
1.07x / 1.05x; its import is 1.02x on x86_64, inside the spread, and **0.98x** on aarch64,
so v0.13 accept 1 is met on one runner only. Details in
[the v0.13 roadmap](https://github.com/rotnov/pycc-next/blob/main/docs/roadmap/v0.13.md).

### Added

- Regions of selected functions (`docs/design/selected-regions.md`, R1). The selected
  functions of a module that call each other by a name no code rebinds (a module global
  bound only by its `def`, or `self.m(...)` of the caller's own class) form a region, and
  `--report-fallbacks` lists each as `region: MODULE: A, B, C (entries: ...)`, an entry
  being a member the profile counted called from outside the region. `pycc profile`'s
  pass 3 prunes a region of its winning set whole, never one member. No code is generated
  differently: a build of the same functions runs the same instructions. This prepares
  native calls inside a region (R2).
- Native calls inside a region (`docs/design/selected-regions.md`, R2). A region's module
  functions get native copies whose `int` positions (parameters, results, and the `int`
  items of tuple results, returned without a tuple) are proven by a prover that also
  trusts an exact `str` passed along the region, a local bound only to members, and the
  positions of a match of a module-global `re.Pattern`. The copies call each other
  directly while the region is intact: a watcher on the module dict and one on the
  members' function objects end that when a member global, a pattern global, a member's
  `__code__` or its `__defaults__` changes, and the members run their generic bodies from
  then on. A copy that closes a cycle checks the recursion limit. A region also takes
  the unselected functions whose results it uses as positions, when the prover types them
  (`--report-fallbacks`: `compiled as a region's callee: MODULE:NAME`). tomli's `loads()`
  with the same 19-function selection: 2,821,369 -> 2,591,280 instructions per call
  (-8.2%), `.text` 598K -> 1,059K; pypng's `read()` (method regions only) unchanged at
  143.01M; pydantic's workload unchanged within noise (its regions get no copies). Two
  new DEVIATIONS rows: an `int` passed between copies and returned unchanged is a new
  object, and a member rebound (or a pattern global replaced) while a region call runs
  can raise `TypeError` for the rest of that call (`skip() returned str, the compiled
  caller needs int`).

### Changed

- R2's code size (`docs/design/selected-regions.md` 5.4). An entry guard unpacks its
  copy's tuple result, so it takes the copy's multi-value twin; the generic arm of a
  region call is a shared cold slow call, one per shape of call, with no traceback frame
  of its own; the lift of a member with a copy takes no object multi-value twin; the region's
  intact flag is read inline; and a compiler-generated function nothing calls (a native
  twin, a region copy, its slow calls and conversions) is no longer emitted. tomli's
  `_parser` with the 19-function selection: `.text` 1,075,175 -> 760,807 B (+27% over
  598,119 B before R2, was +80%), `loads()` 2,592,013 -> 2,543,978 instructions per call
  (-1.8%).
- Native `int` positions in the `str` word paths (`docs/design/selected-regions.md`, R3).
  In a region copy (and in any compiled body with native `int` locals, such as a
  `for i in range(...)` variable), `s[a:b]` whose bounds are native `int` locals, literals or their sums
  and differences calls `pycc_ext_str_slice` directly for an exact `str`;
  `s.startswith(t, a)` with a native `a` compares the bytes inline for compact ASCII text,
  in place of a method call with a boxed position; and `c = s[pos]` with a native `pos`
  makes `c` a tagged character, whose comparisons with one-character literals and
  `frozenset` membership tests read the word. A big `int` position, a `str` subclass or
  (for `startswith` and characters) non-ASCII text takes CPython's own operation, as before.
  tomli's `loads()` with the 19-function selection: 2,543,791 -> 2,469,841 instructions per
  call (-2.9%), `_parser` `.text` 760,807 -> 770,951 B; pypng's `read()` (143.01M) and
  pydantic's workload unchanged.
- Character-class tests without a character object (`docs/design/selected-regions.md`, R4).
  `s[i] in chars` (and `not in`) of an exact compact ASCII `str` indexed in range by a small
  `int` reads the byte and tests the bit of an exact `frozenset`'s cached ASCII mask, in a
  region copy and in a generic body. A `while` loop with such a test, whose `str` and set
  are parameters the loop only reads, checks both once before the loop and copies the
  set's mask, so each test is a bounds check, a byte load and a bit test. Any other operand
  (a `str` subclass, non-ASCII text, an index below zero, past the end or not a small `int`,
  a `set`, a `frozenset` subclass, a set of more than 256 members or with a member that is
  not an exact `str`) takes CPython's own subscript and membership test, as before. tomli's
  `loads()` with the 19-function selection: 2,470,292 -> 2,374,567 instructions per call
  (-3.9%), below the whole-module build from before v0.12's switch (2.427M); `_parser`
  `.text` 770,951 -> 774,151 B; pypng's compiled code identical, pydantic's workload
  unchanged.
- The fixed cost of calls into and inside a region (`docs/design/selected-regions.md`, R5).
  A region copy's frame links onto its compiled caller's frame without checking for a
  signature wrapper or forwarder, and every inline-linked frame unlinks from two words of
  its own; a global read no longer takes a reference to the module's namespace; and a call
  of a compiled method or function through its forwarder that leaves the later parameters
  to their defaults calls the compiled body directly with the defaults the function holds
  at that moment, where it was bound through the forwarder's generic path. tomli's
  `loads()` with the 19-function selection: 2,374,136 -> 2,293,301 instructions per call
  (-3.4%), `_parser` `.text` 774,151 -> 753,527 B; pypng's PngSuite decode, pydantic's
  workload and diff-match-patch's `diff_main` not slower.
- The package checks (`bench/package_check.py`: sources, compiled tree, competitors and
  the profile's training run; `bench/wheel_check.py`) no longer run upstream tests that
  assert only a wall-time ratio measured in one process: test methods named
  `*_near_linear` (9 in mistune, none elsewhere) are dropped and listed. They check no
  output, and a scheduling hiccup on a shared CI runner failed them whatever the build.
  mistune's absolute-deadline tests, which also check output, still run;
  `tests/package_check_suite.rs` checks the selection.

### Fixed

- An exception escaping an observed-type or region copy no longer shows an extra
  traceback frame at the function's `def` line (the entry guard's call of its copy
  records no frame), and an observed-type copy's frame is named as the function, not
  `_pycc_native_...`.
- A profile-selected function's compiled body is specialized for a parameter's exact type
  when that type covers at least 95% of the calls pass 2 sampled, not only when it covers
  all of them (`profile::DOMINANT_PERCENT`, `docs/design/compile-little.md` section 2.5).
  pypng's test suite passes `undo_filter_*` an `array('B')` 1-2 times in 1,000 calls, which
  left the filter loops untyped: from the same profile, pypng's five-filter read goes from
  1.74x to 17.85x CPython's instructions (1.59x to 10.1x wall time) and its PngSuite
  decode from 1.23x to 3.55x (1.11x to 2.61x); tomli is unchanged (1.46x instructions).
  An argument of another type fails the exact-type guard and runs the generic compiled
  body, as before, so results do not change.
- `pycc --help` and `pycc <command> --help` describe every subcommand and flag for the
  user. v0.12.0 printed internal doc comments there (`run`, `check` and `lock` showed
  "Non-strict mode (#3)...", `build` a design-document reference, flag help carried issue
  numbers and decision ids) and no description at all for `test`, `explain`, `init`,
  `clean` and `version`. `tests/cli_help.rs` checks every subcommand's description and help.

### Known gaps
- v0.13 accept 1 (not met on aarch64): pydantic 2.14.2's import is **1.02x** CPython's on
  x86_64 (71.18 ms against 72.50 ms, inside the spread) and **0.98x** on aarch64 (76.00 ms
  against 74.56 ms), median of 15 runs (bench run 43418938409); under callgrind its import
  phase runs 470.80M instructions against CPython's 470.12M on x86_64 (404.09M against
  402.74M on aarch64), so the import is still at parity, not faster. The lever (the build's
  code objects for the texts CPython compiles at import) is not done. Moved to v0.14.
- v0.13 accept 2 (not met for pypng's PngSuite read): 2.72x / 2.88x CPython against 3.10x
  / 3.69x in bench run 41750538464 before v0.12's switch. pypng's five-filter read is met
  on x86_64 (13.94x against 10.42x) and about equal on aarch64 (16.98x against 17.20x,
  inside the spread); tomli's `loads` is met on both (1.81x / 1.56x against 1.70x /
  1.49x). Moved to v0.14.
- v0.13 accept 3 (not met for model definition on aarch64): pydantic's model definition
  runs 1.01x / **0.99x** (inside the spread on both); `model_json_schema()` (1.24x /
  1.22x) and the whole process (1.07x / 1.05x) hold. pystache's import on aarch64 is 0.98x
  (1.00x at v0.12.0, inside both runs' spreads); mistune and pystache otherwise run at
  1.00x-1.01x, as at v0.12.0.
- mypyc builds tomli faster than pycc-next does: 100 `loads()` take 19.10 ms against our
  25.40 ms on x86_64 (1.33x) and 18.70 ms against 25.20 ms on aarch64 (1.35x).
- v0.10 accept 3 is still not met: pystache's own benchmark and mistune's `html` workload
  run at CPython's speed (1.00x and 1.01x on x86_64), not 1.2x faster; the profile leaves
  most of their code in CPython bytecode.
- Open rows of `docs/DEVIATIONS.md`.

### Speed
Bench run [43418938409](https://github.com/rotnov/pycc-next/actions/runs/43418938409) at
e579fe5 (CPython 3.15.2; ubuntu-24.04, AMD EPYC 9V74; ubuntu-24.04-arm, Neoverse-N2; 4
vCPUs each; medians of 11 interleaved fresh-process runs, pydantic of 15). Ratios are
CPython's median over pycc-next's; above 1, pycc-next is faster. Cython's mode is its
default (annotations honored) except tomli and pydantic, where that build fails tomli's
suite or does not compile, so their Cython numbers ignore annotations. v0.12.0's run had
the x86_64 leg on an AMD EPYC 7763, so compare ratios, not times, across the two.

| workload | CPython | pycc-next | vs CPython | vs Cython 3.3.0 | vs mypyc |
|---|---|---|---|---|---|
| nbody | 3.728s | 0.058s | 64.7x | 84.46x faster | 48.64x faster |
| spectral_norm | 1.108s | 0.013s | 82.5x | 91.69x faster | 5.64x faster |
| diff-match-patch `diff_main` | 8.393s | 0.541s | 15.51x | 13.47x faster | does not compile |
| pypng read, five filters | 89.20 ms | 6.40 ms | **13.94x** | 7.48x faster | does not compile |
| pypng read, PngSuite | 64.70 ms | 23.80 ms | 2.72x | 1.75x faster | does not compile |
| tomli `loads` x100 | 46.00 ms | 25.40 ms | 1.81x | 1.40x faster | **1.33x slower** |
| pystache benchmark x1000 | 48.00 ms | 48.20 ms | 1.00x | **1.04x slower** | does not compile |
| mistune `html` x20 | 0.118s | 0.117s | 1.01x | suite fails | does not compile |
| pydantic define | 6.39 ms | 6.35 ms | 1.01x | 1.03x faster | does not compile |
| pydantic schema | 14.86 ms | 12.03 ms | **1.24x** | 1.05x faster | does not compile |
| pydantic roundtrip | 17.20 ms | 17.15 ms | 1.00x | 1.03x faster | does not compile |
| pydantic whole process | 889.02 ms | 828.70 ms | 1.07x | 1.04x faster | does not compile |

Imports, both runners (one import per fresh process; pydantic: `bench/pydantic_workload.py`'s
import phase):

| import | x86_64 CPython | x86_64 pycc-next | ratio | aarch64 CPython | aarch64 pycc-next | ratio |
|---|---|---|---|---|---|---|
| pydantic | 72.50 ms | 71.18 ms | 1.02x | 74.56 ms | 76.00 ms | **0.98x** |
| mistune | 20.79 ms | 20.82 ms | 1.00x | 22.01 ms | 21.96 ms | 1.00x |
| pystache | 9.43 ms | 9.41 ms | 1.00x | 9.96 ms | 10.11 ms | **0.98x** |

aarch64's pydantic define, schema, roundtrip and whole process: **0.99x**, **1.22x**,
**0.99x**, 1.05x (the profile build compiles 5 of pydantic's functions on x86_64 and 3 on
aarch64); diff-match-patch 16.17x, pypng 16.98x and 2.88x, tomli 1.56x (mypyc 1.35x
faster than pycc-next), pystache's benchmark 1.00x, mistune's `html` and full parser
1.00x.

Typed programs (`bench/run.py`, CPython's median over pycc-next's; Rust is `rustc -O`):

| program | x86_64 | aarch64 | vs Rust, x86_64 / aarch64 |
|---|---|---|---|
| collections_bfs | 7.9x | 7.9x | 2.81x / 2.95x slower |
| fib | 20.7x | 27.3x | 1.90x / 1.61x slower |
| mandelbrot | 74.8x | 40.0x | 1.02x / 1.27x slower |
| nbody | 64.7x | 42.8x | 2.05x / 2.07x slower |
| prefix_sum_np (kernel) | 127.4x | 140.0x | 2.77x / 1.61x slower |
| sieve | 25.6x | 20.3x | 2.49x / 1.96x slower |
| spectral_norm | 82.5x | 74.2x | 1.43x / 1.27x slower |
| stencil2d (kernel) | 646.0x | 456.9x | 2.13x / 2.75x slower |

The run's job summary has every program, package workload and phase, with min-max, the
competitors' builds and pydantic's callgrind counts.

Earlier versions: [0.12](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.12.md), [0.11](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.11.md), [0.10](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.10.md), [0.9](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.9.md), [0.8](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.8.md),
[0.6-0.7](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.6-0.7.md), [0.4-0.5](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.4-0.5.md),
[0.1-0.3](https://github.com/rotnov/pycc-next/blob/main/docs/changelog/0.1-0.3.md).
