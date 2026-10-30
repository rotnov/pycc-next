# pycc-next

Compile typed Python to native code at Rust-like speed; type mismatches are compile errors.
Primary artifact: CPython extension module (`pycc build m.py -o m --ext`). `strict` mode by
default; `--non-strict` falls back to `PyObject*` (CPython C API) for unprovable code.

- Oracle: CPython 3.13 — same stdout and exceptions. Known gaps: `docs/DEVIATIONS.md`.
- Build: `export LLVM_SYS_221_PREFIX=/usr/lib/llvm-22 && cargo build --release`.
- Test before push: `cargo test --release --workspace -- --include-ignored` (CI runs exactly
  this with python3 = CPython 3.13) and
  `cargo clippy --release --workspace --all-targets -- -D warnings`. Read-only/unreadable permission
  tests fail only when running as root; everything else must pass.
- Design wrong -> rewrite the module; design right but incomplete -> extend. Split files
  over ~1000 lines you touch. Delete dead code.
- One writer per working tree. Every change goes through a pull request from its own
  branch: never push to `main`. Open the PR early, keep it small, and merge it (merge commit)
  only when the required checks — `test (ubuntu-24.04)`, `test (ubuntu-24.04-arm)`,
  `macos`, `windows` — are green on its head and review threads are answered. Several
  agents work in parallel on separate branches; rebase or merge `main` into your branch
  when it conflicts. If `main` turns red after a merge, fixing it is the next PR.
- CI runs on pull requests only, not again on `main` after a merge. So before merging, the
  green run must be against the current `main`: if `main` moved since that run, merge
  `main` into the branch and wait for CI again. A PR that changes only prose (`blog/`,
  `docs/` except DEVIATIONS.md and DIAGNOSTICS.md, AGENTS.md, CLAUDE.md, `.claude/`) skips
  the four test jobs: it merges once the `changes` job is green and they show as skipped.
- Review findings (Codex and others): collect every finding on the current head, fix
  them all locally, run the local gate once, then push once. Never push one fix per
  finding; each push costs a full CI run (~35 min) and a new review.
- Wait on long commands by running them in the foreground (or as a tracked background
  task), never with `until ! pgrep -f <pattern>` loops: the pattern matches the loop itself.
- Durable artifacts in English.

Skills (`.claude/skills/<name>/SKILL.md`, readable by any agent): `oracle-test`, `bench`,
`blog-post`, `release`.
