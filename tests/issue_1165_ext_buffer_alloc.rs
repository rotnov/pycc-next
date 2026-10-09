//! Part 2a of #1142 (#1165): artifact-owned buffer storage, exercised end
//! to end through the real `pycc build --ext` CLI and a hosted CPython run.
//!
//! Every `Ty::MemoryView` before this part was a view the generated wrapper
//! borrowed from the host for the duration of one call. `a = ndarray(n)`
//! adds the second provenance: storage the artifact allocates, uses, and
//! frees inside the allocating frame, with nothing able to carry it out.
//!
//! This file owns the halves no in-crate unit test can reach:
//!
//! * the diagnostics a real program gets when the producer appears anywhere
//!   but an assignment's right-hand side, at module scope, or with a
//!   non-`int` length;
//! * the native-mode refusal, which is a property of the artifact mode and
//!   so needs the driver rather than a crate;
//! * the hosted arms that prove a compiled allocation is real memory the
//!   compiled code can store into and read back;
//! * and the leak arm, which reads `pycc_rt_buffer_live_views` -- the
//!   allocator pair's own balance counter -- out of the built extension
//!   module with `ctypes` and asserts it is back at zero after every call.
//! * the two refused-length arms, which are the observations that can only
//!   be made from a *hosted* run: a length that aborts and a length that
//!   raises both print to stderr, so only the host process's own exit
//!   status tells them apart (134 against 0);
//! * and the leak arm, whose bullet continues below.
//!   The library is opened by `alloc_probe.__file__`, the sibling
//!   `issue_1054_ext_str_release.rs` probe's convention, so the handle
//!   refers to the same mapping the import created. That arm is the one
//!   thing here that is **not** compiled on Windows, for the reason that
//!   file states in full: MSVC exports from a `.pyd` only the single
//!   `PyInit_<name>` a D-244 `ext` module declares, so a counter linked in
//!   from the `pycc_rt` static archive is present but unreachable through
//!   `ctypes`. Every other arm, diagnostics included, runs on Windows.
//!   That symbol is reachable because `pycc_rt` links into the module as a
//!   staticlib; asserting on it is what distinguishes "the epilogue ran"
//!   from "the process had enough memory not to notice". Its last arm is
//!   the only place a *live* allocation leaves through the exception exit.
//!
//! Every non-`#[ignore]`d test here asserts a front-end diagnostic, and
//! deliberately so: a successful `--ext` build needs CPython development
//! headers at or above the `Py_LIMITED_API` floor, which CI's coverage job
//! does not have when it runs. See
//! `a_program_that_defines_the_spelling_keeps_its_own_meaning` for the one
//! property that had to be restated as a refusal to keep it in that pass.
//!
//! None of these contributes line coverage: CI's coverage job runs
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
    std::fs::write(dir.join("alloc_probe.py"), source).expect("write the subject");
    dir
}

/// Builds the entry module as a CPython extension module directly into
/// `dir`, so a CPython run with `dir` as its working directory imports it.
fn build_ext(dir: &Path) -> Output {
    pycc()
        .arg("build")
        .arg(dir.join("alloc_probe.py"))
        .arg("-o")
        .arg(dir.join("alloc_probe"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn")
}

/// Builds the entry module in the default `native` mode.
fn build_native(dir: &Path) -> Output {
    pycc()
        .arg("build")
        .arg(dir.join("alloc_probe.py"))
        .arg("-o")
        .arg(dir.join("alloc_probe_native"))
        .output()
        .expect("pycc should spawn")
}

/// Runs `program` under CPython with `dir` on the import path.
fn run_hosted(dir: &Path, program: &str) -> Output {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(program)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn")
}

/// The subject: one export that allocates, fills and sums its own storage,
/// and one that reallocates inside the same frame so the free-before-store
/// path (D-074) runs on a real artifact rather than only in codegen's own
/// unit test.
const SUBJECT: &str = "\
def build_and_sum(n: int) -> float:
    a = ndarray(n)
    i = 0
    while i < len(a):
        a[i] = 2.0
        i = i + 1
    s = 0.0
    j = 0
    while j < len(a):
        s = s + a[j]
        j = j + 1
    return s


def rebuild(n: int) -> float:
    a = ndarray(n)
    a = ndarray(n + 1)
    return float(len(a))


def alloc_then_raise(n: int) -> float:
    a = ndarray(n)
    b = ndarray(-1)
    return a[0] + b[0]
";

/// The `NDArray` spelling reaches the same producer, so the second
/// spelling is proven at the artifact level and not only in the checker.
/// #1166 round 8. `ndarray(n + 1)` rather than `ndarray(n)`: a bigint
/// length can only be reached by *promotion* inside the artifact, because
/// the `ext` wrapper rejects an argument outside `[-2**62, 2**62-1]` before
/// the body runs. `n = 2 ** 62 - 1` makes `n + 1` promote, which is exactly
/// the shape the original repro used.
const SUBJECT_PROMOTED_LEN: &str = "\
def sized(n: int) -> float:
    a = ndarray(n + 1)
    return float(len(a))
";
/// The reassign-then-refuse shape, which `SUBJECT`'s `alloc_then_raise`
/// cannot carry: there the refused allocation binds a *different* name, so
/// the slot being overwritten is still null. Here the same name already
/// holds a live view when the second allocation is refused, which is the
/// only arrangement in which a free emitted before the refusal branched
/// away would release a view the exception-exit epilogue then frees again.
/// `#[cfg]`d to match its sole consumer below: an unconditional const with a
/// windows-gated user is dead code there, and `-D warnings` fails on it.
#[cfg(not(target_os = "windows"))]
const SUBJECT_REALLOC_THEN_RAISE: &str = "\
def realloc_then_raise(n: int) -> float:
    a = ndarray(n)
    a = ndarray(-1)
    return a[0]
";
const SUBJECT_NDARRAY: &str = "\
def size(n: int) -> float:
    a = NDArray(n)
    return float(len(a))
";

/// A producer outside an assignment's right-hand side is the named
/// position refusal, not an incidental type mismatch: the bare-statement
/// shape is the one that would otherwise type-check and leak one
/// allocation per call, because `ExprStmt` discards the inferred type.
#[test]
fn a_bare_producer_statement_is_the_named_position_refusal() {
    let dir = fixture(
        "1165_bare_statement",
        "\
def leak(n: int) -> None:
    ndarray(n)
",
    );
    let build = build_ext(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[C0001]"), "{err}");
    assert!(
        err.contains("admitted only as the whole right-hand side of an assignment"),
        "{err}"
    );
}

/// The same refusal covers a call argument, which before this part fell out
/// only incidentally as a parameter-type mismatch.
#[test]
fn a_producer_in_an_argument_position_is_the_same_refusal() {
    let dir = fixture(
        "1165_argument",
        "\
def take(x: float) -> float:
    return x


def go(n: int) -> float:
    return take(ndarray(n))
",
    );
    let build = build_ext(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[C0001]"), "{err}");
    assert!(
        err.contains("admitted only as the whole right-hand side of an assignment"),
        "{err}"
    );
}

/// Module scope gets its own refusal rather than the position one: the
/// ground differs, because a module frame has no epilogue to free the
/// allocation from.
#[test]
fn a_producer_at_module_scope_is_its_own_refusal() {
    let dir = fixture("1165_module_scope", "a = ndarray(4)\n");
    let build = build_ext(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[C0001]"), "{err}");
    assert!(
        err.contains("artifact-owned buffer storage is freed when the allocating function returns"),
        "{err}"
    );
}

/// A non-`int` length is a typed refusal, not the position one.
#[test]
fn a_non_int_length_is_a_typed_refusal() {
    let dir = fixture(
        "1165_length_type",
        "\
def go(k: str) -> float:
    a = ndarray(k)
    return float(len(a))
",
    );
    let build = build_ext(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[T0033]"), "{err}");
    assert!(err.contains("expects an `int` element count"), "{err}");
}

/// Reading an owned buffer beyond the three implemented operations is the
/// *owned* refusal, whose wording differs from the parameter one on
/// purpose: egress does not exist yet (Part 2b of #1142, #1164), whereas
/// handing back a borrowed view would be a use-after-free.
#[test]
fn using_owned_storage_beyond_the_implemented_operations_is_the_owned_refusal() {
    let dir = fixture(
        "1165_owned_use",
        "\
def go(n: int) -> float:
    a = ndarray(n)
    b = a
    return float(len(b))
",
    );
    let build = build_ext(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[C0001]"), "{err}");
    assert!(
        err.contains("bound to buffer storage this `pycc build --ext` artifact allocated"),
        "{err}"
    );
}

/// A program that binds the spelling itself keeps its own meaning, which is
/// D-244's #1129 statement (h) applied to the call position.
///
/// Written as a *refusal* rather than a successful build on purpose. A
/// successful `--ext` build links against CPython development headers, and
/// CI's coverage job probes the interpreter only in `src/main.rs`'s ext
/// emission -- after type checking -- so a success-shaped subject is the one
/// arm in this file that cannot run on a runner whose interpreter predates
/// the `Py_LIMITED_API` floor. Every other non-`#[ignore]`d test here
/// asserts a front-end diagnostic and passes there unchanged, so the
/// shadowing seam is proven the same way.
///
/// `a[0]` is what discriminates the two interpretations. If the producer
/// wrongly hijacked the name, `a` would be `Ty::MemoryView` and `a[0]` a
/// legal element read; because the program's own `def` wins, `a` is a
/// `float` and subscripting it is a typed refusal. The same subject without
/// the user's `def ndarray` builds cleanly, which is the counterfactual this
/// diagnostic stands in for.
#[test]
fn a_program_that_defines_the_spelling_keeps_its_own_meaning() {
    let dir = fixture(
        "1165_shadowed",
        "\
def ndarray(n: int) -> float:
    return float(n)


def go(n: int) -> float:
    a = ndarray(n)
    return a[0]
",
    );
    let build = build_ext(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[T0033]"), "{err}");
    assert!(err.contains("`float` does not support indexing"), "{err}");
}

/// #1166 review finding F2: statement (h) covers a binding the *function*
/// makes, not only one the module makes.
///
/// `local_names` is a whole-body pre-pass, so CPython makes `ndarray` local
/// throughout `go` and the earlier call raises `UnboundLocalError`. Before
/// the fix the producer's statement-(h) guard consulted only the class,
/// generic, function and module-binding tables -- none of which holds a local
/// that the walk has not reached yet -- so the call was recognized as a
/// producer and the frame allocated. Both walkers now decline, and the
/// solver's own `is_local` gate reports the `UnboundLocalError` analogue.
#[test]
fn a_function_local_rebinding_of_the_spelling_wins_over_the_producer() {
    let dir = fixture(
        "1165_local_shadow",
        "\
def go() -> float:
    a = ndarray(4)
    ndarray = 1
    return float(ndarray)
",
    );
    let build = build_ext(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[T0021]"), "{err}");
    assert!(
        err.contains("local name `ndarray` is not bound before this use"),
        "{err}"
    );
}

/// #1166 review finding F3: a `bool` length is an `int` length.
///
/// `docs/TYPE_SYSTEM.md`'s representation table makes `bool` a subtype of
/// `int` (rule 4/D-086), the same rule that already admits a `bool` buffer
/// *index*, and `MirExpr::BufferAlloc`'s codegen already decodes one -- the
/// length goes through `to_numeric_encoded_int`, whose `Scalar::Bool` arm
/// zero-extends and re-tags before the shared checked untag. The `T0033` this
/// used to raise refused a one-element request the whole pipeline supports.
///
/// Written as the *owned-use* refusal for
/// `a_program_that_defines_the_spelling_keeps_its_own_meaning`'s reason: a
/// successful `--ext` build needs CPython development headers. `b = a` is
/// what discriminates the two interpretations -- that message is reachable
/// only if `a` is bound to artifact-owned buffer storage, so reaching it
/// proves `ndarray(True)` was admitted as the producer.
#[test]
fn a_bool_length_is_admitted_as_a_one_element_request() {
    let dir = fixture(
        "1165_bool_length",
        "\
def go() -> float:
    a = ndarray(True)
    b = a
    return float(len(b))
",
    );
    let build = build_ext(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[C0001]"), "{err}");
    assert!(
        err.contains("bound to buffer storage this `pycc build --ext` artifact allocated"),
        "{err}"
    );
    assert!(!err.contains("error[T0033]"), "{err}");
}

/// #1166 review finding F4: a value-less module-level annotation is not a
/// binding, so it must not disarm the native gate.
///
/// `HirStmt::AnnAssign`'s `value` is an `Option`, and `ndarray: int` alone
/// binds nothing at run time -- the check phase records it in `declared`, not
/// in `Environment::bindings`, so `buffer::producer_assignment_ty` still
/// recognizes the producer. The native gate's shadow set nevertheless counted
/// the bare annotation, skipped its refusal, and let an artifact-owned buffer
/// allocation reach a **native** executable past the documented `--ext`-only
/// boundary. This is the counterpart of
/// `allocating_buffer_storage_natively_is_refused_in_its_own_words`, whose
/// subject carries no such annotation.
#[test]
fn a_value_less_module_annotation_does_not_disarm_the_native_gate() {
    let dir = fixture(
        "1165_native_bare_annotation",
        "\
ndarray: int


def go(n: int) -> float:
    a = ndarray(n)
    return a[0]
",
    );
    let build = build_native(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[I0405]"), "{err}");
    assert!(
        err.contains("a native executable has no host to carry it to"),
        "{err}"
    );
}

/// The load-bearing negative for the test above: an annotation that *does*
/// carry an initializer is a real binding, so it still keeps the program's
/// own meaning and the native gate stays silent about it. `T0021` rather than
/// `I0405` is the whole assertion -- calling an `int` is the program's own
/// error, which is exactly statement (h) working.
#[test]
fn an_initialized_module_annotation_still_keeps_the_programs_own_meaning() {
    let dir = fixture(
        "1165_native_initialized_annotation",
        "\
ndarray: int = 3


def go(n: int) -> float:
    a = ndarray(n)
    return a[0]
",
    );
    let build = build_native(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(!err.contains("error[I0405]"), "{err}");
    assert!(err.contains("error[T0021]"), "{err}");
    assert!(
        err.contains("name `ndarray` is bound to a non-callable value"),
        "{err}"
    );
}

/// #1166 review finding: the native gate must honour statement (h)'s
/// *function-local* arm too, not only the module-wide one.
///
/// This is `a_function_local_rebinding_of_the_spelling_wins_over_the_producer`
/// built natively. The two type-checking recognizers already decline here, so
/// `--ext` and `pycc check` both report the program's own `UnboundLocalError`
/// analogue; the native gate, unaware of function-local names, instead
/// reported `I0405` -- whose remedy is "rebuild with `--ext`", which lands on
/// a different error about a different cause. A diagnostic that misidentifies
/// the defect and prescribes an inapplicable remedy is a defect of its own,
/// so the assertion is that native mode now reports the same `T0021` the
/// other two modes do, and no `I0405` at all.
#[test]
fn a_function_local_rebinding_keeps_its_own_meaning_natively_too() {
    let dir = fixture(
        "1165_native_local_shadow",
        "\
def go(n: int) -> float:
    a = ndarray(4)
    ndarray = 1
    return float(ndarray)
",
    );
    let build = build_native(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(!err.contains("error[I0405]"), "{err}");
    assert!(
        err.contains("local name `ndarray` is not bound before this use"),
        "{err}"
    );
}

/// The parameter arm of the same finding, which the body-local arm above does
/// not cover: `function_local_names` starts from the parameter list, and a
/// parameter is a binding from the first statement rather than one the
/// whole-body pre-pass has to reach. Calling an `int` parameter is the
/// program's own error in every mode, so the native gate must stay silent.
#[test]
fn a_parameter_named_for_the_spelling_keeps_its_own_meaning_natively() {
    let dir = fixture(
        "1165_native_param_shadow",
        "\
def go(ndarray: int) -> float:
    a = ndarray(4)
    return float(a)
",
    );
    let build = build_native(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(!err.contains("error[I0405]"), "{err}");
    assert!(
        err.contains("name `ndarray` is bound to a non-callable value"),
        "{err}"
    );
}

/// The function-local layer is per *spelling*, not per function: a body that
/// binds `ndarray` says nothing about `NDArray`, so the producer call on the
/// unshadowed spelling is still a real allocation and still refused. This
/// pins the shape the gate is built out of -- collapsing the per-spelling
/// filter into "this function binds some producer spelling" would silently
/// let this program through the gate while the checker still admits it.
#[test]
fn one_shadowed_spelling_does_not_disarm_the_other() {
    let dir = fixture(
        "1165_native_one_spelling_shadowed",
        "\
def go(n: int) -> float:
    ndarray = 1
    a = NDArray(4)
    return a[0]
",
    );
    let build = build_native(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[I0405]"), "{err}");
    assert!(
        err.contains("allocates buffer storage with `NDArray(n)`"),
        "{err}"
    );
}

/// The native gate is a property of the artifact mode, so it needs the
/// driver. Its message states the `--ext` boundary rather than the missing
/// interpreter, because the allocation itself would link and run natively.
#[test]
fn allocating_buffer_storage_natively_is_refused_in_its_own_words() {
    let dir = fixture("1165_native", SUBJECT);
    let build = build_native(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[I0405]"), "{err}");
    assert!(
        err.contains("a native executable has no host to carry it to"),
        "{err}"
    );
}

/// #1166 review finding A: a call the checker refuses for its *length* is
/// not an allocation, so the native gate must not answer it with `I0405`.
///
/// The gate's walk matches a spelling and an arity. `ndarray("x")` and
/// `ndarray(missing)` match both and bind nothing: `--ext` and `pycc check`
/// report `T0033` and `T0021` for them, so an `I0405` telling the user to
/// rebuild with `--ext` prescribed a remedy that lands on a different error
/// about a different cause -- the defect
/// `a_function_local_rebinding_keeps_its_own_meaning_natively_too` closed for
/// statement (h)'s local arm, in the arm beside it.
#[test]
fn a_length_the_checker_refuses_is_not_a_native_allocation() {
    for (category, source, code) in [
        (
            "1165_native_length_type",
            "\
def go(n: int) -> int:
    a = ndarray(\"x\")
    return n
",
            "error[T0033]",
        ),
        (
            "1165_native_length_unbound",
            "\
def go(n: int) -> int:
    a = ndarray(missing)
    return n
",
            "error[T0021]",
        ),
    ] {
        let dir = fixture(category, source);
        let build = build_native(&dir);
        assert!(!build.status.success(), "{}", stdout_of(&build));
        let err = stderr_of(&build);
        assert!(err.contains(code), "{err}");
        assert!(!err.contains("I0405"), "{err}");
    }
}

/// The under-refusal direction of the filter above, and the reason the gate
/// consumes the checker's verdict instead of inferring the length itself.
///
/// A length may read a module-level global, so any environment the gate
/// could build for itself would have to reproduce the checker's own
/// top-level pass; a narrower one infers a failure where there is none and
/// *skips*, letting an artifact-owned allocation into a native executable.
/// A `bool` length is admitted for the same reason `ndarray(True)` requests
/// one element, so it must stay refused natively too.
#[test]
fn a_length_the_checker_admits_is_still_a_native_allocation() {
    for (category, source) in [
        (
            "1165_native_global_length",
            "\
N = 4


def go(n: int) -> int:
    a = ndarray(N)
    return n
",
        ),
        (
            "1165_native_bool_length",
            "\
def go(n: int) -> int:
    a = ndarray(True)
    return n
",
        ),
    ] {
        let dir = fixture(category, source);
        let build = build_native(&dir);
        assert!(!build.status.success(), "{}", stdout_of(&build));
        let err = stderr_of(&build);
        assert!(err.contains("error[I0405]"), "{err}");
        assert!(
            err.contains("a native executable has no host to carry it to"),
            "{err}"
        );
    }
}

/// The filter is per *function*, not per module: an allocating function that
/// checks clean is still named when a different function in the same program
/// fails its own check, and both diagnostics are reported together.
#[test]
fn a_clean_allocation_is_named_beside_another_functions_failure() {
    let dir = fixture(
        "1165_native_mixed_failure",
        "\
def alloc(n: int) -> int:
    a = ndarray(4)
    return n


def broken(n: int) -> str:
    return 1
",
    );
    let build = build_native(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[I0405]"), "{err}");
    assert!(err.contains("error[T0022]"), "{err}");
}

/// A module-level failure stops the check driver before it visits any
/// function body, so it carries no evidence about any function and every
/// producer gap is suppressed rather than guessed at.
#[test]
fn a_module_level_failure_suppresses_every_producer_gap() {
    let dir = fixture(
        "1165_native_module_failure",
        "\
X: int = \"s\"


def go(n: int) -> int:
    a = ndarray(4)
    return n
",
    );
    let build = build_native(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[T0025]"), "{err}");
    assert!(!err.contains("I0405"), "{err}");
}

/// A `memoryview` in a *signature* refuses the program before the type check
/// is reported, exactly so the signature gets the `I0405` it is documented to
/// get. The producer gap is still filtered against a verdict obtained purely
/// for that purpose -- `src/frontend.rs`'s `resolve_frontend_native` owns why,
/// on the `refused_a_buffer` arm. This program checks clean, so it exercises
/// that filter's `Ok` arm and both `I0405`s survive.
#[test]
fn a_buffer_signature_and_an_allocation_are_both_named() {
    let dir = fixture(
        "1165_native_signature_and_producer",
        "\
def go(v: memoryview) -> int:
    a = ndarray(4)
    return 1
",
    );
    let build = build_native(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(
        err.contains("`go`'s parameter `v` requires `pycc build --ext`")
            || err.contains("requires `pycc build --ext`"),
        "{err}"
    );
    assert!(
        err.contains("a native executable has no host to carry it to"),
        "{err}"
    );
    assert_eq!(err.matches("error[I0405]").count(), 2, "{err}");
}

/// The keep path of the same filter, on the arm where the check actually
/// fails: `broken` is what keys the verdict's error, `alloc`'s allocation is
/// admitted, and the signature gap in `sig` is what routes the program down
/// the `refused_a_buffer` early return in the first place. The filter is per
/// function, so `alloc`'s `I0405` must survive `broken`'s failure -- and the
/// verdict's own `T0022` must stay discarded, because this path exists to
/// deliver `sig`'s signature `I0405` instead of a type error.
#[test]
fn a_signature_gap_does_not_suppress_an_admitted_allocation_elsewhere() {
    let dir = fixture(
        "1165_native_signature_and_admitted_producer",
        "\
def sig(v: memoryview) -> int:
    return 0


def broken(n: int) -> str:
    return 1


def alloc(n: int) -> int:
    a = ndarray(4)
    return n
",
    );
    let build = build_native(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(
        err.contains("`sig`'s parameter `v: memoryview` requires `pycc build --ext`"),
        "{err}"
    );
    assert!(
        err.contains("`alloc` allocates buffer storage with `ndarray(n)`"),
        "{err}"
    );
    assert!(
        !err.contains("T0022"),
        "the verdict obtained for filtering must not be reported: {err}"
    );
}

/// The drop path on the same arm, and the defect it closed (#1165 review
/// round 4): a signature gap used to take an early return that reported every
/// producer gap without ever consulting the type check, so `bad` was told to
/// rebuild with `--ext` -- while `--ext` refuses this very program with a
/// `T0033` naming the `str` element count. The
/// signature's own `I0405` is correct and stays; `bad`'s is a misdirected
/// remedy and must not be reported.
#[test]
fn a_signature_gap_does_not_report_a_producer_the_checker_refuses() {
    let dir = fixture(
        "1165_native_signature_and_refused_producer",
        "\
def sig(v: memoryview) -> int:
    return 0


def bad() -> None:
    a = ndarray(\"x\")
",
    );
    let build = build_native(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(
        err.contains("`sig`'s parameter `v: memoryview` requires `pycc build --ext`"),
        "{err}"
    );
    assert!(
        !err.contains("`bad` allocates buffer storage"),
        "the producer gap `--ext` does not fix must not be reported: {err}"
    );
}

/// Runs `pycc check` on the entry module.
fn check_only(dir: &Path) -> Output {
    pycc()
        .arg("check")
        .arg(dir.join("alloc_probe.py"))
        .output()
        .expect("pycc should spawn")
}

/// The source every module-alias arm below shares: `import math as ndarray`
/// binds the producer spelling to the `math` module, so the program's own
/// binding wins under D-244 #1129 statement (h) and `ndarray(4)` is
/// CPython's `TypeError: 'module' object is not callable`, not an
/// allocation.
const ALIASED_SPELLING: &str = "\
import math as ndarray


def go(n: int) -> int:
    a = ndarray(4)
    return n
";

/// The miscompile pin, and the reason this arm is a category worse than the
/// four statement-(h) arms before it.
///
/// A stdlib module alias binds the spelling in `Environment::std_module_aliases`
/// and in no other table, so the producer's statement-(h) guard -- which
/// consulted the class, generic, function, module-binding and function-local
/// tables -- did not see it. `pycc build --ext` therefore **succeeded**, and
/// the artifact it emitted allocated a buffer for a call the program's own
/// binding makes a `TypeError`. That is a wrong artifact out of a silent
/// success, not a misdirected message: nothing refused it in any mode.
#[test]
fn a_module_alias_of_the_spelling_is_not_a_silent_ext_allocation() {
    let dir = fixture("1165_alias_shadow_ext", ALIASED_SPELLING);
    let build = build_ext(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[T0021]"), "{err}");
    assert!(
        err.contains("call to undefined function `ndarray`"),
        "{err}"
    );
}

/// The same program under `pycc check`, which selects no artifact mode and
/// so reaches neither the native gate nor the `ext` lowering: it used to
/// exit 0 on a program CPython raises on.
#[test]
fn a_module_alias_of_the_spelling_is_refused_by_check() {
    let dir = fixture("1165_alias_shadow_check", ALIASED_SPELLING);
    let check = check_only(&dir);
    assert!(!check.status.success(), "{}", stderr_of(&check));
    // `pycc check` reports on stdout, unlike `pycc build`.
    let err = stdout_of(&check);
    assert!(err.contains("error[T0021]"), "{err}");
    assert!(
        err.contains("call to undefined function `ndarray`"),
        "{err}"
    );
}

/// The native gate's own arm. It walks the HIR rather than an
/// `Environment`, so it needs the import-derived shadow of its own
/// (`pycc_types::imported_producer_spellings`); without it the gate told
/// this program to rebuild with `--ext`, a remedy for an allocation the
/// program does not make and that `--ext` refuses too.
#[test]
fn a_module_alias_of_the_spelling_is_not_a_native_producer_gap() {
    let dir = fixture("1165_alias_shadow_native", ALIASED_SPELLING);
    let build = build_native(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(!err.contains("error[I0405]"), "{err}");
    assert!(
        err.contains("call to undefined function `ndarray`"),
        "{err}"
    );
}

/// The solver's mirror, which the annotated arms above never reach: the
/// constraint solver runs over *unannotated* private helpers (#142), so
/// `_helper` is the only shape that exercises
/// `constraints::resolved_producer_call`'s own statement-(h) guard and the
/// alias gate in its `Call` arm. Before the fix this program built silently
/// too.
///
/// `b = a` is what separates the two solver seams rather than decoration:
/// the admitting seam (`resolved_producer_call`) marks `a` artifact-owned,
/// and `module::merge_solver_first` makes the owned-use `C0001` that marks
/// then raises the message the compiler emits -- so without that seam's own
/// alias arm this program is refused in the *producer's* words instead of
/// the program's own `T0021`.
#[test]
fn a_module_alias_of_the_spelling_is_declined_by_the_solver_too() {
    let dir = fixture(
        "1165_alias_shadow_solver",
        "\
import math as ndarray


def _helper(n):
    a = ndarray(4)
    b = a
    return n


def go(n: int) -> int:
    return _helper(n)
",
    );
    let build = build_ext(&dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(err.contains("error[T0021]"), "{err}");
    assert!(
        err.contains("call to undefined function `ndarray`"),
        "{err}"
    );
}

/// The keep path: the alias arm is per *spelling*, exactly as the
/// function-local layer is. An `import math as m` binds `m`, says nothing
/// about `ndarray`, and must leave a genuine producer call in place -- an
/// over-suppression here would silently stop refusing a native allocation.
///
/// The `--ext` half is written as the owned-use refusal for
/// `a_bool_length_is_admitted_as_a_one_element_request`'s reason (a
/// successful `--ext` build needs CPython development headers): `b = a` is
/// reachable only when `a` is bound to artifact-owned buffer storage, so
/// that message proves the producer was still admitted.
#[test]
fn an_unrelated_module_alias_leaves_the_producer_in_place() {
    let ext_dir = fixture(
        "1165_alias_keep_ext",
        "\
import math as m


def go(n: int) -> float:
    a = ndarray(4)
    b = a
    return float(n) + m.sqrt(b[0])
",
    );
    let build = build_ext(&ext_dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(!err.contains("call to undefined function"), "{err}");
    assert!(
        err.contains("bound to buffer storage this `pycc build --ext` artifact allocated"),
        "{err}"
    );

    // The native half drops the `b = a` discriminator: that owned-use
    // refusal is raised in every artifact mode and would preempt the gate
    // this arm is pinning.
    let native_dir = fixture(
        "1165_alias_keep_native",
        "\
import math as m


def go(n: int) -> float:
    a = ndarray(4)
    return float(n) + m.sqrt(a[0])
",
    );
    let native = build_native(&native_dir);
    assert!(!native.status.success(), "{}", stdout_of(&native));
    let native_err = stderr_of(&native);
    assert!(native_err.contains("error[I0405]"), "{native_err}");
    assert!(
        native_err.contains("allocates buffer storage with `ndarray(n)`"),
        "{native_err}"
    );
}

/// The inventory's other two import forms, pinned so a later change cannot
/// open either one silently into the hole this commit closed.
/// `from ... import ... as ...` is refused before any binding exists and so
/// needs no statement-(h) arm at all. A non-stdlib `import ndarray` binds an
/// opaque CPython module object, which needs no arm in the *check* phase --
/// `foreign::bind_foreign_objects_at` binds the name into
/// `Environment::bindings` as `Ty::Object`, so
/// `buffer::producer_assignment_ty`'s `bindings` arm already declines -- but
/// does need one in the solver, whose own table is separate. The arm and the
/// message it protects are pinned by
/// `a_foreign_import_of_the_spelling_is_declined_by_the_solver_too` below.
///
/// Since Part 1 of #1333 the foreign form is no longer refused at all:
/// binding a CPython object in a function body is admitted, so `a =
/// ndarray(4)` is an ordinary call of the imported object. What stays
/// pinned is that it is never the intrinsic producer -- no buffer-storage
/// `C0001` and no foreign `I0404`. The `--ext` build's own success is not
/// asserted, because it needs CPython development headers.
#[test]
fn the_other_import_forms_of_the_spelling_are_never_the_producer() {
    let symbol_dir = fixture(
        "1165_alias_from_import",
        "\
from math import sqrt as ndarray


def go(n: int) -> int:
    a = ndarray(4)
    return n
",
    );
    let symbol = build_ext(&symbol_dir);
    assert!(!symbol.status.success(), "{}", stdout_of(&symbol));
    assert!(
        stderr_of(&symbol).contains("error[C0001]"),
        "{}",
        stderr_of(&symbol)
    );

    let foreign_dir = fixture(
        "1165_alias_foreign_import",
        "\
import ndarray


def go(n: int) -> int:
    a = ndarray(4)
    return n
",
    );
    let foreign = build_ext(&foreign_dir);
    let foreign_err = stderr_of(&foreign);
    assert!(!foreign_err.contains("error[I0404]"), "{foreign_err}");
    assert!(!foreign_err.contains("error[C0001]"), "{foreign_err}");
    let foreign_check = check_only(&foreign_dir);
    assert!(
        foreign_check.status.success(),
        "{}",
        stdout_of(&foreign_check)
    );
}

/// The sixth statement-(h) arm (#1165 review round 7), and the one whose
/// absence was a *diagnostic-quality* defect rather than a miscompile: the
/// program is refused either way, but in the wrong words and at the wrong
/// line.
///
/// A foreign `import ndarray` binds the spelling only in the solver's
/// `foreign_objects` table -- it is deliberately kept out of
/// `ConstraintEnvironment::bindings`, unlike the check phase's own
/// `Environment`, where `foreign::bind_foreign_objects_at` records it as
/// `Ty::Object` -- so `constraints::resolved_producer_call`'s guard did not
/// see it and treated the call as the intrinsic producer. `b = a` then read
/// a name the solver had marked artifact-owned, and
/// `module::merge_solver_first` made that owned-buffer `C0001` the reported
/// diagnostic, displacing the `I0404` foreign refusal and pointing the span
/// at the `import` line.
///
/// `go` is fully annotated on purpose: the solver walks every body, not
/// only unannotated private helpers, so the simplest shape reproduces it.
///
/// Part 1 of #1333 admits binding a CPython object in a function body, so
/// the program is no longer refused: `a = ndarray(n)` calls the imported
/// object and `b = a` aliases the result. The pin is therefore the absence
/// of the owned-buffer `C0001` (and of `I0404`) on every path, plus a clean
/// `pycc check`. The native build must not report `I0405` either: the native
/// gate's shadow set now includes a foreign import of the spelling, because
/// the foreign refusal that used to preempt that gate is gone. It stops at
/// the embedded build's missing lock instead. The `--ext` build's own
/// success is not asserted, because it needs CPython development headers.
#[test]
fn a_foreign_import_of_the_spelling_is_declined_by_the_solver_too() {
    const SOURCE: &str = "\
import ndarray


def go(n: int) -> int:
    a = ndarray(n)
    b = a
    return n
";
    let dir = fixture("1165_foreign_shadow_solver", SOURCE);
    let build = build_ext(&dir);
    let err = stderr_of(&build);
    assert!(!err.contains("error[I0404]"), "{err}");
    assert!(
        !err.contains("bound to buffer storage this `pycc build --ext` artifact allocated"),
        "{err}"
    );

    // `pycc check` selects no artifact mode and reports on stdout; the
    // solver runs there too, and the program checks clean.
    let check_dir = fixture("1165_foreign_shadow_check", SOURCE);
    let check = check_only(&check_dir);
    assert!(check.status.success(), "{}", stdout_of(&check));

    // The native gate must not claim an allocation: the embedded build
    // stops at the foreign import's missing lock, not at `I0405`.
    let native_dir = fixture("1165_foreign_shadow_native", SOURCE);
    let native = build_native(&native_dir);
    assert!(!native.status.success(), "{}", stdout_of(&native));
    let native_err = stderr_of(&native);
    assert!(!native_err.contains("error[I0405]"), "{native_err}");
    assert!(!native_err.contains("error[I0404]"), "{native_err}");
    assert!(native_err.contains("pycc.lock"), "{native_err}");
}

/// The shape the round-7 review named: a private helper with no annotations
/// at all, which is the only body the constraint solver was ever expected to
/// walk. Same assertions as the annotated arm above.
#[test]
fn a_foreign_import_shadows_the_producer_in_an_unannotated_helper() {
    let dir = fixture(
        "1165_foreign_shadow_helper",
        "\
import ndarray


def _h(n):
    a = ndarray(n)
    b = a
    return n


def go(n: int) -> int:
    return _h(n)
",
    );
    let build = build_ext(&dir);
    let err = stderr_of(&build);
    assert!(!err.contains("error[I0404]"), "{err}");
    assert!(
        !err.contains("bound to buffer storage this `pycc build --ext` artifact allocated"),
        "{err}"
    );
    let check = check_only(&dir);
    assert!(check.status.success(), "{}", stdout_of(&check));
}

/// The keep path for the foreign arm, written exactly as the module-alias
/// keep path above: the arm is keyed on a name the program's own import
/// table binds, so the *same* shape without any `import ndarray` must still
/// be the intrinsic producer. Over-suppression here would silently stop
/// refusing a native allocation.
///
/// The `--ext` half is the owned-use refusal for the reason that keep path
/// states (a successful `--ext` build needs CPython development headers):
/// `b = a` is reachable only when `a` is bound to artifact-owned buffer
/// storage, so that message proves the producer was still admitted. The
/// native half drops `b = a`, whose refusal is raised in every artifact mode
/// and would preempt the gate being pinned.
#[test]
fn no_foreign_import_leaves_the_producer_in_place() {
    let ext_dir = fixture(
        "1165_foreign_keep_ext",
        "\
def go(n: int) -> int:
    a = ndarray(n)
    b = a
    return n
",
    );
    let build = build_ext(&ext_dir);
    assert!(!build.status.success(), "{}", stdout_of(&build));
    let err = stderr_of(&build);
    assert!(!err.contains("error[I0404]"), "{err}");
    assert!(
        err.contains("bound to buffer storage this `pycc build --ext` artifact allocated"),
        "{err}"
    );

    let native_dir = fixture(
        "1165_foreign_keep_native",
        "\
def go(n: int) -> int:
    a = ndarray(n)
    return n
",
    );
    let native = build_native(&native_dir);
    assert!(!native.status.success(), "{}", stdout_of(&native));
    let native_err = stderr_of(&native);
    assert!(native_err.contains("error[I0405]"), "{native_err}");
    assert!(
        native_err.contains("allocates buffer storage with `ndarray(n)`"),
        "{native_err}"
    );
}

/// The whole of Part 2a in one hosted run: the artifact allocates its own
/// storage, stores into it, reads it back, and frees it.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_artifact_allocates_fills_and_sums_its_own_storage() {
    let dir = fixture("1165_hosted_roundtrip", SUBJECT);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = run_hosted(
        &dir,
        "import alloc_probe\n\
         assert alloc_probe.build_and_sum(4) == 8.0, alloc_probe.build_and_sum(4)\n\
         assert alloc_probe.build_and_sum(0) == 0.0\n\
         assert alloc_probe.rebuild(3) == 4.0, alloc_probe.rebuild(3)\n\
         print('ok')\n",
    );
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}

/// The `NDArray` spelling produces the same artifact behavior.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_ndarray_spelling_allocates_the_same_storage() {
    let dir = fixture("1165_hosted_ndarray", SUBJECT_NDARRAY);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = run_hosted(
        &dir,
        "import alloc_probe\n\
         assert alloc_probe.size(6) == 6.0, alloc_probe.size(6)\n\
         print('ok')\n",
    );
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}

/// A negative length raises `ValueError` through D-173's pending-exception
/// protocol rather than aborting, and the failed allocation leaves nothing
/// for the epilogue to free.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_negative_length_raises_value_error_at_the_boundary() {
    let dir = fixture("1165_hosted_negative", SUBJECT);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = run_hosted(
        &dir,
        "import alloc_probe\n\
         try:\n\
         \x20   alloc_probe.build_and_sum(-1)\n\
         except ValueError as error:\n\
         \x20   assert 'negative' in str(error), str(error)\n\
         else:\n\
         \x20   raise AssertionError('a negative length returned normally')\n\
         print('ok')\n",
    );
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}

/// #1166 round 8's P1, corroborated on a real artifact: neither door out of
/// the `ndarray(n)` length aborts the host interpreter any more.
///
/// Two distinct defects shared this arm, and the exit status is what found
/// the second one.
///
/// 1. A **bigint** length was decoded by `pycc_rt_int_untag_checked`, which
///    `panic!`s on a bigint word; a panic unwinding past a plain
///    `extern "C" fn` boundary is caught there and turned into a process
///    abort, and for a D-244 `ext` artifact that process is the host CPython
///    interpreter. `sized(2 ** 62 - 1)` exited **134**, with
///    `pycc_rt_int_untag_checked` on the backtrace. Reachable only by
///    promotion (`n + 1`), since the wrapper rejects a bigint *argument*
///    first -- hence the separate subject.
/// 2. An **oversized inline** length needs no bigint at all:
///    `build_and_sum(2 ** 62 - 1)` decoded cleanly and then overflowed
///    `Vec`'s capacity inside `pycc_rt_buffer_f64_alloc`, whose `panic!` the
///    same boundary turned into the same abort. Also measured at 134.
///
/// The exit status is the load-bearing assertion in both arms. An aborting
/// process prints its panic text to stderr, so a stderr-content check alone
/// passes on the defect as well as on the fix; only the status separates
/// them. Each arm additionally asserts that an in-range length still returns
/// its value, so a fix that refused everything could not pass either.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_bigint_length_raises_overflow_error_instead_of_aborting_the_host() {
    let dir = fixture("1165_hosted_bigint", SUBJECT_PROMOTED_LEN);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = run_hosted(
        &dir,
        "import alloc_probe\n\
         assert alloc_probe.sized(3) == 4.0, alloc_probe.sized(3)\n\
         try:\n\
         \x20   alloc_probe.sized(2 ** 62 - 1)\n\
         except OverflowError as error:\n\
         \x20   assert 'bigint' in str(error), str(error)\n\
         else:\n\
         \x20   raise AssertionError('a bigint length returned normally')\n\
         print('ok')\n",
    );
    // Explicitly the *status*, not `success()`'s boolean and not stderr:
    // 134 is what the abort produced, and naming it is what makes this
    // test's failure message point at the right defect.
    assert_eq!(
        run.status.code(),
        Some(0),
        "the host must not abort (134 was the defect): {}",
        stderr_of(&run)
    );
    assert_eq!(stdout_of(&run), "ok\n");
}

/// The second door of the same P1: an oversized *inline* length, where the
/// abort was inside the allocator rather than the decoder. See
/// `a_bigint_length_raises_overflow_error_instead_of_aborting_the_host` for
/// the full account. `RuntimeError` rather than CPython's `MemoryError`
/// because this runtime carries no `MemoryError` tag to name.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_unallocatable_length_raises_instead_of_aborting_the_host() {
    let dir = fixture("1165_hosted_oversized", SUBJECT);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = run_hosted(
        &dir,
        "import alloc_probe\n\
         assert alloc_probe.build_and_sum(4) == 8.0, alloc_probe.build_and_sum(4)\n\
         try:\n\
         \x20   alloc_probe.build_and_sum(2 ** 62 - 1)\n\
         except RuntimeError as error:\n\
         \x20   assert 'cannot be allocated' in str(error), str(error)\n\
         else:\n\
         \x20   raise AssertionError('an unallocatable length returned normally')\n\
         print('ok')\n",
    );
    assert_eq!(
        run.status.code(),
        Some(0),
        "the host must not abort (134 was the defect): {}",
        stderr_of(&run)
    );
    assert_eq!(stdout_of(&run), "ok\n");
}

/// #1166 review finding F5, corroborated on a real artifact: an allocation
/// reached with an exception *already* pending must not happen at all.
///
/// `xs.pop()` on an empty list raises, and `MirExpr::ListPop` is in
/// `expression_can_set_exception`'s `false` group, so nothing guarded the
/// pending state between that raise and the allocator call. The allocation
/// succeeded and `emit_expr`'s own post-call guard then branched to the
/// handler before `MirStmt::Assign` could store the pointer into the frame's
/// owned slot, leaving the slot at its entry null: one leaked view per call,
/// which the balance counter reports and no other observation in this file
/// would notice.
///
/// `#[ignore]`d, and the codegen-side proof is
/// `pycc_codegen`'s own `a_buffer_allocation_checks_the_pending_state_before_it_allocates`,
/// which runs in CI's coverage job; this arm is corroboration on the built
/// module, not the primary gate. Windows is excluded for the reason the arm
/// below states.
#[test]
#[cfg(not(target_os = "windows"))]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_allocation_reached_with_a_pending_exception_never_happens() {
    let dir = fixture(
        "1165_hosted_stale_pending",
        "\
def stale_pending(n: int) -> float:
    xs: list[int] = []
    try:
        a = ndarray(n)
        b = ndarray(xs.pop())
        return a[0] + b[0]
    except IndexError:
        return -1.0
",
    );
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = run_hosted(
        &dir,
        "import ctypes, alloc_probe\n\
         live = ctypes.CDLL(alloc_probe.__file__).pycc_rt_buffer_live_views\n\
         live.restype = ctypes.c_longlong\n\
         live.argtypes = []\n\
         assert live() == 0, live()\n\
         for _ in range(64):\n\
         \x20   assert alloc_probe.stale_pending(8) == -1.0\n\
         assert live() == 0, live()\n\
         print('ok')\n",
    );
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}

/// The leak arm. `pycc_rt_buffer_live_views` is the allocator pair's own
/// balance counter, linked into the module as part of the `pycc_rt`
/// staticlib and read back through `ctypes`. It must be zero before any
/// call, zero after an ordinary call, zero after the reallocating call --
/// which frees the first view before storing the second (D-074) -- and
/// zero after a call that raised, since the failed allocation returned
/// null and the epilogue must skip it rather than free it.
///
/// The last arm is the one the exception-exit routing actually exists for:
/// `alloc_then_raise` allocates successfully, *then* raises on a second
/// allocation, so the frame leaves through its exception exit with one
/// **live** view rather than a null one. The `build_and_sum(-1)` arm above
/// cannot prove that -- its allocator returned null before `fetch_add`, so
/// an epilogue that freed nothing at all would still show a zero balance.
///
/// Asserting a *balance* rather than watching memory is the point: a frame
/// that never freed would show identical behavior on every other
/// observation in this file.
///
/// Not compiled on Windows, matching `issue_1054_ext_str_release.rs`'s
/// file-level gate and for exactly its reason: the counter is linked into
/// the `.pyd` from a static archive but is absent from its export table,
/// and opening a second shared `pycc_rt` would carry its own `BUFFER_LIVE`
/// and prove nothing. The property is codegen plus runtime bookkeeping and
/// is not platform-specific -- the `pycc_codegen` epilogue tests carry it
/// on every platform, Windows included.
#[test]
#[cfg(not(target_os = "windows"))]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn no_allocation_outlives_the_call_that_made_it() {
    let dir = fixture("1165_hosted_live_views", SUBJECT);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = run_hosted(
        &dir,
        "import ctypes, alloc_probe\n\
         live = ctypes.CDLL(alloc_probe.__file__).pycc_rt_buffer_live_views\n\
         live.restype = ctypes.c_longlong\n\
         live.argtypes = []\n\
         assert live() == 0, live()\n\
         for _ in range(64):\n\
         \x20   alloc_probe.build_and_sum(16)\n\
         assert live() == 0, live()\n\
         for _ in range(64):\n\
         \x20   alloc_probe.rebuild(16)\n\
         assert live() == 0, live()\n\
         for _ in range(64):\n\
         \x20   try:\n\
         \x20       alloc_probe.build_and_sum(-1)\n\
         \x20   except ValueError:\n\
         \x20       pass\n\
         assert live() == 0, live()\n\
         for _ in range(64):\n\
         \x20   try:\n\
         \x20       alloc_probe.alloc_then_raise(16)\n\
         \x20   except ValueError:\n\
         \x20       pass\n\
         \x20   else:\n\
         \x20       raise AssertionError('alloc_then_raise returned normally')\n\
         assert live() == 0, live()\n\
         print('ok')\n",
    );
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}

/// The #1166 round 9 review's reassign-then-refused-allocation arm, on a
/// real artifact. `a = ndarray(n)` then `a = ndarray(-1)`: the slot holds a
/// **live** view when the second allocation is refused, so the frame leaves
/// through its exception exit owing exactly one free. A free emitted before
/// the refusal branched away would make that two, and a double free is not
/// something the balance counter alone would report -- so the arm asserts
/// both that the process survives 64 such calls and that the balance returns
/// to zero.
///
/// The primary gate is the codegen-side
/// `a_refused_reallocation_never_reaches_the_free_before_its_own_store`,
/// which pins the block separation itself and runs in CI's coverage job;
/// this is corroboration on the built module. Windows is excluded for the
/// reason `no_allocation_outlives_the_call_that_made_it` states.
#[test]
#[cfg(not(target_os = "windows"))]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_refused_reallocation_leaves_exactly_one_view_for_the_exception_exit() {
    let dir = fixture("1165_hosted_refused_realloc", SUBJECT_REALLOC_THEN_RAISE);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = run_hosted(
        &dir,
        "import ctypes, alloc_probe\n\
         live = ctypes.CDLL(alloc_probe.__file__).pycc_rt_buffer_live_views\n\
         live.restype = ctypes.c_longlong\n\
         live.argtypes = []\n\
         assert live() == 0, live()\n\
         for _ in range(64):\n\
         \x20   try:\n\
         \x20       alloc_probe.realloc_then_raise(8)\n\
         \x20   except ValueError:\n\
         \x20       pass\n\
         \x20   else:\n\
         \x20       raise AssertionError('realloc_then_raise returned normally')\n\
         assert live() == 0, live()\n\
         print('ok')\n",
    );
    // The status explicitly, not `success()`'s boolean: a double free aborts,
    // and naming the expectation is what makes the failure point at it.
    assert_eq!(
        run.status.code(),
        Some(0),
        "the host must not abort: {}",
        stderr_of(&run)
    );
    assert_eq!(stdout_of(&run), "ok\n");
}

/// The subject for the round-11 leak arm below: two exports that each raise
/// exactly once per call, differing *only* in whether the refused length was
/// a freshly owned bigint. `promoted` reaches the decoder's `OverflowError`
/// through `n + 1`; `negative` reaches the allocator's `ValueError` with an
/// ordinary inline length and no bigint anywhere in it.
#[cfg(unix)]
const SUBJECT_LEAK_PROBE: &str = "\
def promoted(n: int) -> float:
    a = ndarray(n + 1)
    return float(len(a))


def negative(n: int) -> float:
    a = ndarray(n)
    return float(len(a))
";

/// Runs `export(argument)` in a loop, catching the refusal every trip, and
/// returns the host interpreter's own peak resident set together with the
/// number of exceptions it actually caught.
///
/// The measurement is taken *inside* CPython, with `resource.getrusage`,
/// rather than by reaping the child with `wait4` the way
/// `tests/issue_146_bigint_release.rs`'s `peak_rss` module does. Both read
/// the same `ru_maxrss` field; doing it in-process keeps this arm inside
/// this file's existing `run_hosted` convention and needs no `libc`
/// dependency here. `resource` is Unix-only, which is why the whole arm is
/// `#[cfg(unix)]`.
#[cfg(unix)]
fn hosted_peak_rss(dir: &Path, export: &str, argument: &str, trips: u32) -> (f64, u32) {
    let run = run_hosted(
        dir,
        &format!(
            "import resource, alloc_probe\n\
             caught = 0\n\
             for _ in range({trips}):\n\
             \x20   try:\n\
             \x20       alloc_probe.{export}({argument})\n\
             \x20   except (OverflowError, ValueError):\n\
             \x20       caught += 1\n\
             print(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss)\n\
             print(caught)\n"
        ),
    );
    assert!(run.status.success(), "{}", stderr_of(&run));
    let out = stdout_of(&run);
    let mut lines = out.lines();
    let rss: f64 = lines
        .next()
        .unwrap_or_else(|| panic!("the probe should print a peak RSS: {out}"))
        .trim()
        .parse()
        .unwrap_or_else(|error| panic!("peak RSS should parse ({error}): {out}"));
    let caught: u32 = lines
        .next()
        .unwrap_or_else(|| panic!("the probe should print a catch count: {out}"))
        .trim()
        .parse()
        .unwrap_or_else(|error| panic!("the catch count should parse ({error}): {out}"));
    (rss, caught)
}

/// #1166 round 11's P1, on a real artifact: repeatedly catching the refused
/// length does not grow the host process by one `BigIntObj` per call.
///
/// A single catch leaks a single object, which no "raises `OverflowError`"
/// assertion can see -- so this loops, and reads the one observable this
/// repository already uses for a bigint leak (`tests/issue_146_bigint_release.rs`'s
/// peak-RSS module; there is no `pycc_rt_bigint_live_objects` counter to read
/// the way the buffer arms read `pycc_rt_buffer_live_views`).
///
/// Stated as a ratio against a *second shape* rather than against a second
/// trip count, which is the departure from #146's own form and the thing
/// that makes the arm meaningful here. Every raise out of an `ext` export
/// leaks its own exception machinery today -- an enumerated D-181 residual,
/// measured at roughly 160 bytes per catch and unrelated to this fix -- so a
/// same-shape 1x/2x ratio reads ~1.8 whether or not the bigint is retired.
/// `negative` raises exactly as often through the same wrapper and the same
/// pending-exception protocol, with no bigint in it, so that residual is
/// present in both readings and cancels; what remains is the length's own
/// birth reference.
///
/// Measured on the fix's own branch: 105.4MB against 79.5MB (ratio 1.33)
/// before the release call, 86.2MB against 79.6MB (ratio 1.08) after it.
/// The threshold sits between them with margin on both sides. The catch
/// counts are asserted too, so a future change that stopped raising at all
/// could not pass this by making both loops cheap.
#[test]
#[cfg(unix)]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn repeatedly_catching_a_refused_bigint_length_does_not_leak_one_per_call() {
    const TRIPS: u32 = 400_000;
    let dir = fixture("1165_hosted_len_leak", SUBJECT_LEAK_PROBE);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let (leaking_shape, caught) = hosted_peak_rss(&dir, "promoted", "2 ** 62 - 1", TRIPS);
    assert_eq!(caught, TRIPS, "every trip must raise");
    let (control, control_caught) = hosted_peak_rss(&dir, "negative", "-1", TRIPS);
    assert_eq!(control_caught, TRIPS, "every control trip must raise");

    let ratio = leaking_shape / control;
    assert!(
        ratio < 1.20,
        "catching a refused bigint length must not cost a live object per call: \
         promoted={leaking_shape} control={control} ratio={ratio:.4} \
         (a leaked `BigIntObj` per catch reads as ratio ~1.33)"
    );
}

/// #1289 on a real artifact: a buffer allocated by the `try` body and again
/// by the handler that catches the body's refused allocation is definitely
/// bound after the statement, so the checker admits reading it, and the
/// frame still frees exactly the one view each path leaves live. `n = -1`
/// takes the handler path (the body's allocation is refused before it
/// stores anything); `n = 8` takes the body path.
///
/// Corroboration on the built module; the admission itself is pinned in CI
/// by `pycc_types`' `a_definitely_assigned_owned_buffer_is_still_admitted_on_every_join_form`.
/// Windows is excluded for the reason
/// `no_allocation_outlives_the_call_that_made_it` states.
#[test]
#[cfg(not(target_os = "windows"))]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_buffer_bound_by_the_try_body_and_its_handler_is_read_and_freed() {
    let dir = fixture(
        "1289_hosted_try_handler_alloc",
        "\
def try_alloc(n: int) -> float:
    try:
        a = ndarray(n)
    except ValueError:
        a = ndarray(2)
    return float(len(a))
",
    );
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = run_hosted(
        &dir,
        "import ctypes, alloc_probe\n\
         live = ctypes.CDLL(alloc_probe.__file__).pycc_rt_buffer_live_views\n\
         live.restype = ctypes.c_longlong\n\
         live.argtypes = []\n\
         assert live() == 0, live()\n\
         for _ in range(64):\n\
         \x20   assert alloc_probe.try_alloc(8) == 8.0\n\
         \x20   assert alloc_probe.try_alloc(-1) == 2.0\n\
         assert live() == 0, live()\n\
         print('ok')\n",
    );
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), "ok\n");
}
