use pycc_mir::{BinOpKind, MirExpr, MirItem, MirModule, MirStmt, Ty};
use pycc_scratch::ScratchDir;
use std::io::Write;
use std::path::Path;
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

fn build_and_run(label: &str, source: &str) -> std::process::Output {
    let dir = ScratchDir::new(&format!("slice1_{label}")).expect("failed to create scratch dir");
    let src = write_fixture(&dir, &format!("{label}.py"), source);
    let out = dir.join(label);
    let status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success(), "`pycc build` failed for {label}");
    Command::new(&out).output().unwrap()
}

/// Issue #118 Part 1: builds `source` and asserts that `pycc build` fails
/// (compile-time rejection), checking for a diagnostic code in stderr. The
/// strict AOT frontend now rejects maybe-unbound reads at compile time
/// (T0041) rather than leaving them for runtime traps.
fn build_and_expect_compile_error(label: &str, source: &str, expected_code: &str) {
    let dir = ScratchDir::new(&format!("slice1_{label}")).expect("failed to create scratch dir");
    let src = write_fixture(&dir, &format!("{label}.py"), source);
    let out = dir.join(label);
    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "`pycc build` should have failed for {label} but succeeded"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(expected_code),
        "`pycc build` for {label} should report {expected_code}, got: {stderr}"
    );
}

fn compile_mir(label: &str, mir: &MirModule) -> Result<(), String> {
    let dir =
        ScratchDir::new(&format!("slice1_mir_{label}")).expect("failed to create scratch dir");
    pycc_codegen::compile_to_object(mir, &dir.join(format!("{label}.o")), None, false)
}

#[test]
fn public_codegen_api_covers_float_runtime_ops_and_parameter_reassignment() {
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "increment".to_string(),
                params: vec![("value".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![
                    MirStmt::Assign {
                        target: "value".to_string(),
                        value: MirExpr::BinOp {
                            op: BinOpKind::Add,
                            left: Box::new(MirExpr::Name {
                                name: "value".to_string(),
                                ty: Ty::Int,
                            }),
                            right: Box::new(MirExpr::IntLiteral(1)),
                            ty: Ty::Int,
                        },
                    },
                    MirStmt::Return(Some(MirExpr::Name {
                        name: "value".to_string(),
                        ty: Ty::Int,
                    })),
                ],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::BinOp {
                    op: BinOpKind::FloorDiv,
                    left: Box::new(MirExpr::FloatLiteral(5.5)),
                    right: Box::new(MirExpr::FloatLiteral(2.0)),
                    ty: Ty::Float,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };

    compile_mir("public_success_paths", &mir).expect("public codegen paths should compile");
}

#[test]
#[should_panic(expected = "has no local slot")]
fn public_codegen_api_rejects_a_name_without_storage() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "missing".to_string(),
                ty: Ty::Int,
            }],
            ty: Ty::None,
        }))],
        class_defs: Vec::new(),
    };
    let _ = compile_mir("missing_storage", &mir);
}

#[test]
#[should_panic(expected = "call to undefined function")]
fn public_codegen_api_rejects_an_undefined_nested_call() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Call {
                callee: "missing".to_string(),
                args: vec![],
                ty: Ty::Int,
            }],
            ty: Ty::None,
        }))],
        class_defs: Vec::new(),
    };
    let _ = compile_mir("undefined_nested_call", &mir);
}

#[test]
#[should_panic(expected = "an f-string with zero parts should not be reachable")]
fn public_codegen_api_rejects_an_empty_structural_fstring() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::FString(vec![])],
            ty: Ty::None,
        }))],
        class_defs: Vec::new(),
    };
    let _ = compile_mir("empty_structural_fstring", &mir);
}

#[test]
fn public_codegen_api_returns_an_error_for_an_undefined_void_call() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "missing".to_string(),
            args: vec![],
            ty: Ty::None,
        }))],
        class_defs: Vec::new(),
    };
    let error = compile_mir("undefined_void_call", &mir).expect_err("the call should fail");
    assert!(error.contains("missing"));
}

#[test]
fn public_codegen_api_propagates_an_error_from_a_function_body() {
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "broken".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![MirStmt::ExprStmt(MirExpr::Call {
                callee: "missing".to_string(),
                args: vec![],
                ty: Ty::None,
            })],
        }],
        class_defs: Vec::new(),
    };
    let error = compile_mir("undefined_void_call_in_function", &mir)
        .expect_err("the function body should fail");
    assert!(error.contains("missing"));
}

#[test]
#[should_panic(expected = "declared to return a non-`None` value but fell through")]
fn public_codegen_api_rejects_a_non_none_function_that_falls_through() {
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "broken".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![],
        }],
        class_defs: Vec::new(),
    };
    let _ = compile_mir("non_none_fallthrough", &mir);
}

#[test]
#[should_panic(expected = "a top-level statement terminated `main`'s entry block")]
fn public_codegen_api_rejects_a_top_level_return() {
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Return(Some(
            MirExpr::IntLiteral(0),
        )))],
        class_defs: Vec::new(),
    };
    let _ = compile_mir("top_level_return", &mir);
}

#[test]
fn recursive_fibonacci_matches_the_well_known_sequence() {
    let source = "\
def fib(n: int) -> int:
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)

i = 0
while i < 11:
    print(fib(i))
    i = i + 1
";
    let output = build_and_run("fib_recursive", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"0\n1\n1\n2\n3\n5\n8\n13\n21\n34\n55\n");
}

#[test]
fn iterative_fibonacci_overflows_into_a_bigint_and_prints_only_decimal_digits() {
    // `fib(100)` genuinely exceeds `i64::MAX` (19 decimal digits).
    // 354224848179261915075 is independently verified (`python3`'s own
    // iterative fibonacci, not this compiler's bigint path), so this
    // asserts the exact mathematical value rather than only its shape.
    let source = "\
def fib_iter(n: int) -> int:
    a = 0
    b = 1
    i = 0
    while i < n:
        temp = a + b
        a = b
        b = temp
        i = i + 1
    return a

print(fib_iter(100))
";
    let output = build_and_run("fib_iterative_bigint", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"354224848179261915075\n");
}

#[test]
fn a_power_expression_links_and_runs_through_the_real_cli_needing_libm() {
    // Regression coverage for `src/main.rs`'s Linux-only `add_linux_system_libs`
    // fix: `pycc_rt_float_pow` calls into `libm`'s `pow`, which only resolves
    // at link time through the real `pycc build` CLI path (unlike
    // `pycc_codegen`'s own in-process `compile_to_object` tests, which never
    // invoke the host linker at all).
    let source = "\
print(2 ** 10)
print(2.0 ** 0.5)
";
    let output = build_and_run("power_expression_libm", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"1024\n1.4142135623730951\n");
}

#[test]
fn float_and_str_ge_and_ne_execute_with_the_correct_boolean_value() {
    // `pycc_codegen`'s own `compiles_the_remaining_float_comparison_operators`/
    // `compiles_the_remaining_string_comparison_operators` unit tests only
    // prove the generated IR compiles -- they never link, run, or check a
    // value, so a predicate swap (e.g. `FloatPredicate::OGE` -> `OGT`, or the
    // analogous string-branch bug) would still pass every existing test.
    // This goes through the real `pycc` CLI and asserts actual values
    // (verified against `python3` on this exact source).
    let source = "\
print(2.5 >= 2.5)
print(2.5 >= 2.6)
print(2.5 != 2.5)
print(2.5 != 2.6)
print(\"b\" >= \"b\")
print(\"b\" >= \"c\")
print(\"b\" != \"b\")
print(\"b\" != \"c\")
";
    let output = build_and_run("float_str_ge_ne_values", source);
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"True\nFalse\nFalse\nTrue\nTrue\nFalse\nFalse\nTrue\n"
    );
}

#[test]
fn multiplication_promotes_and_float_floor_division_matches_cpython() {
    let source = "\
print(3000000000 * 3000000000)
print(1.0 // 0.1)
";
    let output = build_and_run("numeric_runtime_edges", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"9000000000000000000\n9.0\n");
}

#[test]
fn true_division_by_zero_fails_explicitly() {
    let output = build_and_run("float_division_by_zero", "print(1 / 0)\n");
    assert!(
        !output.status.success(),
        "true division by zero must fail instead of producing infinity"
    );
}

#[test]
fn a_marker_bearing_false_range_step_fails_as_zero_after_codegen_normalization() {
    let source = "\
def false_int() -> int:
    return False

for value in range(0, 2, false_int()):
    print(value)
";
    let output = build_and_run("range_false_marker_step", source);
    assert!(
        !output.status.success(),
        "a marker-bearing False range step must fail as numeric zero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("range() arg 3 must not be zero"),
        "expected pycc_rt's honest zero-step message, got: {stderr}"
    );
}

// `none_typed_parameters_cross_the_user_function_abi` and #167's
// `a_none_call_result_crossing_the_abi_carries_a_falsy_unit_value` moved to
// `tests/issue_167_none_carrier_abi.rs` (AGENTS.md "Keep source files
// decomposable": this file is past the ~1,000-line threshold and #167's own
// work touched this None-carrier ABI cluster).

#[test]
fn a_floor_division_quotient_outside_the_tagged_range_promotes() {
    let source = "\
minimum = 0 - 4611686018427387903 - 1
print(minimum // (0 - 1))
";
    let output = build_and_run("floor_div_promotes", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"4611686018427387904\n");
}

#[test]
fn a_float_power_requiring_an_exception_fails_explicitly() {
    let source = "print(0.0 ** (0.0 - 1.0))\n";
    let output = build_and_run("float_pow_zero_negative", source);
    assert!(
        !output.status.success(),
        "zero to a negative float power must not silently produce infinity"
    );
}

#[test]
fn non_finite_float_power_domains_preserve_real_python_results() {
    let source = "\
print((0.0 - 1.0) ** 1e309)
print((0.0 - 1e309) ** 0.5)
print(0.0 ** (0.0 - 1e309))
";
    let output = build_and_run("float_pow_non_finite", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"1.0\ninf\ninf\n");
}

#[test]
fn fizzbuzz_exercises_int_arithmetic_modulo_elif_chains_and_mixed_print_types() {
    let source = "\
def fizzbuzz(n: int) -> None:
    i = 1
    while i <= n:
        if i % 15 == 0:
            print(\"FizzBuzz\")
        elif i % 3 == 0:
            print(\"Fizz\")
        elif i % 5 == 0:
            print(\"Buzz\")
        else:
            print(i)
        i = i + 1

fizzbuzz(15)
";
    let output = build_and_run("fizzbuzz", source);
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"1\n2\nFizz\n4\nBuzz\nFizz\n7\n8\nFizz\nBuzz\n11\nFizz\n13\n14\nFizzBuzz\n"
    );
}

#[test]
fn a_for_loop_over_range_binds_its_variable_as_int_and_iterates_the_expected_values() {
    // Post-review follow-up (fix round, same shape as the f-string gap
    // caught in the original post-review pass): `docs/ROADMAP.md`'s
    // "Compiler pipeline" and "Language surface" rows cite `tests/slice0.rs`
    // and this file together as proving `if`/`while`/`for`+`range` end to
    // end in `build`/`run`, but neither file previously contained a single
    // `for ... in range(...)` fixture (`tests/slice0.rs`'s two `for` hits
    // are Rust harness loops, not compiled Python source). `for`/`range` is
    // genuinely implemented end to end (`pycc_hir` -> `pycc_types` ->
    // `pycc_mir` -> `pycc_codegen`, each with its own unit tests), so this
    // closes the citation gap rather than adding a new feature. Exercises
    // both the one-argument (`range(stop)`, implicit `start=0`/`step=1`)
    // and two-argument (`range(start, stop)`) forms.
    let source = "\
total = 0
for i in range(5):
    total = total + i
print(total)
for j in range(1, 4):
    print(j)
";
    let output = build_and_run("for_range_basic", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"10\n1\n2\n3\n");
}

#[test]
fn a_basic_f_string_interpolates_a_str_and_an_int_local_between_literal_parts() {
    // Not in the plan brief's own Step 2-5 list, but this file's own task
    // description ("recursive functions, control flow, arithmetic, strings,
    // f-strings") and this task's `docs/ROADMAP.md` update both name
    // f-strings as part of what `tests/slice1_codegen_depth.rs` proves --
    // without a real f-string fixture that citation would be an overclaim.
    // Exercises `HirExpr::FString`/`MirExpr::FString` end to end (parser →
    // HIR → types → MIR → LLVM), plus `print` accepting a `str`-typed
    // argument built from f-string interpolation (Task 8/Task 10's join
    // point).
    let source = "\
n = 7
label = \"answer\"
print(f\"{label} = {n}\")
";
    let output = build_and_run("fstring_basic", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"answer = 7\n");
}

#[test]
fn a_multi_argument_print_space_separates_mixed_str_int_float_and_bool_values() {
    // Second fix-round finding (same overclaim class as the `for`/`range`
    // gap fixed above): `docs/ROADMAP.md`'s "Compiler pipeline" row cites
    // this file (jointly with `tests/slice0.rs`) as proving "type-aware
    // multi-argument `print`", and the "Language surface" row claims
    // `print` for every v0.1 scalar type (`int`/`float`/`bool`/`str`) --
    // but every `print(...)` call in both files was single-argument, and
    // neither file ever printed a `float` or a `bool`. That left the
    // multi-argument space-separator path (`pycc_rt_print_space`, emitted
    // between `print` arguments) and the float/bool `to_str` paths
    // (`pycc_rt_float_to_str`/`pycc_rt_bool_to_str`) with zero e2e coverage
    // even though the row asserts all of them work. All are genuinely
    // implemented (the print argument pipeline's doc comment: "any number of
    // int/float/bool/str arguments"), so this closes a missing e2e
    // citation, not a missing feature -- one mixed-type fixture covering
    // every v0.1 scalar type at once rather than a separate test per type.
    // Exact expected output verified directly against `pycc build`/run
    // before being added here: Rust's `f64` `Display` (which
    // `pycc_rt_float_to_str` uses) renders `2.5` as `"2.5"`, matching
    // CPython's own `str(2.5)`, and `pycc_rt_bool_to_str` renders `True`
    // capitalized, matching CPython's own `str(True)`.
    let source = "\
print(\"x\", 1, 2.5, True)
";
    let output = build_and_run("print_multi_arg_mixed_types", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"x 1 2.5 True\n");
}

#[test]
fn float_of_an_int_bool_and_float_prints_the_correct_value() {
    // #181: `float(x)` for `x: int | float | bool` in a public,
    // fully-annotated function body. Expected output verified directly
    // against CPython 3.14.7: `str(float(3))` == "3.0", `str(float(True))`
    // == "1.0", `str(float(2.5))` == "2.5" -- identity for an
    // already-`float` argument, not just narrowing.
    let source = "\
print(float(3))
print(float(True))
print(float(2.5))
";
    let output = build_and_run("float_int_bool_float", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"3.0\n1.0\n2.5\n");
}

#[test]
fn float_used_as_a_nested_call_argument_compiles_and_runs() {
    // #181 explicitly requires codegen to emit a real conversion usable as
    // a nested expression (e.g. a call argument), not just a statement --
    // `print(...)`'s own codegen arm rejects a nested `print` result, so
    // this proves `float(...)` doesn't hit that same restriction.
    let source = "\
def f(x: float) -> float:
    return x

print(f(float(3)))
";
    let output = build_and_run("float_nested_call_argument", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"3.0\n");
}

#[test]
fn float_satisfies_the_d086_int_to_float_boundary() {
    // The actual motivating scenario from D-086: `is_assignable` requires
    // an explicit conversion for an `int` value crossing a `float`-typed
    // boundary (parameter/return/assignment); `float(x)` is now that
    // conversion's real, working, in-language remedy.
    let source = "\
def to_float(x: int) -> float:
    return float(x)

print(to_float(3))
";
    let output = build_and_run("float_d086_boundary", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"3.0\n");
}

#[test]
fn a_user_defined_float_function_shadows_the_builtin_end_to_end() {
    // Post-merge review finding: `def float(x: int) -> int: return x + 1`
    // is a valid Python program that compiled and ran correctly (printing
    // `6`) on `main` immediately before this builtin landed -- reproduced
    // directly against a pristine checkout before this fix. The hand-
    // recognized builtin must defer to a user's own definition of the same
    // name through the full CLI pipeline (parser -> HIR -> pycc_types ->
    // pycc_mir -> pycc_codegen), not just at the unit-test level.
    let source = "\
def float(x: int) -> int:
    return x + 1

print(float(5))
";
    let output = build_and_run("float_user_defined_shadow", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"6\n");
}

#[test]
fn a_float_call_on_a_bigint_argument_fails_honestly() {
    // Post-merge review finding: `float(x)` for a bigint-valued `x` (an
    // `int` promoted past the tagged smallint range by arithmetic
    // overflow) reaches `to_float`'s `Scalar::Int` arm, which calls
    // `pycc_rt_int_to_float` -- the exact same pre-existing "no
    // bigint-to-float" limitation ordinary arithmetic promotion already has
    // (confirmed directly: `fib_iter(100) + 1.5` fails identically), not a
    // new gap `float()` introduces. Mirrors `list_append_bigint_aborts`'s
    // own "executing test, not just a documented gap" convention (D-106).
    //
    // Part C of #1038 (#1065) turned that failure from a process abort into
    // a D-173 `OverflowError`, so the class name is asserted below; the
    // limitation itself is unchanged.
    let source = "\
def fib_iter(n: int) -> int:
    a = 0
    b = 1
    i = 0
    while i < n:
        temp = a + b
        a = b
        b = temp
        i = i + 1
    return a

print(float(fib_iter(100)))
";
    let output = build_and_run("float_bigint_aborts", source);
    assert!(
        !output.status.success(),
        "a bigint-valued argument to float() must fail loudly, not silently produce a wrong value"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("OverflowError: converting a bigint-valued `int` is not supported yet"),
        "expected pycc_rt's honest bigint message, got: {stderr}"
    );
}

#[test]
fn backend_representation_boundaries_match_the_checked_v0_1_contract() {
    let source = "\
def read_later_global() -> int:
    return later_global

def read_later_label() -> str:
    return later_label

def branch_local(flag: bool) -> int:
    if flag:
        value = 1
    else:
        value = 2
    return value

def accepts_int(value: int) -> int:
    return value

def both_branches_return(flag: bool) -> int:
    if flag:
        return True
    else:
        return False

def returns_none() -> None:
    return

def local_shadows_global() -> int:
    shadowed = 4
    return shadowed

def reassigns_parameter(value: int) -> int:
    value = value + 1
    return value

later_global = 5
later_label = \"global\"
shadowed = 9
print(read_later_global())
print(read_later_label())
print(branch_local(False))
print(accepts_int(True))
print(both_branches_return(False))
print(local_shadows_global())
print(shadowed)
print(reassigns_parameter(10))
for i in range(False, 3, True):
    print(i)
counter = 10
counter = True
print(counter)
print(f\"{returns_none()}\")
";
    let output = build_and_run("backend_representation_boundaries", source);
    assert!(output.status.success());
    // D-141 closes the former silent identity loss on the argument, return,
    // and reassignment lines: those original bool objects remain observable
    // as `True`/`False`. The three `range` targets remain ordinary integers
    // `0`/`1`/`2`, because range consumes its bool operands numerically rather
    // than forwarding their identity.
    assert_eq!(
        output.stdout,
        b"5\nglobal\n2\nTrue\nFalse\n4\n9\n11\n0\n1\n2\nTrue\nNone\n"
    );
}

#[test]
fn calling_a_function_before_its_integer_global_is_initialized_fails() {
    let source = "\
def read_later_global() -> int:
    return later_global

later_global = read_later_global()
";
    let output = build_and_run("uninitialized_integer_global", source);
    assert!(
        !output.status.success(),
        "an LLVM initializer must never become a fabricated Python int"
    );
}

#[test]
fn calling_a_function_before_its_string_global_is_initialized_fails_safely() {
    let source = "\
def read_later_label() -> str:
    return later_label

captured = read_later_label()
later_label = \"global\"
";
    let output = build_and_run("uninitialized_string_global", source);
    assert!(
        !output.status.success(),
        "an uninitialized string global must trap before a null runtime dereference"
    );
}

#[test]
fn a_maybe_bound_integer_local_fails_before_loading_undefined_storage() {
    let source = "\
def read_value(flag: bool) -> int:
    if flag:
        value = 1
    return value

captured = read_value(False)
";
    // Issue #118 Part 1: the strict AOT frontend now rejects a maybe-unbound
    // read at compile time (T0041) rather than leaving it for a runtime trap.
    build_and_expect_compile_error("maybe_bound_integer_local", source, "T0041");
}

#[test]
fn a_maybe_bound_string_local_fails_before_returning_null() {
    let source = "\
def read_label(flag: bool) -> str:
    if flag:
        label = \"ready\"
    return label

captured = read_label(False)
";
    // Issue #118 Part 1: the strict AOT frontend now rejects a maybe-unbound
    // read at compile time (T0041) rather than leaving it for a runtime trap.
    build_and_expect_compile_error("maybe_bound_string_local", source, "T0041");
}

#[test]
fn an_empty_range_leaves_a_new_target_unbound() {
    let source = "\
for item in range(0):
    print(item)
print(item)
";
    // Issue #118 Part 1: the for-loop variable is Maybe after the loop
    // (the range may be empty), so reading it is T0041, rejected at compile time.
    build_and_expect_compile_error("empty_range_target", source, "T0041");
}

#[test]
fn range_targets_keep_the_last_element_and_ignore_body_reassignment_for_iteration() {
    // Issue #118 Part 1: reading a for-loop variable after the loop is now a
    // compile-time error (T0041) when the variable was not definitely bound
    // before the loop. `empty` was pre-bound, so reading it after the loop
    // is fine. But `i` is a loop-only variable, so reading it after the loop
    // is rejected.
    let source_pre_bound = "\

empty = 7
for empty in range(0):
    print(empty)
print(empty)
";
    let output = build_and_run("range_target_pre_bound", source_pre_bound);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"7\n");

    let source_loop_only = "\

for i in range(3):
    print(i)
print(i)
";
    build_and_expect_compile_error("range_target_loop_only", source_loop_only, "T0041");
}

#[test]
fn mandelbrot_ascii_produces_a_grid_of_the_expected_dimensions_and_palette() {
    // A first-cut, deliberately small (20x40) rendering exercising
    // nested `while` loops, `float` arithmetic (including true
    // division), a cascading `if`/`elif`/`else` shade lookup, `str`
    // concatenation building a line character by character, and a
    // recursion-free numeric function. This test only proves the shape
    // (dimensions + palette characters used); the exact-value CPython
    // differential lives in `tests/conformance.rs`'s
    // `mandelbrot_ascii_matches_cpython_3_14_7_byte_for_byte` (PR-6).
    //
    // Deviation from the plan brief: the brief's fixture wrote the
    // plane offsets as unary-minus literals (`-2.0`, `-1.0`). At the time no
    // unary operator was lowered at all, so the checked lowering path
    // reported a spanned `C0001` capability diagnostic before MIR/codegen.
    // #602 has since made `-2.0` lower (the sign folds into the literal), but
    // the fixture is left as written: `0.0 - 2.0` / `0.0 - 1.0` is exactly
    // semantically equivalent (Python's left-to-right `+`/`-` makes
    // `0.0 - 2.0 + x` == `-2.0 + x`) and already exercised by this same
    // fixture's `x2 - y2` subtraction. See this task's report for the
    // corresponding `docs/ROADMAP.md` known-gaps addition.
    let source = "\
def mandel_escape(cx: float, cy: float, max_iter: int) -> int:
    x = 0.0
    y = 0.0
    i = 0
    while i < max_iter:
        x2 = x * x
        y2 = y * y
        if x2 + y2 > 4.0:
            return i
        y = 2.0 * x * y + cy
        x = x2 - y2 + cx
        i = i + 1
    return max_iter

def shade_char(level: int) -> str:
    if level <= 0:
        return \" \"
    if level == 1:
        return \".\"
    if level == 2:
        return \":\"
    if level == 3:
        return \"-\"
    if level == 4:
        return \"=\"
    if level == 5:
        return \"+\"
    if level == 6:
        return \"*\"
    if level == 7:
        return \"#\"
    if level == 8:
        return \"%\"
    return \"@\"

height = 20
width = 40
max_iter = 20
row = 0
while row < height:
    line = \"\"
    col = 0
    while col < width:
        cx = 0.0 - 2.0 + (col / width) * 3.0
        cy = 0.0 - 1.0 + (row / height) * 2.0
        iters = mandel_escape(cx, cy, max_iter)
        level = (iters * 9) // max_iter
        line = line + shade_char(level)
        col = col + 1
    print(line)
    row = row + 1
";
    let output = build_and_run("mandelbrot_ascii", source);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).expect("output should be valid UTF-8");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 20, "expected exactly `height` printed lines");
    let palette: &[char] = &[' ', '.', ':', '-', '=', '+', '*', '#', '%', '@'];
    for (row_index, line) in lines.iter().enumerate() {
        assert_eq!(
            line.chars().count(),
            40,
            "row {row_index} should be exactly `width` characters wide"
        );
        assert!(
            line.chars().all(|c| palette.contains(&c)),
            "row {row_index} contained a character outside the shading palette: {line:?}"
        );
    }
}

#[test]
fn pep_0526_annotated_assignments_compile_and_run_correctly() {
    // `tests/fixtures/pep_0526_var_annotations_smoke.py` exercises both
    // `HirStmt::AnnAssign` shapes end to end: `x: int = 1` (annotated with a
    // value, lowered to a plain `MirStmt::Assign`) and `y: int` followed by a
    // separate `y = 2` (value-less annotation, lowered to `MirStmt::NoOp`,
    // CPython itself does nothing observable for it either). This is a
    // throwaway smoke fixture for this task's own test only -- the real
    // dual-profile conformance fixture is a separate task.
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pep_0526_var_annotations_smoke.py");
    let source = std::fs::read_to_string(&fixture)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", fixture.display()));
    let output = build_and_run("pep_0526_var_annotations", &source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"3\n");
}

#[test]
fn list_int_literal_append_index_len_and_iteration_all_work() {
    // The v0.2 `list[int]` thin slice (D-105) end to end through the real
    // `pycc build` CLI: literal construction, `.append()`, indexed read,
    // `len()`, and `for`-iteration, all in one program. Inside a private
    // helper (D-038's `_`-prefixed convention), which is one of the two
    // places D-105's first scope cut says a `list[int]` value can live;
    // `a_module_level_list_binding_lives_in_a_global_slot` below covers the
    // other. Expected output verified against `python3` on this exact
    // source.
    let source = "\
def _run() -> None:
    x = [10, 20, 30]
    x.append(40)
    print(len(x))
    print(x[0])
    print(x[3])
    for v in x:
        print(v)

_run()
";
    let output = build_and_run("list_int_thin_slice", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"4\n10\n40\n10\n20\n30\n40\n");
}

#[test]
fn a_module_level_list_binding_lives_in_a_global_slot() {
    // The other half of D-105's first scope cut ("`list[int]` values exist
    // ... inside module scope or a private helper"): the same operations as
    // the test above, written at module scope, where the binding becomes an
    // LLVM global rather than a function-local alloca. Task 5's own report
    // flagged that `declare_module_globals` had no `Ty::List(_)` arm and
    // left re-deriving that interaction to this task -- without the arm
    // added here, this exact program aborts the compiler with
    // "a `list[int]`-typed module binding is not supported yet" instead of
    // building. Also covers reading a `list[int]` global from inside a
    // function (D-041), which a function-local list can't reach.
    // Verified against `python3` on this exact source.
    let source = "\
def _total() -> int:
    sum = 0
    for v in xs:
        sum = sum + v
    return sum

xs = [1, 2, 3]
xs.append(4)
print(len(xs))
print(xs[2])
print(_total())
";
    let output = build_and_run("list_module_global", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"4\n3\n10\n");
}

#[test]
fn none_typed_list_append_result_from_issue_242_matches_cpython() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/regress/issue_242.py");
    let source = std::fs::read_to_string(&fixture)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", fixture.display()));
    let output = build_and_run("issue_242_none_local", &source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"None\n");
}

#[test]
fn issue_145_print_evaluates_all_args_before_output() {
    // #145: `print(1, side_effect())` where `side_effect` itself prints --
    // all arguments are evaluated before any output is emitted, so stdout
    // is `2\n1 3\n` (CPython's order), not the interleaved `1 2\n3\n`.
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/regress/issue_145.py");
    let source = std::fs::read_to_string(&fixture)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", fixture.display()));
    let output = build_and_run("issue_145_print_arg_order", &source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"2\n1 3\n");
}

#[test]
fn issue_239_list_subscript_return_infers_int_and_prints_1() {
    // D-146 (#239): an unannotated private helper that assigns a list literal
    // to a local and returns a subscript of that local now infers `int` and
    // builds/runs correctly. Before this fix the solver's
    // `collect_expr_constraints` produced no term for the list literal or the
    // subscript, so the local was never bound and the return failed with the
    // misleading "not bound before this use" (T0021).
    let source = "\
def _first():
    xs = [1]
    return xs[0]


print(_first())
";
    let output = build_and_run("issue_239_list_subscript", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"1\n");
}

#[test]
fn issue_239_list_subscript_return_with_annotation_infers_int_and_prints_1() {
    // D-146 (#239): annotated companion for parity -- the same program with
    // an explicit `-> int` return annotation builds and runs identically.
    let source = "\
def _first() -> int:
    xs = [1]
    return xs[0]


print(_first())
";
    let output = build_and_run("issue_239_list_subscript_annotated", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"1\n");
}

#[test]
fn issue_239_list_pop_return_infers_int_and_prints_2() {
    // D-146 (#239): an unannotated private helper that assigns a list literal
    // to a local, pops from it, and returns the result now infers `int` and
    // builds/runs correctly. The `ListLiteral` arm produces the
    // `Ty::List(Box::new(Ty::Int))` carrier, the `ListPop` arm destructures
    // it to extract `Ty::Int`. Before this fix the `ListPop` arm returned
    // `Ok(None)`, so the local was never bound and the return failed with
    // T0021.
    let source = "\
def _pop():
    xs = [1, 2]
    y = xs.pop()
    return y


print(_pop())
";
    let output = build_and_run("issue_239_list_pop", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"2\n");
}

#[test]
fn none_typed_module_global_list_append_result_is_storable() {
    let source = "\
xs = [1]
result = xs.append(2)
print(result)
print(len(xs))
";
    let output = build_and_run("none_typed_module_global", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"None\n2\n");
}

#[test]
fn none_typed_set_add_result_is_storable() {
    let source = "\
def _run() -> None:
    values = {1}
    result = values.add(2)
    print(result)
    print(len(values))

_run()
";
    let output = build_and_run("none_typed_set_add_result", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"None\n2\n");
}

#[test]
fn none_global_read_before_assignment_traps() {
    let source = "\
def _show() -> None:
    print(result)

_show()
values = [1]
result = values.append(2)
";
    let output = build_and_run("none_global_read_before_assignment", source);
    assert!(
        !output.status.success(),
        "a zero-initialized None carrier must not look assigned before its statement runs"
    );
}

#[test]
fn reading_a_list_global_before_it_is_assigned_traps_instead_of_dereferencing_null() {
    // A `list[int]` module global's storage starts as a null pointer, the
    // same as a `str` global's (see
    // `calling_a_function_before_its_string_global_is_initialized_fails_safely`
    // above). The `initialized` flag every module global carries is what
    // must stop that null from ever reaching `pycc_rt_int_list_len`.
    let source = "\
def _size() -> int:
    return len(later)

captured = _size()
later = [1]
";
    let output = build_and_run("uninitialized_list_global", source);
    assert!(
        !output.status.success(),
        "an uninitialized list global must trap before a null runtime dereference"
    );
}

#[test]
fn iterating_a_list_rereads_its_length_each_step_like_cpython() {
    // CPython's list iterator compares its cursor against the list's
    // *current* length on every `__next__`, so appending inside the loop
    // body extends the iteration. `MirStmt::ForList`'s codegen therefore
    // calls `pycc_rt_int_list_len` inside its loop-test block rather than
    // hoisting it into the preheader -- with the length hoisted, this
    // program would print only "1". Output verified against `python3` on
    // this exact source.
    let source = "\
def _run() -> None:
    xs = [1]
    for v in xs:
        if len(xs) < 3:
            xs.append(v + 1)
        print(v)

_run()
";
    let output = build_and_run("list_iteration_rereads_len", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"1\n2\n3\n");
}

#[test]
fn rebinding_the_iterated_name_inside_the_body_does_not_retarget_the_loop() {
    // The other half of `MirStmt::ForList`'s iteration contract, alongside
    // `iterating_a_list_rereads_its_length_each_step_like_cpython` above:
    // Python binds its iterator to the object the `for` statement
    // evaluated, so rebinding the *name* mid-loop leaves the iteration on
    // the original list. That is why the arm reads the list pointer once,
    // in the loop preheader, rather than per iteration -- with the read
    // moved into the loop-test block this would print "1" and then loop on
    // `[9]` forever. Module scope on purpose: it makes the rebinding write
    // through to the same global slot the loop read from, which is the
    // case that would actually break. Output verified against `python3` on
    // this exact source.
    let source = "\
xs = [1, 2]
for v in xs:
    xs = [9]
    print(v)
";
    let output = build_and_run("list_iteration_name_rebound", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"1\n2\n");
}

#[test]
fn list_targets_keep_the_last_element_and_ignore_body_reassignment() {
    // Issue #118 Part 1: reading a for-loop variable after the loop is now a
    // compile-time error (T0041) when the variable was not definitely bound
    // before the loop. `i` is a loop-only variable, so reading it after the
    // loop is rejected.
    let source = "\

xs = [1, 2, 3]
for i in xs:
    print(i)
print(i)
";
    build_and_expect_compile_error("list_target_lifetime", source, "T0041");
}

#[test]
fn converting_a_list_to_str_stops_the_build_with_an_honest_unsupported_message() {
    // v0.2 has no `str(list)`, and `pycc_types` type-checks both of these
    // unconditionally (its `print` arm accepts any argument type, and an
    // f-string interpolation imposes none), so each passes `pycc check` and
    // then stops in codegen (D-107). Asserts the honest message rather than
    // only the failure, since the whole point of D-107's `Scalar::List`
    // split was to replace silently handing a `PyIntListObj` pointer to a
    // `pycc_rt_*_to_str` function that would read it as a `PyStrObj`.
    //
    // Both source forms, not just `print(xs)`: `to_str` has exactly two
    // call sites in `pycc_codegen` -- `emit_eval_print_arg` and `emit_expr`'s
    // f-string interpolation arm -- and `docs/ARCHITECTURE.md` names both
    // as reachable, so both need evidence rather than one standing in for
    // the other. (An earlier version of this test covered only `print(xs)`
    // while the doc claimed a count of two operations, missing f-strings
    // entirely.)
    for (label, expression) in [
        ("print_list", "print(xs)"),
        ("fstring_list", "print(f\"{xs}\")"),
    ] {
        let source = format!(
            "\
def _run() -> None:
    xs = [1, 2]
    {expression}

_run()
"
        );
        let dir =
            ScratchDir::new(&format!("slice1_{label}")).expect("failed to create scratch dir");
        let src = write_fixture(&dir, &format!("{label}.py"), &source);
        let output = Command::new(pycc_bin())
            .args([
                "build",
                src.to_str().unwrap(),
                "-o",
                dir.join(label).to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(!output.status.success(), "`{expression}` must not build");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("string conversion of a list[T] value is not supported yet"),
            "expected the honest unsupported-conversion message for `{expression}`, got: {stderr}"
        );
    }
}

#[test]
fn appending_a_bigint_valued_element_fails_explicitly_instead_of_corrupting_the_slot() {
    // D-141 supersedes D-106's raw payload but deliberately retains the
    // bigint-container scope cut. `pycc_rt_int_add`/`_mul` promote past
    // D-061's smallint range on overflow, and the shared encoded-boundary
    // validator must reject that value honestly rather than store it as if
    // the current containers supported bigint ownership. Real CPython prints
    // the exact product here; this remains a documented v0.2 scope cut.
    //
    // The overflowing value is built by multiplication rather than written
    // as a literal. That was originally because `tag_smallint_const`
    // rejected an out-of-tagged-range integer *literal* at compile time;
    // since #148/D-178 a literal does reach `.append()` and hits this same
    // boundary (pinned by `tests/issue_148_oversized_int_literal.rs`), so
    // this case now specifically covers the *promotion* ingress.
    // `3000000000 * 3000000000` is the same promotion
    // `multiplication_promotes_and_float_floor_division_matches_cpython`
    // above already pins as reaching a real bigint.
    let source = "\
def _run() -> None:
    xs = [1]
    xs.append(3000000000 * 3000000000)
    print(xs[1])

_run()
";
    let output = build_and_run("list_append_bigint_aborts", source);
    assert!(
        !output.status.success(),
        "a bigint-valued element must fail loudly, not be stored as a supported container value"
    );
    // D-141 retains specifically an *honest panic*, not merely a failure --
    // asserting the message is what distinguishes it from a segfault or any
    // other abort that a missing guard could also produce. `pycc_rt`'s panic
    // handler writes this to stderr before the `extern "C"` boundary turns
    // the unwind into a process abort.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("int boundary does not support bigint-valued values"),
        "expected pycc_rt's honest bigint message, got: {stderr}"
    );
}

#[test]
fn an_out_of_range_index_fails_explicitly() {
    // `pycc_rt_int_list_get`'s own bounds panic, reached through real
    // source: the index is only known at runtime, so there is nothing
    // `pycc_types` could have rejected. Also pins D-108's documented v0.2
    // scope cut that a *negative* index is out of range too rather than
    // CPython's last-element behavior.
    let source = "\
def _run() -> None:
    xs = [1, 2, 3]
    print(xs[0 - 1])

_run()
";
    let output = build_and_run("list_index_out_of_range", source);
    assert!(
        !output.status.success(),
        "a negative index is out of range in v0.2, not CPython's last element"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("list index out of range"),
        "expected pycc_rt's honest bounds message, got: {stderr}"
    );
}

#[test]
fn a_bool_initializer_under_an_int_annotation_widens_before_a_later_reassignment() {
    // Regression test for a cross-task divergence found and fixed while
    // implementing this task: `pycc_types` (Task 4) binds its checker
    // `env` to the *annotation's* type for `x: int = True` (`Ty::Int`), not
    // the initializer's own type (`Ty::Bool`). `pycc_mir`'s lowering must
    // agree with that, per D-074's "first assignment fixes a binding's
    // representation" rule, or the later plain reassignment `x = 5` would
    // silently store into a slot still permanently sized for `bool`.
    // Before this fix this exact program printed `11` (the raw tagged-int
    // bit pattern read back out of a truncated slot), not `5`.
    let source = "\
def f() -> int:
    x: int = True
    x = 5
    return x

print(f())
";
    let output = build_and_run("pep_0526_bool_initializer_widens_to_int", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"5\n");
}

#[test]
fn a_module_level_dict_str_int_literal_supports_len_and_indexed_read() {
    // The `dict[str, int]` thin slice (D-123) end to end through the real
    // `pycc build` CLI: literal construction, `len()`, and indexed read
    // `d[k]`, all in one program, at module scope -- the same scope
    // `a_module_level_list_binding_lives_in_a_global_slot` above covers for
    // `list[int]` (`declare_module_globals`'s own `Ty::Dict(_)` arm, PR-11
    // Task 5). Expected output verified against `python3` on this exact
    // source.
    let source = "\
x = {\"a\": 1, \"b\": 2}
print(len(x))
print(x[\"b\"])
";
    let output = build_and_run("dict_literal_len_and_get", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"2\n2\n");
}

#[test]
fn dict_set_item_updates_an_existing_key_and_appends_a_new_one() {
    // `d[k] = v`'s own insert-or-update contract (D-123), both halves in
    // one program: `x["a"] = 5` updates the existing `"a"` entry in place
    // (`len(x)` stays `1`), then `x["b"] = 2` appends a genuinely new key
    // (`len(x)` grows to `2`). Expected output verified against `python3`
    // on this exact source.
    let source = "\
x = {\"a\": 1}
x[\"a\"] = 5
print(x[\"a\"])
print(len(x))
x[\"b\"] = 2
print(len(x))
";
    let output = build_and_run("dict_set_item_update_and_append", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"5\n1\n2\n");
}

#[test]
fn for_k_in_a_module_level_dict_iterates_its_keys_in_insertion_order() {
    // `for k in d:` (D-123) iterates a dict's keys in insertion order, not
    // sorted order -- `"b"` is inserted first even though `"a"` sorts
    // first, so printing `"b"` before `"a"` is the actual property this
    // test pins (`PyDictObj`'s own D-121 insertion-order guarantee
    // surviving through the real CLI, not just `pycc_codegen`'s own
    // hand-built-MIR unit test). Expected output verified against
    // `python3` on this exact source.
    let source = "\
x = {\"b\": 2, \"a\": 1}
for k in x:
    print(k)
";
    let output = build_and_run("dict_module_global_iteration_order", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"b\na\n");
}

#[test]
fn a_variable_used_as_a_dict_key_survives_a_later_reassignment_of_that_variable() {
    // Regression test for a confirmed use-after-free a pinned-reviewer pass
    // on PR-11 Task 5 caught: `PyDictObj` adopts whatever key pointer it is
    // given as its own permanent reference without incref'ing it itself
    // (D-124), so `d[k] = 1` where `k` is a plain `str` variable used to
    // silently hand the dict a *duplicate*, non-owned reference to the
    // exact same `PyStrObj` `k`'s own slot holds -- a later `k = "xyz"`
    // then decref'd (and, at refcount 1, freed) that object while `d` still
    // pointed to it. The run of single-use literal assignments between the
    // reassignment and the read encourages the allocator to reuse the freed
    // slot before `d` is read back, turning silent corruption into an
    // observable failure if `pycc_codegen`'s `incref_if_str_duplicate` fix
    // (in `MirExpr::DictLiteral`'s and `MirStmt::DictSet`'s own key
    // handling) ever regresses -- confirmed empirically to crash with
    // "pycc_rt: dict key not found" without it. Expected output verified
    // against `python3` on this exact source.
    let source = "\
k = \"abc\"
d = {\"z\": 0}
d[k] = 1
k = \"xyz\"
j0 = \"111\"
j1 = \"222\"
j2 = \"333\"
j3 = \"444\"
j4 = \"555\"
j5 = \"666\"
j6 = \"777\"
j7 = \"888\"
j8 = \"999\"
j9 = \"000\"
print(d[\"abc\"])
print(len(d))
";
    let output = build_and_run("dict_variable_key_survives_reassignment", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"1\n2\n");
}

#[test]
fn growing_a_dict_from_inside_for_k_in_d_iterates_the_newly_added_key_too() {
    // A final whole-branch review flagged a genuinely new, deliberate v0.2
    // CPython divergence introduced by `ForDict`'s own codegen (D-123): the
    // loop bound is `pycc_rt_dict_len`, re-read on every iteration rather
    // than hoisted, so `d[k] = v` inside a `for k in d:` loop body that
    // grows the dict causes the loop to keep going and also visit the
    // newly-added key. Real CPython raises `RuntimeError: dictionary
    // changed size during iteration` for this exact program instead. This
    // test pins the actual, verified behavior (empirically confirmed by
    // running this exact source through the real `pycc build`/execute
    // pipeline before writing the assertion below) so the D-123 Consequences
    // note in
    // `docs/decisions/D-123-dict-str-int-ships-a-d-k-v-insert-or-update.md`
    // and `docs/RUNTIME.md`'s dict line describe enforced behavior, not an
    // inference from a codegen comment. NOT a conformance fixture against
    // CPython (this is the one documented point where pycc and CPython
    // deliberately disagree).
    let source = "\
d = {\"a\": 1}
for k in d:
    print(k)
    d[\"z\"] = 9
print(len(d))
";
    let output = build_and_run("dict_grows_during_for_k_in_d_iteration", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"a\nz\n2\n");
}

#[test]
fn a_module_level_set_int_literal_dedupes_and_supports_len() {
    // The `set[int]` thin slice (D-123) end to end through the real `pycc
    // build` CLI: literal construction (with `PyIntSetObj`'s own dedup on
    // repeated elements, D-121) and `len()`, mirroring the dict coverage
    // above (`a_module_level_dict_str_int_literal_supports_len_and_indexed_
    // read`) -- until this test, `set[int]` had zero non-`#[ignore]`d
    // end-to-end CLI coverage; the only prior coverage exercising this
    // exact behavior was `pycc_codegen`'s own hand-built-MIR unit test
    // `set_literal_and_len` one layer down. `len(x) == 3`, not `4`, is the
    // actual point: the repeated `2` must be deduped, not counted twice.
    // This assertion is order-independent (`len` doesn't observe iteration
    // order), so it needs no CPython oracle disclaimer. Expected output
    // verified against `python3` on this exact source.
    let source = "\
x = {1, 2, 2, 3}
print(len(x))
";
    let output = build_and_run("set_literal_len_dedup", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"3\n");
}

#[test]
fn for_v_in_a_module_level_set_iterates_in_first_insertion_order() {
    // `for v in x:` over a module-level `set[int]` (D-123) end to end
    // through the real `pycc build` CLI, with a duplicate element in the
    // literal -- mirrors the dict iteration-order coverage above
    // (`for_k_in_a_module_level_dict_iterates_its_keys_in_insertion_
    // order`) and pins the same property one layer down that
    // `pycc_codegen`'s own hand-built-MIR unit test
    // (`for_x_in_set_iterates_in_first_insertion_order`) already covers:
    // `PyIntSetObj`'s first-insertion iteration order (D-121) surviving
    // through the full CLI pipeline, not just direct MIR-to-object
    // codegen. `2` printing before `1` (with the second `2` deduped away
    // rather than moving `2`'s position) is pycc's own documented,
    // internally-consistent behavior -- NOT a claim about CPython: this
    // is pinning pycc against itself, not against a CPython oracle.
    // `python3` on this exact source prints `1`/`2` instead (CPython's own
    // set iteration order for small ints is unspecified by the language
    // and happens to come out numeric here, not insertion order), so this
    // is deliberately not a `tests/conformance.rs` fixture.
    let source = "\
x = {2, 1, 2}
for v in x:
    print(v)
";
    let output = build_and_run("set_for_iteration_order", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"2\n1\n");
}

// -- PR-12 Task 9 (D-118): `list[int]` slicing end to end through the real
// `pycc build` CLI. `print(ys)` is deliberately never used below -- `to_str`
// has no `list[int]` arm yet (see `converting_a_list_to_str_stops_the_build_
// with_an_honest_unsupported_message` above) -- every result is walked with
// `for v in ys: print(v)` instead, the same convention D-120's own PEP-709
// fixture uses for the identical reason.

#[test]
fn a_basic_slice_with_explicit_bounds_returns_the_expected_sub_range() {
    // `xs[1:3]` on `[10, 20, 30, 40, 50]` is `[20, 30]`. Expected output
    // verified against `python3` on this exact source.
    let source = "\
xs = [10, 20, 30, 40, 50]
ys = xs[1:3]
print(len(ys))
for v in ys:
    print(v)
";
    let output = build_and_run("slice_basic_explicit_bounds", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"2\n20\n30\n");
}

#[test]
fn slicing_with_every_omitted_bound_form_defaults_correctly() {
    // D-118's own defaulting rule, all three omitted-bound shapes in one
    // program: `xs[:3]` (omitted start), `xs[2:]` (omitted stop), and
    // `xs[:]` (both omitted). Expected output verified against `python3` on
    // this exact source.
    let source = "\
xs = [10, 20, 30, 40, 50]
a = xs[:3]
b = xs[2:]
c = xs[:]
for v in a:
    print(v)
for v in b:
    print(v)
for v in c:
    print(v)
";
    let output = build_and_run("slice_every_omitted_bound_form", source);
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"10\n20\n30\n30\n40\n50\n10\n20\n30\n40\n50\n"
    );
}

#[test]
fn slicing_with_a_step_greater_than_one_skips_elements() {
    // `xs[0:6:2]` (explicit bounds) and `xs[::2]` (both bounds omitted,
    // step only) on `[0, 1, 2, 3, 4, 5]` both give `[0, 2, 4]`. Expected
    // output verified against `python3` on this exact source.
    let source = "\
xs = [0, 1, 2, 3, 4, 5]
ys = xs[0:6:2]
zs = xs[::2]
for v in ys:
    print(v)
for v in zs:
    print(v)
";
    let output = build_and_run("slice_step_greater_than_one", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"0\n2\n4\n0\n2\n4\n");
}

#[test]
fn a_sliced_list_stays_independent_of_further_mutation_on_either_side() {
    // D-107's leak-only policy still requires the slice result to be a
    // *new* allocation: appending to the original list after slicing must
    // not retroactively change the slice's own contents, and appending to
    // the slice must not change the original either. Expected output
    // verified against `python3` on this exact source.
    let source = "\
xs = [1, 2, 3]
ys = xs[0:3]
xs.append(99)
ys.append(77)
for v in xs:
    print(v)
for v in ys:
    print(v)
";
    let output = build_and_run("slice_independent_of_mutation", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"1\n2\n3\n99\n1\n2\n3\n77\n");
}

#[test]
fn an_omitted_stop_bound_reflects_a_side_effect_from_evaluating_start() {
    // Evaluation-order regression test: this task's own originating plan
    // sketch computed `base`'s length (the value an omitted `stop`
    // defaults to) immediately after evaluating `base`, before evaluating
    // `start`/`step`. That ordering is observably wrong here -- `start` is
    // `_grow()`, a helper that appends `99` to `xs` (its own module global)
    // before returning `0` -- and real CPython's own evaluation order
    // (build the whole slice from every sub-expression, *then* apply it,
    // with the length lookup happening only at that final step) means the
    // omitted `stop` must reflect `xs`'s length *after* `_grow()` already
    // ran, i.e. `3`, not the `2` it held when `base` was first evaluated.
    // `pycc_codegen`'s own `MirExpr::Slice` arm defers its `stop`-defaulting
    // length read until after every present bound has already been
    // evaluated for exactly this reason (see that arm's own doc comment).
    // Verified empirically against `python3` on this exact source before
    // this test was written: both give `3\n1\n2\n99\n`.
    let source = "\
xs = [1, 2]
def _grow() -> int:
    xs.append(99)
    return 0

ys = xs[_grow():]
print(len(ys))
for v in ys:
    print(v)
";
    let output = build_and_run("slice_omitted_stop_reflects_start_side_effect", source);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"3\n1\n2\n99\n");
}

#[test]
fn a_runtime_negative_slice_start_raises_instead_of_cpython_last_element_addressing() {
    // D-118's own runtime scope cut, extended from D-108's existing index
    // precedent to slicing: a negative `start` is rejected rather than
    // addressing from the end the way real CPython's `xs[-1:3]` would. Part
    // B of #1038 (#1064) made that rejection a catchable `ValueError`
    // instead of a process abort; the conformance gap itself is unchanged
    // and tracked as issue #1070.
    // `neg = 0 - 1` (`BinOp::Sub`) stands in for a negative literal here.
    // When this test was written no unary operator lowered at all; #602 has
    // since made the literal form `-1` lower too, but `0 - 1` remains an
    // equally valid way to produce a negative runtime `int`.
    let source = "\
xs = [1, 2, 3]
neg = 0 - 1
ys = xs[neg:3]
print(len(ys))
";
    let output = build_and_run("slice_negative_start_raises", source);
    assert!(
        !output.status.success(),
        "a negative slice start must raise rather than silently address from the end"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ValueError: slice start must be non-negative"),
        "expected a catchable ValueError, got: {stderr}"
    );
}
