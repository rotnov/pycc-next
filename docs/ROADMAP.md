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

## v0.11 — Imports faster than CPython

v0.10's CI benchmark job put pydantic's import at 0.93x CPython (455.7M instructions vs
442.9M); the rest is a 10.5 MB extension to load, ~8.8M instructions of libpython object
creation (functions, classes, forwarders, signatures) and ~296 fallback chunks run at
import. v0.11 makes importing compiled packages at least as fast as importing their sources,
then faster, without changing what Python code can observe.

Accept:
1. **pydantic import >= 1.0x**, then **>= 1.2x** as a stretch: median of the CI benchmark
   job (`bench.yml`) for `import pydantic; from pydantic import BaseModel`, compiled tree vs
   CPython from `.pyc`, same pydantic-core wheel; its suite fails no test that passes
   against its sources.
2. **Smaller, faster-loading extensions:** LTO across a whole-package build's modules,
   identical-code folding and cold sections for import-only, error and fallback paths are
   each measured on the pydantic and mistune extensions; what pays ships, the rest is
   recorded with its numbers; the pydantic extension's size and load-time relocations are
   measured and reduced, with no runtime instruction-count regression on the CI bounds.
   **Measured 2026-10-30** (x86_64, glibc 2.39, GNU ld 2.42; callgrind/cachegrind with
   `PYTHONHASHSEED=0`). Shipped: ELF extensions built for a glibc >= 2.36 link with
   `-z pack-relative-relocs` -- pydantic's extension 10,751,680 -> 10,256,144 bytes
   (-4.6%), relocation entries 21,222 -> 349 (`.rela.dyn` 503,712 bytes -> 2,760 plus a
   5,160-byte `.relr.dyn`), `dlopen` 1.214M -> 1.167M instructions; mistune's 3,934,696 ->
   3,717,680 bytes (-5.5%), 9,395 -> 348 entries, `dlopen` 472K -> 460K. An extension's
   module body is `cold` (`.text.unlikely`, away from what runs after import): simulated
   L1i misses on mistune's hot path -2.0% per call. The CI instruction bounds are
   unchanged to the digit (diff-match-patch 131.29M, pypng 13.79M, mistune 57.87M,
   pystache 0.47M per call), and so is the build: no compile step changed. Loading the
   extension is not where the import goes: `dlopen` is 0.3% of pydantic's 376M-instruction
   import, and the import's instruction count and wall time are unchanged within noise.
   Rejected: LTO (members' optimized bitcode linked, `constmerge,mergefunc,globaldce`,
   cross-member inlining, one backend) -- text -0.2%, mistune's hot path 0.0%, and a
   serial backend over the merged module (22 s for pydantic, 6.4 s for mistune, where the
   members' backends now run in parallel); identical-code folding (`lld --icf=all` over
   function sections) -- -0.3% text, and only with lld, which `cc` does not use; function
   and data sections with `--gc-sections` -- -0.4%; hot/cold splitting of error paths
   (`hotcoldsplit`) -- text -0.8% but `.eh_frame` +33%, hot path +0.13% instructions and
   +1.3% L1i misses; `minsize` module bodies -- text -50 KB, `.eh_frame` +70 KB, the import
   no fewer instructions; `cold` on the shim's import-only C functions -- no measurable change;
   `nounwind` generated code (`.eh_frame` -22%) and `--strip-all` (-1.57 MB, the symbol
   table, which no loader maps) -- smaller files, but no unwinding through or naming of
   compiled frames in debuggers and profilers; `-z noseparate-code` -- 4 KB for an
   executable read-only segment. Where pydantic's bytes are: `.text` 4.41 MB (generated
   functions 1.42 MB, module bodies 0.92 MB, wrappers 0.55 MB, runtime 0.44 MB, the
   per-member C units 0.30 MB -- the remaining identical code, which takes moving them
   into the runtime unit over the module record), `.rodata` 3.49 MB (1.32 MB of fallback
   chunk tables and the fallback source), symbol tables 1.57 MB.
3. **Less work at import:** fewer fallback chunks executed at import (functions and classes
   created natively where possible), and lazy creation of module-level objects only where
   it is unobservable to Python code (module `__dict__` contents, `dir()`, `inspect`,
   pickling, import side-effect order and exceptions match CPython); every laziness rule is
   documented with its observability argument and covered by oracle tests.
4. **General, not pydantic-only:** mistune's and pystache's import times are measured in
   the CI benchmark job and are no slower than CPython's; no package check regresses.
