//! Part 1 of #1027 (#1112): `memoryview` as a pycc type and an `ext`
//! boundary carrier, exercised end to end.
//!
//! The refusal *shapes* -- one per arm of the shim's
//! `pycc_ext_unpack_memoryview`, plus the two buffer-release probes -- are
//! owned by `tests/issue_1067_neg004_ext_conformance.rs`, which is where
//! every other D-244 rule 7 refusal is already pinned against a real
//! interpreter. The closed-set guard that a new carrier cannot be added
//! without a refusal arm is owned by
//! `src/ext_build_tests/refusal_completeness.rs`, and the generated C is
//! pinned by `src/ext_build_tests/generated_c.rs`. What is left, and what
//! this file owns, is the *mode* half of Part 1's statement: the same
//! annotation is admitted by `pycc build --ext` and refused by a native
//! `pycc build`, at both positions it can appear in.
//!
//! Neither mode refusal needs CPython development headers, because both are
//! decided in the frontend before the toolchain is probed, so the two
//! refusal arms below run everywhere. Only the arm that actually builds and
//! loads an artifact is `#[ignore]`d, for the reason every `ext` test is.
//! None of the three contributes line coverage: CI's coverage job runs
//! `llvm-cov` without `--include-ignored` and `scripts/check_diff_coverage.py`
//! excludes `tests/` from its denominator either way.

use pycc_scratch::ScratchDir;
use std::path::Path;
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
    std::fs::write(dir.join("view_probe.py"), source).expect("write the subject");
    dir
}

/// Builds the entry module as a CPython extension module directly into
/// `dir`, so a CPython run with `dir` as its working directory imports it.
///
/// The output path carries no extension suffix, exactly as
/// `tests/issue_1084_loop_shape.rs`'s own helper writes it: `pycc build
/// --ext` appends the one its target triple calls for and derives the
/// exported `PyInit_<mod>` name from the path's own spelling, so spelling
/// `.abi3.so` here builds on Unix and then fails the `--ext` name contract
/// on the required windows-latest Tier-1 leg.
fn build_ext(dir: &Path) -> Output {
    pycc()
        .arg("build")
        .arg(dir.join("view_probe.py"))
        .arg("-o")
        .arg(dir.join("view_probe"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn")
}

/// The subject: a `memoryview` at the one position Part 1 admits it, next
/// to an ordinary scalar export so the artifact also proves the carrier
/// changed nothing for every signature that predates it.
///
/// The body does not *read* `v`. Part 1's section 3.5 admits no operation
/// on the value -- a `memoryview` has no literal and no producing
/// expression, so refusing the read refuses aliasing, reassignment,
/// container stores and onward calls all at once -- and
/// `crates/pycc_types/src/tests.rs`'s
/// `reading_a_memoryview_parameter_is_a_capability_gap_rather_than_an_ice`
/// pins that refusal. Indexing the buffer is Part 2 of #1027.
const SUBJECT: &str = "\
def take_view(v: memoryview) -> int:
    return 11


def plain(x: int) -> int:
    return x + 1
";

/// A native `pycc build` refuses a `memoryview` *parameter* with `I0405`.
///
/// `pycc check` is deliberately not the command under test: like the
/// `I0403` foreign-import gate it sits beside, this refusal lives in
/// `src/frontend.rs`'s `resolve_frontend_native`, which only a native
/// `pycc build` reaches -- `check` is artifact-mode agnostic and stays so.
#[test]
fn a_memoryview_parameter_is_refused_by_a_native_build() {
    let dir = fixture("1112_native_param", SUBJECT);
    let build = pycc()
        .arg("build")
        .arg(dir.join("view_probe.py"))
        .arg("-o")
        .arg(dir.join("view_probe"))
        .output()
        .expect("pycc should spawn");
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[I0405]"), "{err}");
    assert!(
        err.contains("`take_view`'s parameter `v: memoryview`"),
        "{err}"
    );
    assert!(err.contains("pycc build --ext"), "{err}");
}

/// Round 5 of the pinned review: the signature refusal survives a body
/// that *reads* the parameter.
///
/// `crates/pycc_types`' `reject_memoryview_read` refuses every use of a
/// `memoryview`-typed name with `C0001`, and the type check used to run
/// before `resolve_frontend_native` reported its artifact gates -- so a
/// native build of a function that merely assigned its own parameter got
/// the read-side `C0001` and never the `I0405` that `docs/RUNTIME.md` and
/// the D-244 amendment promise for a `memoryview` in a signature in any
/// build without `--ext`. The gate is now reported before the type check,
/// and this pins that the documented code is what a user actually sees.
#[test]
fn a_memoryview_parameter_is_refused_with_i0405_even_when_the_body_reads_it() {
    const READS_THE_VIEW: &str = "\
def total(v: memoryview) -> int:
    w = v
    return 0
";
    let dir = fixture("1112_native_param_read", READS_THE_VIEW);
    let build = pycc()
        .arg("build")
        .arg(dir.join("view_probe.py"))
        .arg("-o")
        .arg(dir.join("view_probe"))
        .output()
        .expect("pycc should spawn");
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[I0405]"), "{err}");
    assert!(err.contains("`total`'s parameter `v: memoryview`"), "{err}");
    // The read refusal must not be what reaches the user instead: it
    // describes a use, not the signature the artifact mode refuses.
    assert!(!err.contains("error[C0001]"), "{err}");
}

/// Prioritizing the `memoryview` gate must not swallow the `I0403` it
/// shares a call site with. A program carrying both a foreign import and a
/// `memoryview` signature reports both, exactly as it did while the two
/// gates were reported together after the type check.
#[test]
fn a_foreign_import_is_still_reported_alongside_the_memoryview_refusal() {
    const BOTH: &str = "\
import tkinter


def total(v: memoryview) -> int:
    return 0
";
    let dir = fixture("1112_native_import_and_view", BOTH);
    let build = pycc()
        .arg("build")
        .arg(dir.join("view_probe.py"))
        .arg("-o")
        .arg(dir.join("view_probe"))
        .output()
        .expect("pycc should spawn");
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[I0403]"), "{err}");
    assert!(err.contains("error[I0405]"), "{err}");
}

/// The other position, and the other mode: a `-> memoryview` return is
/// refused in *both*. Natively it is the same `I0405`; under `--ext` it is
/// `C0003`, because `BoundaryCarrier::into_scalar` answers `None` for the
/// buffer carrier and the export boundary has no way to hand a borrowed
/// view back to a host that did not lend it.
#[test]
fn a_memoryview_return_type_is_refused_in_both_modes() {
    const RETURNS_A_VIEW: &str = "\
def make() -> memoryview:
    return make()
";
    let dir = fixture("1112_return", RETURNS_A_VIEW);
    let native = pycc()
        .arg("build")
        .arg(dir.join("view_probe.py"))
        .arg("-o")
        .arg(dir.join("view_probe"))
        .output()
        .expect("pycc should spawn");
    assert!(!native.status.success(), "{}", stdout_of(&native));
    assert!(
        stderr_of(&native).contains("error[I0405]"),
        "{}",
        stderr_of(&native)
    );

    // No development headers are needed for this arm either: `plan_ext`
    // resolves the capability gap on the program itself before it probes
    // the host toolchain, which is exactly the ordering `run_build`
    // documents.
    let ext = build_ext(&dir);
    assert!(!ext.status.success(), "{}", stdout_of(&ext));
    let err = stderr_of(&ext);
    // Part 2b of #1142 (#1164) admits the *signature*: a public module-level
    // `-> memoryview` export is now carried across the boundary, so the
    // `C0003` capability gap that used to answer this program is gone. What
    // still refuses it is its body -- `return make()` is an intra-artifact
    // call to a buffer-returning function, which has no wrapper to own the
    // result. Asserted two-directionally so a regression that reinstates the
    // old gap cannot pass as "still refused".
    assert!(err.contains("error[C0001]"), "{err}");
    assert!(
        err.contains("calling `make`, whose return type is a buffer"),
        "{err}"
    );
    assert!(!err.contains("error[C0003]"), "{err}");
    assert!(!err.contains("its return type `-> memoryview`"), "{err}");
}

/// The shape #1164 opens, refused everywhere it is *not* an export. The
/// arm does not call the producer, so what answers is
/// `src/memoryview_mode.rs`'s own gap rather than the intra-artifact call
/// refusal above -- which is the point: the two refusals bound different
/// halves of the boundary and must stay distinguishable.
///
/// #1174 narrowed this test. It carried four arms while the exemption was
/// *exported and module-level*: a private module-level `def` plus a
/// `@staticmethod`, a `@classmethod` and an instance method of a **public**
/// class, all three refused purely for being methods. Those three are
/// admissions now, and `tests/issue_1174_method_buffer_return.rs` asserts
/// them -- together with the arms that still refuse a method (a private
/// method, a method of a private class, an exception class, an unreachable
/// instance method, a `@property` getter) and the `.static` / `.classmethod`
/// mangled-suffix assertions this loop used to carry, which need a
/// method-bearing subject to mean anything.
#[test]
fn a_buffer_producer_outside_the_export_set_is_refused_in_ext_mode() {
    const CASES: [(&str, &str, &str); 1] = [(
        "1112_private_producer",
        "def _make() -> memoryview:
    a = ndarray(4)
    return a


def total() -> int:
    return 0
",
        "`_make`'s return type is a buffer",
    )];
    for (category, source, expected) in CASES {
        let dir = fixture(category, source);
        let ext = build_ext(&dir);
        assert!(!ext.status.success(), "{}", stdout_of(&ext));
        let err = stderr_of(&ext);
        assert!(!err.contains("panicked"), "{err}");
        assert!(err.contains("error[C0001]"), "{err}");
        assert!(err.contains(expected), "{category}: {err}");
        // The remediation still names the admitted module-level shape, which
        // is the one this arm's subject must move to.
        assert!(
            err.contains("move the buffer-producing code into a public module-level `def`"),
            "{category}: {err}"
        );
        assert!(!err.contains("error[C0003]"), "{category}: {err}");
    }
}

/// The same return type on a *private* function, which `collect_exports`
/// skips: before the round-4 fix nothing refused it, and `--ext` lowered
/// the call's result into `pycc_codegen`'s "a `memoryview`-typed call
/// result is not supported yet" panic -- a compiler crash on valid Python.
/// Native mode was never affected, because `refuse_in_native_mode` walks
/// private functions too; both halves are asserted here so a later fix
/// cannot silently replace the documented `I0405` with the new `C0001`.
#[test]
fn a_private_memoryview_return_type_is_refused_rather_than_crashing_the_compiler() {
    const PRIVATE_VIEW: &str = "def _make() -> memoryview:
    return _make()


def total() -> int:
    _make()
    return 0
";
    let dir = fixture("1112_private_return", PRIVATE_VIEW);
    let ext = build_ext(&dir);
    assert!(!ext.status.success(), "{}", stdout_of(&ext));
    let err = stderr_of(&ext);
    assert!(!err.contains("panicked"), "{err}");
    assert!(err.contains("error[C0001]"), "{err}");
    // Part 2b of #1142 (#1164): the body's own `return _make()` is now what
    // refuses this program, ahead of the export-set gap, because an
    // intra-artifact call to a buffer-returning function has no wrapper to
    // own the result. The property the round-4 fix pinned is unchanged and
    // is what this test still exists for: a refusal rather than a panic.
    assert!(
        err.contains("calling `_make`, whose return type is a buffer"),
        "{err}"
    );
    // Part 2a of #1142 (#1165) gave the buffer type a second provenance:
    // `ndarray(n)` *is* an expression that produces one, so no refusal here
    // may regress to the claim that nothing can produce a buffer to return.
    assert!(
        !err.contains("no expression produces one to return"),
        "{err}"
    );

    let native = pycc()
        .arg("build")
        .arg(dir.join("view_probe.py"))
        .arg("-o")
        .arg(dir.join("view_probe"))
        .output()
        .expect("pycc should spawn");
    assert!(!native.status.success(), "{}", stdout_of(&native));
    assert!(
        stderr_of(&native).contains("error[I0405]"),
        "{}",
        stderr_of(&native)
    );
}

/// Every position that is *not* a signature, refused end to end.
///
/// Round 2 of the pinned review: `annotation_to_ty` is one parser for every
/// annotation position, so admitting `Ty::MemoryView` for a signature
/// admitted it everywhere at once. Three of the five non-signature
/// positions were already closed by gates that predate this branch -- a
/// class attribute and a dataclass field are both restricted to a scalar
/// slot type (since #1266 a value-less class-body annotation is an instance
/// attribute declaration, whose own type gate refuses `memoryview` the same
/// way) -- so what this pins is the two that were not: the bare
/// declaration (`crates/pycc_types`' `reject_memoryview_declaration`, at
/// module scope and in a function body alike) and the protocol attribute
/// (`crates/pycc_hir`'s own D-228 arm in `class/protocol.rs`). All of them
/// are `C0001`, in both artifact modes, because Part 1 adds no expression
/// that produces a `memoryview` value to bind to any of these names.
#[test]
fn every_non_signature_memoryview_position_is_refused() {
    const CASES: [(&str, &str, &str); 5] = [
        (
            "1112_decl_module",
            "y: memoryview

def f() -> int:
    return 1
",
            "declaring `y` as a buffer",
        ),
        (
            "1112_decl_local",
            "def f() -> int:
    x: memoryview
    return 1
",
            "declaring `x` as a buffer",
        ),
        (
            "1112_protocol_attr",
            "from typing import Protocol


class P(Protocol):
    x: memoryview


def f() -> int:
    return 1
",
            "protocol attribute `P.x` has a buffer type",
        ),
        (
            "1112_class_attr",
            "class C:
    x: memoryview

    def __init__(self) -> None:
        pass


def f() -> int:
    return 1
",
            "instance attribute `x` declared in class `C` has type `memoryview`",
        ),
        (
            "1112_dataclass_field",
            "from dataclasses import dataclass


@dataclass
class D:
    x: memoryview


def f() -> int:
    return 1
",
            "dataclass field `x` has type `memoryview`",
        ),
    ];
    for (name, source, expected) in CASES {
        let dir = fixture(name, source);
        let build = pycc()
            .arg("build")
            .arg(dir.join("view_probe.py"))
            .arg("-o")
            .arg(dir.join("view_probe"))
            .output()
            .expect("pycc should spawn");
        assert!(!build.status.success(), "{name}: {}", stdout_of(&build));
        let err = stderr_of(&build);
        assert!(err.contains("error[C0001]"), "{name}: {err}");
        assert!(err.contains(expected), "{name}: {err}");
    }
}

/// Every *read* of the parameter, through both seams that reach a binding.
///
/// "Every read" means every read Part 2 of #1027 (#1113) did *not* admit:
/// the one element load `v[i]` is now a real capability and is pinned by
/// `tests/issue_1113_ext_buffer_index.rs` instead.
///
/// Round 3 of the pinned review: `HirStmt::ForList` and a comprehension's
/// `CompIter::Name` keep their iterable as a plain `String` rather than a `HirExpr::Name`
/// (D-105's HIR shape), so `for x in v` resolves through
/// `lookup_bound_name` and never reaches `infer_expr_in`'s own `Name` arm.
/// The program was still refused, but as `T0033` -- "`memoryview` cannot be
/// iterated" -- which is false about Python and mislabels a capability gap
/// as a type error. Both seams now answer with the one `C0001`.
#[test]
fn every_read_of_a_memoryview_parameter_is_the_same_capability_gap() {
    const CASES: [(&str, &str); 6] = [
        (
            // Round 8 of the pinned review: an operation that inspects its
            // argument's concrete term reported its own type error about a
            // `memoryview` (`len` its `T0033`) before the capability gap
            // could fire. The refusal sits at the solver's shared `Name`
            // read seam, so these arms are one fix, not several.
            //
            // #1116 admitted `len(v)`, so the arm that used to spell it
            // moves to an attribute read -- the same substitution #1113
            // forced on the subscript arm below, and for the same reason:
            // the arm's subject is the shared seam, not the operation that
            // happens to reach it.
            "1112_read_attribute",
            "def total(v: memoryview) -> int:
    return v.nbytes
",
        ),
        (
            // Part 2 of #1027 (#1113) admitted `v[0]` as a *load* and
            // Part 1 of #1142 admitted `v[0] = 1.0` as a store, so neither
            // subscript form this arm has spelled is a capability gap any
            // more. A truth test reads the name and still is -- and it is
            // deliberately not the aliasing shape the `1112_read_alias`
            // arm below already pins, so the table keeps six distinct
            // witnesses of the one seam rather than five and a duplicate.
            "1112_read_truth_test",
            "def total(v: memoryview) -> int:
    if v:
        return 1
    return 0
",
        ),
        (
            "1112_read_argument",
            "def total(v: memoryview) -> int:
    return int(v)
",
        ),
        (
            // Round 7 of the pinned review: a call target is a read of the
            // name too. The gate this one reaches is the solver's D-110
            // mirror in `crates/pycc_types/src/constraints.rs`, not
            // `infer_expr_in`'s own arm -- the solver runs first and a
            // parameter's annotation is already a binding there -- so
            // without the refusal at *that* site the call was reported as
            // the generic `T0021` non-callable binding instead of the
            // capability gap every other read of `v` is.
            "1112_read_call",
            "def total(v: memoryview) -> int:
    return v()
",
        ),
        (
            "1112_read_alias",
            "def total(v: memoryview) -> int:
    w = v
    return 0
",
        ),
        (
            "1112_read_for",
            "def total(v: memoryview) -> int:
    s = 0
    for x in v:
        s = s + 1
    return s
",
        ),
    ];
    for (name, source) in CASES {
        let dir = fixture(name, source);
        let build = build_ext(&dir);
        assert!(!build.status.success(), "{name}: {}", stdout_of(&build));
        let err = stderr_of(&build);
        assert!(err.contains("error[C0001]"), "{name}: {err}");
        assert!(
            err.contains("using `v`, which is bound to a buffer parameter"),
            "{name}: {err}"
        );
    }
}

/// The hosted arm: the same annotation the two arms above refuse builds as
/// an extension module, and the host calls it with a real `memoryview`.
///
/// The release assertion is the one this file adds that no refusal shape
/// can make. `bytearray.append` raises `BufferError` while any exporter
/// holds a view of it, so an append that *succeeds* after the exported
/// call returned and the host's own view was released proves the wrapper
/// released the `Py_buffer` it acquired on the success path -- not only on
/// the bail paths, where a leak would be invisible to a returning call.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_memoryview_export_builds_and_releases_its_buffer_in_the_host() {
    let dir = fixture("1112_hosted", SUBJECT);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(
            "import view_probe\n\
             store = bytearray(24)\n\
             view = memoryview(store).cast('d')\n\
             assert view_probe.take_view(view) == 11\n\
             assert view_probe.plain(41) == 42\n\
             view.release()\n\
             store.append(0)\n\
             print('ok')\n",
        )
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}

/// Round 6 of the pinned review: a `Protocol` method's signature is not an
/// `HirItem::Function`.
///
/// `crates/pycc_hir/src/class/protocol.rs` records a protocol method as a
/// `ProtocolMember::Method` and lowers no function for it, so both
/// `src/memoryview_mode.rs` gates -- which walk `hir.items` -- saw nothing
/// at all and a protocol method naming `memoryview` escaped every refusal
/// this file pins for an ordinary one. The four assertions below are the
/// four arms that split apart, and the split is the fix's whole shape:
/// the *parameter* position is mode-dependent, so it is refused by the
/// native gate only, while the *return* position is unsatisfiable in every
/// mode and is refused at the declaration.
fn protocol_build(dir: &Path, ext: bool) -> Output {
    let mut cmd = pycc();
    cmd.arg("build")
        .arg(dir.join("view_probe.py"))
        .arg("-o")
        .arg(dir.join("view_probe"));
    if ext {
        cmd.arg("--ext");
    }
    cmd.output().expect("pycc should spawn")
}

const PROTOCOL_PARAM: &str = "\
from typing import Protocol


class Sink(Protocol):
    def total(self, v: memoryview) -> int: ...


def plain(x: int) -> int:
    return x + 1
";

#[test]
fn a_protocol_method_s_memoryview_parameter_is_refused_by_a_native_build() {
    let dir = fixture("1112_protocol_param_native", PROTOCOL_PARAM);
    let build = protocol_build(&dir, false);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[I0405]"), "{err}");
    assert!(
        err.contains("`Sink.total`'s parameter 1 `memoryview`"),
        "{err}"
    );
    assert!(err.contains("pycc build --ext"), "{err}");
}

/// The mirror assertion, and the reason the parameter arm is *not* a
/// mode-agnostic refusal: a class really can satisfy the member under
/// `--ext`, because an exported function receives the view and passes it
/// inward. Pinned so a later round does not "fix" this into a refusal.
///
/// The assertion is that no *refusal* is reported, not that the build
/// succeeds. What this test owns is a frontend decision -- both memoryview
/// gates run before the toolchain is probed -- and asserting success would
/// silently make it a CPython-headers test: the required legs run with a
/// 3.9 interpreter, where `--ext` stops at the `Py_LIMITED_API` version
/// check long after the gates have had their say. The arm that really
/// builds and loads an `--ext` artifact is the `#[ignore]`d hosted test at
/// the end of this file, as it is for every other `ext` assertion here.
#[test]
fn a_protocol_method_s_memoryview_parameter_is_not_refused_by_an_ext_build() {
    let dir = fixture("1112_protocol_param_ext", PROTOCOL_PARAM);
    let build = protocol_build(&dir, true);
    let err = stderr_of(&build);
    assert!(!err.contains("error[I0405]"), "{err}");
    assert!(!err.contains("error[C0001]"), "{err}");
    assert!(!err.contains("Sink.total"), "{err}");
}

const PROTOCOL_RETURN: &str = "\
from typing import Protocol


class Source(Protocol):
    def make(self) -> memoryview: ...


def plain(x: int) -> int:
    return x + 1
";

#[test]
fn a_protocol_method_s_memoryview_return_is_refused_in_both_modes() {
    for (name, ext) in [
        ("1112_protocol_ret_native", false),
        ("1112_protocol_ret_ext", true),
    ] {
        let dir = fixture(name, PROTOCOL_RETURN);
        let build = protocol_build(&dir, ext);
        assert!(!build.status.success(), "{name}: {}", stdout_of(&build));
        let err = stderr_of(&build);
        assert!(err.contains("error[C0001]"), "{name}: {err}");
        assert!(
            err.contains("protocol method `Source.make` returns a buffer"),
            "{name}: {err}"
        );
    }
}

/// `lower_protocol_class` copies a base protocol's members into a derived
/// protocol's own `protocol_members` (a class whose base is a protocol is
/// itself lowered as one), so a walk over the assembled vector would report
/// the same declaration twice -- once naming the base, once naming the
/// derived class that merely inherits it. Exactly one diagnostic, naming
/// the declaring class.
#[test]
fn an_inherited_protocol_method_is_reported_once_at_its_declaration() {
    let dir = fixture(
        "1112_protocol_inherited",
        "\
from typing import Protocol


class Sink(Protocol):
    def total(self, v: memoryview) -> int: ...


class Counted(Sink):
    def count(self) -> int: ...


def plain(x: int) -> int:
    return x + 1
",
    );
    let build = protocol_build(&dir, false);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert_eq!(err.matches("error[I0405]").count(), 1, "{err}");
    assert!(
        err.contains("`Sink.total`'s parameter 1 `memoryview`"),
        "{err}"
    );
    assert!(!err.contains("`Counted.total`"), "{err}");
}

/// The keying half: `hir.class_defs` is its own concatenated table, so a
/// protocol class defined in an *imported* module is contributed by that
/// module and the refusal must be grouped under that file, not the entry.
/// Attributing it to the entry file is the defect Part 1 of #1026 fixed for
/// the import table, one table over.
///
/// The assertion is an *ordering* one because the diagnostic is span-less
/// (`ProtocolMember::Method` carries no source range), and
/// `pycc_diag::render_human` prints a ` --> path:line:col` line only for a
/// diagnostic that has a span. What the file key still decides is which
/// per-file group the refusal lands in, and `src/frontend.rs`'s `group`
/// emits those groups in file order -- dependency first, entry last. So an
/// `I0403` planted in the *entry* module pins the attribution exactly: the
/// protocol refusal precedes it when the class is keyed to `sink.py`, and
/// would follow it if the class fell back to the entry file, because within
/// one group the import gaps are pushed first.
#[test]
fn a_protocol_method_refusal_is_grouped_under_the_file_that_declares_the_class() {
    let dir = ScratchDir::new("1112_protocol_multifile").expect("scratch");
    std::fs::write(
        dir.join("sink.py"),
        "\
from typing import Protocol


class Sink(Protocol):
    def total(self, v: memoryview) -> int: ...
",
    )
    .expect("write the dependency");
    std::fs::write(
        dir.join("view_probe.py"),
        "\
import tkinter

from sink import Sink


def plain(x: int) -> int:
    return x + 1
",
    )
    .expect("write the entry");
    let build = protocol_build(&dir, false);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    let protocol_at = err
        .find("error[I0405]")
        .unwrap_or_else(|| panic!("no I0405 in {err}"));
    let import_at = err
        .find("error[I0403]")
        .unwrap_or_else(|| panic!("no I0403 in {err}"));
    assert!(protocol_at < import_at, "{err}");
    assert!(
        err.contains("`Sink.total`'s parameter 1 `memoryview`"),
        "{err}"
    );
}

/// Round 7 of the pinned review: a derived protocol's *override* is the
/// derived class's own declaration, not an inherited copy.
///
/// `lower_protocol_class` copies a base protocol's members into the derived
/// class and a body redeclaration replaces the copied entry, so the
/// "report it once, at the class that declares it" dedup in
/// `src/memoryview_mode.rs` cannot key on the method *name*: `P.f` and
/// `Q.f` share one, while only `Q`'s names a `memoryview`. Keying on the
/// whole `ProtocolMember` is what makes the ancestor's differing signature
/// stop matching, so the override is reported at `Q` -- the only class
/// that declares it.
#[test]
fn a_protocol_override_adding_a_memoryview_parameter_is_refused() {
    let dir = fixture(
        "1112_protocol_override",
        "\
from typing import Protocol


class Base(Protocol):
    def total(self, x: int) -> int: ...


class Sink(Base):
    def total(self, v: memoryview) -> int: ...


def plain(x: int) -> int:
    return x + 1
",
    );
    let build = protocol_build(&dir, false);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert_eq!(err.matches("error[I0405]").count(), 1, "{err}");
    assert!(
        err.contains("`Sink.total`'s parameter 1 `memoryview`"),
        "{err}"
    );
}

/// Round 7 of the pinned review: the wrapper's format refusal names the
/// whole PEP 3118 format string the exporter declared.
///
/// `pycc_ext_unpack_memoryview` used to copy `out->format` into a
/// 16-character buffer before releasing the buffer, so a `ctypes.Structure`
/// array's format -- routinely past thirty characters -- reached the
/// message as a prefix plus `...`, contradicting `docs/RUNTIME.md`'s
/// promise that the refusal names what it saw. The message is now built
/// while the buffer is still held and the release moved after it. Hosted,
/// for the reason every `--ext` build-and-load test here is: `--ext`
/// requires a CPython 3.13+ with development headers, and CI's interpreter
/// is older.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_format_refusal_names_a_long_format_in_full() {
    let dir = fixture("1112_long_format", SUBJECT);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(
            "import ctypes, view_probe\n\
             class Pair(ctypes.Structure):\n\
             \x20   _fields_ = [('abcdefghij', ctypes.c_double), ('klmnopqrst', ctypes.c_double)]\n\
             view = memoryview((Pair * 2)())\n\
             assert len(view.format) > 16, view.format\n\
             try:\n\
             \x20   view_probe.take_view(view)\n\
             except TypeError as error:\n\
             \x20   message = str(error)\n\
             else:\n\
             \x20   raise AssertionError('the wrapper accepted a non-float64 format')\n\
             assert view.format in message, (view.format, message)\n\
             assert '...' not in message, message\n\
             print('ok')\n",
        )
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}
