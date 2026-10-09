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

/// Issue #22: a call before the first `def` of that name must fail at
/// compile time (`pycc check`), matching CPython's `NameError`.
#[test]
fn call_before_def_is_a_compile_error() {
    let dir = ScratchDir::new("issue22_call_before_def").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "call_before_def.py",
        "foo()\n\ndef foo() -> None:\n    print(42)\n",
    );

    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "pycc check should reject call-before-def with exit code 1"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cannot call function `foo` before its definition"),
        "stdout should mention call-before-def, got: {stdout}"
    );
}

/// Issue #22: a call before the first `def` must also fail at build time.
#[test]
fn call_before_def_is_a_build_error() {
    let dir = ScratchDir::new("issue22_build_before_def").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "call_before_def.py",
        "foo()\n\ndef foo() -> None:\n    print(42)\n",
    );
    let out = dir.join("hello");

    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "pycc build should reject call-before-def with exit code 1"
    );
}

/// Issue #22: a redefinition affects only calls executed after that
/// definition. `foo()` before the second `def` prints `1`; `foo()` after
/// it prints `2` -- matching CPython exactly.
#[test]
fn redefinition_affects_only_subsequent_calls() {
    let dir = ScratchDir::new("issue22_redef").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "redef.py",
        "def foo() -> None:\n    print(1)\n\nfoo()\n\ndef foo() -> None:\n    print(2)\n\nfoo()\n",
    );
    let out = dir.join("redef");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for redefinition"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"1\n2\n",
        "redefinition should print 1 then 2, matching CPython"
    );
}

/// Issue #22: recursion after binding works -- a function can call itself
/// in its own body, since the function body sees all module functions.
#[test]
fn recursion_after_binding_works() {
    let dir = ScratchDir::new("issue22_recursion").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "recursion.py",
        "def count(n: int) -> int:\n    if n == 0:\n        return 0\n    return count(n - 1) + 1\n\nprint(count(5))\n",
    );
    let out = dir.join("recursion");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success(), "pycc build should succeed for recursion");

    let output = Command::new(&out).output().unwrap();
    assert_eq!(output.stdout, b"5\n", "recursion should print 5");
}

/// Issue #22: a function body may call a sibling defined later in the
/// module -- Python's late binding evaluates the body at call time, by
/// which point all module-level `def`s have executed.
#[test]
fn function_body_calls_sibling_defined_later() {
    let dir = ScratchDir::new("issue22_sibling_later").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "sibling_later.py",
        "def caller() -> int:\n    return callee()\n\ndef callee() -> int:\n    return 42\n\nprint(caller())\n",
    );
    let out = dir.join("sibling_later");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for a function calling a later sibling"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(output.stdout, b"42\n", "sibling call should print 42");
}

/// Issue #22: a function body may call a sibling defined earlier in the
/// module -- the common case, and it should still work.
#[test]
fn function_body_calls_sibling_defined_earlier() {
    let dir = ScratchDir::new("issue22_sibling_earlier").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "sibling_earlier.py",
        "def callee() -> int:\n    return 42\n\ndef caller() -> int:\n    return callee()\n\nprint(caller())\n",
    );
    let out = dir.join("sibling_earlier");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for a function calling an earlier sibling"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(output.stdout, b"42\n", "sibling call should print 42");
}

/// Issue #22: the redefinition regression fixture in
/// `tests/regress/issue_22_execution_order.py` compiles and produces
/// CPython-matching output (`1\n2\n`).
#[test]
fn redefinition_fixture_matches_cpython() {
    let dir = ScratchDir::new("issue22_fixture").expect("failed to create scratch dir");
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/regress/issue_22_execution_order.py");
    let out = dir.join("execution_order");

    let status = Command::new(pycc_bin())
        .args([
            "build",
            fixture.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for the redefinition fixture"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"1\n2\n",
        "redefinition fixture should print 1 then 2"
    );
}

/// Issue #22 review fix: a redefinition with a different signature must be
/// rejected by `pycc check`. The codegen uses the first definition's LLVM
/// function type for indirect calls through the per-name function-pointer
/// slot, so all definitions of the same name must share one signature.
/// Without this check, the mismatched `fn_type` produces silent runtime UB
/// (a `ptr::copy_nonoverlapping` precondition violation on arm64).
#[test]
fn incompatible_redefinition_is_a_check_error() {
    let dir = ScratchDir::new("issue22_incompat_check").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "incompat.py",
        "def foo(x: int) -> int:\n    return x\n\ndef foo(x: int, y: int) -> int:\n    return x + y\n\nprint(foo(1, 2))\n",
    );

    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "pycc check should reject incompatible redefinition with exit code 1"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cannot redefine function `foo` with a different signature"),
        "stdout should mention incompatible redefinition, got: {stdout}"
    );
}

/// Issue #22 review fix: a redefinition with a different signature must
/// also be rejected by `pycc build`. Both signatures here are fully
/// concrete, so this fixture is rejected by the pre-resolution check
/// (`check_and_resolve` is the first-diagnostic view of
/// `check_and_resolve_all`, whose `checked_function_signatures_all` calls
/// `check_incompatible_redefinitions` before any solver resolution runs).
/// Issue #402 fixed the same pre-resolution check to also reject a
/// same-arity redefinition where one signature still carries `Ty::Infer`
/// (see `incompatible_redefinition_with_unannotated_first_definition_is_a_build_error`
/// below for that case specifically).
#[test]
fn incompatible_redefinition_is_a_build_error() {
    let dir = ScratchDir::new("issue22_incompat_build").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "incompat.py",
        "def foo(x: int) -> int:\n    return x\n\ndef foo(x: int, y: int) -> int:\n    return x + y\n\nprint(foo(1, 2))\n",
    );
    let out = dir.join("incompat");

    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "pycc build should reject incompatible redefinition with exit code 1"
    );
}

/// Issue #402: a same-arity redefinition where the *first* definition is
/// unannotated (`Ty::Infer` for its parameter and return type -- a leading
/// underscore marks it a private helper, per D-038, so the frontend
/// doesn't itself require an annotation) and the second is concrete but
/// structurally different must be rejected by `pycc check`, just like the
/// fully-concrete case above. Before the #402 fix, this specific shape
/// silently collapsed onto one shared resolved signature and was accepted.
#[test]
fn incompatible_redefinition_with_unannotated_first_definition_is_a_check_error() {
    let dir = ScratchDir::new("issue402_incompat_check").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "incompat_infer.py",
        "def _foo(x):\n    return x\n\ndef _foo(x: int) -> None:\n    print(x)\n\n_foo(1)\n",
    );

    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "pycc check should reject an unannotated-first-definition incompatible \
         redefinition with exit code 1"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cannot redefine function `_foo` with a different signature"),
        "stdout should mention incompatible redefinition, got: {stdout}"
    );
}

/// Issue #402: the same unannotated-first-definition redefinition must
/// also be rejected by `pycc build`.
#[test]
fn incompatible_redefinition_with_unannotated_first_definition_is_a_build_error() {
    let dir = ScratchDir::new("issue402_incompat_build").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "incompat_infer.py",
        "def _foo(x):\n    return x\n\ndef _foo(x: int) -> None:\n    print(x)\n\n_foo(1)\n",
    );
    let out = dir.join("incompat_infer");

    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "pycc build should reject an unannotated-first-definition incompatible \
         redefinition with exit code 1"
    );
}

/// Issue #22 review fix: a redefinition with a different return type is
/// also rejected (not just parameter count/type mismatches).
#[test]
fn incompatible_redefinition_with_different_return_type_is_rejected() {
    let dir = ScratchDir::new("issue22_incompat_ret").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "incompat_ret.py",
        "def foo(x: int) -> int:\n    return x\n\ndef foo(x: int) -> None:\n    print(x)\n\nfoo(1)\n",
    );

    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "pycc check should reject redefinition with a different return type"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cannot redefine function `foo` with a different signature"),
        "stdout should mention incompatible redefinition, got: {stdout}"
    );
}

/// Issue #22 review fix: a compatible redefinition (same signature) still
/// works after the incompatible-redefinition check was added. This is the
/// regression fixture's own shape -- same signature, different body.
#[test]
fn compatible_redefinition_with_same_signature_still_works() {
    let dir = ScratchDir::new("issue22_compat_redef").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "compat_redef.py",
        "def foo() -> None:\n    print(1)\n\nfoo()\n\ndef foo() -> None:\n    print(2)\n\nfoo()\n",
    );
    let out = dir.join("compat_redef");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for a compatible redefinition (same signature)"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"1\n2\n",
        "compatible redefinition should print 1 then 2"
    );
}

/// Issue #22 review fix: multiple calls to the same function work correctly.
/// This exercises the single `fnname_` global created once per function name
/// and reused at every call site (the fix for the duplicate-named global
/// that was previously created on each call).
#[test]
fn multiple_calls_to_same_function_work() {
    let dir = ScratchDir::new("issue22_multi_call").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "multi_call.py",
        "def foo(x: int) -> int:\n    return x + 1\n\ndef bar(x: int) -> int:\n    return foo(x) + foo(x) + foo(x)\n\nprint(bar(1))\nprint(foo(2))\nprint(foo(3))\n",
    );
    let out = dir.join("multi_call");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for multiple calls to the same function"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"6\n3\n4\n",
        "multiple calls should produce correct output"
    );
}

#[test]
fn generic_function_calls_dispatch_directly() {
    // Monomorphized generic specializations (`0gen_...` names) dispatch
    // directly through `direct_value`, not through the indirect
    // function-pointer slot. This test covers that code path and verifies
    // a generic function still produces correct output after the
    // execution-order changes.
    let dir = ScratchDir::new("issue22_generic").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "generic.py",
        "def identity[T](x: T) -> T:\n    return x\n\nprint(identity(1))\nprint(identity(\"hi\"))\nprint(identity(2))\n",
    );
    let out = dir.join("generic");

    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pycc build should succeed for a generic function"
    );

    let output = Command::new(&out).output().unwrap();
    assert_eq!(
        output.stdout, b"1\nhi\n2\n",
        "generic function should produce correct output"
    );
}
