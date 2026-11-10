//! The lowerings of docs/design/compile-little.md sections 2.6 and 2.7 in
//! generic bodies, built `--ext --non-strict --select all`:
//!
//! - a dict miss caught by a handler that cannot see the exception goes
//!   straight to the handler (`tests/fixtures/lazy_key_error.py`);
//! - `tuple(a)`, `list(a)` and `sorted(a)` skip the call through the builtin
//!   (`tests/fixtures/sequence_builtins.py`), unless it was rebound
//!   (`tests/fixtures/sequence_builtins_rebound.py`);
//! - a comprehension that unpacks its item over a dict's own `items()`
//!   releases each item before it advances
//!   (`tests/fixtures/comprehension_item_release.py`).
//!
//! Each module prints what CPython prints for its source, with the
//! lowerings on (their counters move) and off (`PYCC_NO_FAST_OPS=1`: every
//! miss raises, every call goes through the builtin, every item is kept
//! until the next, and the counters stay at zero).

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::Command;

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

/// The marker a probe prints between what `main()` printed and the rest.
const MARK: &str = "--- only with the lowerings on ---\n";

/// Builds `tests/fixtures/<fixture>.py` as an extension module, checks that
/// `compiled` functions were compiled, and compares what it prints with
/// CPython's output for the source, with the lowerings on and off. `extra`
/// names a function of the fixture that only the run with the lowerings on
/// calls after `main()`. With them on `counter` must have moved, unless
/// `moves` is false; with them off it must read zero.
fn check(fixture: &str, compiled: &[&str], counter: &str, moves: bool, extra: Option<&str>) {
    let dir = ScratchDir::new(fixture).expect("scratch");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{fixture}.py"));
    let module = format!("{fixture}_ext");
    std::fs::copy(&source, dir.join(format!("{module}.py"))).expect("copy the fixture");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args([
            "build",
            "--ext",
            "--non-strict",
            "--release",
            "--select",
            "all",
            "--report-fallbacks",
        ])
        .arg(dir.join(format!("{module}.py")))
        .arg("-o")
        .arg(dir.join(&module))
        .output()
        .expect("pycc should spawn");
    let report = text(&pycc_scratch::lf(&build.stderr));
    assert!(build.status.success(), "{report}");
    for name in compiled {
        assert!(
            report.contains(&format!("compiled: {module}:{name}\n")),
            "{name}\n{report}"
        );
    }
    std::fs::remove_file(dir.join(format!("{module}.py"))).expect("remove the source");

    // CPython's output: the module body runs `main()`; `extra` follows the
    // marker.
    let oracle_dir = ScratchDir::new(&format!("{fixture}_oracle")).expect("scratch");
    std::fs::copy(&source, oracle_dir.join(format!("{fixture}_src.py"))).expect("copy the fixture");
    let extra_call = |name: &str| extra.map(|f| format!("{name}.{f}()\n")).unwrap_or_default();
    let oracle_probe = format!(
        "import sys\nimport {fixture}_src\nsys.stdout.write({MARK:?})\n{}",
        extra_call(&format!("{fixture}_src"))
    );
    let oracle = python()
        .args(["-c", &oracle_probe])
        .current_dir(&*oracle_dir)
        .output()
        .expect("python3 should spawn");
    assert!(
        oracle.status.success(),
        "{}",
        text(&pycc_scratch::lf(&oracle.stderr))
    );
    let oracle_out = text(&pycc_scratch::lf(&oracle.stdout));
    let (main_out, extra_out) = oracle_out
        .split_once(MARK)
        .expect("the oracle prints the marker");
    assert!(!main_out.is_empty());

    for off in [false, true] {
        // The module body runs `main()` before the patch, as bytecode; the
        // second call runs the compiled bodies.
        let probe = format!(
            "import ctypes, sys\nimport {module}\n{module}.main()\n{}\
             lib = ctypes.CDLL({module}.__file__)\n\
             lib.{counter}.restype = ctypes.c_longlong\n\
             print('COUNT', lib.{counter}(), file=sys.stderr)\n",
            if off {
                String::new()
            } else {
                extra_call(&module)
            }
        );
        let mut run = python();
        run.args(["-c", &probe]).current_dir(&*dir);
        if off {
            run.env("PYCC_NO_FAST_OPS", "1");
        } else {
            run.env_remove("PYCC_NO_FAST_OPS");
        }
        let ours = run.output().expect("python3 should spawn");
        let stderr = text(&pycc_scratch::lf(&ours.stderr));
        assert!(ours.status.success(), "lowerings off: {off}\n{stderr}");
        let expected = format!("{main_out}{main_out}{}", if off { "" } else { extra_out });
        assert_eq!(
            text(&pycc_scratch::lf(&ours.stdout)),
            expected,
            "lowerings off: {off}\n{stderr}"
        );
        let count: i64 = stderr
            .lines()
            .find_map(|line| line.strip_prefix("COUNT "))
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("no counter line\n{stderr}"));
        if off || !moves {
            assert_eq!(count, 0, "{counter}, lowerings off: {off}\n{stderr}");
        } else {
            assert!(count > 0, "{counter} never moved\n{stderr}");
        }
    }
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_dict_miss_caught_unseen_matches_cpython_with_the_lowering_on_and_off() {
    check(
        "lazy_key_error",
        &[
            "plain",
            "chained",
            "bound",
            "reraised",
            "index_first",
            "with_else_finally",
            "nested_both",
            "temporaries",
            "handler_raises",
            "temporary_key",
            "dropped_key",
            "temporary_keys_in_loop",
            "temporary_key_return_in_loop",
            "temporary_key_recursive",
            "meddling_key",
            "main",
        ],
        "pycc_ext_lazy_misses",
        true,
        None,
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn tuple_list_and_sorted_match_cpython_with_the_lowering_on_and_off() {
    check(
        "sequence_builtins",
        &[
            "sorted_comprehension",
            "tuple_sorted_generator",
            "list_of_call",
            "sorted_kept",
            "list_kept",
            "sorted_key",
            "sorted_key_reverse",
            "sorted_compare_fails",
            "identity",
            "stable",
            "sorted_fails_finalized",
            "sorted_mixed_finalized",
            "sorted_spied",
            "sorted_grabbed",
            "identities",
            "failing_sorted",
            "list_finalized",
        ],
        "pycc_ext_direct_builtins",
        true,
        None,
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn rebound_sorted_list_and_tuple_are_called() {
    check(
        "sequence_builtins_rebound",
        &["calls", "main"],
        "pycc_ext_direct_builtins",
        false,
        None,
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_comprehension_over_dict_items_matches_cpython_with_the_lowering_on_and_off() {
    check(
        "comprehension_item_release",
        &[
            "over_generator",
            "over_iterator",
            "items_pairs",
            "items_kept",
            "items_nested",
            "items_values_replaced",
            "items_whole_replaced",
            "items_filtered_replaced",
            "items_fails",
            "main",
        ],
        "pycc_ext_item_recycles",
        true,
        Some("replaced"),
    );
}
