// Issue #925 (Part 2 of #918): container *return* type annotations
// (`list[T]`, `dict[K, V]`, `set[T]`, `tuple[A, B]`) end to end through the
// real `pycc build` / `pycc check` CLI -- parser -> hir -> types -> mir ->
// codegen -> link -> run.
//
// Part 1 (D-228) deliberately rejected return position with a `C0001` while a
// container-typed call result still reached an unhandled codegen case; #925
// added the `crates/pycc_codegen/src/call_result.rs` arms that closed that
// gap and removed the gate. `tests/issue_918_container_annotations.rs` still
// owns the parameter/variable/alias positions.
//
// Every expected stdout below was verified against CPython 3.14 on the same
// source.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn build_and_run(label: &str, source: &str) -> std::process::Output {
    let dir = ScratchDir::new(&format!("issue_925_{label}")).expect("failed to create scratch dir");
    let path = dir.join(format!("{label}.py"));
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(source.as_bytes()).unwrap();
    let out = dir.join(label);
    let status = Command::new(pycc_bin())
        .args(["build", path.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success(), "`pycc build` failed for {label}");
    Command::new(&out).output().unwrap()
}

/// Runs `pycc check` on `source` and returns its rendered diagnostic output,
/// asserting the exit status really is the compile-error one.
fn check_error(label: &str, source: &str) -> String {
    let dir =
        ScratchDir::new(&format!("issue_925_err_{label}")).expect("failed to create scratch dir");
    let path = dir.join(format!("{label}.py"));
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(source.as_bytes()).unwrap();
    let output = Command::new(pycc_bin())
        .args(["check", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{label} should be a compile error"
    );
    String::from_utf8(output.stdout).expect("diagnostics are UTF-8")
}

#[test]
fn a_list_int_return_value_crosses_a_function_boundary_and_runs() {
    let output = build_and_run(
        "list_return",
        "\
def build() -> list[int]:
    return [1, 2, 3]

xs: list[int] = build()
print(len(xs))
print(xs[0])
print(len(build()))
",
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"3\n1\n3\n");
}

#[test]
fn a_dict_str_int_return_value_crosses_a_function_boundary_and_runs() {
    let output = build_and_run(
        "dict_return",
        "\
def build() -> dict[str, int]:
    return {\"a\": 7, \"b\": 9}

d: dict[str, int] = build()
print(d[\"a\"])
print(len(d))
",
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"7\n2\n");
}

#[test]
fn a_set_int_return_value_crosses_a_function_boundary_and_runs() {
    let output = build_and_run(
        "set_return",
        "\
def build() -> set[int]:
    return {1, 2, 3, 3}

s: set[int] = build()
print(len(s))
",
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"3\n");
}

#[test]
fn a_tuple_return_value_crosses_a_function_boundary_and_runs() {
    // The by-value struct family (D-115), the one whose call result is
    // `.into_struct_value()` rather than a pointer.
    let output = build_and_run(
        "tuple_return",
        "\
def build() -> tuple[int, bool, float]:
    return (4, True, 2.5)

t: tuple[int, bool, float] = build()
print(t[0])
print(t[1])
print(t[2])
",
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"4\nTrue\n2.5\n");
}

#[test]
fn a_returned_list_is_the_same_object_the_callee_returned() {
    // Pins the ownership contract measured while planning #925: returning a
    // container is a genuine pointer transfer, not a copy, and it adds no new
    // free site -- lists, dicts and sets stay leak-only (D-107, D-124), so
    // #925 wires no refcount work of its own. Mutating through the returned
    // handle is visible through the original binding, which is exactly what
    // "the same object" means and what CPython does.
    let output = build_and_run(
        "list_identity",
        "\
def same(xs: list[int]) -> list[int]:
    return xs

xs: list[int] = [1]
ys: list[int] = same(xs)
ys.append(4)
print(len(xs))
print(len(ys))
print(xs[1])
",
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"2\n2\n4\n");
}

#[test]
fn a_container_returned_from_a_branch_runs() {
    let output = build_and_run(
        "branch_return",
        "\
def pick(n: int) -> list[int]:
    if n > 0:
        return [1, 2]
    return [3]

print(len(pick(1)))
print(len(pick(0)))
",
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"2\n1\n");
}

#[test]
fn a_discarded_container_result_runs() {
    // A bare expression statement: the call result is produced and dropped.
    // Under D-107/D-124's leak-only ownership this frees nothing, which is
    // the point -- the statement must still compile and run cleanly.
    let output = build_and_run(
        "discarded_return",
        "\
def build() -> list[int]:
    return [1, 2]

build()
print(1)
",
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"1\n");
}

#[test]
fn a_method_returning_a_container_runs() {
    let output = build_and_run(
        "method_return",
        "\
class Box:
    def __init__(self) -> None:
        self.n = 2

    def items(self) -> list[int]:
        return [self.n, self.n]

b = Box()
print(len(b.items()))
print(b.items()[0])
",
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"2\n2\n");
}

#[test]
fn a_protocol_method_returning_a_container_runs() {
    // A protocol *method*'s return type is an ordinary return position.
    // D-228 decision 10's `C0001` covers protocol *attributes* only, and
    // #925 removed the separate return-position gate, so this lowers.
    let output = build_and_run(
        "protocol_method_return",
        "\
from typing import Protocol


class HasItems(Protocol):
    def items(self) -> list[int]: ...


class Box:
    def __init__(self) -> None:
        self.n = 5

    def items(self) -> list[int]:
        return [self.n, self.n, self.n]


def size(h: HasItems) -> int:
    return len(h.items())


print(size(Box()))
",
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"3\n");
}

#[test]
fn a_returned_value_that_disagrees_with_the_container_annotation_is_rejected() {
    // Removing the position gate did not weaken the return-type solver
    // (D-146): the annotation is now checked against what the body actually
    // returns, in both directions.
    let scalar = check_error(
        "scalar_body",
        "def f() -> list[int]:\n    return 5\n\n\nprint(len(f()))\n",
    );
    assert!(
        scalar.contains("error[T0022]: return type mismatch: expected `list[int]`, found `int`"),
        "{scalar}"
    );

    let wrong_family = check_error(
        "wrong_family",
        "def f() -> dict[str, int]:\n    return [1]\n\n\nprint(len(f()))\n",
    );
    assert!(
        wrong_family.contains(
            "error[T0022]: return type mismatch: expected `dict[str, int]`, found `list[int]`"
        ),
        "{wrong_family}"
    );
}

#[test]
fn a_returned_literal_that_fails_its_own_element_gate_reports_that_gate() {
    // The returned *literal*'s own D-122 gate fires before any comparison
    // with the annotation, so the user sees the defect they can act on.
    let rendered = check_error(
        "bad_dict_literal",
        "def f() -> list[int]:\n    return {1: 2}\n\n\nprint(len(f()))\n",
    );
    assert!(
        rendered.contains(
            "error[T0036]: dict[int, int] is not compiled yet (D-122) -- only dict[str, int] is"
        ),
        "{rendered}"
    );
}

#[test]
fn a_function_that_can_fall_through_a_container_return_is_rejected() {
    let rendered = check_error(
        "fall_through",
        "def f(n: int) -> list[int]:\n    if n > 0:\n        return [1]\n\n\nprint(len(f(1)))\n",
    );
    assert!(
        rendered.contains("error[T0022]: function `f` can exit without returning `list[int]`"),
        "{rendered}"
    );
}

#[test]
fn an_empty_list_literal_return_has_no_inferable_element_type() {
    // `-> list[int]: return []` is *not* something #925 fixes, and #1021's
    // empty-container pre-pass deliberately does not reach it either: a return
    // is a non-binding position, and `infer_expr_in` has no expected type to
    // read the annotation from. It reports T0003 rather than T0021 now, with
    // no binding name to name, and #927 is no longer the cited gap.
    let rendered = check_error(
        "empty_literal",
        "def f() -> list[int]:\n    return []\n\n\nprint(len(f()))\n",
    );
    assert!(
        rendered.contains("error[T0003]: an empty list literal has no inferable element type here"),
        "{rendered}"
    );
    assert!(!rendered.contains("issue #927"), "{rendered}");
}
