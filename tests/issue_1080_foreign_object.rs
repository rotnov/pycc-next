//! Part 1 of #1026 (PR 1c of #1080): what a CPython `import` binds, and what
//! the compiler refuses to do with it.
//!
//! Three separable claims live here, and they are kept together because they
//! are the same user-visible story told at three levels:
//!
//! * `pycc check` *accepts* a plain foreign import -- it is a frontend-only
//!   pass (`docs/CLI_SPEC.md`) with no `--ext` flag to judge against, so a
//!   bound-but-unused module object is not an error there;
//! * a plain `pycc build` refuses an import it cannot embed with `I0403`
//!   -- here `tkinter`, which needs Tcl/Tk from outside the interpreter
//!   (D-248 rule 1); a standard-library import builds embedded instead
//!   (`tests/issue_1223_embedded_executable.rs`), and a third-party root
//!   such as `numpy` is bundled from `pycc.lock` or refused there, naming
//!   `pycc lock` (`tests/issue_1242_locked_closure.rs`);
//! * every operation on the bound name other than an attribute load is
//!   refused, and all but one of them with `I0404`. Part 2 of #1026 (#1081)
//!   changed *where* that refusal is decided -- from the read of the
//!   binding to each consuming site, so `numpy.pi` itself could be admitted
//!   -- but not which programs it refuses, which is why every row of the
//!   table below still holds. `crates/pycc_types/src/foreign.rs` documents
//!   the migration. PR 2a's review then bounded the admitted set by
//!   *position* as well: a read of a foreign object was admitted only in a
//!   module body, never inside a function body, and never above the
//!   `import` itself -- both were compile errors before this change and
//!   both had become run-time traps. #1316 later admitted the in-function
//!   read of a module-level foreign name, with a failure edge and a
//!   run-time `NameError` for a read before the import. The one exception
//!   is a general method call (`numpy.sqrt(2.0)`), which PR 2a refused as
//!   `T0043` rather than `I0404` because it added no `Ty::Object` branch ahead of
//!   `class::resolve_method_call`. **PR 2b of #1081 admits that shape**: the
//!   branch exists now and answers `Ty::Object`, so the program type-checks
//!   (`a_general_method_call_on_a_foreign_object_is_accepted`).
//!   `tests/issue_1081_foreign_method_call.rs` owns the rest of that
//!   capability -- what it refuses, and what it does in a real host.
//!
//! The hosted tests at the bottom are `#[ignore]`d and contribute no line
//! coverage (CI's coverage job runs `llvm-cov` without `--include-ignored`);
//! they are run by the Tier-1 `native-build-test` leg's
//! `cargo test --workspace -- --include-ignored`. Everything this change
//! needs *covered* is covered by the non-ignored tests above them and by the
//! unit tests in `src/foreign_import.rs`,
//! `crates/pycc_codegen/src/foreign_import.rs` and
//! `crates/pycc_types/src/foreign/tests.rs`.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// `path` spelled the way `pycc_diag::render_human` spells a diagnostic's
/// `-->` location line: with forward slashes on every platform, so a Windows
/// assertion compares against the rendered form rather than the platform
/// separator. A path interpolated into a diagnostic's *message body* is not
/// normalized that way, so assert those with `Path::display` instead.
fn rendered_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Writes `body` to `dir/m.py` and returns the path.
fn source(dir: &Path, body: &str) -> std::path::PathBuf {
    let src = dir.join("m.py");
    std::fs::write(&src, body).expect("write the fixture source");
    src
}

fn check(dir: &Path, body: &str) -> Output {
    pycc()
        .arg("check")
        .arg(source(dir, body))
        .output()
        .expect("pycc should spawn")
}

/// `pycc check` is frontend-only and mode-agnostic: it has no `--ext` flag,
/// so it cannot know whether the program will be built as an extension
/// module, and refusing a foreign import there would make it unusable for
/// every `--ext` project. The import binds, nothing reads the binding, and
/// there is nothing to report.
#[test]
fn a_bound_but_unused_foreign_import_is_accepted_by_check() {
    let dir = ScratchDir::new("foreign_check_ok").expect("scratch");
    let output = check(&dir, "import numpy\n");
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "");
}

/// Work item 11: the driver gate. A native executable embeds no CPython
/// interpreter, so an import a plain build cannot embed (here `tkinter`,
/// an excluded standard-library root, D-248 rule 1; a third-party root is
/// bundled from `pycc.lock` since #1242) has no meaning there and the
/// build refuses before codegen -- which is what lets
/// `crates/pycc_codegen/src/foreign_import.rs` ignore the item under
/// `!options.ext` rather than assert.
///
/// This is `I0403`'s end-to-end assertion. It deliberately has no fixture
/// under `tests/diagnostics/`: that harness invokes `pycc check`, which
/// (see above) accepts the program, so the plan's "a fixture under
/// `tests/diagnostics/`" is not constructible for this code.
#[test]
fn a_native_build_refuses_a_foreign_import_with_i0403() {
    let dir = ScratchDir::new("foreign_native_refused").expect("scratch");
    let output = pycc()
        .arg("build")
        .arg(source(&dir, "import tkinter\n"))
        .arg("-o")
        .arg(dir.join("m"))
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    // `build` renders diagnostics to stderr, where `check` renders to
    // stdout (`src/frontend.rs`'s `render_all`).
    let rendered = stderr_of(&output);
    assert!(rendered.contains("error[I0403]"), "{rendered}");
    assert!(rendered.contains("`import tkinter`"), "{rendered}");
    assert!(rendered.contains("--ext"), "{rendered}");
}

/// The multi-file shape of the same gate: the `import` lives in a
/// dependency, not in the file named on the command line, and the `I0403`
/// must name the dependency (PR 1c of #1080 review finding 2).
///
/// Both orders are exercised because the import's recorded *item* index is
/// the item count at the moment it lowered, so a trailing import in the
/// dependency records the same linked index as a leading import in the
/// entry -- `src/frontend.rs`'s `owner_of_import` keys on the import
/// table's own position instead, which has no such ambiguity. The third row
/// is the control: an import the entry itself wrote still belongs to the
/// entry.
#[test]
fn a_foreign_import_in_a_dependency_names_the_dependency_not_the_entry() {
    let rows = [
        // (dep.py, main.py, the file the diagnostic must name, its line)
        (
            "import tkinter\ndef f() -> int:\n    return 1\n",
            "from dep import f\nx = f()\n",
            "dep.py",
            1,
        ),
        // The dependency's import is *trailing*: its item index equals the
        // dependency's own end bound, the boundary an item-index join would
        // hand to the entry file instead.
        (
            "def f() -> int:\n    return 1\nimport tkinter\n",
            "from dep import f\nx = f()\n",
            "dep.py",
            3,
        ),
        // The entry's own import, at the same boundary index from the other
        // side.
        (
            "def f() -> int:\n    return 1\n",
            "from dep import f\nimport tkinter\nx = f()\n",
            "main.py",
            2,
        ),
    ];
    for (dep, entry, owner, line) in rows {
        let dir = ScratchDir::new("foreign_native_multifile").expect("scratch");
        std::fs::write(dir.join("dep.py"), dep).expect("write the dependency");
        let main = dir.join("main.py");
        std::fs::write(&main, entry).expect("write the entry");
        let output = pycc()
            .arg("build")
            .arg(&main)
            .arg("-o")
            .arg(dir.join("m"))
            .output()
            .expect("pycc should spawn");
        assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
        let rendered = stderr_of(&output);
        assert!(rendered.contains("error[I0403]"), "{rendered}");
        // `pycc_diag` renders every path with forward slashes
        // (`render_human`'s own `replace('\\\\', "/")`), so the expected
        // location is normalized the same way rather than spelled with the
        // platform separator -- the existing convention in
        // `tests/issue_941_enum_subclass.rs` and `tests/slice0.rs`.
        // The line is the import statement's own, carried on the binding
        // (review round 4 on #1080): every row's import sits at a different
        // line, so a span that reverted to `0` fails here as well.
        let located = format!("{}:{line}:1", rendered_path(&dir.join(owner)));
        assert!(rendered.contains(&located), "{owner}: {rendered}");
        let other = if owner == "dep.py" {
            "main.py"
        } else {
            "dep.py"
        };
        assert!(
            !rendered.contains(&format!("{}:", rendered_path(&dir.join(other)))),
            "{owner}: {rendered}"
        );
    }
}

/// A type error in the program is still reported instead of the `I0403`:
/// the native gate is computed before the type check (its indices have to
/// match the pre-monomorphization item list) but reported after it, so the
/// ordering the single-file gate had is unchanged.
#[test]
fn a_type_error_is_reported_before_the_native_foreign_refusal() {
    let dir = ScratchDir::new("foreign_native_type_error").expect("scratch");
    let output = pycc()
        .arg("build")
        .arg(source(&dir, "import tkinter\n\nx: int = \"s\"\n"))
        .arg("-o")
        .arg(dir.join("m"))
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stderr_of(&output);
    assert!(!rendered.contains("I0403"), "{rendered}");
}

/// Finding 1 of the same review: a private helper that returns the bound
/// module object must never get a `T0021` telling the user to add a return
/// annotation. The helper is unannotated, and its return needs no
/// annotation to be typed (since Part 1 of #1367 a `-> <module>` one would
/// also spell it), so the solver's `Name` arm hands back the concrete
/// `Ty::Object` term. The check phase refused the `return`
/// with `I0404` until Part 1 of #1333 admitted returning a CPython object;
/// the program now type-checks, and the term is what makes that possible.
#[test]
fn an_unannotated_helper_returning_a_foreign_module_type_checks_without_t0021() {
    let dir = ScratchDir::new("foreign_helper_return").expect("scratch");
    let output = check(
        &dir,
        "import numpy\n\ndef _helper():\n    return numpy\n\nx = _helper()\n",
    );
    let rendered = stdout_of(&output);
    assert!(!rendered.contains("T0021"), "{rendered}");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{rendered}{}",
        stderr_of(&output)
    );
}

/// Part 2 of #1026 (#1081): the same helper, returning an *attribute* of
/// the module rather than the module itself.
///
/// The solver's `AttrGet` term types `numpy.pi` as `object` exactly as its
/// `Name` term types `numpy`, which is what keeps the diagnostic right:
/// without the term, signature materialization reports the `T0021` this
/// test rules out. Which pass reports the refusal has moved twice: PR 2a
/// of #1081 refused the in-function read itself, and #1316 admits the read
/// but refused the helper's `return` of a CPython object, and Part 1 of
/// #1333 admits that `return`, so the program now type-checks. "No
/// `T0021`" is still the contract, and the solver term is still what makes
/// it true.
#[test]
fn an_unannotated_helper_returning_a_foreign_attribute_type_checks_without_t0021() {
    let dir = ScratchDir::new("foreign_helper_attr_return").expect("scratch");
    let output = check(
        &dir,
        "import numpy\n\ndef _helper():\n    return numpy.pi\n\nx = _helper()\n",
    );
    let rendered = stdout_of(&output);
    assert!(!rendered.contains("T0021"), "{rendered}");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{rendered}{}",
        stderr_of(&output)
    );
}

/// The one operation Part 2 of #1026 adds: a discarded attribute load.
///
/// It is the only statement position an `object`-typed attribute load can
/// occupy end to end, because every consuming site still refuses the value
/// (the table below). PR 2a therefore ships no user-visible capability --
/// that is the intended state, and this test is what distinguishes "not yet
/// wired up" from "still refused".
#[test]
fn a_discarded_attribute_load_on_a_foreign_module_is_accepted() {
    let dir = ScratchDir::new("foreign_attr_accepted").expect("scratch");
    let output = check(&dir, "import numpy\n\nnumpy.pi\n");
    assert_eq!(output.status.code(), Some(0), "{}", stdout_of(&output));
}

/// A helper that returns a foreign attribute type-checks even when it is
/// called above the `import`.
///
/// PR 2a of #1081 refused the in-function read here, because D-041 checks
/// a body against the module environment as it stands after all top-level
/// code and the artifact then trapped (`SIGTRAP`) on the unbound global.
/// #1316 admits the read and gives it a failure edge instead, and Part 1 of
/// #1333 admits returning the object, so the whole program is accepted. At
/// run time the call raises CPython's own `NameError`, as CPython does;
/// `tests/issue_1333_foreign_in_function.rs` matches that against the host
/// interpreter for a helper called before the import it reads (an annotated
/// helper over a call producer, not this unannotated attribute return).
#[test]
fn a_helper_reading_a_foreign_object_before_its_import_type_checks() {
    let dir = ScratchDir::new("foreign_helper_before_import").expect("scratch");
    let output = check(
        &dir,
        "def _pi():\n    return numpy.pi\n\n_pi()\n\nimport numpy\n",
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
}

/// The direct form of the same regression: a module-body read placed above
/// its own `import`.
///
/// Part 1 refused this through the unconditional `reject_object_read`,
/// which Part 2 removed along with the positional guarantee that rested on
/// it -- the pre-seed in `pycc_types::module` kept the name *bound*, so
/// the read was admitted and trapped at run time just as the helper shape
/// did. Removing the pre-seed makes it an ordinary unbound name, which is
/// also the closer answer: CPython raises `NameError`.
#[test]
fn a_module_body_read_above_its_foreign_import_is_refused() {
    let dir = ScratchDir::new("foreign_read_above_import").expect("scratch");
    let output = check(&dir, "numpy.pi\n\nimport numpy\n");
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert!(rendered.contains("error[T0021]"), "{rendered}");
    assert!(rendered.contains("`numpy` is not defined"), "{rendered}");
}

/// A general method call on the object is now *accepted*.
///
/// PR 2a pinned this exact program as `T0043` ("not a class instance"),
/// reached by falling through to `class::resolve_method_call`, and recorded
/// that PR 2b would admit it rather than rename the refusal. It does: the
/// `Ty::Object` branch added ahead of that call answers `Ty::Object`, so
/// the program type-checks and lowers to a `MirExpr::ObjMethodCall`.
///
/// Since #1095 that includes the four D-105 String-keyed container
/// spellings (`append`/`pop`/`get`/`add`): the foreign import keeps the
/// method reading beside the container one, and the object receiver takes
/// it, so `numpy.append(1)` left the refusal table below.
#[test]
fn a_general_method_call_on_a_foreign_object_is_accepted() {
    let dir = ScratchDir::new("foreign_method_call_ok").expect("scratch");
    for body in [
        "import numpy\n\nnumpy.sqrt(2.0)\n",
        "import numpy\n\nnumpy.append(1)\n",
    ] {
        let output = check(&dir, body);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{body}: {}",
            stdout_of(&output)
        );
        assert_eq!(stdout_of(&output), "", "{body}");
    }
}

/// Every shape that *consumes* the binding, one row per refusing site.
///
/// `tests/diagnostics/i0404_foreign_module_operation.py` pins the exact
/// rendering of one of them; this states the *set*. Part 1 guaranteed the
/// set by refusing the read itself, so no `Ty::Object` value could escape
/// into any operation at all. Part 2 admits the read and refuses each
/// consumer separately, which makes this table the invariant rather than a
/// consequence of one: a row that silently started compiling is now a real
/// hole rather than an impossibility.
#[test]
fn every_operation_on_a_foreign_module_is_refused_with_i0404() {
    let dir = ScratchDir::new("foreign_i0404").expect("scratch");
    let bodies = [
        // An expression-position read: a `match` subject. A D-105
        // String-keyed method spelling (`numpy.append(1)`) is admitted since
        // #1095 (the test above). A module-body direct call
        // (`numpy(1)`) is admitted since #1313 and raises CPython's own
        // `TypeError` at run time; `tests/issue_1313_foreign_direct_call.rs`
        // pins it. `print(numpy)` and `f"{numpy}"` are admitted since #1340
        // (the test below).
        "import numpy\n\nmatch numpy:\n    case 1:\n        pass\n",
        // The iterable of a comprehension.
        "import numpy\n\nxs = [e for e in numpy]\n",
    ];
    for body in bodies {
        let output = check(&dir, body);
        assert_eq!(output.status.code(), Some(1), "{body}");
        assert!(stdout_of(&output).contains("error[I0404]"), "{body}");
    }
}

/// The two rows #1340 took out of the table above: printing the module
/// object and interpolating it into an f-string both check clean.
/// `tests/issue_1340_print_object.rs` runs them against CPython.
#[test]
fn printing_and_interpolating_a_foreign_module_check_clean() {
    let dir = ScratchDir::new("foreign_print_ok").expect("scratch");
    for body in [
        "import numpy\n\nprint(numpy)\n",
        "import numpy\n\nprint(f\"{numpy}\")\n",
    ] {
        let output = check(&dir, body);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{body}: {}",
            stdout_of(&output)
        );
    }
}

/// The two rows #1325 took out of the table above: binding the module
/// object to a module-level name and iterating it with a bare-name `for`.
/// Both check clean; a bare module is not iterable, so the loop raises
/// CPython's own `TypeError` at run time, which
/// `tests/issue_1325_foreign_binding.rs` pins.
#[test]
fn binding_and_iterating_a_foreign_module_at_module_scope_check_clean() {
    let dir = ScratchDir::new("foreign_bind_ok").expect("scratch");
    for body in [
        "import numpy\n\nx = numpy\n",
        "import numpy\n\nfor x in numpy:\n    pass\n",
    ] {
        let output = check(&dir, body);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{body}: {}",
            stdout_of(&output)
        );
    }
}

/// Rebinding is not an operation on the object, so it is not `I0404`
/// either: it is a second, non-foreign binding of the name, which the
/// refusal below rejects at lowering before any pass reads the name's type
/// at all. Pinned rendering lives in
/// `tests/diagnostics/c0001_foreign_module_shadowed_import.py`; what
/// matters here is that the two codes do not overlap.
#[test]
fn rebinding_a_foreign_module_is_refused_rather_than_i0404() {
    let dir = ScratchDir::new("foreign_rebind").expect("scratch");
    let output = check(&dir, "import numpy\n\nnumpy = 3\n");
    let rendered = stdout_of(&output);
    assert!(rendered.contains("error[C0001]"), "{rendered}");
    assert!(!rendered.contains("I0404"), "{rendered}");
}

/// Three rounds of review on #1080 each found one more pass that did not
/// apply "a foreign import supersedes an earlier binding of the same name"
/// positionally -- the check pass, then the constraint solver, then export
/// discovery, which kept a `PyMethodDef` for a `def` the import supersedes
/// so the host called a stale function where CPython hands back a module
/// object. The rule now is the refusal instead: a module that binds one
/// name both foreign and non-foreign is rejected at lowering, whichever
/// order the two bindings are written in, which is also what Part 1 already
/// does for the cross-module case. Supporting either order is later work
/// (#1026).
///
/// `import json` is deliberately a *real* stdlib module pycc does not
/// implement, which is what makes it a foreign binding rather than a
/// `pycc_std` one.
#[test]
fn a_foreign_import_below_a_same_named_def_is_refused() {
    let dir = ScratchDir::new("foreign_shadows_def").expect("scratch");
    let output = check(
        &dir,
        "def json() -> int:\n    return 1\n\n\nimport json\n\nx = json()\n",
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert!(rendered.contains("error[C0001]"), "{rendered}");
    assert!(
        rendered.contains("shadowing a foreign import is not supported yet"),
        "{rendered}"
    );
}

/// The other order is refused by the same rule. This is the shape whose
/// acceptance produced the wrong *artifact*: type checking succeeded, but
/// `collect_exports` created a `PyMethodDef` for the `def` even though the
/// import is the name's final binding, so a host importing the extension
/// saw and could call the stale function.
#[test]
fn a_def_below_a_foreign_import_is_refused() {
    let dir = ScratchDir::new("foreign_shadowed_by_def").expect("scratch");
    let output = check(
        &dir,
        "import json\n\ndef json() -> int:\n    return 1\n\n\nx = json()\n",
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    assert!(
        stdout_of(&output).contains("shadowing a foreign import is not supported yet"),
        "{}",
        stdout_of(&output)
    );
}

/// A plain assignment is a binding exactly as a `def` is, so it is refused
/// on the same rule. Before the refusal, the check pass's unconditional
/// pre-seed made `check_assignment` see `json` as already having
/// representation `object` and rejected the *first* statement with a
/// `T0023` -- a diagnostic about the assignment, for a conflict the import
/// below it introduced.
#[test]
fn an_assignment_above_a_foreign_import_is_refused() {
    let dir = ScratchDir::new("foreign_shadows_assignment").expect("scratch");
    let output = check(&dir, "json = 1\nimport json\n");
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert!(
        rendered.contains("shadowing a foreign import is not supported yet"),
        "{rendered}"
    );
    assert!(!rendered.contains("T0023"), "{rendered}");
}

/// The same shape with a later call. The solver's own source-order pass
/// reaches a call collector that checks `bindings` first, so the `int` term
/// the assignment left there produced the generic "bound to a non-callable
/// value" `T0021` before validation could say anything about the import.
#[test]
fn an_assignment_above_a_foreign_import_is_refused_before_the_call() {
    let dir = ScratchDir::new("foreign_shadows_assignment_call").expect("scratch");
    let output = check(&dir, "json = 1\nimport json\n\ny = json()\n");
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert!(
        rendered.contains("shadowing a foreign import is not supported yet"),
        "{rendered}"
    );
    assert!(!rendered.contains("T0021"), "{rendered}");
}

/// The refusal is bounded by the shadowing: a foreign import whose name
/// nothing else in the module binds is an ordinary `object` callee, and the
/// solver's own pass types the call before signature materialization can
/// report a `T0021` no annotation could satisfy. Part 1's `I0404` for the
/// helper's `return` is gone since Part 1 of #1333, so the program
/// type-checks; at run time the call raises CPython's own
/// `TypeError: 'module' object is not callable`, which
/// `tests/issue_1333_foreign_in_function.rs` matches against the host.
#[test]
fn an_unshadowed_foreign_import_is_an_ordinary_object_callee() {
    let dir = ScratchDir::new("foreign_unshadowed_helper").expect("scratch");
    let output = check(
        &dir,
        "import json\n\ndef _helper():\n    return json()\n\n\ny = _helper()\n",
    );
    let rendered = stdout_of(&output);
    assert!(!rendered.contains("T0021"), "{rendered}");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{rendered}{}",
        stderr_of(&output)
    );
}

/// A second `import` binding the same local name shadows the foreign one
/// exactly as a `def` does, and `definition_spans` -- the table the refusal
/// first consulted -- never carries an import's own binding, so this shape
/// slipped past it (review round 4 on #1080). Used, it reached the solver
/// and reported a receiver diagnostic against `import json`, the statement
/// the alias supersedes; unused, it was accepted silently.
#[test]
fn a_second_import_rebinding_a_foreign_name_is_refused() {
    for body in [
        "import json\nimport math as json\n\ny = json.sqrt(4.0)\n",
        "import json\nimport math as json\n",
    ] {
        let dir = ScratchDir::new("foreign_rebound_by_import").expect("scratch");
        let output = check(&dir, body);
        assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
        let rendered = stdout_of(&output);
        assert!(
            rendered.contains("shadowing a foreign import is not supported yet"),
            "{rendered}"
        );
        assert!(!rendered.contains("is a local name here"), "{rendered}");
    }
}

/// Two foreign imports of the same name bound to the same module are
/// admitted since #1291: both producers yield the same CPython module
/// object, so which one a read resolves to cannot change what it reads.
/// (#1291 needs this for an `if`/`else` that imports the module in each
/// arm.) Binding the same name to two *different* modules stays refused on
/// the shadowing rule, once.
#[test]
fn a_duplicated_foreign_import_is_admitted() {
    let dir = ScratchDir::new("foreign_duplicate_import").expect("scratch");
    let output = check(
        &dir,
        "import numpy
import numpy
",
    );
    assert_eq!(output.status.code(), Some(0), "{}", stdout_of(&output));
    assert_eq!(stdout_of(&output), "");

    let dir = ScratchDir::new("foreign_duplicate_import_other").expect("scratch");
    let output = check(
        &dir,
        "import numpy
import colorsys as numpy
",
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert_eq!(
        rendered
            .matches("shadowing a foreign import is not supported yet")
            .count(),
        // Twice per diagnostic: `render_human` prints the message on the
        // `error[...]` line and again under the source caret.
        2,
        "{rendered}"
    );
}

/// `I0403` points at the `import` statement. Every one of them used to be
/// built with `Span::new(0, 0)`, so a foreign import that was not the first
/// statement reported at `<file>:1:1` and underlined an unrelated line
/// (review round 4 on #1080). The fixture's import sits on line 5 precisely
/// so a hard-coded zero span fails the assertion.
#[test]
fn a_native_refusal_points_at_the_import_statement() {
    let dir = ScratchDir::new("foreign_native_span").expect("scratch");
    let output = pycc()
        .arg("build")
        .arg(source(
            &dir,
            "def g() -> int:\n    return 1\n\n\nimport tkinter\n",
        ))
        .arg("-o")
        .arg(dir.join("m"))
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stderr_of(&output);
    assert!(rendered.contains("error[I0403]"), "{rendered}");
    assert!(
        rendered.contains(&format!("{}:5:1", rendered_path(&dir.join("m.py")))),
        "{rendered}"
    );
    assert!(rendered.contains("5 | import tkinter"), "{rendered}");
}

/// A function local that happens to share a foreign import's name is an
/// ordinary local. The per-body environment strips the module-level opaque
/// marker for every local name; the foreign marker is the second half of
/// that same fact and is stripped with it. Left behind, the local's own
/// assignment would be read back through the foreign provenance and infer
/// as `object`, turning an unresolved-container inference into a misleading
/// return-type mismatch (`T0022: expected return type `object``).
#[test]
fn a_local_shadowing_a_foreign_import_is_not_a_foreign_object() {
    let dir = ScratchDir::new("foreign_local_shadow").expect("scratch");
    let output = check(
        &dir,
        "import numpy\n\ndef _helper():\n    numpy = {\"x\": 1}\n    return numpy\n\n\nz = _helper()\n",
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert!(!rendered.contains("T0022"), "{rendered}");
    assert!(
        rendered.contains("cannot infer return type of private helper `_helper`"),
        "{rendered}"
    );
}

/// Builds `body` as an extension module named `module` inside `dir`.
fn build_ext(dir: &Path, module: &str, body: &str) {
    let build = pycc()
        .arg("build")
        .arg(source(dir, body))
        .arg("-o")
        .arg(dir.join(module))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
}

fn python(dir: &Path, script: &str) -> Output {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn")
}

/// The obligation recorded on #1080 (issuecomment-5658045798), deferred by
/// PR 1a and discharged here: a foreign import of a module that does not
/// exist must surface to the host as CPython's own `ModuleNotFoundError`,
/// raised out of the `Py_mod_exec` slot, and the extension module must not
/// end up in `sys.modules`.
///
/// That is the whole reason `pycc_ext_obj_import` returns CPython's `NULL`
/// convention untouched instead of translating the failure: the exception
/// the host sees is the one `PyImport_ImportModule` set.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_missing_foreign_module_raises_module_not_found_error_in_the_host() {
    let dir = ScratchDir::new("foreign_missing_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_missing_import_mod",
        "import pycc_no_such_module_1080\n\ndef answer() -> int:\n    return 42\n",
    );
    let run = python(
        &dir,
        "import sys\n\
         try:\n\
         \x20   import pycc_missing_import_mod\n\
         except ModuleNotFoundError as e:\n\
         \x20   assert 'pycc_no_such_module_1080' in str(e), str(e)\n\
         else:\n\
         \x20   raise AssertionError('the import should have failed')\n\
         assert 'pycc_missing_import_mod' not in sys.modules\n",
    );
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
}

/// The success path of the same entry point: a module that really exists
/// imports, the `Py_mod_exec` slot returns 0, and the artifact's own
/// exports work afterwards. Without this, every hosted assertion here
/// would be satisfied by a `pycc_ext_obj_import` that always failed.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_existing_foreign_module_imports_and_the_artifact_stays_usable() {
    let dir = ScratchDir::new("foreign_present_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_present_import_mod",
        "import json

def answer() -> int:
    return 42
",
    );
    let run = python(
        &dir,
        "import pycc_present_import_mod as m
assert m.answer() == 42, m.answer()
",
    );
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
}

/// PR 2a of #1081 review finding 1, end to end: a failed attribute lookup
/// must surface to the host as CPython's own `AttributeError`, and the
/// module body must stop there.
///
/// Before the fix, `foreign_attr::emit` emitted no `NULL` check and the
/// pending-exception guard that follows the load reads pycc's own state
/// (D-173), which CPython's error indicator leaves untouched. The body ran
/// to completion with CPython's exception still set, and the interpreter
/// reported `SystemError: execution of module ... raised unreported
/// exception` with the real `AttributeError` visible only as a chained
/// cause. The load now takes the same module-exec failure edge a failed
/// `pycc_ext_obj_import` takes, so the exception CPython set is the
/// exception the host sees.
///
/// The side effect written below the load is what proves the body stopped
/// rather than merely reported: it must not have run.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_failed_attribute_lookup_raises_attribute_error_in_the_host() {
    let dir = ScratchDir::new("foreign_attr_failure_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_missing_attr_mod",
        "import json\n\njson.pycc_no_such_attribute_1081\nprint(\"ran past the load\")\n",
    );
    let run = python(
        &dir,
        "try:\n\
         \x20   import pycc_missing_attr_mod\n\
         except AttributeError as e:\n\
         \x20   assert 'pycc_no_such_attribute_1081' in str(e), str(e)\n\
         else:\n\
         \x20   raise AssertionError('the attribute load should have failed')\n",
    );
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    assert!(
        !stdout_of(&run).contains("ran past the load"),
        "the module body must stop at the failed load: {}",
        stdout_of(&run)
    );
}

/// The hosted half of the #1080 ordering obligation
/// (issuecomment-5658356849), and the only form of it that is *observable*
/// rather than structural: a module-body statement with a side effect,
/// written above a failing import, must have run before the import fails,
/// exactly as CPython runs a module body statement by statement (D-244
/// rule 3).
///
/// A failing import is what makes the ordering observable at all -- with a
/// successful one, both orders produce the same output. The structural
/// halves are non-ignored and carry the coverage:
/// `crates/pycc_mir/src/tests/import.rs` for the item's position in the IR
/// and `crates/pycc_codegen/src/foreign_import.rs` for the emitted call's
/// position in the entry block.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_statement_above_a_failing_foreign_import_has_already_run_in_the_host() {
    let dir = ScratchDir::new("foreign_order_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_import_order_mod",
        "print(\"before the import\")\n\
         import pycc_no_such_module_1080\n\
         print(\"after the import\")\n\n\
         def answer() -> int:\n    return 42\n",
    );
    let run = python(
        &dir,
        "try:\n\
         \x20   import pycc_import_order_mod\n\
         except ModuleNotFoundError:\n\
         \x20   pass\n\
         else:\n\
         \x20   raise AssertionError('the import should have failed')\n",
    );
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    let printed = stdout_of(&run);
    assert!(printed.contains("before the import"), "{printed}");
    assert!(!printed.contains("after the import"), "{printed}");
}

/// PR 1c of #1080 review finding 1, end to end. `monomorphize` drops every
/// original generic function, which used to leave the import's recorded
/// position pointing past the end of the item list: this exact module
/// aborted the build with `insertion index (is 2) should be <= len (is 0)`
/// out of `pycc_mir::splice_foreign_imports`. The structural assertions --
/// that the recomputed position is both in range and still ahead of the
/// items that followed the import in the source -- are non-ignored, in
/// `crates/pycc_types/src/foreign/tests.rs`; this is the hosted
/// confirmation that the artifact such a module produces really loads.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_module_whose_generics_are_all_dropped_still_builds_and_loads() {
    let dir = ScratchDir::new("foreign_dropped_generics_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_dropped_generics_mod",
        "def _a[T](x: T) -> T:\n    return x\n\n\n\
         def _b[T](x: T) -> T:\n    return x\n\n\
         import json\n\n\
         def answer() -> int:\n    return 42\n",
    );
    let run = python(
        &dir,
        "import pycc_dropped_generics_mod as m\nassert m.answer() == 42, m.answer()\n",
    );
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
}

/// Runs `pycc check` over a two-file project and returns its stdout with
/// the process's own exit code asserted to be 1.
fn check_project(tag: &str, dep: &str, entry: &str) -> (pycc_scratch::ScratchDir, String) {
    let dir = ScratchDir::new(tag).expect("scratch");
    std::fs::write(dir.join("dep.py"), dep).expect("write the dependency");
    let main = dir.join("main.py");
    std::fs::write(&main, entry).expect("write the entry");
    let output = pycc()
        .arg("check")
        .arg(&main)
        .output()
        .expect("pycc should spawn");
    let rendered = stdout_of(&output);
    assert_eq!(output.status.code(), Some(1), "{rendered}");
    (dir, rendered)
}

/// Finding A of the #1087 review: `from dep import json`, where `json` is
/// `dep.py`'s own foreign import, used to clone the `Foreign` binding with
/// its dependency-local item index into the entry module. `link` then
/// rebased that index as though it belonged to the entry, so `--ext` built
/// either an out-of-range splice (a `pycc_mir` panic) or a second
/// `pycc_ext_obj_import` for one source statement. It is now refused while
/// lowering, so `check` catches it before any build path runs.
#[test]
fn re_exporting_a_dependency_s_foreign_import_is_refused_across_files() {
    for dep in [
        // The shape that panicked: three items before the import, so the
        // rebased index ran past the entry module's item vector.
        "def a() -> int:\n    return 1\ndef b() -> int:\n    return 2\n\
         def c() -> int:\n    return 3\nimport json\n",
        // The smaller variant, with a single preceding item.
        "def a() -> int:\n    return 1\nimport json\n",
        // No preceding item at all.
        "import json\n",
    ] {
        let (dir, rendered) = check_project("foreign_reexport", dep, "from dep import json\n");
        assert!(rendered.contains("error[C0001]"), "{rendered}");
        assert!(
            rendered.contains(
                "binds `json` to the CPython module object `json`; re-exporting a \
                 foreign import across project modules is not supported yet"
            ),
            "{rendered}"
        );
        // The refusal is reported at the importing statement, in the entry.
        assert!(
            rendered.contains(&format!("{}:1:1", rendered_path(&dir.join("main.py")))),
            "{rendered}"
        );
    }
}

/// Finding B of the #1087 review: a top-level definition of a name another
/// module binds to a CPython module object used to survive linking, so the
/// dependency's own `json()` call silently resolved to the entry's
/// function instead of raising CPython's `TypeError`. Both dependency
/// orders are covered: the shadowing definition in the entry (linked last)
/// and in a dependency linked before the foreign module.
#[test]
fn a_definition_shadowing_another_module_s_foreign_import_is_refused() {
    let (dir, rendered) = check_project(
        "foreign_shadow_entry",
        "import json\ndef f() -> int:\n    return json()\n",
        "from dep import f\n\n\ndef json() -> int:\n    return 1\n\n\nx: int = f()\n",
    );
    assert!(rendered.contains("error[C0001]"), "{rendered}");
    // The message names both modules by their display paths. `render_human`
    // normalizes the `-->` location line to forward slashes but interpolates a
    // message body verbatim, so a body assertion uses the platform separator.
    assert!(
        rendered.contains(&format!(
            "module `{}` defines `json`, which `{}` binds to a CPython module object; \
             shadowing a foreign import across modules is not supported yet",
            dir.join("main.py").display(),
            dir.join("dep.py").display()
        )),
        "{rendered}"
    );
    assert!(
        rendered.contains(&format!("{}:4:1", rendered_path(&dir.join("main.py")))),
        "the diagnostic is at the shadowing definition: {rendered}"
    );
}

/// The reverse link order: `shadow.py` is linked before the module whose
/// foreign import it shadows, so an incremental check over the names
/// already linked would not see the collision.
#[test]
fn a_definition_linked_before_the_foreign_module_is_refused_too() {
    let dir = ScratchDir::new("foreign_shadow_dep").expect("scratch");
    std::fs::write(dir.join("shadow.py"), "def json() -> int:\n    return 1\n")
        .expect("write the shadowing dependency");
    std::fs::write(
        dir.join("dep.py"),
        "import json\ndef f() -> int:\n    return json()\n",
    )
    .expect("write the foreign dependency");
    let main = dir.join("main.py");
    std::fs::write(
        &main,
        "from shadow import json\nfrom dep import f\n\n\nx: int = f()\n",
    )
    .expect("write the entry");
    let output = pycc()
        .arg("check")
        .arg(&main)
        .output()
        .expect("pycc should spawn");
    let rendered = stdout_of(&output);
    assert_eq!(output.status.code(), Some(1), "{rendered}");
    assert!(rendered.contains("error[C0001]"), "{rendered}");
    assert!(
        rendered.contains("shadowing a foreign import across modules is not supported yet"),
        "{rendered}"
    );
    assert!(
        rendered.contains(&format!("{}:1:1", rendered_path(&dir.join("shadow.py")))),
        "the diagnostic names the shadowing module: {rendered}"
    );
}
