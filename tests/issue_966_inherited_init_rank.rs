//! #966: an implicit `object`-style constructor ranks **last** in
//! constructor resolution.
//!
//! [D-225] gives every class that declares no `__init__` -- and inherits
//! none through its MRO -- an implicit zero-argument constructor, written
//! into that class's *own* method table. Every constructor walk then took
//! the first `__init__` it met along the MRO, so for `class C(A, B)` with
//! `A` init-less and `B` declaring a real `__init__`, `A`'s implicit stub
//! won and `B.__init__` never ran: `B`'s attribute slots stayed
//! uninitialized and reading one aborted in `pycc_rt`. CPython ranks the
//! equivalent (`object.__init__`) last, and so does pycc now.
//!
//! The two exception-shaped consequences of the same ranking change -- the
//! `class MyError(Base, Exception)` raise becoming legal, and binding that
//! class as a value becoming `C0001` -- are asserted in
//! `tests/issue_912_no_init_class.rs`, which already owns the D-225
//! synthesized-ancestor-constructor surface.
//!
//! [D-225]: ../docs/decisions/D-225-synthesize-an-implicit-zero-argument-constructor.md

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write_fixture(dir: &std::path::Path, source: &str) -> std::path::PathBuf {
    let path = dir.join("prog.py");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    path
}

/// Asserts that `pycc check` rejects `source` with a diagnostic containing
/// both `code` and `needle`. Matched on code plus message substring rather
/// than a rendered path, which prints with forward slashes on Windows CI.
fn assert_rejected(tag: &str, source: &str, code: &str, needle: &str) {
    let dir = ScratchDir::new(&format!("issue966_{tag}")).expect("failed to create scratch dir");
    let src = write_fixture(&dir, source);
    let out = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    let rendered = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !out.status.success(),
        "{tag}: pycc check should reject the program, but it succeeded"
    );
    assert!(
        rendered.contains(code) && rendered.contains(needle),
        "{tag}: diagnostic should contain {code:?} and {needle:?}, got:\n{rendered}"
    );
}

/// Compile and run `source`, asserting it prints `expected_stdout`.
fn assert_runs(tag: &str, source: &str, expected_stdout: &str) {
    let dir = ScratchDir::new(&format!("issue966_{tag}")).expect("failed to create scratch dir");
    let src = write_fixture(&dir, source);
    let out = dir.join("prog");

    let build = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{tag}: pycc build should succeed, got: {}",
        String::from_utf8_lossy(&build.stderr)
    );

    let run = Command::new(&out).output().unwrap();
    assert!(
        run.status.success(),
        "{tag}: the built program should exit 0"
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        expected_stdout,
        "{tag}: unexpected program output"
    );
}

/// Compile `source` successfully, then assert running it hits the documented
/// D-072 exit-`101` boundary with `message` on stderr -- the boundary an
/// uncaught Python exception exits through too.
///
/// `pycc run` rather than a direct exec of the built binary, so the `101`
/// is the driver's own mapping, per `docs/CLI_SPEC.md`.
fn assert_runtime_abort(tag: &str, source: &str, message: &str) {
    let dir = ScratchDir::new(&format!("issue966_{tag}")).expect("failed to create scratch dir");
    let src = write_fixture(&dir, source);

    let check = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "{tag}: pycc check should accept the program: {}",
        String::from_utf8_lossy(&check.stderr)
    );

    let run = Command::new(pycc_bin())
        .args(["run", src.to_str().unwrap()])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert_eq!(
        run.status.code(),
        Some(101),
        "{tag}: expected the exit-101 boundary, got {:?}: {stderr}",
        run.status.code()
    );
    assert!(
        stderr.contains(message),
        "{tag}: expected stderr to contain {message:?}, got: {stderr}"
    );
}

/// The issue's own program. `A` is init-less, so before #966 its implicit
/// constructor out-ranked `B.__init__` and `c.z` read an uninitialized slot.
#[test]
fn a_later_base_s_real_constructor_outranks_an_earlier_base_s_implicit_one() {
    assert_runs(
        "issue_repro",
        "class A:\n    def ping(self) -> int:\n        return 9\n\n\nclass B:\n    def __init__(self) -> None:\n        self.z = 1\n\n\nclass C(A, B):\n    pass\n\n\nc = C()\nprint(c.z)\nprint(c.ping())\n",
        "1\n9\n",
    );
}

/// The implicit constructor is skipped wherever it sits in the MRO, not
/// merely at position 1: here it reaches `C` transitively through `A`'s own
/// init-less base.
#[test]
fn an_implicit_constructor_inherited_transitively_is_also_outranked() {
    assert_runs(
        "transitive",
        "class Base:\n    pass\n\n\nclass A(Base):\n    pass\n\n\nclass B:\n    def __init__(self) -> None:\n        self.z = 4\n\n\nclass C(A, B):\n    pass\n\n\nc = C()\nprint(c.z)\n",
        "4\n",
    );
}

/// The mis-ranking was not only a runtime defect: the *checker* resolved the
/// implicit constructor's zero-parameter signature, so passing `B`'s real
/// argument was rejected `T0021` at compile time. CPython prints `5`.
#[test]
fn the_checker_resolves_the_later_base_s_constructor_arity() {
    assert_runs(
        "arity",
        "class A:\n    def ping(self) -> int:\n        return 9\n\n\nclass B:\n    def __init__(self, v: int) -> None:\n        self.z = v\n\n\nclass C(A, B):\n    pass\n\n\nc = C(5)\nprint(c.z)\n",
        "5\n",
    );
}

/// `super().__init__()` ranks constructors by the same rule and carried the
/// same defect: it resolved `A`'s implicit stub and left `B`'s slot unset.
#[test]
fn super_init_skips_an_implicit_constructor_on_an_earlier_base() {
    assert_runs(
        "super_init",
        "class A:\n    pass\n\n\nclass B:\n    def __init__(self) -> None:\n        self.z = 1\n\n\nclass C(A, B):\n    def __init__(self) -> None:\n        super().__init__()\n\n\nc = C()\nprint(c.z)\n",
        "1\n",
    );
}

/// Regression guard for the mandatory fallback pass at both `super()` sites:
/// when the *only* constructor above the current class is an implicit one,
/// `super().__init__()` must still resolve to it. Without the fallback this
/// program panics the compiler (`pycc_mir`) or is rejected `T0044`
/// (`pycc_types`).
#[test]
fn super_init_still_reaches_an_implicit_constructor_when_it_is_the_only_one() {
    assert_runs(
        "super_init_only_implicit",
        "class A:\n    pass\n\n\nclass C(A):\n    def __init__(self) -> None:\n        self.q = 4\n        super().__init__()\n\n\nc = C()\nprint(c.q)\n",
        "4\n",
    );
}

/// Regression guard for the fallback pass at the two *instantiation* sites:
/// an MRO whose every constructor is implicit still resolves and runs.
#[test]
fn an_all_implicit_mro_still_instantiates() {
    assert_runs(
        "all_implicit",
        "class A:\n    pass\n\n\nclass B:\n    pass\n\n\nclass C(A, B):\n    def hi(self) -> str:\n        return \"ok\"\n\n\nc = C()\nprint(c.hi())\n",
        "ok\n",
    );
}

/// The ranking keys on **provenance**, never on shape. An explicitly written
/// `def __init__(self) -> None: pass` lowers to exactly the same empty body
/// as the implicit constructor, but it is a real constructor and must keep
/// ranking first -- CPython raises `AttributeError` for this program, and
/// since #1388 so does pycc, with CPython's own message.
///
/// The exception *is* the discriminator: if any shape-based detection crept
/// in, `B.__init__` would run and this would print `1` instead.
#[test]
fn an_explicitly_written_empty_constructor_still_outranks_a_later_base() {
    assert_runtime_abort(
        "explicit_empty_init",
        "class A:\n    def __init__(self) -> None:\n        pass\n\n\nclass B:\n    def __init__(self) -> None:\n        self.z = 1\n\n\nclass C(A, B):\n    pass\n\n\nc = C()\nprint(c.z)\n",
        "AttributeError: 'C' object has no attribute 'z'",
    );
}

/// A `@dataclass` base's generated `__init__` is a *real* constructor: it
/// shares `synthesize_dataclass_init` with the implicit one but is never
/// flagged, so it must keep ranking first. Flagging inside that shared
/// helper rather than at `ensure_init`'s call site would silently break
/// this, turning `class C(SomeDataclass, B)` into a fresh soundness bug.
///
/// The arity is the discriminator: `C(7)` only type-checks if `A`'s
/// one-field dataclass constructor is the one resolved. `B` deliberately
/// declares no attributes, so this pins ranking alone and does not also
/// depend on the multi-base slot-aliasing limitation pinned below.
#[test]
fn a_dataclass_base_s_generated_constructor_still_ranks_first() {
    assert_runs(
        "dataclass_base",
        "from dataclasses import dataclass\n\n\n@dataclass\nclass A:\n    a: int\n\n\nclass B:\n    def __init__(self) -> None:\n        return\n\n\nclass C(A, B):\n    pass\n\n\nc = C(7)\nprint(c.a)\n",
        "7\n",
    );
}

/// The multi-base slot-aliasing limitation this file used to pin as *live*
/// behaviour is now rejected outright by #969's layout gate.
///
/// A class with two or more bases that each declare attribute slots hit a
/// pre-existing MRO **slot-aliasing** defect in `mro_attrs`: the derived
/// class's flat layout is assigned most-base-first, while each base's own
/// `__init__` addresses slots against that base's *own* layout. The failure
/// was not uniformly loud -- the slot the running constructor never wrote
/// aborted in `pycc_rt`, while the slot it aliased over returned a silently
/// wrong value where CPython raises `AttributeError`.
///
/// #969 rejects the shape at HIR lowering with `C0001` instead
/// ([D-234]), so both reads below are now compile errors. The behaviour is
/// pinned end to end in `tests/issue_969_mi_slot_layout.rs`; it is kept here
/// because #966's own constructor-ranking rules are stated against this
/// shape, and a future relaxation of the gate must revisit both files.
///
/// [D-234]: ../docs/decisions/D-234-reject-multiple-inheritance-whose-base-layouts-are.md
#[test]
fn the_multiple_slot_bearing_base_layout_is_now_rejected_by_the_969_gate() {
    const CLASSES: &str = "class B:\n    def __init__(self) -> None:\n        self.z = 1\n\n\nclass D:\n    def __init__(self) -> None:\n        self.w = 2\n\n\nclass C(B, D):\n    pass\n\n\nc = C()\n";

    assert_rejected(
        "alias_unwritten",
        &format!("{CLASSES}print(c.z)\n"),
        "C0001",
        "instance layout is not a prefix of",
    );

    assert_rejected(
        "alias_wrong_value",
        &format!("{CLASSES}print(c.w)\n"),
        "C0001",
        "instance layout is not a prefix of",
    );
}
