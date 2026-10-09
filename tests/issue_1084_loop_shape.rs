//! Part 5 of #1026: the end-to-end loop shape that Parts 1-4 were built for,
//! exercised as one program rather than one operation at a time.
//!
//! Parts 1-4 each pinned a single operation on a foreign `object` value:
//! `tests/issue_1080_foreign_object.rs` owns what a foreign `import` binds,
//! `tests/issue_1081_foreign_method_call.rs` the method call,
//! `tests/issue_1082_foreign_len_and_truth.rs` `len`, truth testing, the
//! subscript load and `for` iteration, and
//! `tests/issue_1083_foreign_conversions.rs` the four scalar conversions and
//! the fixed-arity all-`float` tuple unpack. None of them ran the *shape* the
//! issue was filed for: a module-scope `for` loop that calls a method on a
//! foreign object, unpacks its result into a typed tuple, and accumulates a
//! `float`. This file owns exactly that shape, and
//! `tests/issue_1084_refcount_probe.rs` owns what the shape costs in
//! references.
//!
//! **The differential oracle is valid for this subject, and the reason is
//! narrow.** D-244 rule 7 keeps the type boundary closed at the thunk export
//! seam, where NEG-005 refuses an annotated parameter whose type the exported
//! thunk cannot carry. `total_x()` takes no arguments, so this subject never
//! reaches that deviation, and the compiled artifact is therefore expected to
//! agree with CPython's own execution of the same source *byte for byte*.
//! That is what `the_loop_shape_matches_cpython` asserts. What does not
//! generalize is narrower than "any annotated parameter": D-244's Part 1
//! amendment, statement (e), records that rule 7's conforming-call oracle is
//! unaffected by widening the admitted parameter set, so a subject whose
//! every annotated parameter has a type the thunk *does* carry still has an
//! admissible oracle -- `tests/issue_1114_numpy_oracle.rs` is one, over a
//! `memoryview` and an `int`. It is a parameter whose type the thunk
//! **cannot** carry that reaches NEG-005's deviation and has no oracle,
//! which is why the #1067 harness carries none.
//!
//! **The fixture layout is deliberate.** The foreign stub `pycc_p5_mesh.py`
//! sits in the scratch directory root and the pycc entry module in `src/`
//! beneath it, with both arms run with the scratch root as the working
//! directory. Placing the stub *beside* the entry module instead makes pycc
//! resolve it as a project module and refuse the program with `C0001 module
//! namespace bindings ... not supported yet` -- the import would no longer be
//! foreign, and the test would stop testing #1026. The plain-CPython arm
//! `exec`s the entry module's source in a fresh namespace rather than running
//! it as a script, because a script run would put `src/` at the front of
//! `sys.path` and break the stub's own resolution.
//!
//! The hosted tests contribute no line coverage (CI's coverage job runs
//! `llvm-cov` without `--include-ignored`) and add no Rust lines outside
//! `tests/`; they are run by the Tier-1 `native-build-test` leg's
//! `cargo test --workspace -- --include-ignored`.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

/// Normalizes the two line-ending conventions a captured stream can carry.
///
/// Both arms below print through CPython's text layer, which translates `\n`
/// to `\r\n` on Windows, so every captured-output assertion in this file
/// normalizes first -- the same convention as
/// `tests/issue_1083_foreign_conversions.rs`.
fn normalize_newlines(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn stdout_of(output: &Output) -> String {
    normalize_newlines(&output.stdout)
}

fn stderr_of(output: &Output) -> String {
    normalize_newlines(&output.stderr)
}

/// The foreign module the subject imports. It is an ordinary CPython module
/// that pycc never compiles: `GetPoint` returns a three-`float` tuple and
/// `GetNumberOfPoints` a plain `int`, mirroring the mesh-accessor shape #1026
/// was filed against.
///
/// The methods are named `GetPoint`/`GetNumberOfPoints` rather than, say,
/// `get`/`count` on purpose: a container-named method on a foreign object
/// goes through #1095's receiver dispatch, and a name that collides with a
/// pycc builtin protocol would test that seam instead of this one.
const STUB: &str = "\
class _Grid:
    def GetPoint(self, i):
        return (float(i), 0.5, 0.25)

    def GetNumberOfPoints(self):
        return 4


GRID = _Grid()
";

/// The subject: every Part 1-4 operation the loop shape needs, in one
/// module-scope `for` loop, exported through a zero-argument thunk.
const SUBJECT: &str = "\
import pycc_p5_mesh

sx: float = 0.0
for i in range(4):
    p: tuple[float, float, float] = pycc_p5_mesh.GRID.GetPoint(i)
    sx = sx + p[0]


def total_x() -> float:
    return sx
";

/// Writes the two-file fixture and returns the scratch root both arms run in.
fn fixture(category: &str) -> ScratchDir {
    let dir = ScratchDir::new(category).expect("scratch");
    std::fs::write(dir.join("pycc_p5_mesh.py"), STUB).expect("write the foreign stub");
    std::fs::create_dir_all(dir.join("src")).expect("create the entry directory");
    std::fs::write(dir.join("src").join("mesh_probe.py"), SUBJECT).expect("write the subject");
    dir
}

/// Builds the subject as a CPython extension module directly into `dir`, so
/// that a CPython run with `dir` as its working directory imports it.
///
/// The output path carries no extension suffix, exactly as
/// `tests/issue_1081_foreign_method_call.rs`'s own helper writes it:
/// `pycc build --ext` appends the one its target triple calls for, and it
/// derives the exported `PyInit_<mod>` name from the path's own spelling.
/// Spelling `.abi3.so` here builds on Unix and then fails the `--ext` name
/// contract on Windows, where the module name would come out as
/// `mesh_probe.abi3.so` and is not an identifier.
fn build_ext(dir: &Path) -> Output {
    pycc()
        .arg("build")
        .arg(dir.join("src").join("mesh_probe.py"))
        .arg("-o")
        .arg(dir.join("mesh_probe"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn")
}

/// Runs `script` under the host CPython with `dir` as the working directory.
fn python(dir: &Path, script: &str) -> Output {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn")
}

/// `pycc check` admits the whole shape -- the foreign method call and the
/// fixed-arity tuple unpack, inside one module-scope loop.
///
/// The `p[0]` that follows is a *native* index into the unpacked
/// `tuple[float, float, float]`, not a foreign subscript load;
/// `pycc_ext_obj_getitem` is `tests/issue_1082_foreign_len_and_truth.rs`'s.
///
/// This arm needs no CPython headers, so it is the non-ignored guard that the
/// shape stays admitted even where the hosted arms cannot run.
#[test]
fn the_loop_shape_type_checks() {
    let dir = fixture("p5_loop_shape_check");
    let check = pycc()
        .arg("check")
        .arg(dir.join("src").join("mesh_probe.py"))
        .current_dir(&*dir)
        .output()
        .expect("pycc should spawn");
    assert!(
        check.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&check),
        stderr_of(&check)
    );
}

/// The shape builds as an extension module and the host imports it and calls
/// the exported thunk.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_loop_shape_builds_and_runs_in_the_host() {
    let dir = fixture("p5_loop_shape_hosted");
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let run = python(&dir, "import mesh_probe; print(mesh_probe.total_x())");
    assert!(run.status.success(), "{}", stderr_of(&run));
    // 0.0 + 1.0 + 2.0 + 3.0, the x components `GetPoint` returns.
    assert_eq!(stdout_of(&run), "6.0\n");
}

/// The differential oracle: the compiled artifact and CPython's own execution
/// of the identical source, under the identical interpreter, produce the same
/// `repr`.
///
/// `repr` rather than `str` so that a `float` result is compared at full
/// precision. Both arms print through the same interpreter, so a difference
/// here is a difference in what pycc computed, never in how the two were
/// formatted.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_loop_shape_matches_cpython() {
    let dir = fixture("p5_loop_shape_oracle");
    let build = build_ext(&dir);
    assert!(build.status.success(), "{}", stderr_of(&build));

    let compiled = python(&dir, "import mesh_probe; print(repr(mesh_probe.total_x()))");
    assert!(compiled.status.success(), "{}", stderr_of(&compiled));

    let interpreted = python(
        &dir,
        "ns = {}\n\
         exec(open('src/mesh_probe.py').read(), ns)\n\
         print(repr(ns['total_x']()))\n",
    );
    assert!(interpreted.status.success(), "{}", stderr_of(&interpreted));

    assert_eq!(
        stdout_of(&compiled),
        stdout_of(&interpreted),
        "the compiled artifact and CPython disagree on the same source"
    );
}
