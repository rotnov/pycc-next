//! Part 4 of #1026: an annotated module-level assignment of a CPython object
//! to a fixed-arity all-`float` `tuple` (PR 4c of #1083).
//!
//! `tests/issue_1080_foreign_object.rs` owns what a foreign `import` binds
//! and the refusal table around it; `tests/issue_1081_foreign_method_call.rs`
//! owns the method call; `tests/issue_1082_foreign_len_and_truth.rs` owns
//! `len`, truth testing, the subscript load and `for` iteration;
//! `tests/issue_1083_foreign_conversions.rs` owns the four scalar
//! conversions. This file owns the tuple unpack.
//!
//! **Strict container, converting elements.** The shim helper
//! `pycc_ext_obj_unpack_float_tuple` checks `PyTuple_Check` -- not
//! `CheckExact`, so a `tuple` subclass such as a structseq is admitted --
//! with exactly the declared arity, and then converts each item with
//! `PyNumber_Float`. The container half is strict because D-115/D-116 leave
//! no shape for a differently sized sequence: the destination is a by-value
//! LLVM struct of exactly `arity` `double`s, fixed at compile time. The
//! element half converts because the author wrote `float` in the
//! annotation, which is the same explicit-conversion reasoning PR 4a's
//! module doc sets out for `float(o)` -- D-244 rule 7 closes the *thunk
//! export seam*, where a value crosses implicitly, not a conversion the
//! source names. The runtime object's items are therefore never
//! type-checked; a bad item fails at run time with whatever exception
//! CPython's own protocol raises, which
//! `a_non_numeric_item_raises_in_the_host` pins as an ordinary `ValueError`
//! rather than anything pycc invents.
//!
//! **The admission is module-level and annotation-driven only.** PEP 585's
//! variadic `tuple[float, ...]` has no fixed arity and stays refused, as
//! does any mixed annotation; the same statement inside a function body
//! is refused too -- #1316 admits the read of `gc` there, and the ordinary
//! assignability check refuses the `object` value with `T0025`.
//! `crates/pycc_types/src/tests/foreign_float_tuple.rs` pins each of those
//! refusals against the checker directly; the tests below pin the two that
//! are worth seeing through the public CLI.
//!
//! The hosted tests contribute no line coverage (CI's coverage job runs
//! `llvm-cov` without `--include-ignored`); they are run by the Tier-1
//! `native-build-test` leg's `cargo test --workspace -- --include-ignored`.
//! Every new Rust line is covered by the non-ignored tests here, by
//! `crates/pycc_codegen/src/foreign_len.rs`'s IR assertions, and by the unit
//! tests in `crates/pycc_types` and `crates/pycc_mir`.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

/// Normalizes the two line-ending conventions a single captured stream can
/// carry at once, for the reason
/// `tests/issue_1083_foreign_conversions.rs`'s own copy states: a
/// pycc-compiled `print` writes `\n` verbatim while the host CPython
/// driver's `print` goes through CPython's text layer, which translates to
/// `\r\n` on Windows.
fn normalize_newlines(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn stdout_of(output: &Output) -> String {
    normalize_newlines(&output.stdout)
}

fn stderr_of(output: &Output) -> String {
    normalize_newlines(&output.stderr)
}

fn check(dir: &Path, body: &str) -> Output {
    let src = dir.join("m.py");
    std::fs::write(&src, body).expect("write the fixture source");
    pycc()
        .arg("check")
        .arg(&src)
        .output()
        .expect("pycc should spawn")
}

fn build_ext(dir: &Path, module: &str, body: &str) {
    let src = dir.join(format!("{module}.py"));
    std::fs::write(&src, body).expect("write the fixture source");
    let build = pycc()
        .arg("build")
        .arg(&src)
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

/// Every fixed arity is admitted, and the bound name is an ordinary tuple.
///
/// The arity is a parameter of the shim helper rather than a constant, so
/// the interesting evidence is that more than one of them type-checks --
/// `tuple[float, float, float]` is the plan's motivating shape, but nothing
/// in the admission rule is specific to three. The second half of each body
/// is what makes the first useful: indexing the bound name and doing
/// arithmetic on the result both go through the ordinary `float` paths, so
/// the unpacked value is not something that has to stay anonymous the way
/// the object itself does.
#[test]
fn every_fixed_arity_all_float_tuple_annotation_is_admitted() {
    let dir = ScratchDir::new("foreign_float_tuple_admitted").expect("scratch");
    for annotation in [
        "tuple[float]",
        "tuple[float, float]",
        "tuple[float, float, float]",
        "tuple[float, float, float, float, float]",
    ] {
        let body =
            format!("import gc\n\nv: {annotation} = gc.get_threshold()\nprint(v[0] + 1.0)\n");
        let output = check(&dir, &body);
        assert!(
            output.status.success(),
            "{annotation} should be admitted\nstdout: {}\nstderr: {}",
            stdout_of(&output),
            stderr_of(&output)
        );
    }
}

/// The two shapes a reader is most likely to reach for next stay refused.
///
/// A mixed annotation has no all-`float` destination to unpack into, and
/// PEP 585's variadic `tuple[float, ...]` has no arity at all -- the helper
/// takes the arity as an argument precisely because it must always be
/// known. Neither refusal is new: both are the unchanged assignability
/// diagnostic the checker already produced for a foreign object, which is
/// the point worth pinning here. The variadic form is refused one layer
/// earlier still, at annotation lowering, so its code differs from the
/// mixed form's, and a `str` element is refused earlier still by D-116's
/// own element-type gate, which is why the table carries the code per row.
#[test]
fn a_mixed_or_variadic_tuple_annotation_keeps_its_refusal() {
    let dir = ScratchDir::new("foreign_float_tuple_refused").expect("scratch");
    for (code, annotation) in [
        ("T0025", "tuple[float, int]"),
        ("T0025", "tuple[int, int, int]"),
        ("T0039", "tuple[float, str]"),
        ("T0053", "tuple[float, ...]"),
    ] {
        let body = format!("import gc\n\nv: {annotation} = gc.get_threshold()\n");
        let output = check(&dir, &body);
        assert!(
            !output.status.success(),
            "{annotation} should be refused\nstdout: {}",
            stdout_of(&output)
        );
        let text = stdout_of(&output);
        assert!(
            text.contains(code),
            "{annotation} should be refused with {code}, got:\n{text}"
        );
    }
}

/// The refusal for every *other* `object` operation names this one now.
///
/// `object_operation_unsupported`'s message enumerates what #1026 has
/// implemented, and PR 4c narrows it by one entry. The
/// `tests/diagnostics/i0404_foreign_module_operation.expected.txt` fixture
/// pins the whole rendering; this asserts only the clause, so that a future
/// part adding a further operation does not have to edit this file to keep
/// the CLI-level evidence honest.
#[test]
fn the_unchanged_refusal_now_names_the_admitted_assignment() {
    let dir = ScratchDir::new("foreign_float_tuple_message").expect("scratch");
    // A `match` subject is still refused; `print(gc)`, the former probe, is
    // admitted since #1340.
    let output = check(
        &dir,
        "import gc\n\nmatch gc:\n    case 1:\n        print(1)\n",
    );
    assert!(!output.status.success(), "{}", stdout_of(&output));
    let text = stdout_of(&output);
    assert!(
        text.contains("an annotated module-level assignment to a fixed-arity all-`float` `tuple`"),
        "{text}"
    );
}

/// The unpacked tuple reaches the host as ordinary `float`s.
///
/// `gc.get_threshold()` answers a genuine `tuple` of three `int`s, so this
/// exercises both halves of the rule at once: the container passes
/// `PyTuple_Check` with the declared arity, and `PyNumber_Float` widens each
/// `int` item. Comparing against the host interpreter's own answer rather
/// than a literal keeps the test honest on a CPython whose thresholds
/// differ from this machine's.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_unpacked_tuple_reaches_the_host() {
    let dir = ScratchDir::new("foreign_float_tuple_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_float_tuple_mod",
        "import gc\n\
         \n\
         v: tuple[float, float, float] = gc.get_threshold()\n\
         print(v[0])\n\
         print(v[1])\n\
         print(v[2])\n",
    );
    let run = python(&dir, "import pycc_float_tuple_mod\n");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    let expected = python(
        &dir,
        "import gc\n\
         for item in gc.get_threshold():\n\
         \x20   print(float(item))\n",
    );
    assert!(expected.status.success(), "{}", stderr_of(&expected));
    assert_eq!(
        stdout_of(&run),
        stdout_of(&expected),
        "stderr: {}",
        stderr_of(&run)
    );
}

/// A non-tuple operand fails at run time, naming the declared arity.
///
/// This is the strict half of the rule seen from the host: `gc.garbage` is
/// a `list`, which no arity makes acceptable, so the module-exec function
/// takes the unconditional failure edge and CPython propagates the
/// `TypeError` out of the `import` itself. Nothing is printed, which is the
/// other half of the evidence -- the failure is not a value the compiled
/// code carries on with.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_non_tuple_object_raises_type_error_in_the_host() {
    let dir = ScratchDir::new("foreign_float_tuple_bad_container_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_float_tuple_bad_mod",
        "import gc\n\
         \n\
         v: tuple[float, float, float] = gc.garbage\n\
         print(v[0])\n",
    );
    let run = python(
        &dir,
        "try:\n\
         \x20   import pycc_float_tuple_bad_mod\n\
         except TypeError as e:\n\
         \x20   print(e)\n",
    );
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    assert_eq!(
        stdout_of(&run),
        "expected a tuple of 3 floats, got a 'list' object\n",
        "stderr: {}",
        stderr_of(&run)
    );
}

/// A tuple of the wrong length fails at run time too, and says so.
///
/// The arity gate is separate from the type gate so that this case gets its
/// own message: a reader who declared two `float`s against a three-element
/// tuple is told the length, not merely that the object is unacceptable.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_wrong_length_tuple_raises_type_error_in_the_host() {
    let dir = ScratchDir::new("foreign_float_tuple_bad_arity_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_float_tuple_arity_mod",
        "import gc\n\
         \n\
         v: tuple[float, float] = gc.get_threshold()\n\
         print(v[0])\n",
    );
    let run = python(
        &dir,
        "try:\n\
         \x20   import pycc_float_tuple_arity_mod\n\
         except TypeError as e:\n\
         \x20   print(e)\n",
    );
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    assert_eq!(
        stdout_of(&run),
        "expected a tuple of 2 floats, got a tuple of length 3\n",
        "stderr: {}",
        stderr_of(&run)
    );
}

/// A non-numeric item fails at run time with CPython's own exception.
///
/// The elements are converted, never type-checked, so pycc has no
/// item-level refusal of its own to raise: `sys.version_info` is a genuine
/// `tuple` subclass of the declared length whose fourth item is the string
/// `'final'`, and what reaches the importer is exactly the `ValueError`
/// `float('final')` raises in the interpreter. That is the documented
/// consequence of the converting half of the rule, not a gap in it.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_non_numeric_item_raises_in_the_host() {
    let dir = ScratchDir::new("foreign_float_tuple_bad_item_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_float_tuple_item_mod",
        "import sys\n\
         \n\
         v: tuple[float, float, float, float, float] = sys.version_info\n\
         print(v[0])\n",
    );
    let run = python(
        &dir,
        "try:\n\
         \x20   import pycc_float_tuple_item_mod\n\
         except ValueError as e:\n\
         \x20   print(\"ValueError\")\n",
    );
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    assert_eq!(
        stdout_of(&run),
        "ValueError\n",
        "stderr: {}",
        stderr_of(&run)
    );
}
