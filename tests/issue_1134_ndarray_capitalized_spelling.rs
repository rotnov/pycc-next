//! #1134: the bare name `NDArray` as a third source spelling of the `ext`
//! boundary's buffer carrier.
//!
//! The capitalized `numpy.typing` spelling is what 18 of the 19
//! array-parameter occurrences in the #1039 census actually write, so it is
//! the spelling a reader of that census meets first. It is registered on
//! exactly the terms `ndarray` was (#1129): one line in
//! `annotation_to_ty`'s `Expr::Name` arm, resolved *after* `class_defs` and
//! the alias table, lowering to `Ty::MemoryView` with no new `Ty` variant,
//! no new refusal arm and no new diagnostic family.
//!
//! **What this does not do, stated here so the file cannot be read as more
//! than it is:** at #1134 it made none of those 19 occurrences compile.
//! Every one of them also needed a name binding that did not exist yet —
//! `from numpy.typing import NDArray` was refused by the *foreign-import*
//! path, `import numpy as np` was #883, and an attribute-form base
//! `np.ndarray` is #889. Registering the name is necessary, not sufficient.
//! Since #1291 `import numpy as np` binds a CPython module object, and since
//! #1380 `from numpy.typing import NDArray` is admitted as a buffer-carrier
//! import, so those two bindings are no longer missing; the census itself
//! was not re-measured for either (its corpus is not available to the
//! change), so no count is claimed here.
//!
//! Scope, against the two predecessors. The run-time half — the widened
//! `PyObject_CheckBuffer` admission predicate and the four wrapper arms — is
//! #1129's and is keyed on the resolved `Ty::MemoryView`, not on any
//! spelling, so it is reached identically here and is not restated: its
//! hosted, interpreter-loading evidence lives in
//! `tests/issue_1129_ndarray_buffer_carrier.rs`. The *subscripted* form is
//! #1130's, admitted by the resolved type alone, and
//! `tests/issue_1130_subscripted_annotations.rs`'s
//! `every_way_of_naming_the_buffer_carrier_is_subscriptable` carries
//! `NDArray` as its witness that a further spelling needs no edit there.
//! What is left for this file is the registration itself, its precedence,
//! and the refusals it inherits — every one of which resolves on the
//! program before `plan_ext` probes the host toolchain, so no test here is
//! `#[ignore]`d and all of them run inside the coverage job.

use pycc_scratch::ScratchDir;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

/// Writes `source` as the entry module of a fresh scratch directory.
fn fixture(category: &str, source: &str) -> ScratchDir {
    let dir = ScratchDir::new(category).expect("scratch");
    std::fs::write(dir.join("nd_probe.py"), source).expect("write the subject");
    dir
}

/// Runs `pycc check` on the fixture, returning success and the two streams
/// concatenated.
///
/// Both streams are read rather than guessing: `check` renders diagnostics
/// on stdout where `build` renders on stderr.
fn check(source: &str, category: &str) -> (bool, String) {
    let dir = fixture(category, source);
    let out = pycc()
        .arg("check")
        .arg(dir.join("nd_probe.py"))
        .output()
        .expect("pycc should spawn");
    (
        out.status.success(),
        format!("{}{}", stdout_of(&out), stderr_of(&out)),
    )
}

/// The subject: the sweep loop the carrier has always admitted, at a
/// parameter spelled `NDArray`.
const SUBJECT: &str = "\
def total(b: NDArray) -> float:
    s: float = 0.0
    i: int = 0
    for i in range(len(b)):
        s = s + b[i]
    return s
";

/// An `NDArray` parameter type-checks with no interpreter, no numpy and no
/// import.
///
/// `import numpy` builds only from a `pycc.lock` closure (#1242), so a
/// spelling gated on an import would need numpy installed; the bare name
/// is recognized
/// without one, exactly as `Any`, `Annotated`, `TypeAlias`, `Self` and
/// `ndarray` are (D-244's #1129 amendment, statement (c)).
///
/// The acceptance is proved by the *use* rather than by the exit code: the
/// body reads `len(b)` and `b[i]` as a native `float` load, which only the
/// buffer carrier supports, so an accept caused by resolving to something
/// else could not have type-checked this body.
#[test]
fn an_ndarray_capitalized_parameter_type_checks_without_any_import() {
    let (ok, text) = check(SUBJECT, "1134_check");
    assert!(ok, "{text}");
}

/// It is the *carrier* it lowers to, not merely something that compiled.
///
/// The element read yields `float` and nothing else does, so the control
/// arm — the same annotation with an `int` return — must be the `T0022`
/// mismatch. Without the control, a spelling that resolved to some other
/// nameable type would pass the acceptance above.
#[test]
fn the_capitalized_spelling_lowers_to_the_buffer_carrier_itself() {
    let (ok, text) = check(
        "def f(a: NDArray) -> float:\n    return a[0]\n",
        "1134_element",
    );
    assert!(ok, "{text}");

    let (ok, text) = check(
        "def f(a: NDArray) -> int:\n    return a[0]\n",
        "1134_element_control",
    );
    assert!(!ok, "{text}");
    assert!(text.contains("error[T0022]"), "{text}");
    assert!(
        text.contains("return type mismatch: expected `int`, found `float`"),
        "{text}"
    );
}

/// The exact program `tests/issue_1130_subscripted_annotations.rs` used to
/// pin as its undefined-base `C0001` is now an accept.
///
/// That file's regression guard was written on the #1130 issue title's own
/// example, a bare unbound `NDArray[int]`, which this registration flips
/// from refusal to acceptance. The guard is restated there on a name
/// nothing binds; the flip itself is stated here, verbatim, so it is
/// asserted somewhere rather than merely removed. #1130 supplies the
/// subscript admission and this issue supplies the name -- neither alone
/// accepts this program.
#[test]
fn the_1130_undefined_base_reproducer_is_now_an_accept() {
    let (ok, text) = check(
        "def f(a: NDArray[int]) -> float:\n    return 0.0\n",
        "1134_flip_from_1130_guard",
    );
    assert!(ok, "{text}");
}

/// Diagnostics render the canonical `memoryview` for the new spelling.
///
/// D-244 statement (b)'s operative consequence: the spellings are one pycc
/// type, so a message that *renders a resolved type* prints the compiler's
/// canonical name for it regardless of how the user spelled the parameter —
/// exactly as it already does for `type Arr = memoryview`. `T0021` is the
/// cheapest such message to reach, and reaching it at all is itself
/// evidence: only `Ty::MemoryView` has an index rule to violate.
///
/// The `ndarray` arm rides along so the two ordinary-identifier spellings
/// are shown to agree, which a per-spelling `Ty` variant would have broken.
#[test]
fn a_rendered_type_prints_the_canonical_spelling_for_every_source_spelling() {
    for (category, spelling) in [
        ("1134_render_capitalized", "NDArray"),
        ("1134_render_lowercase", "ndarray"),
        ("1134_render_canonical", "memoryview"),
    ] {
        let (ok, text) = check(
            &format!("def f(a: {spelling}) -> float:\n    return a[\"x\"]\n"),
            category,
        );
        assert!(!ok, "{text}");
        assert!(text.contains("error[T0021]"), "{text}");
        assert!(text.contains("`memoryview` index must be `int`"), "{text}");
    }
}

/// A program that binds `NDArray` itself keeps its own meaning for the name.
///
/// This is D-244's #1129 review-round-3 amendment, statement (h), restated
/// for the third spelling: `NDArray` is an ordinary identifier, not a
/// Python builtin or a `typing` name, so it is resolved *after* `class_defs`
/// and the alias table rather than beside `memoryview` in the keyword list.
/// Reserving it ahead of them would make a module-level `class NDArray` or
/// `type NDArray = ...` mean something Python does not — a local definition
/// shadows an imported name, not the other way round — and would refuse
/// programs that compiled before the spelling existed.
///
/// Both arms are proved by a *use* the carrier could not support: a method
/// call on the class instance, and `+ 1` on the alias-to-`int`. An arm
/// asserting only the exit code would pass even if the carrier had won and
/// the body had been ignored.
///
/// `tests/issue_1130_subscripted_annotations.rs` already carries the
/// subscripted counterpart for the class-binding arm, through the `NDArray`
/// class fixtures its own tests are written on.
#[test]
fn a_program_that_binds_ndarray_capitalized_itself_keeps_its_own_meaning() {
    let (ok, text) = check(
        "class NDArray:\n    def m(self) -> int:\n        return 1\n\n\ndef g(a: NDArray) -> int:\n    return a.m()\n",
        "1134_class_shadow",
    );
    assert!(
        ok,
        "a user-defined `class NDArray` must win over the buffer spelling: {text}"
    );

    let (ok, text) = check(
        "type NDArray = int\n\n\ndef g(a: NDArray) -> int:\n    return a + 1\n",
        "1134_alias_shadow",
    );
    assert!(
        ok,
        "a user-defined `type NDArray` alias must win over the buffer spelling: {text}"
    );
}

/// `memoryview`'s precedence is untouched by the addition.
///
/// The contrast that makes the arm above a statement about *ordinary
/// identifiers* rather than about the carrier in general: `memoryview` is a
/// reserved name the `Expr::Name` arm answers before either table, so the
/// same alias shape does **not** win there and the bare name still lowers
/// to the carrier. Adding a third ordinary-identifier spelling must not
/// have moved that line.
#[test]
fn the_reserved_spelling_keeps_its_precedence() {
    let (ok, text) = check(
        "type memoryview = int\n\n\ndef g(a: memoryview) -> int:\n    return a + 1\n",
        "1134_reserved_precedence",
    );
    assert!(!ok, "{text}");
    assert!(text.contains("error[C0001]"), "{text}");
}

/// Every position that is *not* an `--ext` parameter is refused for the new
/// spelling exactly as it is for the other two.
///
/// The half of the change that is deliberately not a widening: `NDArray`
/// lowers to `Ty::MemoryView`, so it inherits every existing refusal rather
/// than opening a third, laxer path to them. The two message styles are
/// #1129's documented split — a message naming the *type* stays
/// spelling-neutral ("a buffer"), and a message quoting a whole *signature
/// position* back renders the canonical `memoryview`.
#[test]
fn every_non_parameter_ndarray_capitalized_position_is_refused() {
    // A bare declaration: `C0001` in both modes, because nothing produces a
    // buffer value to bind to the name. Spelling-neutral message.
    let dir = fixture(
        "1134_decl",
        "def f() -> int:\n    x: NDArray\n    return 1\n",
    );
    let native = pycc()
        .arg("build")
        .arg(dir.join("nd_probe.py"))
        .arg("-o")
        .arg(dir.join("nd_probe"))
        .output()
        .expect("pycc should spawn");
    assert!(!native.status.success(), "{}", stdout_of(&native));
    let err = stderr_of(&native);
    assert!(err.contains("error[C0001]"), "{err}");
    assert!(err.contains("declaring `x` as a buffer"), "{err}");

    // A public `-> NDArray` return under `--ext` is *admitted* since Part 2b
    // of #1142 (#1164). What refuses this program is its body: an
    // intra-artifact call to a buffer-returning function has no wrapper to
    // own the result. Two-directional, so a regression that reinstates the
    // old `C0003` gap cannot pass as "still refused".
    let dir = fixture("1134_return", "def make() -> NDArray:\n    return make()\n");
    let ext = pycc()
        .arg("build")
        .arg(dir.join("nd_probe.py"))
        .arg("-o")
        .arg(dir.join("nd_probe"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(!ext.status.success(), "{}", stdout_of(&ext));
    let err = stderr_of(&ext);
    assert!(err.contains("error[C0001]"), "{err}");
    assert!(
        err.contains("calling `make`, whose return type is a buffer"),
        "{err}"
    );
    assert!(!err.contains("error[C0003]"), "{err}");

    // The `C0003` remediation still enumerates what the boundary does carry,
    // so it still has to name this spelling — the one the census says a
    // reader is most likely to have written. Shown a list without it, they
    // read the list as "not that type at all". Reached through an
    // uncarriable *parameter* now that the return position is carried.
    let dir = fixture(
        "1134_gap_list",
        "def take(v: list[int]) -> int:\n    return 0\n",
    );
    let ext = pycc()
        .arg("build")
        .arg(dir.join("nd_probe.py"))
        .arg("-o")
        .arg(dir.join("nd_probe"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(!ext.status.success(), "{}", stdout_of(&ext));
    let err = stderr_of(&ext);
    assert!(err.contains("error[C0003]"), "{err}");
    assert!(
        err.contains("(or its other spellings `ndarray` and `NDArray`)"),
        "{err}"
    );

    // An `NDArray` *signature* in a build without `--ext`: `I0405`, naming
    // the parameter in the canonical spelling.
    let dir = fixture("1134_native", SUBJECT);
    let native = pycc()
        .arg("build")
        .arg(dir.join("nd_probe.py"))
        .arg("-o")
        .arg(dir.join("nd_probe"))
        .output()
        .expect("pycc should spawn");
    assert!(!native.status.success(), "{}", stdout_of(&native));
    let err = stderr_of(&native);
    assert!(err.contains("error[I0405]"), "{err}");
    assert!(err.contains("parameter `b: memoryview`"), "{err}");
}

/// #1291 admits an undotted `import X as Y` of a CPython module on the terms
/// of a plain `import X`, so the aliased binding the census's `np.`
/// occurrences need no longer stops the check. This is the flip the #1134
/// amendment on D-244 anticipates, pinned as an acceptance.
#[test]
fn import_numpy_as_np_binds_a_cpython_module_since_1291() {
    let (ok, text) = check(
        "import numpy as np\n\n\ndef f(a: NDArray) -> float:\n    return a[0]\n",
        "1291_import_as",
    );
    assert!(ok, "`import numpy as np` should check cleanly: {text}");
}

/// `pycc explain I0405` names the third spelling.
///
/// D-244's #1129 review-rounds-6-and-7 amendment, statement (j): `explain`
/// is a second normative surface for the code, and a user who reached
/// `I0405` by writing `NDArray` is otherwise told only about a type they
/// never wrote.
#[test]
fn explain_i0405_names_the_capitalized_spelling() {
    let out = pycc()
        .arg("explain")
        .arg("I0405")
        .output()
        .expect("pycc should spawn");
    let text = format!("{}{}", stdout_of(&out), stderr_of(&out));
    assert!(out.status.success(), "{text}");
    assert!(text.contains("NDArray"), "{text}");
}

/// The gaps this issue does *not* close, pinned so the disclosure above is
/// checked rather than merely asserted in prose.
///
/// Of the three bindings the census needs, `import numpy as np` compiles
/// since #1291 (pinned by `import_numpy_as_np_binds_a_cpython_module_since_1291`)
/// and `from numpy.typing import NDArray` checks cleanly since #1380, which
/// admits it as a buffer-carrier import (pinned by
/// `the_census_from_import_checks_cleanly_since_1380` below and owned by
/// `tests/issue_1380_buffer_carrier_from_import.rs`). The attribute-form base
/// is still refused, and its arm pins the *cause* and not only the exit
/// status: a refusal that migrated to a different reason would leave the
/// module doc's claim true by accident and unmeasured. The census itself was
/// not re-measured for #1380 (its corpus is not available to the change), so
/// no count of compiling occurrences is claimed.
#[test]
fn registering_the_name_does_not_make_the_census_bindings_compile() {
    // An attribute-form annotation base, #889.
    let (ok, text) = check(
        "def f(a: np.ndarray) -> float:\n    return a[0]\n",
        "1134_gap_attribute_base",
    );
    assert!(!ok, "the attribute base unexpectedly compiles: {text}");
    assert!(text.contains("error[C0001]"), "{text}");
    assert!(
        text.contains("only a bare name type annotation is supported so far"),
        "{text}"
    );
}

/// #1380 admits the census's own from-import as a buffer-carrier import, so
/// the binding no longer stops the check.
#[test]
fn the_census_from_import_checks_cleanly_since_1380() {
    let (ok, text) = check(
        "from numpy.typing import NDArray\n\n\ndef f(a: NDArray) -> float:\n    return a[0]\n",
        "1380_from_import",
    );
    assert!(
        ok,
        "`from numpy.typing import NDArray` should check cleanly: {text}"
    );
}
