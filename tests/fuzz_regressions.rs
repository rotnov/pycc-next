//! Regressions found by the differential fuzzer (`fuzz/`, v0.6 accept 4):
//! each fixture `tests/fixtures/fuzz_<topic>.py` is a reduced program that
//! once printed or raised something CPython does not. It is built in strict
//! mode, in `--non-strict` mode and as an `--ext` module imported by CPython,
//! and every run is compared with the live CPython oracle (`PYCC_PYTHON`,
//! default `python3`): stdout byte-for-byte, the uncaught exception's last
//! stderr line (with the module prefix of a class the program defines
//! removed) and the exit status.

#[path = "stdlib_support/mod.rs"]
mod support;

use pycc_scratch::ScratchDir;
use std::process::{Command, Output};
use support::{fixture, pycc, python, rendered};

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

/// stdout, the last stderr line without a `module.` prefix, and the status.
fn observed(output: &Output, module: &str) -> (String, String, Option<i32>) {
    let last = text(&pycc_scratch::lf(&output.stderr))
        .lines()
        .last()
        .unwrap_or_default()
        .to_string();
    let last = last
        .strip_prefix(&format!("{module}."))
        .map(str::to_string)
        .unwrap_or(last);
    (text(&pycc_scratch::lf(&output.stdout)), last, output.status.code())
}

/// Builds `tests/fixtures/<name>.py` in every mode and compares each run with
/// CPython's. `env` is applied to every run (the oracle's too).
fn assert_matches_cpython_with(name: &str, env: &[(&str, &str)]) {
    let source = fixture(&format!("{name}.py"));
    let mut oracle = Command::new(python());
    oracle.arg(&source).envs(env.iter().copied());
    let expected = observed(&oracle.output().expect("the oracle runs"), "__main__");
    let dir = ScratchDir::new(&format!("fuzz_{name}")).expect("scratch");
    for (mode, flags) in [
        ("strict", &["--release"][..]),
        ("non-strict", &["--release", "--non-strict"][..]),
    ] {
        let binary = dir.join(format!("app-{mode}"));
        let build = pycc()
            .arg("build")
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .args(flags)
            .output()
            .expect("pycc spawns");
        assert!(
            build.status.success(),
            "{name} ({mode}): {}",
            rendered(&build)
        );
        let mut run = Command::new(&binary);
        run.envs(env.iter().copied());
        let got = observed(&run.output().expect("the program runs"), "__main__");
        assert_eq!(got, expected, "{name} ({mode})");
    }
    let ext = dir.join("ext");
    std::fs::create_dir_all(&ext).expect("ext dir");
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(ext.join(name))
        .arg("--ext")
        .output()
        .expect("pycc spawns");
    assert!(
        build.status.success(),
        "{name} (--ext): {}",
        rendered(&build)
    );
    let mut run = Command::new(python());
    run.current_dir(&ext)
        .arg("-c")
        .arg(format!("import {name}"))
        .envs(env.iter().copied());
    let got = observed(&run.output().expect("python3 runs"), name);
    assert_eq!(got, expected, "{name} (--ext)");
}

fn assert_matches_cpython(name: &str) {
    assert_matches_cpython_with(name, &[]);
}

/// A `--non-strict` native program's embedded CPython writes `sys.stdout` as
/// `python3` does: UTF-8 under the C/POSIX locale (PEP 540) and under a UTF-8
/// locale, so a statement running in CPython prints non-ASCII text.
#[test]
#[cfg(unix)]
fn embedded_cpython_prints_utf8_under_the_c_locale() {
    let source = fixture("fuzz_fallback_utf8_stdout.py");
    let dir = ScratchDir::new("fuzz_utf8_stdout").expect("scratch");
    let binary = dir.join("app");
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .args(["--release", "--non-strict"])
        .output()
        .expect("pycc spawns");
    assert!(build.status.success(), "{}", rendered(&build));
    for locale in [None, Some("C"), Some("POSIX"), Some("C.UTF-8")] {
        let run = |program: &mut Command| {
            program
                .env_remove("LANG")
                .env_remove("LC_CTYPE")
                .env_remove("LC_ALL");
            program
                .env_remove("PYTHONIOENCODING")
                .env_remove("PYTHONUTF8");
            if let Some(locale) = locale {
                program.env("LC_ALL", locale);
            }
            observed(&program.output().expect("spawns"), "__main__")
        };
        let expected = run(Command::new(python()).arg(&source));
        assert_eq!(expected.2, Some(0), "{expected:?}");
        assert_eq!(
            run(&mut Command::new(&binary)),
            expected,
            "LC_ALL={locale:?}"
        );
    }
}

/// `extend`, `+=` and `insert` on a bounded `deque` respect `maxlen`:
/// extending drops from the left, inserting into a full deque raises
/// CPython's `IndexError`.
#[test]
fn deque_maxlen_extend_and_insert() {
    assert_matches_cpython("fuzz_deque_maxlen");
}

/// A refused `**mapping` call near the end of the file reports a diagnostic
/// at the call: the generated binding helper's statements and parameters
/// once kept offsets past the end of the source, and rendering them panicked.
#[test]
fn a_refused_double_star_call_is_reported_at_the_call() {
    let dir = ScratchDir::new("fuzz_double_star_span").expect("scratch");
    let path = dir.join("subject.py");
    let source = "def main() -> None:\n\n    def opts(name: str, **kwargs: int) -> str:\n        return name\n    opts(**3)\nmain\n";
    std::fs::write(&path, source).expect("write");
    let build = |flags: &[&str]| {
        pycc()
            .arg("build")
            .arg(&path)
            .arg("-o")
            .arg(dir.join("prog"))
            .args(flags)
            .output()
            .expect("pycc spawns")
    };
    let strict = build(&[]);
    let text = rendered(&strict);
    assert!(!strict.status.success(), "{text}");
    assert!(!text.contains("panicked"), "{text}");
    assert!(text.contains("subject.py:5:5"), "{text}");
    // Non-strict: `main` runs in CPython; it is never called.
    let non_strict = build(&["--non-strict"]);
    assert!(non_strict.status.success(), "{}", rendered(&non_strict));
    let run = Command::new(dir.join("prog")).output().expect("runs");
    assert_eq!(
        observed(&run, "__main__"),
        (String::new(), String::new(), Some(0))
    );
}

/// `repr()` of a `str` escapes the characters `str.isprintable()` rejects
/// beyond ASCII, as CPython does.
#[test]
fn str_repr_escapes_unprintable_unicode() {
    assert_matches_cpython("fuzz_str_repr_unicode");
}

/// The same order when the display is built as a CPython object
/// (`--non-strict`).
#[test]
fn a_constant_set_display_object_iterates_in_cpythons_order() {
    let source = fixture("fuzz_constant_set_object.py");
    let expected = support::outcome(&Command::new(python()).arg(&source).output().expect("runs"));
    let got = support::run_native("fuzz_constant_set_object", &source, &["--release", "--non-strict"]);
    assert_eq!(got, expected);
}

/// `repr()` of a float with two shortest round-tripping strings is the one
/// nearest its exact value, ties to even (`137931034482758.62`).
#[test]
fn float_repr_breaks_ties_like_cpython() {
    assert_matches_cpython("fuzz_float_repr_ties");
}

/// `for ... in zip(...)` sees the sequences change under it as CPython's
/// iterators do (shrinking, growing, the name rebound), `else` included.
#[test]
fn zip_loops_follow_their_sequences_as_they_change() {
    assert_matches_cpython("fuzz_zip_mutation");
}

/// `match`: a starred capture binds the remaining elements, a nested
/// or-pattern is an `or`, a missing mapping key fails the case; a star
/// before the end and a `**rest` capture are refused, not miscompiled.
#[test]
fn match_sequence_mapping_and_or_patterns() {
    assert_matches_cpython("fuzz_match_patterns");
    for (source, expected) in [
        (
            "xs: list[int] = [1]\nmatch xs:\n    case [*a, b]:\n        print(b)\n    case _:\n        pass\n",
            "a starred sub-pattern before the end of a sequence pattern is not supported yet",
        ),
        (
            "d: dict[str, int] = {}\nmatch d:\n    case {'k': 1, **rest}:\n        print(rest)\n    case _:\n        pass\n",
            "a `**rest` capture in a mapping pattern is not supported yet",
        ),
    ] {
        let text = support::check_fails(source);
        assert!(text.contains(expected), "{text}");
    }
}

/// A set display of constants iterates in CPython's order (its folded
/// frozenset constant's), not in the order of one-by-one insertion.
#[test]
fn a_constant_set_display_iterates_in_cpythons_order() {
    assert_matches_cpython("fuzz_constant_set_display");
}

/// `crates/pycc_rt/src/unicode_tables.rs` matches the oracle's own
/// `str.isprintable()` and `unicodedata.decimal` (its generator's `--check`).
#[test]
#[ignore = "requires the CPython oracle (python3) on PATH"]
fn the_unicode_tables_are_current() {
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/gen_unicode_tables.py");
    let check = Command::new(python())
        .arg(script)
        .arg("--check")
        .output()
        .expect("python3 runs");
    assert!(check.status.success(), "{}", rendered(&check));
}

/// `pow(a, b, m)` with a bigint operand, and `round()` of a float beyond
/// 63 bits, compute the bigint result.
#[test]
fn bigint_powmod_and_round() {
    assert_matches_cpython("fuzz_bigint_powmod_round");
}

/// An `--ext --non-strict` module's exception class with its own members
/// (`__init__`, a method, a class attribute) runs in CPython, so Python code
/// (the module's own fallback code and an importer) sees those members.
#[test]
fn ext_non_strict_exception_classes_keep_their_members() {
    let source = fixture("fuzz_ext_exception_members.py");
    let dir = ScratchDir::new("fuzz_ext_exception_members").expect("scratch");
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("fuzz_ext_exception_members"))
        .args(["--ext", "--non-strict"])
        .output()
        .expect("pycc spawns");
    assert!(build.status.success(), "{}", rendered(&build));
    let driver = "import sys\nsys.path.insert(0, sys.argv[1])\nimport fuzz_ext_exception_members as m\n\
                  e = m.SubErr(7)\nprint(repr(e), e.code, isinstance(e, m.Err))\n\
                  try:\n    m.boom(2)\nexcept m.E as caught:\n    print(repr(caught), caught.hello(), caught.level)\n\
                  print(m.boom(-1), m.risky(4))\n";
    let run = |dir: &std::path::Path| {
        let output = Command::new(python())
            .args(["-B", "-c", driver])
            .arg(dir)
            .output()
            .expect("python3 runs");
        observed(&output, "fuzz_ext_exception_members")
    };
    let expected = run(source.parent().expect("fixtures"));
    assert_eq!(expected.2, Some(0), "{expected:?}");
    assert_eq!(run(&dir), expected);
}

/// A `KeyError` for an `int` or tuple key escaping an `--ext` module holds
/// the key itself (`KeyError: 0`), as CPython's does, not the key's text.
#[test]
fn key_error_for_a_non_str_key_crosses_the_ext_boundary() {
    assert_matches_cpython("fuzz_key_error_int_key");
}

/// A `str` method run through CPython inside another method call's
/// arguments is called on its own receiver, not on the outer call's.
#[test]
fn a_fallback_str_method_inside_a_method_call() {
    let source = fixture("fuzz_fallback_str_method_in_call.py");
    let expected = support::outcome(&Command::new(python()).arg(&source).output().expect("runs"));
    let flags = &["--release", "--non-strict"][..];
    let got = support::run_native("fuzz_fallback_str_method", &source, flags);
    assert_eq!(got, expected, "{flags:?}");
}

/// An `--ext --non-strict` module's twinned function is defined where its
/// `def` is, even with no blank line after it, so CPython code right after
/// it can call it.
#[test]
fn a_twin_is_defined_before_the_next_statement() {
    assert_ext_non_strict_matches("fuzz_twin_before_use");
}

/// An `--ext --non-strict` module's compiled `except AppError:` catches an
/// exception of a subclass that runs in CPython (one defining `__init__`):
/// the bridge maps it to the compiled class's tag.
#[test]
fn a_compiled_handler_catches_a_fallback_exception_subclass() {
    assert_ext_non_strict_matches("fuzz_ext_exception_subclass");
}

/// Builds `tests/fixtures/<name>.py` as an `--ext --non-strict` module and
/// compares importing it with running the source under CPython.
fn assert_ext_non_strict_matches(name: &str) {
    let source = fixture(&format!("{name}.py"));
    let expected = observed(&Command::new(python()).arg(&source).output().expect("runs"), "__main__");
    let dir = ScratchDir::new(&format!("fuzz_{name}")).expect("scratch");
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join(name))
        .args(["--ext", "--non-strict"])
        .output()
        .expect("pycc spawns");
    assert!(build.status.success(), "{}", rendered(&build));
    let run = Command::new(python())
        .current_dir(&*dir)
        .args(["-c", &format!("import {name}")])
        .output()
        .expect("python3 runs");
    assert_eq!(observed(&run, name), expected);
}

/// `break` out of a `for` over a `str` or a tuple skips its `else:`.
#[test]
fn for_else_over_a_str_or_tuple_respects_break() {
    assert_matches_cpython("fuzz_for_else_str");
}

/// `int()`/`float()` of a `str` read Unicode decimal digits.
#[test]
fn int_and_float_read_unicode_decimal_digits() {
    assert_matches_cpython("fuzz_unicode_digits");
}

/// A set display meeting a CPython object crosses as a `set`, not a
/// `frozenset` (`--non-strict`).
#[test]
fn a_set_display_object_operand_crosses_as_a_set() {
    let source = fixture("fuzz_set_display_object_operand.py");
    let expected = support::outcome(&Command::new(python()).arg(&source).output().expect("runs"));
    let got = support::run_native("fuzz_set_operand", &source, &["--release", "--non-strict"]);
    assert_eq!(got, expected);
}

/// Every argument of `print` is evaluated before any is converted: a dict
/// or set a later argument mutates prints as it ends up.
#[test]
fn print_converts_containers_after_evaluating_every_argument() {
    assert_matches_cpython("fuzz_print_argument_order");
}

/// The same `KeyError` uncaught in a `--non-strict` native program that
/// embeds CPython (a class runs there) also holds the key itself.
#[test]
fn key_error_for_a_non_str_key_in_an_embedding_program() {
    let source = fixture("fuzz_key_error_int_key_fallback.py");
    let expected = support::outcome(&Command::new(python()).arg(&source).output().expect("runs"));
    let got = support::run_native(
        "fuzz_key_error_fallback",
        &source,
        &["--release", "--non-strict"],
    );
    assert_eq!(got, expected);
}

/// An unknown keyword that compiled code binds itself gets CPython 3.13's
/// "Did you mean" when a parameter name is close enough (issue #10).
#[test]
fn an_unexpected_keyword_suggests_a_close_parameter() {
    assert_matches_cpython("fuzz_unexpected_keyword_suggestion");
}
