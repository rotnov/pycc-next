# Roadmap

Milestones in order. A milestone is done when every Accept item is met with evidence
(a commit, a CI run or a `bench/` measurement), then released with the `release` skill.


## Released

| Milestone | Released | Details |
|---|---|---|
| v0.1 — Restart | 2026-10-11 | [v0.1-v0.7](roadmap/v0.1-v0.7.md) |
| v0.2 — Rust speed, numpy, more Python | 2026-10-12 | [v0.1-v0.7](roadmap/v0.1-v0.7.md) |
| v0.3 — Packages, stdlib, everyday code | 2026-10-14 | [v0.1-v0.7](roadmap/v0.1-v0.7.md) |
| v0.4 — Fast untyped code, no leaks | 2026-10-16 | [v0.1-v0.7](roadmap/v0.1-v0.7.md) |
| v0.5 — Typed packages at native speed | 2026-10-16 | [v0.1-v0.7](roadmap/v0.1-v0.7.md) |
| v0.6 — Drop-in for real projects | 2026-10-17 | [v0.1-v0.7](roadmap/v0.1-v0.7.md) |
| v0.7 — Real-world Python: frameworks, Windows | 2026-10-17 | [v0.1-v0.7](roadmap/v0.1-v0.7.md) |
| v0.8 — Library code faster than CPython | 2026-10-21 | [v0.8](roadmap/v0.8.md) |
| v0.9 — Typed hot data, native end to end | 2026-10-21 | [v0.9](roadmap/v0.9.md) |
| v0.10 — Native dicts, exact errors, trustworthy numbers | 2026-10-30 | [v0.10](roadmap/v0.10.md) (accept 3 not met) |
| v0.11 — Imports faster than CPython | 2026-11-01 | [v0.11](roadmap/v0.11.md) (accept 1 and accept 4 for pystache not met) |
| v0.12 — Imports faster than CPython, finished | 2026-11-12 | [v0.12](roadmap/v0.12.md) (accept 1 and accept 3's import target not met) |
| v0.13 — Compile little, compile well | 2026-11-13 | [v0.13](roadmap/v0.13.md) (accept 1 on aarch64, accept 2 for pypng's PngSuite read and accept 3's model definition on aarch64 not met) |

## v0.14 — Compile little, compile well, finished

v0.13 let the functions a profile selects call each other natively inside a region
(selected-regions R1-R5) and specialized a parameter for its dominant observed type. At
the release (bench run 43418938409 at e579fe5, CPython 3.15.2, x86_64 / aarch64) tomli's
`loads()` runs 1.81x / 1.56x CPython, above its whole-module build, pypng's five-filter
read 13.94x / 16.98x, pydantic's `model_json_schema()` 1.24x / 1.22x and its whole
process 1.07x / 1.05x. What is left: pydantic's import is at parity, not above it (1.02x
inside the spread / 0.98x), its model definition is 0.99x on aarch64, and pypng's PngSuite
read (2.72x / 2.88x) is below its whole-module build. What v0.13 did and measured:
[v0.13](roadmap/v0.13.md); the designs: [compile-little](design/compile-little.md),
[selected-regions](design/selected-regions.md).

Accept (carried over from v0.13, not met there on both runners):
1. **pydantic import >= 1.0x**: median of the CI benchmark job (`bench.yml`) for the
   import phase of `bench/pydantic_workload.py`, compiled tree vs CPython from `.pyc`,
   same pydantic-core wheel, on both runners; its suite fails no test that passes against
   its sources. v0.13: 1.02x / 0.98x, at parity under callgrind (470.80M instructions
   against 470.12M on x86_64). The modules run as bytecode, so the import is at parity by
   construction; the lever is v0.12's lever 11, the build's code objects for generated
   dataclass methods and forward-reference texts, which a prototype measured at 1.14x
   (73.3 ms -> 64.2 ms locally).
2. **pypng's PngSuite read no slower than before the switch**: in `bench.yml` on both
   runners, at least 3.10x / 3.69x CPython (bench run 41750538464 at 0f97b93). v0.13:
   2.72x / 2.88x. pypng's five-filter read (10.42x / 17.20x then; 13.94x / 16.98x now)
   and tomli's `loads` (1.70x / 1.49x then; 1.81x / 1.56x now) must not fall below those
   ratios.
3. **Hold the met targets** in `bench.yml` on both runners: pydantic
   `model_json_schema()` >= 1.15x (v0.13: 1.24x / 1.22x), model definition >= 1.0x
   (1.01x / 0.99x: not met on aarch64), whole process >= 1.0x (1.07x / 1.05x); mistune's
   and pystache's imports and benchmarks no slower than at v0.13.0.

Levers for the schema phase's margin, measured locally on 2026-11-12:
1. idiom lowering in `_make_json_hashable` (#124 did the first part); left: inline exact
   `isinstance` and lazy frames, about 1.25x estimated with the rest;
2. a cheaper boundary between bytecode and compiled properties and methods.
