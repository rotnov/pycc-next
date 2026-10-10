---
name: oracle-test
description: Use when adding or changing compiler behavior in pycc-next and you need an end-to-end test proving the compiled program matches CPython 3.13 output and exceptions (native build and --ext module).
---

# Oracle test

Correct = same stdout and same exception as `python3` (CPython 3.13).

1. Write the program as a fixture: `tests/fixtures/<topic>.py`. Cover the success path,
   edge cases (empty, negative, boundaries, nesting) and at least one raised exception.
   Print everything you want compared.
2. Get the oracle output: `python3 tests/fixtures/<topic>.py`.
3. Add a test in `tests/<topic>.rs` that:
   - runs `env!("CARGO_BIN_EXE_pycc")` with `build <fixture> -o <scratch>/prog --release`
     (use `pycc_scratch::ScratchDir` for the temp dir),
   - runs the binary and compares stdout byte-for-byte with the output of `python3 <fixture>`
     run in the same test (do not hard-code expected text when CPython can produce it),
   - for exceptions, compares the exception type and message line, and the exit status (1).
4. If the feature crosses the extension boundary, also build with `--ext`, import the module
   from `python3 -c`, and compare with importing the `.py` source.
5. Unsupported constructs must fail with a spanned `C0001` diagnostic, never miscompile:
   add a `pycc check` test asserting the diagnostic for anything you deliberately leave out.
6. Run `cargo test --release --test <topic>` then the full workspace before pushing.

Existing references: `tests/conformance.rs` (`run_conformance_fixture`),
`tests/issue_1018_aug_assign.rs` (check-fails helpers, `--ext` arms).
