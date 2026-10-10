//! Builtin calls (`len`, `float`, `bool`, `int`, `str`, `math`) and list-typed operands.

use super::*;

#[test]
#[should_panic(expected = "pycc_codegen: internal error: list operator Add")]
fn a_list_typed_binop_over_non_list_operands_is_an_internal_error() {
    // `pycc_types::list_ops` types a list-result `BinOp` only for a list
    // operand, so this hand-built shape is malformed MIR.
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(MirExpr::IntLiteral(1)),
                    right: Box::new(MirExpr::IntLiteral(2)),
                    ty: Ty::List(Box::new(Ty::Int)),
                },
            }],
        }],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("binop_list_result_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("binop_list_result_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

/// Part 1 of #1319: `frozenset[int]` joins the named container arm, so a
/// hand-built `frozenset[int]`-result `BinOp` (which `pycc_types` refuses
/// as `T0021`) names the type instead of reaching the generic catch-all.
#[test]
#[should_panic(expected = "binary operators are not supported on frozenset[int] yet")]
fn a_frozenset_result_binop_is_not_yet_supported() {
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::BinOp {
                    op: BinOpKind::BitOr,
                    left: Box::new(MirExpr::IntLiteral(1)),
                    right: Box::new(MirExpr::IntLiteral(2)),
                    ty: Ty::FrozenSet(Box::new(Ty::Int)),
                },
            }],
        }],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("binop_frozenset_result_panics")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("binop_frozenset_result_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
fn compiles_a_function_with_a_list_int_parameter_and_list_int_return_value() {
    // `def f(x: list[int]) -> list[int]: return x`. Both halves are
    // producible from real source now: the *parameter* half since D-228
    // (issue #918), and the `-> list[int]` half since #925 removed the
    // return-position `C0001` (see
    // `tests/issue_925_container_returns.rs`). The MIR shape below is
    // kept hand-built as the direct codegen-level pin, and Task 5
    // (D-089) requires this MIR shape to compile *cleanly* rather than
    // panic: `ty_to_basic_type`'s `List(_)` arm (parameter type, and
    // transitively the return type via `compile_to_object`'s `fn_type`
    // delegation) and `emit_expr`'s `Name` arm's `List(_)` arm must
    // agree on the same pointer representation, or `module.verify()`
    // inside `compile_to_object` would reject the mismatched IR.
    // Deliberately does not link or run the resulting object: `f` is
    // never called, and no caller could construct the annotated
    // `list[int]` argument it wants, so this only proves the codegen
    // shape is internally consistent, not that the program is
    // meaningful to run.
    //
    // Also the regression test for a review finding on this task's
    // first pass: `return x` is a bare `Name` read of a `list[int]`
    // parameter, which is exactly the shape `incref_if_str_duplicate`
    // dispatches on -- before `str_value_is_a_duplicate_reference` was
    // gated on `ty: Ty::Str` (see that function's own doc comment),
    // this test's own generated object emitted a spurious call to
    // `pycc_rt_str_incref` on the list pointer. Asserted against
    // directly below, using the equivalently-shaped
    // `compiles_a_function_with_a_float_parameter_and_float_return_value`
    // above as the known-clean baseline (a bare `float`-typed `Name`
    // return, which has never referenced any `pycc_rt_str_*` symbol).
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![("x".to_string(), Ty::List(Box::new(Ty::Int)))],
            return_ty: Ty::List(Box::new(Ty::Int)),
            body: vec![MirStmt::Return(Some(MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::List(Box::new(Ty::Int)),
            }))],
        }],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("list_int_param_and_return")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("list_int_param_and_return.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let obj_bytes = std::fs::read(&obj_path).expect("object file should be readable");
    let references_symbol =
        |name: &str| obj_bytes.windows(name.len()).any(|w| w == name.as_bytes());
    assert!(
        !references_symbol("pycc_rt_str_incref"),
        "a list[int]-typed bare-Name read must not be treated as a duplicate str \
             reference and incref'd as if it were one"
    );
    assert!(
        !references_symbol("pycc_rt_str_decref"),
        "a list[int]-typed value must not be decref'd as if it were str"
    );
}

#[test]
fn passing_a_list_value_as_a_function_argument_marshals_it_like_a_pointer() {
    // `def f(x: list[int]) -> list[int]: return x` plus
    // `def g(x: list[int]) -> list[int]: return f(x)` -- the caller adds
    // the one shape the test directly above does not reach:
    // `build_call_to`'s argument-marshalling match, whose `Scalar::List`
    // arm Task 11a (D-107) put in the *pass-through* bucket. That claim
    // ("a list pointer marshals identically to a str pointer -- it's an
    // opaque pointer either way") is exactly what `module.verify()`
    // inside `compile_to_object` checks here: if the marshalled argument
    // disagreed with `ty_to_basic_type`'s parameter type, LLVM would
    // reject the call instruction outright.
    //
    // Same not-linked, not-run caveat as the test above: neither `f`
    // nor `g` is ever called, and their annotated `list[int]`
    // parameters are unreachable from real source, so this proves the
    // codegen shape only. The `pycc_rt_str_*` assertion carries over for
    // the same reason -- a `list[int]` argument is a bare `Name` read,
    // the exact shape `incref_if_str_duplicate` dispatches on, and
    // `build_call_to` calls that helper on every argument.
    //
    // Both functions return `int`, not `list[int]`: this test is about
    // the *argument* side, which is the side that has a `Scalar::List`
    // arm to verify here. The result side gained its own `Ty::List` arm
    // in #925 (`call_result_scalar`), covered by
    // `a_list_typed_call_result_is_returned_as_a_pointer` below.
    let list_int = || Ty::List(Box::new(Ty::Int));
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![("x".to_string(), list_int())],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::IntLiteral(0)))],
            },
            MirItem::Function {
                name: "g".to_string(),
                params: vec![("x".to_string(), list_int())],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::Call {
                    callee: "f".to_string(),
                    args: vec![MirExpr::Name {
                        name: "x".to_string(),
                        ty: list_int(),
                    }],
                    ty: Ty::Int,
                }))],
            },
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("list_int_passed_as_argument")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("list_int_passed_as_argument.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let obj_bytes = std::fs::read(&obj_path).expect("object file should be readable");
    let references_symbol =
        |name: &str| obj_bytes.windows(name.len()).any(|w| w == name.as_bytes());
    assert!(
        !references_symbol("pycc_rt_str_incref"),
        "a list[int]-typed argument must not be incref'd as if it were a duplicate str \
             reference"
    );
}

#[test]
fn assigning_a_list_value_stores_the_raw_pointer() {
    // Covers `emit_assign`'s `Scalar::List` arm, the third member of
    // Task 11a's pass-through bucket (D-107), in isolation: it calls
    // `emit_assign` directly with a hand-built `Scalar::List` and a
    // hand-built `StorageSlot` so the store instruction itself is what
    // `f.verify(true)` judges, with no surrounding list construction to
    // fail first. (`xs = [1, 2, 3]` reaches this same arm through real
    // MIR in the list tests further below; this one pins the store's IR
    // shape rather than the program's output.)
    //
    // `f.verify(true)` is the real assertion: `ty_to_basic_type`
    // allocated this slot as a pointer, so a `store` of anything but a
    // pointer-typed value would be rejected as malformed IR. Also
    // proves no `str`-style refcount traffic is emitted -- D-107 keeps
    // `list[T]` leak-only for v0.2.
    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();
    let fn_type = context.void_type().fn_type(&[], false);
    let f = module.add_function("f", fn_type, None);
    let block = context.append_basic_block(f, "entry");
    builder.position_at_end(block);

    let ptr = builder
        .build_alloca(context.ptr_type(inkwell::AddressSpace::default()), "xs")
        .expect("build_alloca should not fail for a fresh block");
    let locals = HashMap::from([(
        "xs".to_string(),
        StorageSlot {
            ptr,
            ty: Ty::List(Box::new(Ty::Int)),
            initialized: None,
        },
    )]);
    let value = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    let rt = declare_rt_functions(&context, &module);
    emit_assign(&context, &builder, &rt, &locals, "xs", Scalar::List(value));
    builder
        .build_return(None)
        .expect("build_return should not fail for a void function");

    assert!(
        f.verify(true),
        "storing a list[T] pointer into its own pointer-typed slot must be valid IR"
    );
}

#[test]
#[should_panic(expected = "`len` takes exactly 1 argument, got 0")]
fn a_len_call_with_the_wrong_argument_count_is_an_internal_error() {
    // `pycc_types` already rejects a mis-arity `len` call with T0033
    // (its own `len` arm checks `arg_tys.len() != 1` before anything
    // else), so this is hand-built malformed MIR exercising codegen's
    // own defensive backstop -- the same convention as
    // `referencing_a_name_with_no_bound_local_is_an_internal_error`
    // above.
    let mir = list_fixture_module(vec![MirStmt::ExprStmt(MirExpr::Call {
        callee: "len".to_string(),
        args: vec![],
        ty: Ty::Int,
    })]);
    let dir = pycc_scratch::ScratchDir::new("len_wrong_arity_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(&mir, &dir.join("len_wrong_arity_panics.o"), None, false);
}

#[test]
#[should_panic(expected = "`float` takes exactly 1 argument, got 0")]
fn a_float_call_with_the_wrong_argument_count_is_an_internal_error() {
    // `pycc_types` already rejects a mis-arity `float` call with T0021
    // (its own `float` arm checks `arg_tys.len() != 1` before anything
    // else), so this is hand-built malformed MIR exercising codegen's
    // own defensive backstop, mirroring
    // `a_len_call_with_the_wrong_argument_count_is_an_internal_error`
    // immediately above.
    let mir = list_fixture_module(vec![MirStmt::ExprStmt(MirExpr::Call {
        callee: "float".to_string(),
        args: vec![],
        ty: Ty::Float,
    })]);
    let dir = pycc_scratch::ScratchDir::new("float_wrong_arity_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(&mir, &dir.join("float_wrong_arity_panics.o"), None, false);
}

#[test]
#[should_panic(expected = "`bool` takes exactly 1 argument, got 0")]
fn a_bool_call_with_the_wrong_argument_count_is_an_internal_error() {
    // Part 4 of #1026 (PR 4a of #1083). `pycc_types` admits the `bool`
    // builtin only for exactly one `Ty::Object` argument and leaves every
    // other shape to C0001, so this is hand-built malformed MIR exercising
    // codegen's own defensive backstop, mirroring
    // `a_float_call_with_the_wrong_argument_count_is_an_internal_error`
    // immediately above.
    let mir = list_fixture_module(vec![MirStmt::ExprStmt(MirExpr::Call {
        callee: "bool".to_string(),
        args: vec![],
        ty: Ty::Bool,
    })]);
    let dir = pycc_scratch::ScratchDir::new("bool_wrong_arity_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(&mir, &dir.join("bool_wrong_arity_panics.o"), None, false);
}

#[test]
#[should_panic(expected = "`bool` takes a CPython object argument")]
fn a_bool_call_on_a_non_object_argument_is_an_internal_error() {
    // Part 4 of #1026 (PR 4a of #1083). `bool(1)` keeps its C0001 refusal in
    // `pycc_types`, so reaching codegen at all means the MIR is malformed;
    // this pins the second of the `bool` arm's two backstops.
    let mir = list_fixture_module(vec![MirStmt::ExprStmt(MirExpr::Call {
        callee: "bool".to_string(),
        args: vec![MirExpr::IntLiteral(1)],
        ty: Ty::Bool,
    })]);
    let dir = pycc_scratch::ScratchDir::new("bool_non_object_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(&mir, &dir.join("bool_non_object_panics.o"), None, false);
}

#[test]
#[should_panic(expected = "`int` takes exactly 1 argument, got 0")]
fn an_int_call_with_the_wrong_argument_count_is_an_internal_error() {
    // Part 4 of #1026 (PR 4b of #1083). `int` and `str` share one emission
    // arm, so this pins its arity backstop; the `str` spelling of the same
    // call reaches the identical `panic!` and needs no second test, while
    // the *operand* backstop below is a different line and does.
    let mir = list_fixture_module(vec![MirStmt::ExprStmt(MirExpr::Call {
        callee: "int".to_string(),
        args: vec![],
        ty: Ty::Int,
    })]);
    let dir = pycc_scratch::ScratchDir::new("int_wrong_arity_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(&mir, &dir.join("int_wrong_arity_panics.o"), None, false);
}

#[test]
#[should_panic(expected = "`str` takes a CPython object argument")]
fn a_str_call_on_a_non_object_argument_is_an_internal_error() {
    // Part 4 of #1026 (PR 4b of #1083). `str(1)` keeps its C0001 refusal in
    // `pycc_types`, so reaching codegen at all means the MIR is malformed.
    // Spelled with `str` rather than `int` so the shared arm's `{callee}`
    // interpolation is observed on both names across the two tests.
    let mir = list_fixture_module(vec![MirStmt::ExprStmt(MirExpr::Call {
        callee: "str".to_string(),
        args: vec![MirExpr::IntLiteral(1)],
        ty: Ty::Str,
    })]);
    let dir = pycc_scratch::ScratchDir::new("str_non_object_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(&mir, &dir.join("str_non_object_panics.o"), None, false);
}

#[test]
#[should_panic(expected = "`math.sqrt` takes exactly 1 argument, got 0")]
fn a_math_sqrt_call_with_the_wrong_argument_count_is_an_internal_error() {
    // `pycc_types` already rejects a mis-arity `math.sqrt` call with
    // T0021, so this is hand-built malformed MIR exercising codegen's
    // own defensive backstop, mirroring
    // `a_float_call_with_the_wrong_argument_count_is_an_internal_error`
    // immediately above.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "math.sqrt".to_string(),
            args: vec![],
            ty: Ty::Float,
        }))],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("math_sqrt_wrong_arity_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(
        &mir,
        &dir.join("math_sqrt_wrong_arity_panics.o"),
        None,
        false,
    );
}

#[test]
#[should_panic(expected = "`math.sqrt`'s argument was not a float")]
fn a_math_sqrt_call_on_a_non_float_argument_is_an_internal_error() {
    // The other half of `pycc_types`' own T0021 `math.sqrt` check (a
    // non-`float` argument) -- hand-built malformed MIR, since
    // `pycc_types::std_qualified_symbol`'s call-site check already
    // rejects this before codegen runs for any legitimate source.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
            callee: "math.sqrt".to_string(),
            args: vec![MirExpr::IntLiteral(1)],
            ty: Ty::Float,
        }))],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("math_sqrt_non_float_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(&mir, &dir.join("math_sqrt_non_float_panics.o"), None, false);
}

#[test]
#[should_panic(expected = "`len`'s argument did not evaluate to a list")]
fn a_len_call_on_a_non_list_argument_is_an_internal_error() {
    // The other half of `pycc_types`' own T0033 `len` check (a
    // non-`list[T]` argument), and the naturally reachable cover for
    // `expect_list_pointer`'s shared panic: `emit_expr` really does
    // return a non-`List` scalar here, no self-inconsistent MIR needed.
    let mir = list_fixture_module(vec![MirStmt::ExprStmt(MirExpr::Call {
        callee: "len".to_string(),
        args: vec![MirExpr::IntLiteral(1)],
        ty: Ty::Int,
    })]);
    let dir =
        pycc_scratch::ScratchDir::new("len_non_list_panics").expect("failed to create scratch dir");
    let _ = compile_to_object(&mir, &dir.join("len_non_list_panics.o"), None, false);
}

#[test]
#[should_panic(expected = "`n` did not evaluate to a list")]
fn appending_to_a_non_list_local_is_an_internal_error() {
    // `n = 1` then `n.append(2)`: `pycc_types` rejects this with T0033
    // ("value does not support list operations"), so codegen only sees
    // it as hand-built malformed MIR. Covers `emit_list_name_read`'s
    // own use of `expect_list_pointer`, the shared check
    // `MirStmt::ForList`'s list operand and `MirExpr::Subscript`'s base
    // also go through.
    let mir = list_fixture_module(vec![
        MirStmt::Assign {
            target: "n".to_string(),
            value: MirExpr::IntLiteral(1),
        },
        MirStmt::ExprStmt(MirExpr::ListAppend {
            list: pycc_mir::MirContainerReceiver::Name("n".to_string()),
            value: Box::new(MirExpr::IntLiteral(2)),
        }),
    ]);
    let dir = pycc_scratch::ScratchDir::new("append_to_non_list_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(&mir, &dir.join("append_to_non_list_panics.o"), None, false);
}

#[test]
#[should_panic(expected = "`never_bound` has no local slot")]
fn iterating_a_name_with_no_local_slot_is_an_internal_error() {
    // `for v in never_bound:` where nothing ever bound `never_bound` --
    // `pycc_types` rejects an unbound list operand (T0033/T0021, see
    // its `lookup_bound_name` helper), so this is codegen's own
    // defensive backstop for `emit_list_name_read`'s slot lookup, the
    // one branch of it `appending_to_a_non_list_local_is_an_internal_
    // error` above does not reach.
    let mir = list_fixture_module(vec![MirStmt::ForList {
        var_ty: pycc_mir::Ty::Int,
        var: "v".to_string(),
        list: "never_bound".to_string(),
        body: vec![],
    }]);
    let dir = pycc_scratch::ScratchDir::new("for_list_unbound_name_panics")
        .expect("failed to create scratch dir");
    let _ = compile_to_object(
        &mir,
        &dir.join("for_list_unbound_name_panics.o"),
        None,
        false,
    );
}

#[test]
fn math_sqrt_call_codegens_and_runs() {
    // `print(math.sqrt(2.0))` end to end through `compile_to_object` --
    // D-136 Task 4's one lowered stdlib function, calling the real
    // libm `sqrt` symbol. Expected output verified against `python3`
    // on this exact source (`math.sqrt(2.0)` -> `1.4142135623730951`).
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(print_expr(MirExpr::Call {
            callee: "math.sqrt".to_string(),
            args: vec![MirExpr::FloatLiteral(2.0)],
            ty: Ty::Float,
        }))],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("math_sqrt_call").expect("failed to create scratch dir");
    let obj_path = dir.join("math_sqrt_call.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("math_sqrt_call");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1.4142135623730951\n");
}

#[test]
fn math_pi_codegens_and_runs() {
    // `print(math.pi)` end to end -- D-136 Task 4's compile-time float
    // constant, no runtime call at all. Expected output verified
    // against `python3` on this exact source.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(print_expr(MirExpr::Name {
            name: "math.pi".to_string(),
            ty: Ty::Float,
        }))],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("math_pi").expect("failed to create scratch dir");
    let obj_path = dir.join("math_pi.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("math_pi");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3.141592653589793\n");
}

#[test]
fn float_call_codegens_and_runs() {
    // `x = 3\nprint(float(x))\n` end to end through `compile_to_object`
    // -- #181's `float()` hand-recognized builtin, mirroring
    // `dict_literal_construction_codegens_and_runs`'s own shape
    // immediately above. Expected output verified against `python3` on
    // this exact source.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::IntLiteral(3),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "float".to_string(),
                args: vec![MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }],
                ty: Ty::Float,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("float_call").expect("failed to create scratch dir");
    let obj_path = dir.join("float_call.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("float_call");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3.0\n");
}

#[test]
fn a_user_defined_float_function_codegens_and_runs_instead_of_the_builtin() {
    // Post-merge review finding: `def float(x: int) -> int: return x +
    // 1` was a valid, working program on `main` immediately before this
    // builtin landed -- reproduced directly against a pristine
    // checkout, printing `6`. Without the `user_functions.contains_key`
    // guard, this would silently emit the builtin's own float
    // conversion instead of a real call to the user's function.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "float".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(1)),
                    ty: Ty::Int,
                }))],
            },
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "float".to_string(),
                args: vec![MirExpr::IntLiteral(5)],
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("user_defined_float_call")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("user_defined_float_call.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("user_defined_float_call");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"6\n");
}
