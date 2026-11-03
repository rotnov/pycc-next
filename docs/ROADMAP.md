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

Baseline against CPython 3.15 (the oracle after v0.11.0, [bench run
41427758082](https://github.com/rotnov/pycc-next/actions/runs/41427758082) at 147fac2,
CPython 3.15.1; ubuntu-24.04 AMD EPYC 9V45 / ubuntu-24.04-arm Neoverse-N2, 4 vCPUs each;
medians of 15 runs for pydantic 2.14.2, of 11 for the others; x86_64 / aarch64):

| workload | x86_64 | aarch64 |
|---|---|---|
| pydantic import | 0.96x | 0.94x |
| pydantic model definition | 0.80x | 0.75x |
| pydantic `model_json_schema()` | 0.88x | 0.75x |
| pydantic round-trips | 0.96x | 0.93x |
| `import mistune` | 0.98x | 0.98x |
| `import pystache` | 0.94x | 0.93x |
| `mistune.html` / full parser | 0.89x / 0.86x | 0.78x / 0.76x |
| pystache's own benchmark | 0.65x | 0.68x |

The v0.11 figures below were measured against CPython 3.13 (bench run 41248409468).

Accept (carried over from v0.11, not met there):
1. **pydantic import >= 1.0x**, then **>= 1.2x** as a stretch: median of the CI benchmark
   job (`bench.yml`) for the import phase of `bench/pydantic_workload.py`, compiled tree vs
   CPython from `.pyc`, same pydantic-core wheel, on both runners; its suite fails no test
   that passes against its sources. v0.11 (CPython 3.13): 0.97x / 0.98x; against 3.15:
   0.96x / 0.94x.
2. **pystache's import no slower than CPython's** in the CI benchmark job, on both
   runners, and no package check regresses. v0.11 (CPython 3.13): 0.97x / 0.94x; against
   3.15: 0.94x / 0.93x.

### Progress: the levers

Each lever is measured locally against the merged head before it (PBS CPython 3.15.2,
`valgrind --tool=callgrind`, `PYTHONHASHSEED=0`; own share of the import as in the
gap report: the package's dependencies imported first, the same process importing
nothing subtracted; repeated runs vary by about 0.3M for pydantic, so a lever's own
mechanism is also quoted by its inclusive cost). Baseline at d5f6752: pydantic 143.47M
against CPython's 136.24M (0.950x); mistune 19.93M against 22.53M; pystache 7.47M
against 6.97M.

| lever | change | pydantic | mistune | pystache |
|---|---|---|---|---|
| 1 | `frozenset` constants rebuilt in source order along build-time paths, not by walking every nested code object (`ORDERS_FIX` 1.92M -> 0.36M inclusive) | -1.55M | 0 | 0 |
| 2 | a `str` literal only handed to CPython made as a CPython `str` directly, one per text of the module (the literals' share 1.75M -> 1.10M inclusive) | -0.65M | -0.15M | -0.03M |
| 3 | forwarders patched from owner paths split at build time, each class's members read from its dict (`pycc_ext_forward` 1.05M -> 0.58M inclusive) | -0.47M | -0.07M | -0.03M |
| 10 | 3.15: the module dict watched from the end of its body, not during it (the watcher callback 108K -> 4.5K inclusive, plus CPython's event dispatch) | -0.15M | -0.02M | -0.01M |
| 6 | the class maker's skip list one `frozenset` lookup and its slot test a type compare (`pycc_ext_make_class` 1.15M -> 1.02M inclusive, pystache), the finder's spec path built without split and join (`pycc_pkg_find_spec` 0.44M -> 0.34M, pydantic); publication keys stay `PyUnicode_InternFromString` (692 names, 0.52M in pydantic): a marshalled table would have to make and intern the same strings | -0.10M | 0 | -0.16M |
| 7 (caches) | the shim's attribute-name cache and inline caches filled densely behind a hash index (pystache's touched `.bss` pages 33 -> 13); minor faults of the import 1349 -> 1249 / 382 -> 308 / 186 -> 149, instructions unchanged | 0 | 0 | 0 |
