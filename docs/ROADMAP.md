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

## v0.13 — Compile little, compile well

v0.12 switched non-strict `--ext` builds to compiling only the functions a `pycc profile`
run measures as winners, with the modules run as CPython bytecode. At the release (bench
run 43192816730 at 5b7cc6c, CPython 3.15.2, x86_64 / aarch64) pydantic's
`model_json_schema()` runs 1.20x / 1.23x CPython, model definition 1.00x / 1.01x, the
whole process 1.07x / 1.07x, and mistune's and pystache's imports and benchmarks are at
parity. What is left: pydantic's import is at parity, not above it (0.99x / 0.99x), and
pypng and tomli, compiled whole-module before the switch, lost most of their speed. What
v0.12 did and measured: [v0.12](roadmap/v0.12.md); the design:
[compile-little](design/compile-little.md).

Accept (carried over from v0.12, not met there):
1. **pydantic import >= 1.0x**: median of the CI benchmark job (`bench.yml`) for the
   import phase of `bench/pydantic_workload.py`, compiled tree vs CPython from `.pyc`,
   same pydantic-core wheel, on both runners; its suite fails no test that passes against
   its sources. v0.12: 0.99x / 0.99x. The modules run as bytecode, so the import is at
   parity by construction; 43% of its instructions are CPython compiling texts at run
   time (`compile()` of 737 forward references, `exec` or `eval` of 101 generated texts,
   53 of them `dataclasses` methods). The lever: v0.12's lever 11 again, the build's
   code objects for generated dataclass methods and forward-reference texts, which a
   prototype measured at 1.14x (73.3 ms -> 64.2 ms locally).
2. **pypng and tomli no slower than before the switch**: in `bench.yml` on both runners,
   pypng's two reads and tomli's `loads` at least as fast relative to CPython as in bench
   run 41750538464 at 0f97b93 (pypng five filters 10.42x / 17.20x, PngSuite 3.10x /
   3.69x; tomli 1.70x / 1.49x). v0.12: pypng 2.02x / 1.95x and 1.24x / 1.20x, tomli
   1.42x / 1.31x. The plan: [selected-regions](design/selected-regions.md), native calls
   inside a region of selected functions, PRs R1-R6. Measured locally: a hand-written region
   takes `loads()` from 2.808M to 2.657M instructions (old build 2.427M, CPython 4.082M).
   With native `int` operands in the `str` word paths and character-class tests that make no
   character object, it is projected at about 2.37-2.41M: accept 2 is met only with all
   three, by a few percent.
3. **Hold v0.12's met targets** in `bench.yml` on both runners: pydantic
   `model_json_schema()` >= 1.15x (v0.12: 1.20x / 1.23x), model definition >= 1.0x
   (1.00x / 1.01x), whole process >= 1.0x (1.07x / 1.07x); mistune's and pystache's
   imports and benchmarks no slower than at v0.12.0.

Levers for the schema phase's margin, measured locally on 2026-11-12:
1. pass 3 of `pycc profile` prunes losing members from its winning set (#122, in v0.12;
   1.08x-1.12x schema locally before #124);
2. idiom lowering in `_make_json_hashable` (#124 did the first part: dict items through
   `PyDict_Next`, `tuple(sorted(genexpr))` sorted in place, tuple displays by
   `PyTuple_New`); left: inline exact `isinstance` and lazy frames, about 1.25x estimated
   with the rest;
3. a cheaper boundary between bytecode and compiled properties and methods.
