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

## v0.12 — Imports faster than CPython, finished

v0.11 brought pydantic's import from 0.89x / 0.93x of CPython (x86_64 / aarch64) to
0.97x / 0.98x and mistune's to 1.04x / 1.03x (bench run 41248409468), but not pydantic's
or pystache's to parity. The remaining gap is spread thin: the CI workload's pydantic
import phase costs 212.67M instructions against CPython's 208.77M; at the last breakdown
(after lever 12) no item was above 0.75M (unmarshalling the chunk tables, the `str`
literal cache, string hashing, relocations), and decorated `def`s and class statements
still run as `exec`'d chunks to keep their tracebacks exact. What v0.11 left and where
the time goes: [v0.11](roadmap/v0.11.md).

Accept (carried over from v0.11, not met there):
1. **pydantic import >= 1.0x**, then **>= 1.2x** as a stretch: median of the CI benchmark
   job (`bench.yml`) for the import phase of `bench/pydantic_workload.py`, compiled tree vs
   CPython from `.pyc`, same pydantic-core wheel, on both runners; its suite fails no test
   that passes against its sources. v0.11: 0.97x / 0.98x.
2. **pystache's import no slower than CPython's** in the CI benchmark job, on both
   runners, and no package check regresses. v0.11: 0.97x / 0.94x.
