# pycc-next

Compile typed Python to native code at Rust-like speed; type mismatches are compile errors.
Primary artifact: CPython extension module (`pycc build m.py -o m --ext`). `strict` mode by
default; `--non-strict` falls back to `PyObject*` (CPython C API) for unprovable code.

- Oracle: CPython 3.13 — same stdout and exceptions. Known gaps: `docs/DEVIATIONS.md`.
- Build: `export LLVM_SYS_221_PREFIX=/usr/lib/llvm-22 && cargo build --release`.
- Test before push: `cargo test --release --workspace -- --include-ignored` (CI runs exactly
  this with python3 = CPython 3.13). Read-only/unreadable permission
  tests fail only when running as root; everything else must pass.
- Design wrong -> rewrite the module; design right but incomplete -> extend. Split files
  over ~1000 lines you touch. Delete dead code.
- One writer per working tree. Push small commits to `main`; keep CI green.
- Durable artifacts in English.

Skills (`.claude/skills/<name>/SKILL.md`, readable by any agent): `oracle-test`, `bench`,
`blog-post`, `release`.
