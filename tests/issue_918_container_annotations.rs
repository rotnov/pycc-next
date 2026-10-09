// D-228 (issue #918), Part 1: parameterized container type annotations
// (`list[T]`, `set[T]`, `dict[K, V]`, `tuple[A, B, ...]`) end to end through
// the real `pycc build` CLI -- parser -> hir -> types -> mir -> codegen ->
// link -> run.
//
// Deliberately contains no container *return* type: return position was
// rejected by design in Part 1 and delivered separately by issue #925, whose
// own end-to-end coverage lives in `tests/issue_925_container_returns.rs`.
// It does contain a container-typed protocol *method*
// parameter: D-228 decision 10's gate covers protocol *attributes* only, and
// a protocol method's parameter is an ordinary parameter position. Every
// expected stdout below was verified against CPython 3.14 on the same
// source.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn build_and_run(label: &str, source: &str) -> std::process::Output {
    let dir = ScratchDir::new(&format!("issue_918_{label}")).expect("failed to create scratch dir");
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

#[test]
fn a_list_int_parameter_crosses_a_function_boundary_and_runs() {
    let output = build_and_run(
        "list_param",
        "\
def total(xs: list[int]) -> int:
    s = 0
    for x in xs:
        s = s + x
    return s

xs: list[int] = [1, 2, 3]
print(total(xs))
print(len(xs))
",
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"6\n3\n");
}

#[test]
fn a_dict_str_int_parameter_crosses_a_function_boundary_and_runs() {
    let output = build_and_run(
        "dict_param",
        "\
def lookup(d: dict[str, int], k: str) -> int:
    return d[k]

d: dict[str, int] = {\"a\": 7, \"b\": 9}
print(lookup(d, \"a\"))
print(lookup(d, \"b\"))
print(len(d))
",
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"7\n9\n2\n");
}

#[test]
fn a_set_int_parameter_crosses_a_function_boundary_and_runs() {
    let output = build_and_run(
        "set_param",
        "\
def count(s: set[int]) -> int:
    return len(s)

s: set[int] = {1, 2, 3, 3}
print(count(s))
",
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"3\n");
}

#[test]
fn a_scalar_element_tuple_parameter_crosses_a_function_boundary_and_runs() {
    // Heterogeneous int/bool/float elements -- the full set `T0039` admits.
    let output = build_and_run(
        "tuple_param",
        "\
def first(t: tuple[int, bool, float]) -> int:
    return t[0]

def second(t: tuple[int, bool, float]) -> bool:
    return t[1]

t: tuple[int, bool, float] = (42, True, 1.5)
print(first(t))
print(second(t))
",
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"42\nTrue\n");
}

#[test]
fn a_type_alias_naming_a_container_lowers_in_parameter_position() {
    // The PEP 695 alias table is one of the four positions Part 1 covers.
    let output = build_and_run(
        "alias_param",
        "\
type Ints = list[int]

def total(xs: Ints) -> int:
    s = 0
    for x in xs:
        s = s + x
    return s

xs: Ints = [4, 5]
print(total(xs))
",
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"9\n");
}

#[test]
fn a_local_container_annotation_lowers_inside_a_function_body() {
    let output = build_and_run(
        "local_annotation",
        "\
def run() -> int:
    xs: list[int] = [10, 20]
    d: dict[str, int] = {\"k\": 30}
    return xs[0] + xs[1] + d[\"k\"]

print(run())
",
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"60\n");
}

#[test]
fn a_container_annotation_lowers_in_a_protocol_method_parameter_and_runs() {
    // D-228 decision 10 gates protocol *attributes* only. A protocol
    // *method*'s parameter is an ordinary parameter position, so a
    // container-typed one lowers, builds and runs -- this is the end-to-end
    // half of the asymmetry pinned in `pycc_hir` by
    // `a_container_annotation_lowers_in_a_protocol_method_parameter`.
    // Verified against CPython 3.14 on the same source.
    let output = build_and_run(
        "protocol_method_param",
        "\
from typing import Protocol


class P(Protocol):
    def total(self, xs: list[int]) -> None: ...


class Impl:
    def total(self, xs: list[int]) -> None:
        print(len(xs))


def use(p: P) -> None:
    p.total([1, 2, 3])


use(Impl())
",
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"3\n");
}
