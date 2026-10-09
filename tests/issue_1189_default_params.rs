//! End-to-end proof for Part 2 of #884
//! ([#1189](https://github.com/rotnov/pycc/issues/1189)): a call that omits a
//! defaulted argument compiles and runs to exactly the same output as the
//! call that writes the same literal explicitly, and a default outside the
//! admitted subset fails `pycc check` at its own `def`.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write_fixture(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(source.as_bytes()).unwrap();
    path
}

/// Builds `source` and returns the executable's stdout.
fn build_and_run(dir: &ScratchDir, name: &str, source: &str) -> String {
    let src = write_fixture(dir, &format!("{name}.py"), source);
    let out = dir.join(name);
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success(), "`pycc build` failed for {name}");
    let output = Command::new(&out).output().unwrap();
    assert!(
        output.status.success(),
        "the built program failed for {name}"
    );
    String::from_utf8(output.stdout).unwrap()
}

/// `pycc check`'s combined stdout and stderr, which must be a failure.
fn check_err(dir: &ScratchDir, name: &str, source: &str) -> String {
    let src = write_fixture(dir, &format!("{name}.py"), source);
    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success(), "`pycc check` unexpectedly passed");
    let mut rendered = String::from_utf8_lossy(&output.stdout).into_owned();
    rendered.push_str(&String::from_utf8_lossy(&output.stderr));
    rendered
}

const SHOW: &str = "def show(i: int = -7, f: float = 1.5, b: bool = False, s: str = \"hi\") \
                    -> None:\n    print(i)\n    print(f)\n    print(b)\n    print(s)\n\n";

#[test]
fn an_omitted_default_produces_the_same_output_as_its_explicit_twin() {
    let dir = ScratchDir::new("e2e_issue_1189").expect("failed to create scratch dir");
    let filled = build_and_run(&dir, "filled", &format!("{SHOW}show()\n"));
    let explicit = build_and_run(
        &dir,
        "explicit",
        &format!("{SHOW}show(-7, 1.5, False, \"hi\")\n"),
    );
    assert_eq!(filled, "-7\n1.5\nFalse\nhi\n");
    assert_eq!(filled, explicit);
}

#[test]
fn every_call_shape_of_a_defaulted_def_agrees() {
    let dir = ScratchDir::new("e2e_issue_1189_shapes").expect("failed to create scratch dir");
    const DEF: &str = "def pair(a: int, b: int = 3) -> None:\n    print(a)\n    print(b)\n\n";
    let expected = "1\n3\n";
    for (name, call) in [
        ("positional_short", "pair(1)"),
        ("positional_full", "pair(1, 3)"),
        ("mixed", "pair(1, b=3)"),
        ("keyword_short", "pair(a=1)"),
        ("keyword_full", "pair(b=3, a=1)"),
    ] {
        assert_eq!(
            build_and_run(&dir, name, &format!("{DEF}{call}\n")),
            expected,
            "call: {call}"
        );
    }
}

#[test]
fn both_i64_extremes_survive_as_defaults() {
    let dir = ScratchDir::new("e2e_issue_1189_edges").expect("failed to create scratch dir");
    assert_eq!(
        build_and_run(
            &dir,
            "edges",
            "def edges(lo: int = -9223372036854775808, hi: int = 9223372036854775807) -> None:\n\
             \x20   print(lo)\n\x20   print(hi)\n\nedges()\n",
        ),
        "-9223372036854775808\n9223372036854775807\n"
    );
}

#[test]
fn an_unadmitted_default_fails_check_at_its_own_def() {
    let dir = ScratchDir::new("e2e_issue_1189_bad").expect("failed to create scratch dir");
    let rendered = check_err(
        &dir,
        "bad",
        "def f(a: int = [1]) -> None:\n    print(a)\n\nf()\nf()\n",
    );
    assert!(
        rendered.contains("C0001")
            && rendered.contains(
                "only a literal `int`, `float`, `bool`, `str`, or `None` default parameter \
                 value is supported yet"
            ),
        "unexpected diagnostic: {rendered}"
    );
    // Rejection lives at the `def`, so two call sites do not multiply it.
    assert_eq!(
        rendered.matches("error[C0001]").count(),
        1,
        "unexpected diagnostic: {rendered}"
    );
}

#[test]
fn a_default_of_the_wrong_type_fails_check_with_t0021() {
    let dir = ScratchDir::new("e2e_issue_1189_ty").expect("failed to create scratch dir");
    let rendered = check_err(
        &dir,
        "mismatch",
        "def f(a: float = 1) -> None:\n    print(a)\n\nf()\n",
    );
    assert!(
        rendered.contains("T0021")
            && rendered
                .contains("default value of parameter `a` of `f` expects `float`, got `int`"),
        "unexpected diagnostic: {rendered}"
    );
}

#[test]
fn a_default_on_a_protocol_member_keeps_its_capability_refusal() {
    // A method default is admitted since the method part of #1140
    // (`tests/issue_1140_method_defaults.rs`); a `Protocol` member, which
    // has no body to serve it, keeps the refusal.
    let dir = ScratchDir::new("e2e_issue_1189_method").expect("failed to create scratch dir");
    let rendered = check_err(
        &dir,
        "method",
        "from typing import Protocol\n\nclass P(Protocol):\n    \
         def m(self, a: int = 1) -> None: ...\n",
    );
    assert!(
        rendered.contains("C0001")
            && rendered.contains("default parameter values are not supported yet"),
        "unexpected diagnostic: {rendered}"
    );
}

#[test]
fn an_imported_defs_default_is_not_filled_across_the_module_boundary() {
    // The signature table is per module, exactly as Part 1's keyword binding
    // is, so an imported `def`'s default is not filled and the short call
    // keeps the arity error at the import. Widening both to the module
    // boundary is Part 3 of #884.
    let dir = ScratchDir::new("e2e_issue_1189_import").expect("failed to create scratch dir");
    write_fixture(
        &dir,
        "helper_1189.py",
        "def scaled(value: int, factor: int = 3) -> int:\n    return value * factor\n",
    );
    let rendered = check_err(
        &dir,
        "importer",
        "from helper_1189 import scaled\n\nprint(scaled(2))\n",
    );
    assert!(
        rendered.contains("T0021") && rendered.contains("`scaled` expects 2 argument(s), got 1"),
        "unexpected diagnostic: {rendered}"
    );
}

/// A call textually above its callee is still `pycc_types`' ordering error,
/// unchanged by Part 2: the HIR binder fills the default from the signature
/// table, which is collected before any item is lowered and therefore knows
/// the `def` already, but the definition-order rule is a separate later
/// check and this part does not touch it.
#[test]
fn a_forward_call_to_a_defaulted_def_keeps_its_own_ordering_error() {
    let dir = ScratchDir::new("e2e_issue_1189_forward").expect("failed to create scratch dir");
    let rendered = check_err(
        &dir,
        "forward",
        "f(1)\ndef f(a: int, b: int = 2) -> None:\n    print(a)\n    print(b)\n",
    );
    assert!(rendered.contains("error[T0021]"), "{rendered}");
    assert!(
        rendered.contains("cannot call function `f` before its definition"),
        "{rendered}"
    );
}

/// A name bound more than once at module level is left out of the signature
/// table (`docs/TYPE_SYSTEM.md`, "Keyword arguments and default parameter
/// values on a redefined name"): pycc dispatches a redefined `def` in source
/// order, while the table is static, so filling either `def`'s default could
/// silently call the other with the wrong value (CPython prints `1` then `2`
/// here). The short call is left exactly as written and `pycc_types`' arity
/// check rejects it.
#[test]
fn a_short_call_to_a_redefined_def_fails_check_with_the_arity_error() {
    let dir = ScratchDir::new("e2e_issue_1189_redef_short").expect("failed to create scratch dir");
    let rendered = check_err(
        &dir,
        "redef_short",
        "def foo(a: int = 1) -> None:\n    print(a)\n\nfoo()\n\n\
         def foo(a: int = 2) -> None:\n    print(a)\n\nfoo()\n",
    );
    assert!(
        rendered.contains("error[T0021]")
            && rendered.contains("`foo` expects 1 argument(s), got 0"),
        "unexpected diagnostic: {rendered}"
    );
}

/// The same exclusion covers Part 1's keyword binding: with the two `def`s'
/// parameters in opposite orders, binding against either one would compute
/// the wrong difference at one of the two calls (CPython prints `9` twice).
#[test]
fn a_keyword_call_to_a_redefined_def_keeps_the_capability_rejection() {
    let dir = ScratchDir::new("e2e_issue_1189_redef_kw").expect("failed to create scratch dir");
    let rendered = check_err(
        &dir,
        "redef_kw",
        "def foo(a: int, b: int) -> None:\n    print(a - b)\n\nfoo(a=10, b=1)\n\n\
         def foo(b: int, a: int) -> None:\n    print(a - b)\n\nfoo(a=10, b=1)\n",
    );
    assert!(
        rendered.contains("error[C0001]")
            && rendered.contains("keyword call arguments are not supported yet"),
        "unexpected diagnostic: {rendered}"
    );
}

/// Only keyword binding and default filling are withdrawn from a redefined
/// name: a call that supplies every argument positionally still builds and
/// still reaches whichever `def` is bound at that point in source order.
/// Issue #22's `redefinition_affects_only_subsequent_calls` pins the same
/// dispatch for a zero-parameter `def`; this pins it for defaulted ones.
#[test]
fn full_positional_calls_to_a_redefined_defaulted_def_run_in_source_order() {
    let dir = ScratchDir::new("e2e_issue_1189_redef_full").expect("failed to create scratch dir");
    let stdout = build_and_run(
        &dir,
        "redef_full",
        "def foo(a: int = 1) -> None:\n    print(a)\n\nfoo(5)\n\n\
         def foo(a: int = 2) -> None:\n    print(a + 100)\n\nfoo(5)\n",
    );
    assert_eq!(stdout, "5\n105\n");
}
