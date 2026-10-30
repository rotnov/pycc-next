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
2. **Smaller, faster-loading extensions:** whole-package builds use LTO across the
   package's modules, identical-code folding and cold sections for error and fallback
   paths; the pydantic extension's size and load-time relocations are measured and reduced,
   with no runtime instruction-count regression on the CI bounds.
3. **Less work at import:** fewer fallback chunks executed at import (functions and classes
   created natively where possible), and lazy creation of module-level objects only where
   it is unobservable to Python code (module `__dict__` contents, `dir()`, `inspect`,
   pickling, import side-effect order and exceptions match CPython); every laziness rule is
   documented with its observability argument and covered by oracle tests.
4. **General, not pydantic-only:** mistune's and pystache's import times are measured in
   the CI benchmark job and are no slower than CPython's; no package check regresses.
