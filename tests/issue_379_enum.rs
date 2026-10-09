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

fn build_and_run(dir: &std::path::Path, src: &std::path::Path, bin_name: &str) -> (bool, Vec<u8>) {
    let out = dir.join(bin_name);
    let build_status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    if !build_status.success() {
        return (false, Vec::new());
    }
    let output = Command::new(&out).output().unwrap();
    (output.status.success(), output.stdout)
}

fn check_fails(dir: &std::path::Path, src: &std::path::Path) -> bool {
    let out = dir.join("should_not_compile");
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    !status.success()
}

/// #379: `Color.RED.value` returns the integer value assigned to the member.
#[test]
fn enum_member_value_lookup() {
    let dir = ScratchDir::new("379_value").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "value.py",
        "class Color(Enum):\n    RED = 1\n    GREEN = 2\n    BLUE = 3\nprint(Color.RED.value)\nprint(Color.GREEN.value)\nprint(Color.BLUE.value)\n",
    );
    let (ok, stdout) = build_and_run(&dir, &src, "value");
    assert!(
        ok,
        "pycc build and run should succeed for enum value lookup"
    );
    assert_eq!(stdout, b"1\n2\n3\n", "enum member values should be 1, 2, 3");
}

/// #379: `Color.RED.name` returns the member's name as a string.
#[test]
fn enum_member_name_lookup() {
    let dir = ScratchDir::new("379_name").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "name.py",
        "class Color(Enum):\n    RED = 1\n    GREEN = 2\n    BLUE = 3\nprint(Color.RED.name)\nprint(Color.GREEN.name)\nprint(Color.BLUE.name)\n",
    );
    let (ok, stdout) = build_and_run(&dir, &src, "name");
    assert!(ok, "pycc build and run should succeed for enum name lookup");
    assert_eq!(
        stdout, b"RED\nGREEN\nBLUE\n",
        "enum member names should be RED, GREEN, BLUE"
    );
}

/// #379: `for c in Color:` iterates members in declaration order.
#[test]
fn enum_iteration_declaration_order() {
    let dir = ScratchDir::new("379_iter").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "iter.py",
        "class Color(Enum):\n    RED = 1\n    GREEN = 2\n    BLUE = 3\nfor c in Color:\n    print(c.value)\n    print(c.name)\n",
    );
    let (ok, stdout) = build_and_run(&dir, &src, "iter");
    assert!(ok, "pycc build and run should succeed for enum iteration");
    assert_eq!(
        stdout, b"1\nRED\n2\nGREEN\n3\nBLUE\n",
        "enum iteration should produce members in declaration order"
    );
}

/// #379: a non-integer member value is rejected with C0001.
#[test]
fn non_integer_member_value_is_rejected() {
    let dir = ScratchDir::new("379_nonint").expect("failed to create scratch dir");
    let src = write_fixture(&dir, "nonint.py", "class Color(Enum):\n    RED = 1.5\n");
    assert!(
        check_fails(&dir, &src),
        "a non-integer enum member value should be a compile error"
    );
}

/// #379: a duplicate member name is rejected with C0001.
#[test]
fn duplicate_member_is_rejected() {
    let dir = ScratchDir::new("379_dup").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "dup.py",
        "class Color(Enum):\n    RED = 1\n    RED = 2\n",
    );
    assert!(
        check_fails(&dir, &src),
        "a duplicate enum member name should be a compile error"
    );
}

/// #379: `class C(Enum, Other):` — multiple bases with Enum is rejected.
#[test]
fn multiple_bases_with_enum_is_rejected() {
    let dir = ScratchDir::new("379_multibase").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "multibase.py",
        "class Other:\n    def __init__(self) -> None:\n        return\nclass C(Enum, Other):\n    RED = 1\n",
    );
    assert!(
        check_fails(&dir, &src),
        "class C(Enum, Other) with multiple bases should be a compile error"
    );
}

/// #379: a method definition in an enum body is rejected.
#[test]
fn method_in_enum_body_is_rejected() {
    let dir = ScratchDir::new("379_method").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "method.py",
        "class Color(Enum):\n    RED = 1\n    def f(self) -> int:\n        return 1\n",
    );
    assert!(
        check_fails(&dir, &src),
        "a method definition in an enum body should be a compile error"
    );
}

/// #379: enum iteration inside a function body works (nested unrolling).
#[test]
fn enum_iteration_inside_function() {
    let dir = ScratchDir::new("379_fniter").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "fniter.py",
        "class Color(Enum):\n    RED = 1\n    GREEN = 2\ndef print_values() -> None:\n    for c in Color:\n        print(c.value)\nprint_values()\n",
    );
    let (ok, stdout) = build_and_run(&dir, &src, "fniter");
    assert!(
        ok,
        "pycc build and run should succeed for enum iteration inside a function"
    );
    assert_eq!(
        stdout, b"1\n2\n",
        "enum iteration inside a function should produce member values in order"
    );
}

/// #379: accessing a non-member attribute on an enum class is rejected
/// with T0044 (the existing class-attribute rejection).
#[test]
fn non_member_attribute_on_enum_class_is_rejected() {
    let dir = ScratchDir::new("379_nonmember").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "nonmember.py",
        "class Color(Enum):\n    RED = 1\nprint(Color.NONEXISTENT.value)\n",
    );
    assert!(
        check_fails(&dir, &src),
        "accessing a non-member on an enum class should be a compile error"
    );
}

/// #379: a generic enum class (`class C[T](Enum):`) is rejected — enums
/// cannot have type parameters.
#[test]
fn generic_enum_class_is_rejected() {
    let dir = ScratchDir::new("379_generic").expect("failed to create scratch dir");
    let src = write_fixture(&dir, "generic.py", "class Color[T](Enum):\n    RED = 1\n");
    assert!(
        check_fails(&dir, &src),
        "a generic enum class should be a compile error"
    );
}

/// #379: an enum member assignment with multiple targets (chain assignment)
/// is rejected.
#[test]
fn multiple_targets_in_enum_member_is_rejected() {
    let dir = ScratchDir::new("379_multitarget").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "multitarget.py",
        "class Color(Enum):\n    RED = GREEN = 1\n",
    );
    assert!(
        check_fails(&dir, &src),
        "an enum member assignment with multiple targets should be a compile error"
    );
}

/// #379: an enum member assignment with a non-name target (attribute
/// access) is rejected.
#[test]
fn non_name_target_in_enum_member_is_rejected() {
    let dir = ScratchDir::new("379_nonname").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "nonname.py",
        "class Color(Enum):\n    Color.RED = 1\n",
    );
    assert!(
        check_fails(&dir, &src),
        "an enum member assignment with a non-name target should be a compile error"
    );
}

/// #379: an enum member value that overflows i64 is rejected.
#[test]
fn overflow_enum_member_value_is_rejected() {
    let dir = ScratchDir::new("379_overflow").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "overflow.py",
        "class Color(Enum):\n    RED = 99999999999999999999999999\n",
    );
    assert!(
        check_fails(&dir, &src),
        "an enum member value that overflows i64 should be a compile error"
    );
}

/// #379: an enum member value that is a non-literal expression is rejected.
#[test]
fn non_literal_enum_member_value_is_rejected() {
    let dir = ScratchDir::new("379_nonlit").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "nonlit.py",
        "x = 1\nclass Color(Enum):\n    RED = x\n",
    );
    assert!(
        check_fails(&dir, &src),
        "an enum member value that is a non-literal expression should be a compile error"
    );
}

/// #379: `from enum import Enum` works — the import resolves and `Enum`
/// is usable as a base class marker.
#[test]
fn from_enum_import_enum_works() {
    let dir = ScratchDir::new("379_fromimport").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "fromimport.py",
        "from enum import Enum\nclass Color(Enum):\n    RED = 1\n    GREEN = 2\nprint(Color.RED.value)\nprint(Color.GREEN.value)\n",
    );
    let (ok, stdout) = build_and_run(&dir, &src, "fromimport");
    assert!(
        ok,
        "pycc build and run should succeed with `from enum import Enum`"
    );
    assert_eq!(stdout, b"1\n2\n", "enum values should be 1 and 2");
}

/// #379: `import enum` then referencing `enum.Enum` as a value is
/// rejected — `Enum` is a class marker, not a first-class value.
#[test]
fn import_enum_dotted_enum_used_as_value_is_rejected() {
    let dir = ScratchDir::new("379_dottedval").expect("failed to create scratch dir");
    let src = write_fixture(&dir, "dottedval.py", "import enum\nprint(enum.Enum)\n");
    assert!(
        check_fails(&dir, &src),
        "referencing `enum.Enum` as a value should be a compile error"
    );
}

/// #379: `import enum` then calling `enum.Enum()` is rejected — `Enum`
/// is a class marker, not a callable function.
#[test]
fn import_enum_dotted_enum_called_is_rejected() {
    let dir = ScratchDir::new("379_dottedcall").expect("failed to create scratch dir");
    let src = write_fixture(&dir, "dottedcall.py", "import enum\nenum.Enum()\n");
    assert!(
        check_fails(&dir, &src),
        "calling `enum.Enum()` should be a compile error"
    );
}

/// #379: a non-enum `for` loop over a list (not an enum class) that
/// contains a nested enum loop in its body — exercises the recursive
/// `unroll_enum_loops_in_stmts` fallback for `ForList`.
#[test]
fn non_enum_for_list_with_nested_enum_loop_unrolls() {
    let dir = ScratchDir::new("379_nestedlist").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "nestedlist.py",
        "class Color(Enum):\n    RED = 1\n    GREEN = 2\nxs = [1, 2]\nfor x in xs:\n    for c in Color:\n        print(c.value)\n",
    );
    let (ok, stdout) = build_and_run(&dir, &src, "nestedlist");
    assert!(
        ok,
        "pycc build and run should succeed for a non-enum for-list with a nested enum loop"
    );
    assert_eq!(
        stdout, b"1\n2\n1\n2\n",
        "nested enum loop should unroll inside a non-enum for-list body"
    );
}

/// #379: an enum loop nested inside an `if` statement — exercises the
/// recursive `unroll_enum_loops_in_stmts` `If` arm.
#[test]
fn enum_loop_nested_inside_if_unrolls() {
    let dir = ScratchDir::new("379_ifnest").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "ifnest.py",
        "class Color(Enum):\n    RED = 1\n    GREEN = 2\nx = 1\nif x:\n    for c in Color:\n        print(c.value)\n",
    );
    let (ok, stdout) = build_and_run(&dir, &src, "ifnest");
    assert!(
        ok,
        "pycc build and run should succeed for an enum loop nested inside an if"
    );
    assert_eq!(
        stdout, b"1\n2\n",
        "nested enum loop should unroll inside an if body"
    );
}

/// #379: an enum loop nested inside a `while` statement — exercises the
/// recursive `unroll_enum_loops_in_stmts` `While` arm.
#[test]
fn enum_loop_nested_inside_while_unrolls() {
    let dir = ScratchDir::new("379_whilenest").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "whilenest.py",
        "class Color(Enum):\n    RED = 1\n    GREEN = 2\ni = 0\nwhile i < 1:\n    for c in Color:\n        print(c.value)\n    i = i + 1\n",
    );
    let (ok, stdout) = build_and_run(&dir, &src, "whilenest");
    assert!(
        ok,
        "pycc build and run should succeed for an enum loop nested inside a while"
    );
    assert_eq!(
        stdout, b"1\n2\n",
        "nested enum loop should unroll inside a while body"
    );
}

/// #379: an enum loop nested inside a `for i in range(...)` statement —
/// exercises the recursive `unroll_enum_loops_in_stmts` `ForRange` arm.
#[test]
fn enum_loop_nested_inside_for_range_unrolls() {
    let dir = ScratchDir::new("379_fornest").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "fornest.py",
        "class Color(Enum):\n    RED = 1\n    GREEN = 2\nfor i in range(1):\n    for c in Color:\n        print(c.value)\n",
    );
    let (ok, stdout) = build_and_run(&dir, &src, "fornest");
    assert!(
        ok,
        "pycc build and run should succeed for an enum loop nested inside a for-range"
    );
    assert_eq!(
        stdout, b"1\n2\n",
        "nested enum loop should unroll inside a for-range body"
    );
}
