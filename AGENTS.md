# Agent instructions

## What we build
- Compile typed Python to native code at Rust-like speed; type mismatches are compile
  errors, never runtime surprises.
- Primary artifact: a CPython extension module (`pycc build m.py -o m --ext`). Standalone
  binaries exist too, but interop with numpy/torch is the point.
- `strict` (default): every type known, native representations. `non-strict` (per module):
  unprovable code falls back to `PyObject*` C-API calls.

## Correctness
- Oracle: CPython 3.13. Correct = same stdout and same exceptions as CPython.
- Every behavior change gets an end-to-end test that compiles a Python program and
  compares against CPython. Unit tests are optional; oracle tests are not.
- A deliberate deviation from CPython (e.g. for speed) is listed in `docs/DEVIATIONS.md`
  with the reason.

## Speed
- Numbers or it didn't happen: `python3 bench/run.py` (CPython vs pycc-next vs Rust).
- Publish regressions as honestly as wins. Never cherry-pick runs.

## Rewrite vs fix
- If the design is wrong (wrong representation, wrong abstraction, special case per type),
  rewrite the module. If the design is right but incomplete, extend it.
- When you rewrite or touch a file over ~1000 lines, split the part you touch.
- Delete dead code and stale comments instead of preserving them.

## Workflow
- Build: `export LLVM_SYS_221_PREFIX=/usr/lib/llvm-22 && cargo build --release`.
- Before pushing: `cargo test --release --workspace`. Known environment-only failures when
  running as root: the read-only/unreadable-permission tests (lock, project_config,
  a_read_only_directory_*). Everything else must pass.
- Commit small, push to `main`. CI is one job; keep it green.
- One writer per working tree: never run two agents editing the same checkout.

## Communication
- Durable artifacts (code, docs, commits, blog) in English.
- `blog/_posts/`: first person, punchy, honest - wins and failures with real numbers.
  Post on every notable step.
- No ADR registry, no process gates, no governance tooling.
