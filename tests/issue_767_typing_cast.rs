// Issue #767: `typing.cast(T, value)` is a special-cased builtin call.
//
// `cast` is a runtime no-op in CPython -- it only changes a static
// checker's view of `value`'s type -- so pycc type-checks the call as `T`
// and lowers the whole expression to `value` alone at MIR time. Nothing
// about `cast` ever reaches codegen.
//
// These tests exercise the fix end-to-end through the public CLI: `pycc
// check` accepts the import and the call, and `pycc build` produces a
// binary whose output is identical to the same program with every `cast(T,
// v)` replaced by `v` alone -- the observable definition of "runtime
// no-op".

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write_fixture(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(source.as_bytes()).unwrap();
    path
}

/// The `cast`-using program. Covers a scalar cast inside an annotated
/// function, a scalar cast at module level, and a cast to a user-defined
/// class whose attribute is then read through the cast result.
const WITH_CAST: &str = "\
from typing import cast

class Box:
    def __init__(self, v: int) -> None:
        self.v = v

def bump(x: int) -> int:
    return cast(int, x) + 1

def unwrap(b: Box) -> int:
    return cast(Box, b).v

print(bump(41))
print(cast(str, \"ok\"))
print(unwrap(Box(7)))
";

/// The same program with every `cast(T, v)` replaced by `v` alone. If
/// `cast` really is a no-op, the two binaries print byte-identical output.
const WITHOUT_CAST: &str = "\
class Box:
    def __init__(self, v: int) -> None:
        self.v = v

def bump(x: int) -> int:
    return x + 1

def unwrap(b: Box) -> int:
    return b.v

print(bump(41))
print(\"ok\")
print(unwrap(Box(7)))
";

fn build_and_run(dir: &std::path::Path, stem: &str, source: &str) -> String {
    let src = write_fixture(dir, &format!("{stem}.py"), source);
    let exe = dir.join(stem);
    let build = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", exe.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "pycc build should succeed for `{stem}`; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&exe).output().unwrap();
    assert!(
        run.status.success(),
        "`{stem}` should exit successfully; stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    String::from_utf8_lossy(&run.stdout).into_owned()
}

/// #767: `pycc check` accepts `from typing import cast` together with
/// scalar and user-class casts. Before this change the import alone failed
/// with `C0002` ("module `typing` has no importable symbol named `cast`").
#[test]
fn typing_cast_import_and_calls_check_successfully() {
    let dir = ScratchDir::new("767_check").expect("failed to create scratch dir");
    let src = write_fixture(&dir, "with_cast.py", WITH_CAST);
    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "pycc check should succeed for `from typing import cast`; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// #767: `pycc build` compiles the `cast`-using program, and the binary
/// prints exactly what the cast-free equivalent prints -- the observable
/// definition of `cast` being a runtime no-op, and the check that the MIR
/// seam really elides the call instead of emitting one codegen cannot
/// lower.
#[test]
fn typing_cast_build_output_matches_the_cast_free_equivalent() {
    let dir = ScratchDir::new("767_build").expect("failed to create scratch dir");
    let with_cast = build_and_run(&dir, "with_cast", WITH_CAST);
    let without_cast = build_and_run(&dir, "without_cast", WITHOUT_CAST);
    assert_eq!(
        with_cast, without_cast,
        "`cast(T, v)` must produce exactly the output of `v` alone"
    );
    assert_eq!(with_cast, "42\nok\n7\n", "unexpected program output");
}

/// #767: a program defining its own `def cast(...)` calls that function,
/// not the builtin special case -- the same user-definition-takes-priority
/// rule `float`/`isinstance`/`issubclass` follow, verified at runtime
/// rather than only in the type checker.
#[test]
fn a_user_defined_cast_function_takes_priority_end_to_end() {
    let dir = ScratchDir::new("767_shadow").expect("failed to create scratch dir");
    let out = build_and_run(
        &dir,
        "user_cast",
        "def cast(a: int, b: int) -> int:\n    return a * b\n\nprint(cast(6, 7))\n",
    );
    assert_eq!(
        out, "42\n",
        "a user-defined `cast` must be called, not elided to its second argument"
    );
}

/// #767 review fix (second pass, D-198): a genuine down-cast to a subclass
/// with an attribute the source class lacks is rejected by `pycc check`
/// itself, before `pycc build` ever lowers it to MIR. This is the CLI-level
/// counterpart to `cast_down_to_a_derived_class_is_c0001` in
/// `pycc_types::tests` -- proving the rejection actually happens on the
/// public `check` path, not only inside the crate's own unit-test harness.
/// Without the `check_cast` fix this program used to type-check and then
/// either panic in `pycc_mir` (an unannotated binding never reaches
/// `Derived`'s slot layout at all) or abort at runtime with an
/// out-of-bounds `pycc_rt` instance-slot read.
#[test]
fn a_down_cast_to_a_derived_class_is_rejected_before_build() {
    let dir = ScratchDir::new("767_downcast").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "down_cast.py",
        "from typing import cast\n\nclass Base:\n    def __init__(self, a: int) -> None:\n        self.a = a\n\nclass Derived(Base):\n    def __init__(self, a: int, b: int) -> None:\n        self.a = a\n        self.b = b\n\ndef f(base: Base) -> int:\n    return cast(Derived, base).b\n\nprint(f(Base(1)))\n",
    );
    let check = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !check.status.success(),
        "pycc check should reject a down-cast to a class with extra attributes"
    );
    let stdout = String::from_utf8_lossy(&check.stdout);
    assert!(
        stdout.contains("C0001") && stdout.contains("narrow the value's attribute layout"),
        "expected a C0001 layout-narrowing diagnostic, got: {stdout}"
    );
}

/// #767 review fix (third pass, D-198): an up-cast that crosses no
/// method-override boundary builds and runs, and calling an inherited
/// (non-overridden) method through the cast result produces exactly the
/// output calling it on the original value would -- the CLI-level positive
/// counterpart to the down-cast rejection above, and the empirical check the
/// reviewer's own `note` finding asked for: this is the up-cast build+run
/// path no earlier test exercised end-to-end.
#[test]
fn an_up_cast_with_no_overridden_methods_builds_and_runs() {
    let dir = ScratchDir::new("767_upcast").expect("failed to create scratch dir");
    let with_cast = build_and_run(
        &dir,
        "with_upcast",
        "from typing import cast\n\nclass Base:\n    def __init__(self, a: int) -> None:\n        self.a = a\n    def describe(self) -> int:\n        return self.a\n\nclass Derived(Base):\n    def __init__(self, a: int, b: int) -> None:\n        self.a = a\n        self.b = b\n\ndef f(d: Derived) -> int:\n    return cast(Base, d).describe()\n\nprint(f(Derived(3, 4)))\n",
    );
    let without_cast = build_and_run(
        &dir,
        "without_upcast",
        "class Base:\n    def __init__(self, a: int) -> None:\n        self.a = a\n    def describe(self) -> int:\n        return self.a\n\nclass Derived(Base):\n    def __init__(self, a: int, b: int) -> None:\n        self.a = a\n        self.b = b\n\ndef f(d: Derived) -> int:\n    return d.describe()\n\nprint(f(Derived(3, 4)))\n",
    );
    assert_eq!(
        with_cast, without_cast,
        "an up-cast across no override must produce exactly the output calling the method \
         directly on the value would"
    );
    assert_eq!(with_cast, "3\n", "unexpected program output");
}

/// #767 review fix (third pass, D-198): the CLI-level counterpart to
/// `cast_up_across_an_overridden_method_is_c0001` in `pycc_types::tests` --
/// proving the method-dispatch rejection actually happens on the public
/// `check` path, before `pycc build` could ever reach `pycc_mir`'s static
/// dispatch and silently call `Base`'s implementation instead of
/// `Derived`'s override.
#[test]
fn an_up_cast_across_an_overridden_method_is_rejected_before_build() {
    let dir = ScratchDir::new("767_override").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "override_cast.py",
        "from typing import cast\n\nclass Base:\n    def __init__(self, a: int) -> None:\n        self.a = a\n    def describe(self) -> int:\n        return self.a\n\nclass Derived(Base):\n    def __init__(self, a: int, b: int) -> None:\n        self.a = a\n        self.b = b\n    def describe(self) -> int:\n        return self.a + self.b\n\ndef f(d: Derived) -> int:\n    return cast(Base, d).describe()\n\nprint(f(Derived(3, 4)))\n",
    );
    let check = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !check.status.success(),
        "pycc check should reject an up-cast across an overridden method"
    );
    let stdout = String::from_utf8_lossy(&check.stdout);
    assert!(
        stdout.contains("C0001")
            && stdout.contains("`describe`")
            && stdout.contains("statically resolve"),
        "expected a C0001 method-dispatch diagnostic naming `describe`, got: {stdout}"
    );
}
