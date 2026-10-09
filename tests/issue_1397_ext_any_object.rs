//! D-258 (#1397): in a `pycc build --ext` artifact, `Any`, `object`, a bare
//! `list`/`dict`/`tuple`/`set`, and a container with an object-typed
//! argument are the opaque CPython object -- in a parameter, a return, a
//! local, a class-body declaration and a `self.` attribute, in every module
//! of the artifact -- and a public signature carries it across the boundary
//! as the `PyObject *` itself. A `native` build of the same source keeps
//! today's refusals.
//!
//! The non-ignored tests pin, through the CLI, the refusals that remain and
//! the codes they carry; every one fails before a C compiler or CPython is
//! needed. The hosted tests are `#[ignore]`d and contribute no line
//! coverage; the Tier-1 `native-build-test` leg runs them with `cargo test
//! --workspace -- --include-ignored`. The lines this change needs covered
//! are executed by the non-ignored tests here,
//! `crates/pycc_hir/src/import/tests/annotations_ext.rs`, and
//! `src/ext_build_tests/generated_c.rs`/`exports.rs`/`refusal_completeness.rs`.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn host_python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

fn write(dir: &Path, file: &str, body: &str) -> PathBuf {
    let path = dir.join(file);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    std::fs::write(&path, body).expect("write the fixture source");
    path
}

/// `pycc build --ext` of `body` as `m.py`; the rendered stdout and stderr.
fn build_ext(dir: &Path, body: &str) -> (Output, String) {
    let source = write(dir, "m.py", body);
    let output = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("m"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    let rendered = format!("{}{}", stdout_of(&output), stderr_of(&output));
    (output, rendered)
}

/// `body` fails `pycc build --ext` with exactly one error, `code`, whose
/// message contains `needle`.
fn assert_ext_error(tag: &str, body: &str, code: &str, needle: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    let (output, rendered) = build_ext(&dir, body);
    assert_eq!(output.status.code(), Some(1), "{tag}: {rendered}");
    assert_eq!(rendered.matches("error[").count(), 1, "{tag}: {rendered}");
    assert!(
        rendered.contains(&format!("error[{code}]")),
        "{tag}: {rendered}"
    );
    assert!(rendered.contains(needle), "{tag}: {rendered}");
}

const ANY: &str = "from typing import Any\n";

/// `Any` and `object` are not generic: a subscript on either is `T0044`.
#[test]
fn a_subscripted_any_or_object_is_t0044() {
    assert_ext_error(
        "1397_any_subscript",
        &format!("{ANY}def f(x: Any[int]) -> int:\n    return 1\n"),
        "T0044",
        "`Any` is not subscriptable, so `Any[...]` is not a valid type annotation",
    );
    assert_ext_error(
        "1397_object_subscript",
        "def f(x: object[int]) -> int:\n    return 1\n",
        "T0044",
        "builtin type `object` is not subscriptable",
    );
}

/// D-258 rule 4's literal half after Part 2d of #1371, which builds a list
/// display bound to an object slot as a CPython `list`
/// (`tests/issue_1371_object_list_display.rs`): what stays refused is a dict
/// display into the slot (`T0003`), a list element no packer boxes
/// (`I0404`), and an unannotated empty `[]` with no object evidence
/// (`T0003`).
#[test]
fn the_literals_part_2d_leaves_refused_into_an_object_slot() {
    assert_ext_error(
        "1397_empty_dict",
        "def f(x: object) -> int:\n    d: object = {}\n    return 1\n",
        "T0003",
        "an empty dict literal",
    );
    assert_ext_error(
        "1397_list_of_none",
        &format!("{ANY}def f(x: Any) -> Any:\n    y: object = [x, None]\n    return y\n"),
        "I0404",
        "a `None` element in a list display bound to a CPython object",
    );
    assert_ext_error(
        "1397_empty_list_no_evidence",
        "def f(x: object) -> int:\n    xs = []\n    return len(xs)\n",
        "T0003",
        "an empty list literal has no inferable element type for `xs`",
    );
}

/// `None` into the object (#1387): since Part 1 of #1387 a bare `return`
/// or `return None` in a `-> Any` function returns CPython's `None`
/// (`tests/issue_1387_none_into_object_return.rs`), and since #1475 a
/// `None`-typed expression there is boxed too; an `Optional` value, which
/// has no single packer, is still a return-type mismatch.
#[test]
fn an_optional_into_an_object_return_is_still_t0022() {
    assert_ext_error(
        "1397_none_return",
        &format!("{ANY}def f(x: Any, n: int | None) -> Any:\n    return n\n"),
        "T0022",
        "expected return type `object`, got `int | None`",
    );
}

/// A `native` program keeps `T0002` and the `object`/bare-container
/// `C0001`s, byte for byte, through `pycc check` and `pycc build`.
#[test]
fn a_native_build_keeps_every_refusal() {
    let dir = ScratchDir::new("1397_native").expect("scratch");
    let source = write(
        &dir,
        "m.py",
        &format!("{ANY}def f(x: Any, y: object, z: list) -> int:\n    return 1\n"),
    );
    for args in [vec!["check"], vec!["build", "-o"]] {
        let mut command = pycc();
        command.args(&args);
        if args.len() == 2 {
            command.arg(dir.join("native"));
        }
        let output = command.arg(&source).output().expect("pycc should spawn");
        let rendered = format!("{}{}", stdout_of(&output), stderr_of(&output));
        assert_eq!(output.status.code(), Some(1), "{args:?}: {rendered}");
        assert!(
            rendered.contains(
                "error[T0002]: `Any` is not permitted in pycc code outside a declared interop \
                 boundary"
            ),
            "{args:?}: {rendered}"
        );
    }
    for (file, annotation, needle) in [
        (
            "n.py",
            "object",
            "error[C0001]: type annotation `object` is not supported yet",
        ),
        (
            "l.py",
            "list",
            "error[C0001]: a bare `list` type annotation is not supported yet",
        ),
    ] {
        let output = pycc()
            .arg("check")
            .arg(write(
                &dir,
                file,
                &format!("def f(y: {annotation}) -> int:\n    return 1\n"),
            ))
            .output()
            .expect("pycc should spawn");
        let rendered = stdout_of(&output);
        assert_eq!(output.status.code(), Some(1), "{rendered}");
        assert!(rendered.contains(needle), "{rendered}");
    }
}

// ---------------------------------------------------------------------
// Hosted: the artifact against CPython's own run of the same source.
// ---------------------------------------------------------------------

fn artifact_name() -> &'static str {
    if cfg!(windows) { "m.pyd" } else { "m.abi3.so" }
}

fn run_in(dir: &Path, script: &str) -> Output {
    host_python()
        .args(["-B", "-c", script])
        .current_dir(dir)
        .env("PYTHONUNBUFFERED", "1")
        .output()
        .expect("python3 should spawn")
}

/// Builds `body` (plus any `deps` written beside it) as the extension `m`,
/// runs `script` against it and against CPython importing the same source,
/// and asserts both succeed with the same stdout, which is returned.
fn assert_matches_cpython(tag: &str, body: &str, deps: &[(&str, &str)], script: &str) -> String {
    let compiled_dir = ScratchDir::new(tag).expect("scratch");
    let source_dir = ScratchDir::new(&format!("{tag}_src")).expect("scratch");
    for (file, text) in deps {
        write(&compiled_dir, file, text);
        write(&source_dir, file, text);
    }
    let source = write(&source_dir, "m.py", body);
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(compiled_dir.join("m"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}{}",
        stdout_of(&build),
        stderr_of(&build)
    );
    assert!(compiled_dir.join(artifact_name()).is_file());
    // The dependency sources stay beside the artifact only for CPython's
    // oracle run; the compiled artifact linked them in.
    for (file, _) in deps {
        std::fs::remove_file(compiled_dir.join(file)).expect("remove dep");
    }
    let compiled = run_in(&compiled_dir, script);
    let oracle = run_in(&source_dir, script);
    for (what, run) in [("pycc", &compiled), ("cpython", &oracle)] {
        assert!(
            run.status.success(),
            "{what}: {}{}",
            stdout_of(run),
            stderr_of(run)
        );
    }
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    stdout_of(&compiled)
}

/// Every position the issue names, round-tripped through the boundary with
/// its identity intact.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn any_object_and_object_containers_cross_the_boundary_unchanged() {
    let stdout = assert_matches_cpython(
        "1397_hosted_positions",
        "from typing import Any, Dict, List\n\
         def ident(x: Any) -> Any:\n    y: Any = x\n    return y\n\
         def ident_obj(x: object) -> object:\n    return x\n\
         def keep(xs: list, d: Dict[str, Any]) -> List[Any]:\n    return xs\n\
         class Box:\n    v: Any\n\
         \x20   def __init__(self, v: Any, w: tuple) -> None:\n\
         \x20       self.v = v\n        self.w = w\n\
         \x20   def get(self) -> Any:\n        return self.v\n\
         \x20   def pair(self) -> tuple:\n        return self.w\n",
        &[],
        "import m\no = object()\nxs = [1, 'a']\nt = (1, 2)\n\
         print(m.ident(o) is o, m.ident_obj(xs) is xs, m.keep(xs, {}) is xs)\n\
         b = m.Box(o, t)\nprint(b.get() is o, b.pair() is t, m.ident(None) is None)\n",
    );
    assert_eq!(stdout, "True True True\nTrue True True\n");
}

/// D-258 rule 1 reaches every module of the artifact: a dependency's `->
/// Any` helper compiles and hands its object back unchanged.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_dependency_module_s_any_signature_compiles_in_an_ext_build() {
    let stdout = assert_matches_cpython(
        "1397_hosted_dependency",
        "from typing import Any\nfrom dep import _pick\n\
         def pick(x: Any) -> Any:\n    return _pick(x)\n",
        &[(
            "dep.py",
            "from typing import Any\ndef _pick(x: Any) -> Any:\n    return x\n",
        )],
        "import m\nxs = [3]\nprint(m.pick(xs) is xs)\n",
    );
    assert_eq!(stdout, "True\n");
}

/// A public parameter annotated with a foreign class is the same
/// `Ty::Object`, so it crosses as the object itself with no run-time class
/// check -- exactly what CPython does with an annotation, and what #1386
/// decided (D-244's #1386 amendment; `tests/issue_1386_foreign_class_boundary.rs`
/// pins the method, constructor and type-variable shapes).
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_foreign_class_parameter_passes_through_unchecked() {
    let stdout = assert_matches_cpython(
        "1397_hosted_foreign",
        "from fractions import Fraction\n\
         def num(f: Fraction) -> Fraction:\n    return f\n",
        &[],
        "import m\nfrom fractions import Fraction\nf = Fraction(3, 4)\n\
         print(m.num(f) is f, m.num('not a fraction'))\n",
    );
    assert_eq!(stdout, "True not a fraction\n");
}
