//! #1156 (W0 of #882): `__name__` reads as a module-level `str` constant.
//!
//! THE RULE is stated once, in the module documentation of
//! `crates/pycc_hir/src/dunder_name.rs`, and quoted verbatim by
//! `docs/STDLIB_PLAN.md`'s "Tier 0 — builtins" section. This file
//! deliberately does not restate it: a third copy is a third thing to keep in
//! step, and two successive review rounds found this header still describing
//! behavior the rule no longer had. Read it there; the tests below are its
//! executable form.
//!
//! The value is `"__main__"` for `pycc check` and for a native
//! `pycc build`/`pycc run`, and the extension module's own name for a
//! `pycc build --ext`.
//!
//! Neither gate counts a module-scope `if TYPE_CHECKING:` body, in the entry
//! module or in a dependency: #790 constant-folds it away, so it binds and
//! reads nothing at run time.
//!
//! Known gap, deliberate: only the *entry* module is given a name, and a
//! dependency that mentions `__name__` at all withholds even that. Part 1 of
//! #881 links every module of a program into one flat namespace, so a
//! per-module `__name__` global would collide, and linking places every
//! dependency's top-level statements ahead of the entry module's seed, so a
//! dependency's read would observe the global before the seed stored
//! anything. Every such program is therefore a `T0021` rather than a value,
//! which the multi-module tests at the end of this file pin.
//!
//! Only the last test is `#[ignore]`d: it builds an artifact and asks an
//! installed CPython to import it, which is a property of the machine rather
//! than of the change under test (the convention
//! `tests/issue_1143_ext_methods.rs` states).

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

/// Builds `source` as a native executable and returns its stdout. Panics with
/// the compiler's own diagnostics when the build fails, so a regression shows
/// the diagnostic rather than an opaque missing-file error.
fn build_and_run(category: &str, source: &str) -> String {
    let dir = ScratchDir::new(category).expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(&src, source).expect("write the fixture source");
    let out = dir.join("m");
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    let run = Command::new(&out)
        .output()
        .expect("the artifact should spawn");
    assert!(run.status.success(), "{}", stderr_of(&run));
    stdout_of(&run)
}

/// `pycc check` on `source`, returning success plus both streams: `check`
/// renders diagnostics on stdout where `build` renders them on stderr, so
/// both are read rather than guessing which one a subcommand uses.
fn check(category: &str, source: &str) -> (bool, String) {
    let dir = ScratchDir::new(category).expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(&src, source).expect("write the fixture source");
    let out = pycc()
        .arg("check")
        .arg(&src)
        .output()
        .expect("pycc should spawn");
    (
        out.status.success(),
        format!("{}{}", stdout_of(&out), stderr_of(&out)),
    )
}

// -- `TYPE_CHECKING`-guarded bodies ----------------------------------

#[test]
fn a_type_checking_guarded_binding_leaves_the_seed_intact() {
    // #790 constant-folds the guarded body away, so the assignment emits no
    // store. Counting it as a user binding withheld the seed and turned the
    // live read below into a `T0021` for a program CPython runs fine.
    assert_eq!(
        build_and_run(
            "name_type_checking_binding",
            "from typing import TYPE_CHECKING\n\nif TYPE_CHECKING:\n    __name__ = 7\n\nprint(__name__)\n",
        ),
        "__main__\n",
    );
}

#[test]
fn a_qualified_type_checking_guarded_binding_leaves_the_seed_intact() {
    assert_eq!(
        build_and_run(
            "name_type_checking_qualified",
            "import typing\n\nif typing.TYPE_CHECKING:\n    __name__ = 7\n\nprint(__name__)\n",
        ),
        "__main__\n",
    );
}

#[test]
fn an_aliased_type_checking_guarded_binding_leaves_the_seed_intact() {
    // The spelling that needs a real import binding to resolve: the scans
    // reconstruct the module's own `typing as t` alias, so they fold exactly
    // the body `lower_stmt` folds.
    assert_eq!(
        build_and_run(
            "name_type_checking_aliased",
            "import typing as t\n\nif t.TYPE_CHECKING:\n    __name__ = 7\n\nprint(__name__)\n",
        ),
        "__main__\n",
    );
}

#[test]
fn an_elif_type_checking_guarded_binding_leaves_the_seed_intact() {
    // `lower_elif_else_clauses` folds the guard in the `elif` position too
    // (#790), so the dead assignment cannot collide with the seed.
    assert_eq!(
        build_and_run(
            "name_type_checking_elif",
            "from typing import TYPE_CHECKING\n\nif False:\n    pass\nelif TYPE_CHECKING:\n    __name__ = 7\n\nprint(__name__)\n",
        ),
        "__main__\n",
    );
}

#[test]
fn the_else_arm_of_a_type_checking_guard_still_binds() {
    // Only the guarded body is dead. The `else` is live whenever the guard is
    // skipped -- at run time, always -- so it shadows the seed like any other
    // module-scope binding. The diagnostic is `T0041` rather than `T0021`
    // because the binding sits on one arm of a conditional: with the seed
    // withheld, definite-assignment analysis reaches the read on a path that
    // never bound the name.
    let (ok, rendered) = check(
        "name_type_checking_else",
        "from typing import TYPE_CHECKING\n\nif TYPE_CHECKING:\n    pass\nelse:\n    __name__ = 7\n\nprint(__name__)\n",
    );
    assert!(!ok, "{rendered}");
    assert!(rendered.contains("T0041"), "{rendered}");
}

// -- the provided binding -------------------------------------------

#[test]
fn a_module_level_read_prints_dunder_main() {
    assert_eq!(
        build_and_run("dn_module", "print(__name__)\n"),
        "__main__\n"
    );
}

#[test]
fn a_read_inside_a_function_body_sees_the_same_value() {
    assert_eq!(
        build_and_run(
            "dn_function",
            "\
def report() -> None:
    print(__name__)

report()
print(__name__)
",
        ),
        "__main__\n__main__\n"
    );
}

#[test]
fn the_dunder_main_guard_takes_its_true_branch() {
    assert_eq!(
        build_and_run(
            "dn_guard",
            "\
if __name__ == \"__main__\":
    print(\"entry\")
else:
    print(\"imported\")
",
        ),
        "entry\n"
    );
}

#[test]
fn the_binding_is_a_str_and_supports_str_operations() {
    assert_eq!(
        build_and_run(
            "dn_str",
            "\
greeting: str = \"hello \" + __name__
print(greeting)
print(__name__ != \"other\")
"
        ),
        "hello __main__\nTrue\n"
    );
}

#[test]
fn pycc_check_accepts_a_read_with_no_diagnostic() {
    let (ok, rendered) = check("dn_check", "print(__name__)\n");
    assert!(ok, "{rendered}");
    assert_eq!(rendered, "");
}

// -- a user binding at module top level wins outright ---------------

#[test]
fn a_module_level_assignment_wins_over_the_seed() {
    assert_eq!(
        build_and_run("dn_user_assign", "__name__ = \"custom\"\nprint(__name__)\n",),
        "custom\n"
    );
}

#[test]
fn a_module_level_annotated_assignment_wins_over_the_seed() {
    assert_eq!(
        build_and_run(
            "dn_user_ann_assign",
            "__name__: str = \"custom\"\nprint(__name__)\n",
        ),
        "custom\n"
    );
}

#[test]
fn a_user_binding_of_a_non_str_type_is_accepted_because_nothing_is_seeded() {
    // The seed is withheld for the *whole* module, so the user's binding
    // introduces the name with its own type rather than rebinding a `str`.
    assert_eq!(
        build_and_run("dn_user_int", "__name__ = 7\nprint(__name__ + 1)\n"),
        "8\n"
    );
}

#[test]
fn a_read_that_precedes_a_module_level_assignment_is_t0021() {
    // The documented deviation from CPython: CPython's interpreter-provided
    // module name would still be visible here.
    let (ok, rendered) = check("dn_precedes", "print(__name__)\n__name__ = \"custom\"\n");
    assert!(!ok, "{rendered}");
    assert!(
        rendered.contains("error[T0021]") && rendered.contains("name `__name__` is not defined"),
        "{rendered}"
    );
}

// -- a value-less annotation only declares a type ------------------

#[test]
fn a_value_less_annotation_does_not_withhold_the_seed() {
    // `__name__: str` emits no store in CPython, so the module name survives;
    // only an annotated *assignment* shadows the compiler-provided binding.
    assert_eq!(
        build_and_run("dn_ann_only", "__name__: str\nprint(__name__)\n"),
        "__main__\n"
    );
}

#[test]
fn a_value_less_annotation_of_another_type_still_yields_the_str_seed() {
    // The annotation declares nothing, so the seed's own `str` type is what
    // every later use sees -- a mismatched one is a type error at the use
    // site, never a silently mistyped global.
    assert_eq!(
        build_and_run("dn_ann_int", "__name__: int\nprint(__name__)\n"),
        "__main__\n"
    );
    let (ok, rendered) = check("dn_ann_int_use", "__name__: int\nprint(__name__ + 1)\n");
    assert!(!ok, "{rendered}");
    assert!(
        rendered.contains("operator Add is not defined for `str` and `int`"),
        "{rendered}"
    );
}

// -- expression-level bindings the top-level scan deliberately skips --
//
// `binds_dunder_name_at_module_scope` scans all of module scope, not just the
// direct children of the module body and not just statement-level targets.
// These pin the forms an earlier revision missed -- a `match` capture, a
// binding nested in a top-level compound statement, a walrus, an `except ...
// as` name -- each of which was seeded and then rejected with `T0023` for
// rebinding the seeded `str`.

#[test]
fn a_match_capture_over_a_str_subject_wins_over_the_seed() {
    assert_eq!(
        build_and_run(
            "dn_match_str",
            "x: str = \"a\"\nmatch x:\n    case __name__:\n        pass\nprint(__name__)\n",
        ),
        "a\n"
    );
}

#[test]
fn a_match_capture_over_a_non_str_subject_wins_over_the_seed() {
    // The capture binds an `int` global. Nothing is seeded, so this is an
    // ordinary `int` name and arithmetic on it type-checks -- where an earlier
    // revision seeded a `str` first and rejected the capture with `T0023`.
    assert_eq!(
        build_and_run(
            "dn_match_int",
            "x: int = 7\nmatch x:\n    case __name__:\n        pass\nprint(__name__ + 1)\n",
        ),
        "8\n"
    );
}

#[test]
fn an_except_as_binding_wins_over_the_seed() {
    assert_eq!(
        build_and_run(
            "dn_except_as",
            concat!(
                "try:\n",
                "    raise ValueError(\"e\")\n",
                "except ValueError as __name__:\n",
                "    pass\n",
                "print(1)\n",
            ),
        ),
        "1\n"
    );
}

#[test]
fn a_str_valued_walrus_binding_of_dunder_name_is_t0050() {
    // #774: `str` is not a supported walrus value, so this *particular* walrus
    // shape cannot bind a module name at all today.
    let (ok, rendered) = check("dn_walrus", "print((__name__ := \"custom\"))\n");
    assert!(!ok, "{rendered}");
    assert!(rendered.contains("error[T0050]"), "{rendered}");
}

#[test]
fn a_scalar_walrus_binding_wins_over_the_seed_as_a_bare_expression_statement() {
    // #774 permits `int`/`float`/`bool`/`None` as walrus values, so this is a
    // fully supported top-level binding of the name and THE RULE's "a top-level
    // user binding wins outright" must hold for it: nothing is seeded, and the
    // `int` rebind is not rejected with `T0023`.
    assert_eq!(
        build_and_run("dn_walrus_int", "print((__name__ := 7))\n"),
        "7\n"
    );
}

#[test]
fn a_scalar_walrus_binding_in_a_top_level_if_condition_wins_over_the_seed() {
    // The walrus scan walks *into* a top-level compound statement's own header,
    // which is where PEP 572's own motivating shape puts it.
    assert_eq!(
        build_and_run(
            "dn_walrus_if",
            "if (__name__ := 7) > 3:\n    pass\nprint(__name__)\n",
        ),
        "7\n"
    );
}

#[test]
fn a_walrus_binding_inside_a_function_body_does_not_withhold_the_seed() {
    // A walrus binds in the enclosing *function or module* scope (PEP 572), so
    // one inside a function body is a local and must leave the module seed
    // intact -- exactly as a plain `__name__ = "x"` in a function body does.
    assert_eq!(
        build_and_run(
            "dn_walrus_local",
            concat!(
                "def f() -> int:\n",
                "    if (__name__ := 7) > 3:\n",
                "        return 1\n",
                "    return 0\n",
                "\n",
                "print(f())\n",
                "print(__name__)\n",
            ),
        ),
        "1\n__main__\n"
    );
}

#[test]
fn a_dependencys_nested_binding_withholds_the_entry_seed() {
    // The driver asks `pycc_hir` for the same module-scope answer the entry
    // module's own gate uses, so a dependency binding nested inside a compound
    // statement withholds the entry seed exactly as a direct one does. The
    // earlier `definition_spans` gate recorded only direct children and so
    // seeded the entry anyway, which failed the program with `T0023`.
    assert_eq!(
        build_and_run_program(
            "dn_dep_nested",
            "from dep import helper\n\nprint(__name__)\nprint(helper())\n",
            concat!(
                "flag: bool = True\n",
                "if flag:\n",
                "    __name__ = \"dep\"\n",
                "else:\n",
                "    __name__ = \"other\"\n",
                "\n",
                "def helper() -> int:\n",
                "    return 1\n",
            ),
        ),
        "dep\n1\n"
    );
}

#[test]
fn a_dependencys_walrus_binding_withholds_the_entry_seed() {
    // The cross-module half of the gate, reached through the walrus door: a
    // dependency's own top-level `(__name__ := 7)` is the program's single
    // `__name__` global in #881's flat namespace, so the entry module is not
    // seeded and both modules read the dependency's value.
    assert_eq!(
        build_and_run_program(
            "dn_dep_walrus",
            "from dep import helper\n\nprint(__name__)\nprint(helper())\n",
            "print((__name__ := 7))\n\ndef helper() -> int:\n    return 1\n",
        ),
        "7\n7\n1\n"
    );
}

#[test]
fn a_binding_nested_in_a_top_level_compound_statement_withholds_the_seed() {
    // Both arms bind, so the name is bound on every path and this is an
    // ordinary `str` global with no seed behind it.
    assert_eq!(
        build_and_run(
            "dn_nested_if",
            concat!(
                "flag: bool = True\n",
                "if flag:\n",
                "    __name__ = \"custom\"\n",
                "else:\n",
                "    __name__ = \"other\"\n",
                "print(__name__)\n",
            ),
        ),
        "custom\n"
    );
}

#[test]
fn a_non_str_binding_nested_in_a_top_level_compound_statement_withholds_the_seed() {
    // The arm that made the old "benign limit" framing wrong: with a seed in
    // front of it this was `T0023`, because an `int` cannot rebind a `str`.
    assert_eq!(
        build_and_run(
            "dn_nested_if_int",
            concat!(
                "flag: bool = True\n",
                "__name__ = 0\n",
                "if flag:\n",
                "    __name__ = 7\n",
                "print(__name__)\n",
            ),
        ),
        "7\n"
    );
}

#[test]
fn a_conditionally_bound_dunder_name_behaves_like_any_other_name() {
    // THE RULE's "resolves through the ordinary name path, exactly as before
    // this change", made observable: the diagnostic is the same `T0041` a
    // plain conditionally-bound name gets, not a seed-specific one.
    let (ok, rendered) = check(
        "dn_nested_if_unbound",
        "flag: bool = True\nif flag:\n    __name__ = 7\nprint(__name__)\n",
    );
    assert!(!ok, "{rendered}");
    assert!(rendered.contains("error[T0041]"), "{rendered}");
    let (control_ok, control) = check(
        "dn_nested_if_control",
        "flag: bool = True\nif flag:\n    other = 7\nprint(other)\n",
    );
    assert!(!control_ok, "{control}");
    assert!(control.contains("error[T0041]"), "{control}");
}

// -- a function-local binding shadows only inside that function -----

#[test]
fn a_function_local_binding_shadows_only_within_that_function() {
    assert_eq!(
        build_and_run(
            "dn_local",
            "\
def local() -> None:
    __name__ = \"local\"
    print(__name__)

local()
print(__name__)
",
        ),
        "local\n__main__\n"
    );
}

// -- the multi-module gap -------------------------------------------

/// Writes a two-file program into a fresh scratch directory and runs
/// `pycc check` on its entry module, returning success plus both streams.
fn check_program(category: &str, entry: &str, dependency: &str) -> (bool, String) {
    let dir = ScratchDir::new(category).expect("scratch");
    std::fs::write(dir.join("dep.py"), dependency).expect("write the dependency");
    let src = dir.join("m.py");
    std::fs::write(&src, entry).expect("write the entry module");
    let out = pycc()
        .arg("check")
        .arg(&src)
        .output()
        .expect("pycc should spawn");
    (
        out.status.success(),
        format!("{}{}", stdout_of(&out), stderr_of(&out)),
    )
}

/// Builds a two-file program (`dep.py` plus the `m.py` entry) and returns the
/// entry artifact's stdout, panicking with the compiler's own diagnostics when
/// the build fails.
fn build_and_run_program(category: &str, entry: &str, dependency: &str) -> String {
    let dir = ScratchDir::new(category).expect("scratch");
    std::fs::write(dir.join("dep.py"), dependency).expect("write the dependency");
    let src = dir.join("m.py");
    std::fs::write(&src, entry).expect("write the entry module");
    let out = dir.join("m");
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    let run = Command::new(&out)
        .output()
        .expect("the artifact should spawn");
    assert!(run.status.success(), "{}", stderr_of(&run));
    stdout_of(&run)
}

// -- a dependency's own top-level binding withholds the entry seed --
//
// The program is one flat namespace (Part 1 of #881), so the seed and a
// dependency's binding are the same global. Seeding anyway would silently
// overwrite a `str`-valued dependency binding and would fail the whole
// program with `T0023` for any other type; both compiled before #1156, so
// both must keep compiling to the same values.

#[test]
fn a_dependencys_str_binding_withholds_the_entry_seed() {
    assert_eq!(
        build_and_run_program(
            "dn_dep_str",
            "from dep import helper\nprint(__name__)\nprint(helper())\n",
            "__name__ = \"dep\"\n\ndef helper() -> str:\n    return __name__\n",
        ),
        "dep\ndep\n"
    );
}

#[test]
fn a_dependencys_non_str_binding_withholds_the_entry_seed() {
    assert_eq!(
        build_and_run_program(
            "dn_dep_int",
            "from dep import helper\nprint(__name__)\nprint(helper())\n",
            "__name__ = 7\n\ndef helper() -> int:\n    return __name__\n",
        ),
        "7\n7\n"
    );
}

#[test]
fn a_dependency_function_body_read_withholds_the_entry_seed() {
    // A dependency that only *reads* the name still withholds the seed. An
    // earlier revision let the read observe the entry module's value, which
    // held only while nothing in the dependency ran before the seed -- and
    // `program::link` puts every dependency statement ahead of it. Withholding
    // makes the name undefined, so the read is a `T0021` rather than a value
    // whose validity depends on where the call happens to appear.
    let (ok, rendered) = check_program(
        "dn_dep_body_read",
        "from dep import report\n\nreport()\nprint(__name__)\n",
        "\
def report() -> None:
    print(__name__)
",
    );
    assert!(!ok, "{rendered}");
    assert!(rendered.contains("error[T0021]"), "{rendered}");
}

#[test]
fn a_dependencys_top_level_call_into_its_own_function_withholds_the_entry_seed() {
    // The shape a binding-only dependency test cannot see: nothing in the
    // dependency's *module scope* binds or reads the name, yet its top-level
    // call reaches a function-body read -- which runs before the entry seed
    // stores anything. Before this gate `pycc check` accepted the program and
    // the built artifact aborted at codegen's uninitialized-global trap, the
    // one outcome D-246 rules out. It is a diagnostic now.
    let (ok, rendered) = check_program(
        "dn_dep_indirect",
        "from dep import show\n\nprint(__name__)\n",
        "\
def show() -> str:
    return __name__

print(show())
",
    );
    assert!(!ok, "{rendered}");
    assert!(rendered.contains("error[T0021]"), "{rendered}");
}

#[test]
fn a_dependency_main_guard_withholds_the_entry_seed() {
    // `if __name__ == "__main__":` in a dependency is the common shape this
    // gate protects: the flat namespace would give the dependency the entry
    // module's own name, so the guard would be true in an imported module and
    // its body would run -- silently unlike CPython. Withholding the seed
    // turns that into a diagnostic instead.
    let (ok, rendered) = check_program(
        "dn_dep_guard",
        "from dep import f\n\nprint(__name__)\nprint(f())\n",
        "\
def f() -> int:
    return 1

if __name__ == \"__main__\":
    print(\"dependency main\")
",
    );
    assert!(!ok, "{rendered}");
    assert!(rendered.contains("error[T0021]"), "{rendered}");
}

#[test]
fn a_dependency_that_never_mentions_the_name_leaves_the_entry_seed_intact() {
    // The control for the three tests above: the gate keys on the dependency
    // mentioning the name, not on the program having a dependency at all.
    assert_eq!(
        build_and_run_program(
            "dn_dep_silent",
            "from dep import f\n\nprint(__name__)\nprint(f())\n",
            "def f() -> int:\n    return 1\n",
        ),
        "__main__\n1\n"
    );
}

#[test]
fn a_dependency_read_is_t0021_when_the_entry_module_never_references_the_name() {
    // No reference in the entry module means Gate 1 withholds the seed, so
    // the program has no `__name__` global at all.
    let (ok, rendered) = check_program(
        "dn_multi_unseeded",
        "from dep import report\n\nreport()\n",
        "\
def report() -> None:
    print(__name__)
",
    );
    assert!(!ok, "{rendered}");
    assert!(
        rendered.contains("error[T0021]") && rendered.contains("dep.py"),
        "{rendered}"
    );
}

#[test]
fn a_dependencys_own_top_level_read_is_t0021() {
    // Linking concatenates dependencies before the entry module, so the
    // entry's seed has not run yet when a dependency's module-level code
    // does — the type checker reports the read against the dependency.
    let (ok, rendered) = check_program(
        "dn_multi_top_level",
        "from dep import report\n\nreport()\nprint(__name__)\n",
        "\
print(__name__)


def report() -> None:
    pass
",
    );
    assert!(!ok, "{rendered}");
    assert!(
        rendered.contains("error[T0021]") && rendered.contains("dep.py"),
        "{rendered}"
    );
}

// -- `--ext` uses the extension module's own name -------------------

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_extension_module_reads_its_own_module_name() {
    let dir = ScratchDir::new("dn_ext").expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(
        &src,
        "\
def modname() -> str:
    return __name__
",
    )
    .expect("write the fixture source");
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("dunder_probe"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    let cwd: &Path = &dir;
    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(
            "\
import dunder_probe
assert dunder_probe.modname() == \"dunder_probe\", dunder_probe.modname()
assert dunder_probe.modname() == dunder_probe.__name__, dunder_probe.modname()
",
        )
        .current_dir(cwd)
        .output()
        .expect("python3 should spawn");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
}
