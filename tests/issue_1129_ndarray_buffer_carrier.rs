//! #1129: the bare name `ndarray` as a second spelling of the `ext`
//! boundary's buffer carrier, and the widened admission predicate that
//! makes a bare array reach it.
//!
//! Two halves, and they are one change: the annotation arm alone would
//! compile a signature that fails at run time for every real array, and the
//! run-time widening alone would be unreachable from any source program.
//!
//! What lives here rather than in `tests/issue_1114_numpy_oracle.rs` is
//! exactly what that file cannot carry. It is numpy's oracle, so every
//! assertion in it skips on a host without numpy; the annotation arm and
//! the buffer-protocol arms must be stated on a host that has never heard
//! of numpy, because the widened predicate is the *buffer protocol* and
//! not numpy — an `array.array('d')` from the standard library is as much
//! a conforming operand as an `ndarray` is, and CI installs numpy into one
//! job only (`native-build-test`'s hosted ext floor interpreter).
//!
//! The scope boundary, so a later reader does not over-read this file:
//! #1129 owns *carrier registration* — which pycc type the boundary admits
//! and what the wrapper does with the object. How a user may *spell* a
//! type is owned elsewhere: attribute-qualified `numpy.ndarray` by #889,
//! import aliasing by #883/#963/#964. Neither is advanced here, and no
//! numpy source file in the wild is made compilable by this change on its
//! own. The *subscripted* spelling was owned by #1130 and has since landed:
//! three tests in this file assert the carrier's own subscripted form, and
//! the rest of #1130's surface lives in
//! `tests/issue_1130_subscripted_annotations.rs`.
//!
//! The nine tests below that need no interpreter at all — every refusal
//! and acceptance they assert is resolved on the program before `plan_ext`
//! probes the host toolchain — are not `#[ignore]`d, and they are what runs
//! inside the coverage job. Only the tenth, which builds an artifact and
//! loads it into a live interpreter, is hosted.

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
    std::fs::write(dir.join("nd_probe.py"), source).expect("write the subject");
    dir
}

/// Builds the entry module as a CPython extension module directly into
/// `dir`, so a CPython run with `dir` as its working directory imports it.
///
/// The output path carries no extension suffix, for the reason
/// `tests/issue_1114_numpy_oracle.rs`'s own helper records: `pycc build
/// --ext` appends the one its target triple calls for and derives the
/// exported `PyInit_<mod>` name from the path's own spelling.
fn build_ext(dir: &Path) -> Output {
    pycc()
        .arg("build")
        .arg(dir.join("nd_probe.py"))
        .arg("-o")
        .arg(dir.join("nd_probe"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn")
}

/// The subject: the same loop the buffer carrier has always admitted, at a
/// parameter spelled `ndarray` instead of `memoryview`.
///
/// `len(b)` and `b[i]` are both in the body deliberately. Neither is a new
/// capability (#1116 and Part 2 of #1027 own them) and neither branches on
/// the spelling — the constraint solver's two interceptions dispatch on
/// `Ty::MemoryView`, which is what `ndarray` lowers to — but "it should
/// work for free" is a claim, so the subject exercises it rather than
/// assuming it.
const SUBJECT: &str = "\
def total(b: ndarray) -> float:
    s: float = 0.0
    i: int = 0
    for i in range(len(b)):
        s = s + b[i]
    return s
";

/// A program that binds `ndarray` itself keeps its own meaning for the
/// name, because the spelling is resolved *after* `class_defs` and the
/// alias table rather than beside `memoryview` in the keyword list.
///
/// This is the rule the placement exists for, not a corner case. Every
/// other name that list reserves is a Python builtin or a `typing` name;
/// `ndarray` is an ordinary identifier, and in Python a module-level
/// definition shadows an imported name rather than losing to it. Reserving
/// it first was measured to refuse both programs below, each of which
/// compiles clean before the spelling exists and must keep doing so after.
#[test]
fn a_program_that_binds_ndarray_itself_keeps_its_own_meaning() {
    // A class of that name: the parameter is an instance of it, so reading
    // the parameter is an ordinary read and not the buffer capability gap.
    let cls = fixture(
        "1129_shadow_class",
        "class ndarray:\n    pass\n\ndef g(a: ndarray) -> int:\n    b = a\n    return 1\n",
    );
    let out = pycc()
        .arg("check")
        .arg(cls.join("nd_probe.py"))
        .output()
        .expect("pycc should spawn");
    assert!(
        out.status.success(),
        "a user-defined `class ndarray` must win over the buffer spelling: {}{}",
        stdout_of(&out),
        stderr_of(&out)
    );

    // An alias of that name, which resolves one link later in the same
    // chain: `a` is an `int`, so arithmetic on it type-checks.
    let alias = fixture(
        "1129_shadow_alias",
        "type ndarray = int\n\ndef g(a: ndarray) -> int:\n    return a + 1\n",
    );
    let out = pycc()
        .arg("check")
        .arg(alias.join("nd_probe.py"))
        .output()
        .expect("pycc should spawn");
    assert!(
        out.status.success(),
        "a user-defined `type ndarray` alias must win over the buffer spelling: {}{}",
        stdout_of(&out),
        stderr_of(&out)
    );
}

/// The subscripted form lowers to the same carrier the bare form does.
///
/// #1130 admits a subscripted annotation whose base resolves to a type pycc
/// can name nominally, and the buffer carrier is one. This test used to
/// assert a refusal with a per-spelling noun; it now asserts the acceptance,
/// which is D-244 statement (b) -- the spellings are one pycc type with
/// no run-time observable difference -- restated for the subscripted form.
/// The type argument is erased and never lowered, so `[float]` is not
/// modelled and not checked.
///
/// The acceptance is proved by the *use*, not by the exit code alone: only
/// the carrier supports `a[0]` yielding `float`, so a control arm returning
/// that element as `int` must be the `T0022` mismatch. Without the control,
/// an accept caused by resolving to something else entirely would pass.
#[test]
fn a_subscripted_carrier_spelling_lowers_to_the_carrier_itself() {
    for spelling in ["ndarray", "memoryview"] {
        let dir = fixture(
            "1130_subscript_carrier",
            &format!("def f(a: {spelling}[float]) -> float:\n    return a[0]\n"),
        );
        let out = pycc()
            .arg("check")
            .arg(dir.join("nd_probe.py"))
            .output()
            .expect("pycc should spawn");
        // `check` renders on stdout where `build` renders on stderr, so
        // both are read rather than guessing which one this subcommand
        // uses.
        let err = format!("{}{}", stdout_of(&out), stderr_of(&out));
        assert!(out.status.success(), "{err}");

        let control = fixture(
            "1130_subscript_carrier_control",
            &format!("def f(a: {spelling}[float]) -> int:\n    return a[0]\n"),
        );
        let out = pycc()
            .arg("check")
            .arg(control.join("nd_probe.py"))
            .output()
            .expect("pycc should spawn");
        assert!(!out.status.success(), "{}", stdout_of(&out));
        let err = format!("{}{}", stdout_of(&out), stderr_of(&out));
        assert!(err.contains("error[T0022]"), "{err}");
        assert!(
            err.contains("return type mismatch: expected `int`, found `float`"),
            "{err}"
        );
    }
}

/// A container of the carrier lowers its element like any other, and is
/// then refused by the container capability gate rather than by name
/// resolution.
///
/// This is the shape `src/memoryview_mode.rs`'s `offending_position` relies
/// on when it compares `*ty == Ty::MemoryView` flatly instead of recursing:
/// `list[ndarray]` really does lower to `Ty::List(Ty::MemoryView)`, and what
/// keeps it out of a lowered function's parameters is `check_container_ty`'s
/// `T0034`, which admits only `Ty::Int` as a list element. Pinning it here
/// means a later widening of that gate breaks this test rather than silently
/// making the flat comparison miss a nested carrier.
///
/// It also states the canonical-rendering rule at one more surface: the
/// `ndarray` spelling is echoed back as `memoryview`, because the message
/// renders the resolved type rather than naming the carrier as its reason.
#[test]
fn a_container_of_the_carrier_is_refused_by_the_container_gate() {
    for spelling in ["ndarray", "memoryview"] {
        let dir = fixture(
            "1129_container",
            &format!("def f(xs: list[{spelling}]) -> int:\n    return 1\n"),
        );
        let out = pycc()
            .arg("check")
            .arg(dir.join("nd_probe.py"))
            .output()
            .expect("pycc should spawn");
        assert!(!out.status.success(), "{}", stdout_of(&out));
        let err = format!("{}{}", stdout_of(&out), stderr_of(&out));
        assert!(err.contains("error[T0034]"), "{err}");
        assert!(
            err.contains("list[memoryview] is not compiled yet (D-105) -- only list[int] is"),
            "{err}"
        );
    }
}

#[test]
fn a_shadowed_spelling_is_named_by_whichever_binding_actually_wins() {
    // `memoryview` is a reserved keyword the `Expr::Name` arm decides before
    // it reads the alias table, so `type memoryview = int` does not win and
    // the subscripted form reaches the carrier -- which #1130 now accepts,
    // where it used to be the `buffer type` refusal. Proved by the element
    // read: only the carrier yields `float` from `a[0]`.
    let dir = fixture(
        "1130_subscript_shadow_memoryview",
        "type memoryview = int\n\ndef f(a: memoryview[float]) -> float:\n    return a[0]\n",
    );
    let out = pycc()
        .arg("check")
        .arg(dir.join("nd_probe.py"))
        .output()
        .expect("pycc should spawn");
    let err = format!("{}{}", stdout_of(&out), stderr_of(&out));
    assert!(out.status.success(), "{err}");

    // `ndarray` is an ordinary identifier resolved only after the alias
    // table, so there the alias really does win, `ndarray` means `int`, and
    // a subscript on a scalar alias keeps #931's refusal with the truthful
    // noun. This arm is unchanged by #1130.
    let dir = fixture(
        "1129_subscript_shadow",
        "type ndarray = int\n\ndef f(a: ndarray[float]) -> int:\n    return 1\n",
    );
    let out = pycc()
        .arg("check")
        .arg(dir.join("nd_probe.py"))
        .output()
        .expect("pycc should spawn");
    assert!(!out.status.success(), "{}", stdout_of(&out));
    let err = format!("{}{}", stdout_of(&out), stderr_of(&out));
    assert!(err.contains("error[T0044]"), "{err}");
    assert!(
        err.contains("type alias `ndarray` is not subscriptable"),
        "{err}"
    );
}

/// The same precedence holds when the shadowing alias targets a class.
///
/// With `type memoryview = C` the reserved keyword still wins and the
/// subscript is the buffer carrier, not `C`; with `type ndarray = C` the
/// alias wins and the subscript is `C`. #1130 **strengthens** D-244
/// statement (k) here rather than weakening it: before this change both arms
/// were refusals, so the only witness (k) had was a pair of
/// differently-worded rejections, and a pair of nouns is weak evidence about
/// which binding the pipeline actually resolved. Both arms are now
/// acceptances, so each is pinned by a *use* that only its own winner
/// supports -- `a[0]` yielding `float` for the carrier, `a.m()` for the
/// class -- and each arm additionally asserts that the other winner's use is
/// refused. That is four assertions where there used to be two nouns.
#[test]
fn an_alias_to_a_class_wins_for_ndarray_and_loses_to_memoryview() {
    const CLASS: &str = "class C:\n    def __init__(self) -> None:\n        self.v = 1\n\n    def m(self) -> int:\n        return self.v\n\n";

    // `memoryview`: the keyword wins, so the parameter is the carrier.
    let dir = fixture(
        "1130_subscript_class_alias_memoryview",
        &format!(
            "{CLASS}type memoryview = C\n\ndef f(a: memoryview[int]) -> float:\n    return a[0]\n"
        ),
    );
    let out = pycc()
        .arg("check")
        .arg(dir.join("nd_probe.py"))
        .output()
        .expect("pycc should spawn");
    let err = format!("{}{}", stdout_of(&out), stderr_of(&out));
    assert!(out.status.success(), "{err}");
    // ...and it is *not* `C`: a member read only `C` supports is refused.
    let dir = fixture(
        "1130_subscript_class_alias_memoryview_control",
        &format!(
            "{CLASS}type memoryview = C\n\ndef f(a: memoryview[int]) -> int:\n    return a.m()\n"
        ),
    );
    let out = pycc()
        .arg("check")
        .arg(dir.join("nd_probe.py"))
        .output()
        .expect("pycc should spawn");
    assert!(!out.status.success(), "{}", stdout_of(&out));

    // `ndarray`: the alias wins, so the parameter is `C`.
    let dir = fixture(
        "1130_subscript_class_alias_ndarray",
        &format!("{CLASS}type ndarray = C\n\ndef f(a: ndarray[int]) -> int:\n    return a.m()\n"),
    );
    let out = pycc()
        .arg("check")
        .arg(dir.join("nd_probe.py"))
        .output()
        .expect("pycc should spawn");
    let err = format!("{}{}", stdout_of(&out), stderr_of(&out));
    assert!(out.status.success(), "{err}");
    // ...and it is *not* the carrier: an element read the carrier supports
    // is refused because `C` does not support indexing.
    let dir = fixture(
        "1130_subscript_class_alias_ndarray_control",
        &format!("{CLASS}type ndarray = C\n\ndef f(a: ndarray[int]) -> float:\n    return a[0]\n"),
    );
    let out = pycc()
        .arg("check")
        .arg(dir.join("nd_probe.py"))
        .output()
        .expect("pycc should spawn");
    assert!(!out.status.success(), "{}", stdout_of(&out));
    let err = format!("{}{}", stdout_of(&out), stderr_of(&out));
    assert!(err.contains("error[T0033]"), "{err}");
}

/// The two protocol-member refusals name the thing, not any one spelling.
///
/// They are capability gaps of the same family as `ext_return_gap`: nothing
/// produces a buffer, so no class could satisfy the member. Both fired on
/// `Ty::MemoryView` and so already covered the second spelling, but their
/// text was a hard-coded `memoryview` and their authority a bare "Part 1 of
/// #1027" -- so a program that wrote `ndarray` was told about a type it
/// never wrote, and cited a change that is no longer the whole rule.
#[test]
fn a_protocol_member_spelled_ndarray_is_refused_without_naming_a_spelling() {
    for (name, source, expected) in [
        (
            "1129_protocol_ret",
            "from typing import Protocol\n\n\nclass P(Protocol):\n    def m(self) -> ndarray: ...\n\n\ndef f() -> int:\n    return 1\n",
            "protocol method `P.m` returns a buffer",
        ),
        (
            "1129_protocol_attr",
            "from typing import Protocol\n\n\nclass P(Protocol):\n    x: ndarray\n\n\ndef f() -> int:\n    return 1\n",
            "protocol attribute `P.x` has a buffer type",
        ),
    ] {
        let dir = fixture(name, source);
        let out = pycc()
            .arg("check")
            .arg(dir.join("nd_probe.py"))
            .output()
            .expect("pycc should spawn");
        assert!(!out.status.success(), "{name}: {}", stdout_of(&out));
        let err = format!("{}{}", stdout_of(&out), stderr_of(&out));
        assert!(err.contains("error[C0001]"), "{name}: {err}");
        assert!(err.contains(expected), "{name}: {err}");
        assert!(err.contains("#1027 and #1129"), "{name}: {err}");
    }
}

/// `pycc explain I0405` is the code's second normative surface, and a user
/// who hit it on a signature spelled anything other than `memoryview` must
/// be told that spelling exists. The explanation names every source
/// spelling the carrier currently has, which is why this asserts the whole
/// set rather than the one #1129 added: a fourth spelling registered
/// without extending the explanation fails here.
#[test]
fn explain_i0405_names_every_source_spelling() {
    let out = pycc()
        .arg("explain")
        .arg("I0405")
        .output()
        .expect("pycc should spawn");
    assert!(out.status.success(), "{}", stderr_of(&out));
    let text = stdout_of(&out);
    assert!(text.contains("ndarray"), "{text}");
    assert!(text.contains("memoryview"), "{text}");
    assert!(text.contains("NDArray"), "{text}");
}

/// The annotation compiles, with no interpreter, no numpy, and no import.
///
/// The one assertion in this file that depends on nothing about the host
/// at all. `import numpy` builds only from a `pycc.lock` closure (#1242),
/// so a spelling that required one would need numpy installed; the bare
/// name is recognized
/// without it, exactly as `Any`, `Annotated`, `TypeAlias` and `Self` are.
#[test]
fn an_ndarray_parameter_type_checks_without_any_import() {
    let dir = fixture("1129_check", SUBJECT);
    let check = pycc()
        .arg("check")
        .arg(dir.join("nd_probe.py"))
        .output()
        .expect("pycc should spawn");
    assert!(
        check.status.success(),
        "{}{}",
        stdout_of(&check),
        stderr_of(&check)
    );
}

/// Every position that is *not* an `--ext` parameter is refused for the new
/// spelling exactly as it is for the old one.
///
/// This is the half of the change that is deliberately *not* a widening:
/// `ndarray` lowers to `Ty::MemoryView`, so it inherits every existing
/// refusal rather than opening a second, laxer path to them.
///
/// Two message styles, and which one a site uses is a deliberate split
/// rather than an inconsistency. A message that names the *type* is
/// spelling-neutral ("a buffer"), because a user who wrote `ndarray` must
/// not be told about a `memoryview` they never mentioned; a message that
/// quotes a whole *signature position* back (`I0405`, and `C0003`'s
/// `-> memoryview`) renders the canonical spelling, because a `Ty` is the
/// compiler's canonical name for a type -- the same thing already happens
/// for `type Arr = memoryview`. #1129's D-244 amendment records the split
/// rather than leaving it to be discovered. All four arms are asserted
/// here so a later reword cannot quietly move a site from one style to the
/// other.
#[test]
fn every_non_parameter_ndarray_position_is_refused() {
    // A bare declaration: `C0001`, in both modes, because nothing produces
    // a buffer value to bind to the name. The message is the reworded,
    // spelling-neutral one — a user who wrote `ndarray` must not be told
    // about a `memoryview` they never mentioned.
    let dir = fixture(
        "1129_decl",
        "def f() -> int:\n    x: ndarray\n    return 1\n",
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

    // A public `-> ndarray` return under `--ext` is *admitted* since Part 2b
    // of #1142 (#1164), so what refuses this program is its body: an
    // intra-artifact call to a buffer-returning function has no wrapper to
    // own the result. Two-directional, so a regression that reinstates the
    // old `C0003` gap cannot pass as "still refused".
    let dir = fixture("1129_return", "def make() -> ndarray:\n    return make()\n");
    let ext = build_ext(&dir);
    assert!(!ext.status.success(), "{}", stdout_of(&ext));
    let err = stderr_of(&ext);
    assert!(err.contains("error[C0001]"), "{err}");
    assert!(
        err.contains("calling `make`, whose return type is a buffer"),
        "{err}"
    );
    assert!(!err.contains("error[C0003]"), "{err}");

    // The `C0003` remediation still enumerates what the boundary carries,
    // so it still has to name every spelling: a user who reached the message
    // by writing `ndarray` or `NDArray` and is shown a list without it reads
    // the list as "not that type at all". Reached through an uncarriable
    // *parameter* now that the return position is carried.
    let dir = fixture(
        "1129_gap_list",
        "def take(v: list[int]) -> int:\n    return 0\n",
    );
    let ext = build_ext(&dir);
    assert!(!ext.status.success(), "{}", stdout_of(&ext));
    let err = stderr_of(&ext);
    assert!(err.contains("error[C0003]"), "{err}");
    assert!(
        err.contains("(or its other spellings `ndarray` and `NDArray`)"),
        "{err}"
    );

    // A *private* `-> ndarray` return under `--ext`: `C0001`, from the
    // separate walk that closes what `collect_exports` never visits. The
    // public arm above cannot reach this site at all -- `collect_exports`
    // answers a public offender with `C0003` first -- which is exactly why
    // this one is stated: the site was left spelling-pinned and citing
    // only #1027 while its two documented siblings in `crates/pycc_types`
    // were reworded, and no existing assertion would have caught it.
    let dir = fixture(
        "1129_private_return",
        "def _f() -> ndarray:\n    a = ndarray(4)\n    return a\n",
    );
    let ext = build_ext(&dir);
    assert!(!ext.status.success(), "{}", stdout_of(&ext));
    let err = stderr_of(&ext);
    assert!(err.contains("error[C0001]"), "{err}");
    assert!(err.contains("`_f`'s return type is a buffer"), "{err}");
    // This used to pin `#1129` as the proxy for "kept in step with its two
    // siblings in `crates/pycc_types`". Part 2a of #1142 (#1165) retired the
    // whole issue list from this message: `ndarray(n)` made its old tail --
    // "no expression produces one to return" -- false, and a fresh list
    // would go stale again the same way when egress lands in Part 2b
    // (#1164). The two properties the citation was standing in for are
    // asserted directly instead: the refusal still names the artifact mode
    // it is scoped to, and it no longer claims nothing can produce a buffer.
    assert!(err.contains("pycc build --ext"), "{err}");
    assert!(
        !err.contains("no expression produces one to return"),
        "{err}"
    );
    // And the spelling the user did not write appears nowhere in the
    // diagnostic. Scoped to the `error[C0001]` line rather than asserted
    // over the whole stream: stderr also carries scratch paths and, on some
    // hosts, toolchain warnings, none of which this arm owns.
    let line = err
        .lines()
        .find(|line| line.contains("error[C0001]"))
        .expect("the C0001 line");
    assert!(!line.contains("memoryview"), "{line}");

    // An `ndarray` *signature* in a build without `--ext`: `I0405`, the
    // artifact-mode refusal, naming the parameter in the canonical
    // spelling.
    let dir = fixture("1129_native", SUBJECT);
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

/// The widened admission predicate, stated on the standard library alone.
///
/// The boundary's first refusal arm moved from `PyMemoryView_Check` to
/// `PyObject_CheckBuffer`, so what it admits is every conforming *buffer
/// exporter* and not numpy: an `array.array('d')` and a `memoryview` are
/// both accepted at a parameter spelled `ndarray`, a `bytes` passes arm 1
/// and is refused by the format arm, and only an object that exports
/// nothing at all is refused by arm 1 itself. That set is the deliberate
/// consequence of the predicate, not a test convenience — `docs/RUNTIME.md`
/// and D-244's #1129 amendment both state it in those terms.
///
/// Hosted, for the reason every `--ext` build-and-load test is: `--ext`
/// requires a CPython 3.13+ with development headers, and CI's coverage
/// interpreter is a different one.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_ndarray_parameter_admits_every_conforming_buffer_exporter() {
    let dir = fixture("1129_exporters", SUBJECT);
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(
            "import array, nd_probe\n\
             assert not nd_probe.__file__.endswith('.py'), nd_probe.__file__\n\
             data = array.array('d', [1.5, -2.25, 3.0])\n\
             # A conforming exporter that is not a `memoryview`: accepted\n\
             # bare, which is the whole statement of the widening.\n\
             assert nd_probe.total(data) == 2.25, nd_probe.total(data)\n\
             # And a `memoryview` over the same store, which the boundary\n\
             # accepted before this change and must still accept.\n\
             assert nd_probe.total(memoryview(data)) == 2.25\n\
             def refused(arg):\n\
             \x20   try:\n\
             \x20       nd_probe.total(arg)\n\
             \x20   except BaseException as error:\n\
             \x20       return type(error).__name__, str(error)\n\
             \x20   raise AssertionError('the thunk accepted %r' % (type(arg),))\n\
             # A buffer exporter of the wrong element type reaches the\n\
             # format arm now, not arm 1.\n\
             kind, text = refused(b'abcdefgh')\n\
             assert kind == 'TypeError', (kind, text)\n\
             assert \"format 'B'\" in text, text\n\
             # Exports no buffer at all: the one arm the widening did not\n\
             # move, in its reworded pycc-authored text.\n\
             kind, text = refused([1.5, 2.5])\n\
             assert kind == 'TypeError', (kind, text)\n\
             assert text == (\"total() argument 1: 'list' object \"\n\
             \x20                'does not export a buffer'), text\n\
             print('ok')\n",
        )
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    assert_eq!(stdout_of(&run), "ok\n");
}
