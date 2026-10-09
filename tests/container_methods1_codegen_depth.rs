// PR-12 Task 11 (D-119): `list.pop()`/`dict.get(key, default)`/
// `set.add(value)` end to end through the real `pycc build` CLI. Mirrors
// `tests/slice1_codegen_depth.rs`'s own `build_and_run` convention exactly
// (real Python source, not hand-built MIR): the hand-built-MIR tests that
// actually count toward `cargo llvm-cov -p pycc_codegen` live inline in
// `crates/pycc_codegen/src/lib.rs`'s own test module (a workspace-root
// `tests/*.rs` integration binary is attributed to the `pycc` package, not
// `pycc_codegen`) -- these real-source tests exist for genuine
// front-to-back empirical confidence (parser -> hir -> types -> mir ->
// codegen -> link -> run), not for coverage.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write_fixture(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(source.as_bytes()).unwrap();
    path
}

fn build_and_run(label: &str, source: &str) -> std::process::Output {
    let dir = ScratchDir::new(&format!("container_methods1_{label}"))
        .expect("failed to create scratch dir");
    let src = write_fixture(&dir, &format!("{label}.py"), source);
    let out = dir.join(label);
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success(), "`pycc build` failed for {label}");
    Command::new(&out).output().unwrap()
}

#[test]
fn list_pop_removes_and_returns_the_last_element() {
    // Expected output verified against `python3` on this exact source.
    let source = "\
xs = [1, 2, 3]
y = xs.pop()
print(y)
print(len(xs))
";
    let output = build_and_run("list_pop_basic", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"3\n2\n");
}

#[test]
fn list_pop_on_an_empty_list_raises_a_catchable_index_error() {
    // Part B of #1038 (#1064) replaced D-119's honest-panic convention here
    // with a D-173 pending-exception raise, so this now matches CPython:
    // `IndexError`, catchable, rather than a process abort. Verified against
    // `python3` that the source is valid Python raising `IndexError` there
    // too. The uncaught form is pinned by
    // `tests/issue_1064_list_set_float_raises.rs`; this one pins that the
    // whole-program path is catchable from the compiled binary.
    //
    // Emptied via one `.pop()` rather than written as a literal `xs = []`:
    // an empty list literal's element type cannot be inferred without an
    // annotation (T0021, D-105 -- `list[T]` annotations are not supported
    // yet), so this is the only way real, type-checked Python source can
    // produce an empty `list[int]` here.
    let source = "\
xs = [1]
xs.pop()
try:
    y = xs.pop()
except IndexError as e:
    print(e)
";
    let output = build_and_run("list_pop_empty_raises", source);
    assert!(
        output.status.success(),
        "popping from an empty list must raise a catchable IndexError: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"pop from empty list\n");
}

#[test]
fn pop_twice_on_the_same_list_in_one_statement_removes_in_order() {
    // This task's own brief flags exactly this shape (`xs = [xs.pop(),
    // xs.pop()]`) as a place a Task-5a-class evaluation-order bug could
    // hide. Real CPython evaluates the two elements strictly left to
    // right, each `.pop()` observing the previous one's mutation: the
    // first removes `5` (leaving `[1,2,3,4]`), the second removes the new
    // last element `4` (leaving `[1,2,3]`). Expected output verified
    // against `python3` on this exact source.
    let source = "\
xs = [1, 2, 3, 4, 5]
ys = [xs.pop(), xs.pop()]
for v in ys:
    print(v)
for v in xs:
    print(v)
";
    let output = build_and_run("list_pop_twice_same_statement", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"5\n4\n1\n2\n3\n");
}

#[test]
fn dict_get_returns_the_stored_value_or_the_default() {
    // `99` stands in for a sentinel "not found" default rather than a
    // negative literal: when this test was written no unary operator lowered
    // at all, so a negative `int` literal could not appear directly in real
    // Python source here. #602 has since made `-1` lower, but the sentinel
    // reads no worse and the test's subject is `dict.get`, not literal signs.
    // Expected output verified against `python3` on this exact source:
    // `1` (present key) then `99` (missing key, no `KeyError`, unlike
    // plain `d["z"]`).
    let source = "\
d = {\"a\": 1}
print(d.get(\"a\", 99))
print(d.get(\"z\", 99))
";
    let output = build_and_run("dict_get_or_default_basic", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"1\n99\n");
}

#[test]
fn dict_get_nested_in_its_own_default_argument_resolves_correctly() {
    // This task's own brief flags exactly this nested shape (`d =
    // d.get("k", d.get("j", 0))`). `dict.get()` never mutates its dict, so
    // there is no ordering hazard the way `list.pop()` has -- both the
    // outer and inner `.get()` read the same unchanged `d`. `"k"` is
    // absent, so the outer call's own `default` sub-expression
    // (`d.get("a", 99)`) must itself be evaluated; `"a"` is present, so it
    // resolves to `1`. Expected output verified against `python3` on this
    // exact source.
    let source = "\
d = {\"a\": 1}
y = d.get(\"k\", d.get(\"a\", 99))
print(y)
";
    let output = build_and_run("dict_get_or_default_nested_default", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"1\n");
}

#[test]
fn set_add_grows_the_set_and_a_repeated_value_still_dedups() {
    // This task's own brief flags `s.add(x); s.add(x)`-shaped repeated
    // calls as a shape to verify dedup still holds for, across two
    // separate statements. Expected output verified against `python3` on
    // this exact source: `3` then `3` again (the repeated `.add(1)` does
    // not grow the set).
    let source = "\
s = {1, 2}
s.add(3)
print(len(s))
s.add(1)
print(len(s))
";
    let output = build_and_run("set_add_grows_and_dedups", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"3\n3\n");
}

#[test]
fn growing_a_set_with_add_during_its_own_iteration_raises_instead_of_looping_forever() {
    // Review finding (P1) on this same task: `ForSet`'s loop bound
    // re-reads `pycc_rt_int_set_len` every iteration (mirroring `ForDict`,
    // D-123's own accepted divergence), but `set.add(value)` existing in
    // this same commit makes that reachable for the first time. `x + 1`
    // is always a value not yet in `s`, so without a check this loop would
    // never terminate -- real CPython raises a catchable
    // `RuntimeError: Set changed size during iteration` on the very first
    // mutation instead, and Part B of #1038 (#1064) makes pycc do the same:
    // the check raises (D-173) rather than aborting, and `ForSet`'s
    // loop-test gained an `pycc_rt_exception_active() == 0` conjunct so the
    // loop actually terminates -- a D-173 raise returns normally, and the
    // length it compares against is exactly the one the body just grew.
    // Left uncaught here (the catchable form is pinned by
    // `tests/issue_1064_list_set_float_raises.rs`) so this keeps asserting
    // the bounded-process property it was written for.
    let source = "\
s = {1}
for x in s:
    s.add(x + 1)
print(len(s))
";
    let output = build_and_run("set_add_during_iteration_raises", source);
    assert!(
        !output.status.success(),
        "growing a set from inside its own `for` loop must raise, not loop forever or silently \
         extend the iteration"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("RuntimeError: Set changed size during iteration"),
        "expected CPython's own resize message, got: {stderr}"
    );
}
