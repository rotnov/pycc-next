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

fn build_and_run(dir: &std::path::Path, src_name: &str, source: &str) -> (bool, Vec<u8>, String) {
    let src = write_fixture(dir, src_name, source);
    let out = dir.join(src_name.replace(".py", ""));
    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    if !output.status.success() {
        return (
            false,
            Vec::new(),
            String::from_utf8_lossy(&output.stderr).to_string(),
        );
    }
    let run = Command::new(&out).output().unwrap();
    (
        run.status.success(),
        run.stdout.clone(),
        String::from_utf8_lossy(&run.stderr).to_string(),
    )
}

fn check_only(dir: &std::path::Path, src_name: &str, source: &str) -> (bool, String) {
    let src = write_fixture(dir, src_name, source);
    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

// -- Literal patterns --

#[test]
fn match_literal_int_builds_and_runs() {
    let dir = ScratchDir::new("381_lit").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "lit.py",
        "x = 2\nmatch x:\n    case 1:\n        print(1)\n    case 2:\n        print(2)\n    case _:\n        print(0)\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"2\n");
}

#[test]
fn match_literal_string_builds_and_runs() {
    let dir = ScratchDir::new("381_str").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "str.py",
        "x = \"hi\"\nmatch x:\n    case \"hi\":\n        print(1)\n    case \"bye\":\n        print(2)\n    case _:\n        print(0)\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"1\n");
}

// -- Capture patterns --

#[test]
fn match_capture_builds_and_runs() {
    let dir = ScratchDir::new("381_cap").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "cap.py",
        "x = 42\nmatch x:\n    case y:\n        print(y)\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"42\n");
}

// -- Wildcard patterns --

#[test]
fn match_wildcard_builds_and_runs() {
    let dir = ScratchDir::new("381_wild").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "wild.py",
        "x = 5\nmatch x:\n    case 1:\n        print(1)\n    case _:\n        print(0)\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"0\n");
}

// -- Singleton patterns: True, False, None --

#[test]
fn match_singleton_true_builds_and_runs() {
    let dir = ScratchDir::new("381_true").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "true.py",
        "x = True\nmatch x:\n    case True:\n        print(1)\n    case False:\n        print(2)\n    case _:\n        print(0)\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"1\n");
}

#[test]
fn match_singleton_false_builds_and_runs() {
    let dir = ScratchDir::new("381_false").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "false.py",
        "x = False\nmatch x:\n    case True:\n        print(1)\n    case False:\n        print(2)\n    case _:\n        print(0)\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"2\n");
}

#[test]
fn match_singleton_none_builds_and_runs() {
    let dir = ScratchDir::new("381_none").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "none.py",
        "def f() -> None:\n    pass\nmatch f():\n    case None:\n        print(1)\n    case _:\n        print(0)\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"1\n");
}

// -- Exhaustive bool matching --

#[test]
fn match_bool_exhaustive_builds_and_runs() {
    let dir = ScratchDir::new("381_bool").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "bool.py",
        "x = True\nmatch x:\n    case True:\n        print(1)\n    case False:\n        print(0)\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"1\n");
}

// -- Guards --

#[test]
fn match_guard_builds_and_runs() {
    let dir = ScratchDir::new("381_guard").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "guard.py",
        "x = 5\nmatch x:\n    case y if y > 3:\n        print(y)\n    case _:\n        print(0)\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"5\n");
}

#[test]
fn match_guard_false_falls_through() {
    let dir = ScratchDir::new("381_guard2").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "guard2.py",
        "x = 1\nmatch x:\n    case y if y > 3:\n        print(y)\n    case _:\n        print(0)\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"0\n");
}

// -- Or-patterns --

#[test]
fn match_or_pattern_builds_and_runs() {
    let dir = ScratchDir::new("381_or").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "or.py",
        "x = 2\nmatch x:\n    case 1 | 2 | 3:\n        print(1)\n    case _:\n        print(0)\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"1\n");
}

// -- Non-exhaustive diagnostics --

#[test]
fn match_non_exhaustive_is_a_check_error() {
    let dir = ScratchDir::new("381_nonex").expect("failed to create scratch dir");
    let (ok, err) = check_only(
        &dir,
        "nonex.py",
        "def f(x: int) -> None:\n    match x:\n        case 0:\n            print(0)\n        case 1:\n            print(1)\n",
    );
    assert!(!ok, "non-exhaustive match should fail check");
    assert!(
        err.contains("T0030"),
        "should report T0030 for non-exhaustive match, got: {err}"
    );
}

// -- Pass-only case bodies --

#[test]
fn match_pass_only_body_builds_and_runs() {
    let dir = ScratchDir::new("381_pass").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "pass.py",
        "x = 1\nmatch x:\n    case 1:\n        pass\n    case _:\n        print(0)\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"");
}

// -- Capture availability in case body --

#[test]
fn match_capture_available_in_body() {
    let dir = ScratchDir::new("381_capbody").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "capbody.py",
        "x = 7\nmatch x:\n    case y:\n        print(y)\n        print(y + 1)\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"7\n8\n");
}

// -- Definite-assignment joining --

#[test]
fn match_definite_assignment_join() {
    let dir = ScratchDir::new("381_def").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "def.py",
        "x = 3\nresult = 0\nmatch x:\n    case 1:\n        result = 10\n    case 2:\n        result = 20\n    case _:\n        result = 30\nprint(result)\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"30\n");
}

// -- Type mismatch diagnostics --

#[test]
fn match_guard_not_bool_is_a_check_error() {
    let dir = ScratchDir::new("381_guardty").expect("failed to create scratch dir");
    let (ok, err) = check_only(
        &dir,
        "guardty.py",
        "def f(x: int) -> None:\n    match x:\n        case y if y:\n            print(y)\n        case _:\n            print(0)\n",
    );
    assert!(!ok, "non-bool guard should fail check");
    assert!(
        err.contains("T0021"),
        "should report T0021 for non-bool guard, got: {err}"
    );
}

// -- Nested match --

#[test]
fn match_nested_builds_and_runs() {
    let dir = ScratchDir::new("381_nested").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "nested.py",
        "x = 1\ny = 2\nmatch x:\n    case 1:\n        match y:\n            case 2:\n                print(12)\n            case _:\n                print(10)\n    case _:\n        print(0)\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"12\n");
}

// -- Match inside a function --

#[test]
fn match_inside_function_builds_and_runs() {
    let dir = ScratchDir::new("381_fn").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "fn.py",
        "def classify(n: int) -> str:\n    match n:\n        case 0:\n            return \"zero\"\n        case _:\n            return \"other\"\nprint(classify(0))\nprint(classify(5))\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"zero\nother\n");
}

// -- Match with return in each arm --

#[test]
fn match_with_return_in_each_arm_builds_and_runs() {
    let dir = ScratchDir::new("381_ret").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "ret.py",
        "def f(x: int) -> int:\n    match x:\n        case 1:\n            return 10\n        case 2:\n            return 20\n        case _:\n            return 0\nprint(f(1))\nprint(f(2))\nprint(f(3))\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"10\n20\n0\n");
}

// -- Match with Maybe binding join (if-without-else in a case body) --

#[test]
fn match_maybe_binding_join_builds_and_runs() {
    // A case body that assigns `y` only inside an `if` without `else`
    // leaves `y` as `Maybe` in that case's env.  The join then hits
    // the `BindingState::Maybe` arm, exercising the `ty()` method on
    // a `Maybe` binding through the full pycc binary.  `y` is never
    // read after the match, so the Maybe binding is not a T0021 error.
    let dir = ScratchDir::new("381_maybe").expect("failed to create scratch dir");
    let (ok, out, err) = build_and_run(
        &dir,
        "maybe.py",
        "def f(x: int) -> None:\n    match x:\n        case 0:\n            if x > 0:\n                y = 1\n        case _:\n            pass\nprint(0)\n",
    );
    assert!(ok, "build/run failed: {err}");
    assert_eq!(out, b"0\n");
}
