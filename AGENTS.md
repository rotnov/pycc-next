# Agent instructions

- Goal: compile typed Python to native code at Rust-like speed; type errors are
  compile errors. CPython interop (extension modules) is the primary artifact.
- Oracle: CPython 3.13. Correct means same stdout/exceptions as CPython.
- Before pushing: `cargo build --release && cargo test --release --workspace`.
- Keep it lean: no ADR registry, no process gates. Record notable progress or
  decisions as a short post in `blog/_posts/`.
- Durable artifacts (code, docs, commits, blog) in English.
- Files over ~1000 lines: split the part you touch.
