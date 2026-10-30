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

## v0.11 — Imports faster than CPython (in progress)

Importing a compiled package at least as fast as importing its sources, then faster,
without changing what Python code can observe: pydantic's import >= 1.0x (stretch 1.2x),
smaller and faster-loading extensions, less work at import, and mistune's and pystache's
imports measured and no slower than CPython's. Accept items, progress, the 2026-10-30
baseline (where the import time goes) and the ranked plan: [v0.11](roadmap/v0.11.md).
