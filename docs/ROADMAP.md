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
   **Status 2026-11-06:** met locally (1.05x-1.07x), not yet confirmed on CI. The 1.2x
   stretch is withdrawn. Under the plan below, modules run as CPython bytecode, so the
   import is at parity by construction and has no path to 1.2x.
2. **pystache's import no slower than CPython's** in the CI benchmark job, on both
   runners, and no package check regresses. v0.11 (CPython 3.13): 0.97x / 0.94x; against
   3.15: 0.94x / 0.93x.
   **Status 2026-11-06:** not met (0.94-0.96x locally). The remaining 0.5 ms is spent
   loading the extension. Under the plan below, a package with nothing selected gets no
   extension, so its import is CPython's.
3. **(added 2026-11-06) Library code: compile little, compile well**
   ([design](design/compile-little.md), [#78](https://github.com/rotnov/pycc-next/issues/78)).
   The targets are ratios of CPython time to pycc time, as medians of `bench.yml` on both
   runners:
   - pydantic `model_json_schema()` >= 1.2x;
   - model definition >= 1.0x;
   - whole process >= 1.0x;
   - pydantic import stays >= 1.0x;
   - mistune's and pystache's imports and benchmarks no slower than at a92ee70.
   **Status (PR 5 of the design):** non-strict `--ext` builds compile only the selected
   functions, and `pycc profile` selects only measured winners (it builds the candidates
   and times the training workload against them). A served module runs one code object
   marshalled at build time; the chunk tables, dataclass texts, forward references,
   direct defs and `--exact-cpython` are deleted (pydantic's whole-module `.so` 1.81 MB
   smaller). A winner must be faster than CPython's first quartile, so a win
   inside the noise is not compiled, and a recorded verdict is kept until the source
   changes. Locally, with the training profiles, no candidate of pydantic (5), mistune
   (12) or pystache (0) is a measured winner, so nothing is compiled and every phase is
   at CPython's time (pydantic: 31 interleaved rounds, every median within 1.5% of
   CPython's and inside its interquartile range); the three suites pass.
   **Status (PR 5a, the selection fix):** a function is selected with its region, the
   connected hot functions around it, judged by their combined self time; a crossing is
   rejected by its cost (calls times the measured call overhead against self time), and
   the region is capped by size. Pass 3 keeps the region only when it beats its hot
   core. The training input is every workload a package check runs. diff-match-patch's
   `diff_main` runs 2.68x faster than CPython (54-function region, 32 compiled; module
   mode: 6.94x); pydantic, mistune and pystache select nothing and stay at parity. The
   rest of the gap to module mode is type-specialized bodies (PR 8, the next checkpoint:
   >= 6x on `diff_main`, schema >= 1.05x), then direct calls (PR 7) if they measure.
   **Status (PR 8, first part: observed-type specialization):** a selected function's
   compiled body gets a native copy typed with the exact types the profile observed for
   its parameters, behind an exact-type guard; a parameter that escapes is the argument
   object itself, and one the body binds again is not specialized. `diff_main` in profile
   mode runs 6.98x CPython (`diff_bisect` alone, with `text1: str, text2: str`), so **the diff_main bar is
   met**. pydantic schema stays at 0.99x and **the schema bar (>= 1.05x) is missed**:
   pydantic's hot parameters each take many types, so whole-body specialization does
   not apply. Per-operation specialization (the design's section 2.5) is the remaining
   lever.

   Every step reports wall time, instructions, I1 misses and branch mispredictions, and the
   extension's size (the design's section 3). The three suites fail no test that passes
   against their sources. **Not met:** the plan's pull requests 1-12 are not started.

Tracked until accept 3 made them targets: pydantic's model definition and `model_json_schema()`
phases, and the workload's whole-process wall time (`process`: one fresh interpreter
running every phase, start-up and shutdown included). Every `bench/pydantic_check.py
--time` run reports all three, and `bench.yml` publishes them. The compiled tree is the
slower side in all three, and the goal is to bring each to at least 1.0x. Against 3.15,
CI measured definition at 0.80x / 0.75x and `model_json_schema()` at 0.88x / 0.75x (bench
run 41427758082); that run did not record the whole-process time.

These are local medians on main with #75, against main before it (2a3ef35), with PBS
CPython 3.15.2 on a 4-vCPU x86_64 VM: 21 interleaved fresh-process runs, each phase x20,
nothing else running, load average about 1.3. The cells are ms with the IQR, and the
ratio is CPython time / pycc time:

| workload | CPython | pycc at 2a3ef35 | pycc with #75 | ratio with #75 |
|---|---|---|---|---|
| pydantic model definition | 5.83 (5.74-6.30) | 7.57 (7.38-8.35) | 7.55 (7.45-7.83) | 0.77x |
| pydantic `model_json_schema()` | 11.37 (10.76-12.20) | 15.06 (14.09-15.89) | 14.14 (13.83-16.02) | 0.80x |
| whole process | 776 (752-794) | 896 (870-918) | 885 (848-919) | 0.88x |

Where the time goes, from callgrind (`valgrind --tool=callgrind`, cache and branch simulation) and perf:

- **The instruction counts are close.** With #75, per repetition, schema runs 109.8M instructions against CPython's 108.1M, and definition 29.6M against 26.8M.
- **The extra time is instruction-cache misses and branch mispredictions.** At 2a3ef35, per schema repetition, there are 0.80M I1 misses against 0.38M, and 1.56M mispredictions against 1.28M.
- **The compiled bodies are thin.** At 2a3ef35, of the extension's 32M instructions per schema repetition, 24.6M are in the shim's generic object helpers: calls, attribute and item reads, `isinstance`, displays, iteration and reference counting. The interpreter's own misses also grow by about 60%, spread across all of its code: two hot code bases share the cache.
- **Compiler flags do not help.** Compiling the shim with `-Os` or `-O3` instead of `-O2` measured no faster.

### Plan 2026-11-06: compile little, compile well

The levers below tuned the "compile everything" model one cost at a time. The import
reached parity locally, but definition and schema stayed at 0.77-0.80x. A headroom
experiment ([design](design/compile-little.md), section 1) shows why. The compiled tree
runs about as many instructions as CPython (1175M against 1190M over import, definition
and schema), with about twice the I1 misses (6.7M against 3.4M) from a 10.9 MB `.so`.
Three hot functions written as ideal C and injected into plain CPython are 3-8x faster
than both. With them, schema takes 9.33 ms against CPython's 11.68 ms (1.25x) and
definition is unchanged.

The plan:

- modules stay bytecode;
- only functions a profile shows hot are compiled, each with exact-type fast paths inline
  and idioms lowered;
- the generic helpers become the cold fallback;
- most of what the levers built is deleted (the design's section 7).

Accept 3 holds the targets, and the design's section 8 the ordered pull requests. Levers
11 and 12 and the chunk tables are deleted (pull request 5, after the switch made them
unnecessary); mirrored imports and classes made once go with non-strict module-body
compilation (pull request 6).


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
| 8 | a published class whose bases are none, `object` or compiled classes made the same way made once at its statement from its compiled spec, no template made at import (pystache: 16 classes once, own share 7.27M -> 6.43M); minor faults 1455 -> 1446 / unchanged / 127 -> 121 | -0.74M | -0.15M | -0.83M |
| cold calls (removed) | the shim's and the package unit's stable-ABI calls routed through functions CPython's import runs (`hot_api.c`), so a BOLT-built interpreter's cold text is touched less (pystache: 28 -> 23 windows of 64 KiB); minor faults 1456 -> 1440 / 760 -> 756 / 126 -> 123. Deleted by the design's pull request 2: removing it left pydantic's and pystache's import inside main's IQR (81 rounds) and within 0.22M instructions | -0.41M (noise 0.3M) | 0 | 0 |
| 11 | a dataclass's generated methods compiled at build time by the target interpreter's own `dataclasses` on a stand-in of the class, run at import for a generated text equal to the build's instead of compiling it (pydantic: 33 of 50 texts; own share 139.80M -> 94.28M at 0f97b93, CPython 135.89M) | -45.5M | -0.34M | 0 |
| 12 | the forward references `typing` makes for a `TypedDict`'s or `NamedTuple`'s string annotations compiled at build time, used at import for a text equal to the build's (pydantic: 103 of the 132 references made outside `pydantic_core`; own share 94.50M -> 89.24M); a text several dataclasses or classes make gets one code object per use | -5.3M | 0 | 0 |
| load: no symbolizer | an ELF module's panic and allocation-failure handlers wrapped to the runtime's, which print the message and abort without walking the stack, so the standard library's stack symbolizer is not linked (pystache's code 971 KB -> 772 KB, symbols bound at `dlopen` 362 -> 349; wall time, 121 rounds: 12.11 -> 11.95 ms against CPython's 11.22, pydantic's 77.6 -> 76.7 ms against 81.9) | 0 | 0 | 0 |

### Progress 2026-11-04: where the import gates stand (not met)

These are local wall times on main at ae0923e (levers 1-12), with PBS CPython 3.15.2 on a 4-vCPU x86_64 VM. Each figure comes from `lbench` (bench.yml's import phases): 81 interleaved fresh-process rounds, nothing else running, load average about 1.3. The ratio is CPython time / pycc time, by median, with the IQR in ms:

| workload | CPython | pycc | ratio |
|---|---|---|---|
| pydantic import | 83.9 (79.7-89.7) | 78.4 (75.1-86.2) | 1.07x |
| `import pystache` | 11.19 (10.42-12.36) | 11.85 (11.32-12.81) | 0.94x |
| `import mistune` | 24.33 (22.86-27.90) | 24.32 (22.97-26.16) | 1.00x |

Two noisier repetitions on the same tree gave 1.12x / 1.07x for pydantic and 0.90x / 0.77x for pystache.

pydantic's suite against the compiled tree fails the 4 `test_exports` tests that also fail against its sources, plus one more: `test_type_checking_model_dump_signature_in_sync_with_base_model`. That test re-executes `pydantic.root_model.__file__`, which a whole-package build does not have (`docs/DEVIATIONS.md`), and it fails the same way before v0.12.

Accept 1 is met locally (stretch not met). Accept 2 is not met. CI has not measured v0.12 yet.

pystache's remaining gap is not in instructions: its own share is 6.92M against CPython's 6.88M. These are the costs measured so far, paired with CPython over 121 to 151 rounds:

- The gap is about 0.8-1.0 ms in all.
- The one `dlopen` of `pystache/__init__.abi3.so` costs about 0.4 ms. PBS links its stdlib extensions in, so CPython's import does no `dlopen` at all. Measured alone, an empty `.so` takes 132 µs, the same 360 imported symbols with no code 264 µs, and the real `.so` 454 µs.
- Faulting in the `.so`'s pages costs about 0.2 ms more: 15 text windows, 6 read-only pages, the 15 `.data`/`.data.rel.ro` pages that the 2954 relative relocations write, and 16 `.bss` pages.
- Pre-touching the 23 text windows of the interpreter that only the compiled import enters measured no difference, and `madvise(MADV_POPULATE_READ)` on the `.so` made the import slower.
- What is left points at the load itself:
  - fewer imported symbols, for example C-API calls bound on first use rather than all 360 at `RTLD_NOW`;
  - fewer pointer-bearing data pages;
  - import-time code and state placed together;
  - a smaller `.so`. About 290 KB of its 950 KB of text is Rust's standard library, including the backtrace symbolizer.

### Progress 2026-11-04 (later): the load levers, and where the gates stand (not met)

These are the four load levers listed above, each measured on the same VM against the main it would replace. Only the smaller `.so` was worth a pull request:

- **A smaller `.so`** (#73, the "load: no symbolizer" row above): the backtrace symbolizer is no longer linked. pystache's code went from 971 KB to 772 KB and its symbols bound at `dlopen` from 362 to 349. Its `dlopen` went from 274 µs to 261 µs, and its import from 12.11 ms to 11.95 ms over 121 rounds (CPython 11.22 ms).
- **C-API calls bound on first use** were not built. Binding only the 127 symbols the import calls would save about 30 µs of the 100 µs that all 362 symbol lookups cost (a `dlsym` measurement). CPython opens extensions with `RTLD_NOW`, so the binding would have to go through a table of pycc's own.
- **Fewer pointer-bearing data pages**: the 13 pages `.data` and `.data.rel.ro` copy on write are the shim's method, getset and type tables. Those are written by their relocations whether or not they are `const`. Removing the faults would need position-independent tables throughout the shim, for about 30 µs. Linking without RELRO saved 20 µs and is ruled out, because RELRO is a hardening measure.
- **Import-time code and state placed together**: LLVM already puts the module bodies in `.text.unlikely`, so they sit together. The import runs 103 KB of the extension's code over 12 of its 64 KiB windows. Pre-faulting the module's `.bss` with `MADV_POPULATE_WRITE` made the import slower: all of it cost about 100 more faults, and its first 64 KB was within noise.

Main at e2dbaaa, with the same VM, `lbench` harness and PBS CPython 3.15.2 as above. Each row is interleaved fresh-process rounds with nothing else running. The ratio is CPython time / pycc time, by median, with the IQR in ms:

| workload | rounds | load average | CPython | pycc | ratio |
|---|---|---|---|---|---|
| `import pystache` | 151 | 0.7 | 11.27 (10.65-12.44) | 11.73 (11.09-12.69) | 0.96x |
| `import pystache` | 121 | 2.6, just after a build | 10.87 (10.18-11.76) | 11.60 (10.98-12.52) | 0.94x |
| pydantic import | 61 | 0.8 | 82.6 (79.0-87.0) | 78.8 (74.8-81.6) | 1.05x |

The latest CI benchmark, bench run 41750538464 at 0f97b93 (before #66, #70, #71 and #73), gave aarch64 0.97x for pydantic's import and 0.97x for pystache's.

Accept 1 is met locally, but CI has not confirmed it on both runners, and the stretch is not met. Accept 2 is not met: about 0.5 ms of pystache's 11 ms import remains, all in loading the extension (`dlopen`, and faulting in its pages and the interpreter's cold text), not in instructions.

What is left:
- lazy code objects (lever 9);
- the rest of lever 7 (steps 2 and 3) and lever 13;
- binding C-API calls on first use.

Each is estimated at well under 0.1 ms for pystache. Reaching parity there likely needs a load with no `dlopen` at all, as CPython's built-in extensions have, or a much smaller extension.
