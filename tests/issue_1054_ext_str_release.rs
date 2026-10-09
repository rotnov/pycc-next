//! #1054 end to end: a hosted `ext` call must not leak a `PyStrObj` per call.
//!
//! Before this change every entry-block `str` slot a compiled function
//! allocated was simply abandoned at the function's exit. In a normal
//! `pycc build` that is a bounded, documented leak (D-074) because the
//! process exits; in a D-244 hosted `ext` module the process is somebody
//! else's long-running interpreter, so the same leak is unbounded -- one
//! `PyStrObj` per call, forever.
//!
//! The observable is `pycc_rt_str_live_objects`, an unconditionally exported
//! `pycc_rt` symbol (a `#[cfg(test)]` counter would not exist here at all:
//! the built `.so` links the ordinary non-test `libpycc_rt.a`). It is read
//! through `ctypes.CDLL` against the *already loaded* module file, so the
//! handle refers to the same mapping the import created rather than a second
//! copy with its own statics.
//!
//! **PEP 683.** The counter this file reads is pycc's own, not CPython's, so
//! immortality does not distort it directly -- but the subject strings are
//! still built at run time rather than written as literals, so that nothing
//! here depends on whether a given interpreter interned them.
//!
//! `#[ignore]`d like every other hosted test in this directory: it needs an
//! installed CPython 3.13+ with development headers, which is a property of
//! the machine and not of the change. CI runs it on the non-Windows Tier-1
//! `native-build-test` legs through that job's
//! `cargo test --workspace -- --include-ignored`. It therefore earns no line
//! coverage (`llvm-cov` runs without that flag); the changed lines are
//! covered by the `issue_1054_*` tests in `crates/pycc_codegen/src/tests.rs`
//! and by `pycc_rt`'s own unit test.
//!
//! **Not compiled on Windows**, by the file-level `cfg` below. MSVC exports
//! from a DLL only what is declared `__declspec(dllexport)` or listed in a
//! `.def` file, and a D-244 `ext` module declares exactly one export, its
//! `PyMODINIT_FUNC PyInit_<name>`. `pycc_rt` is linked in as a static archive
//! (`crate-type = ["staticlib", "rlib"]`), so `pycc_rt_str_live_objects` is
//! *present* in the `.pyd` but not *reachable* through `ctypes`, and there is
//! no separate shared `pycc_rt` to open instead -- a second copy would carry
//! its own `STR_LIVE` and prove nothing. macOS and Linux export every global
//! symbol of a shared library by default, which is why the probe works there.
//! The property under test is codegen plus runtime refcounting and is not
//! platform-specific: the ten `issue_1054_*` IR tests carry it on every
//! platform, Windows included. Exporting the counter from the `ext` artifact
//! so this probe covers all Tier-1 platforms is tracked separately.
#![cfg(not(target_os = "windows"))]

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Builds `body` as an extension module named `module` inside `dir`.
/// Identical in shape to `tests/issue_1049_ext_str.rs`'s own helper.
fn build_ext(dir: &Path, module: &str, body: &str) {
    let src = dir.join("m.py");
    std::fs::write(&src, body).expect("write the fixture source");
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join(module))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
}

/// Runs `script` with the built module importable from `dir`.
fn run_python(dir: &Path, script: &str) {
    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        stderr_of(&run)
    );
}

/// The whole point of the issue: repeated hosted calls whose parameters and
/// locals are `str` leave the live-object count exactly where they found it.
///
/// Three zero-delta exports, one per ownership shape the epilogue has to get
/// right: `echo` returns its own parameter (the returned duplicate is
/// increfd by the `return`, so the slot release balances the incoming
/// reference); `sink` never returns the string at all (the pure-leak shape
/// the issue was filed about); and `alias` binds a function-scoped *local*
/// on top of the parameter, which is C2 -- the leak was never
/// parameter-only.
///
/// The exceptional exit is asserted *differentially* rather than against
/// zero, and deliberately so. A `raise` leaks its own message string
/// independently of this change: `boom_free`, which has no `str` slot at
/// all, leaks exactly as much per call as `boom` does, so the `str`
/// parameter contributes nothing on that path. That pre-existing leak is
/// outside #1054's scope (it is a temporary the exception object owns, not
/// an entry-block slot); asserting the *difference* is what proves the
/// `exception_exit` routing releases the parameter without pretending the
/// unrelated leak is fixed.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_hosted_ext_call_releases_every_str_slot_it_owns() {
    let dir = ScratchDir::new("ext_str_release").expect("scratch");
    build_ext(
        &dir,
        "pycc_str_release",
        "def echo(s: str) -> str:\n    return s\n\n\
         def sink(s: str) -> int:\n    return 1\n\n\
         def alias(s: str) -> int:\n    t = s\n    return 1\n\n\
         def boom(s: str) -> str:\n    raise ValueError(\"boom\")\n\n\
         def boom_free(n: int) -> int:\n    raise ValueError(\"boom\")\n",
    );
    run_python(
        &dir,
        // Every line carries its own indentation inside the literal:
        // Rust's `\`-continuation strips the next source line's leading
        // whitespace, so borrowed indentation would not survive.
        "import ctypes\n\
         import pycc_str_release as m\n\
         lib = ctypes.CDLL(m.__file__)\n\
         live = lib.pycc_rt_str_live_objects\n\
         live.restype = ctypes.c_int64\n\
         live.argtypes = []\n\
         subject = ''.join(chr(ord('a') + i % 26) for i in range(40))\n\
         def delta(call):\n    \
         call()\n    \
         before = live()\n    \
         for _ in range(500):\n        call()\n    \
         return live() - before\n\
         def raising(call):\n    \
         def run():\n        \
         try:\n            call()\n        \
         except ValueError:\n            pass\n    \
         return run\n\
         assert m.echo(subject) == subject\n\
         assert m.sink(subject) == 1\n\
         assert m.alias(subject) == 1\n\
         for name, call in (\n\
         ('echo', lambda: m.echo(subject)),\n\
         ('sink', lambda: m.sink(subject)),\n\
         ('alias', lambda: m.alias(subject)),\n\
         ):\n    \
         d = delta(call)\n    \
         assert d == 0, '%s leaked %d PyStrObj over 500 calls' % (name, d)\n\
         with_str = delta(raising(lambda: m.boom(subject)))\n\
         without_str = delta(raising(lambda: m.boom_free(1)))\n\
         assert with_str == without_str, (\n\
         'the exceptional exit leaked the str parameter: %d vs %d'\n\
         % (with_str, without_str))\n\
         print('ok')\n",
    );
}
