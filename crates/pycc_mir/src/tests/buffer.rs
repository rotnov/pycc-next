//! Buffer element loads (`MirExpr::BufferGet`, Part 2 of #1027) and buffer
//! length reads (`MirExpr::BufferLen`, #1116).
//!
//! `b[i]` on a `memoryview`-typed name is the one expression #1027 admits
//! over a buffer. Lowering routes it away from `MirExpr::Subscript` -- whose
//! own `ty()` panics on a base that is neither list nor tuple -- into a node
//! of its own whose type is unconditionally `Ty::Float`, matching the
//! `f64`-element view the `--ext` wrapper fills.

use crate::*;
use pycc_hir::{HirExpr, HirItem, HirModule, HirStmt, Ty};

/// A module holding one function whose first parameter `b` is a
/// `memoryview`, which is the only way such a binding can exist: #1027
/// admits `Ty::MemoryView` solely as a parameter of a `pycc build --ext`
/// export, and adds no expression that produces one.
fn module_with_buffer_fn(return_ty: Ty, body: Vec<HirStmt>) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "total".to_string(),
            params: vec![
                ("b".to_string(), Ty::MemoryView),
                ("i".to_string(), Ty::Int),
            ],
            return_ty,
            body,
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    }
}

/// The lowered statements of the single function `module_with_buffer_fn`
/// builds.
fn function_body(module: &MirModule) -> &[MirStmt] {
    let MirItem::Function { body, .. } = &module.items[0] else {
        panic!("expected the module's only item to be a function");
    };
    body
}

#[test]
fn a_subscript_of_a_memoryview_lowers_to_a_buffer_get_typed_float() {
    let hir = module_with_buffer_fn(
        Ty::Float,
        vec![HirStmt::Return(Some(HirExpr::Subscript {
            base: Box::new(HirExpr::Name("b".to_string())),
            index: Box::new(HirExpr::Name("i".to_string())),
        }))],
    );
    let mir = build(&hir);
    let [MirStmt::Return(Some(expr))] = function_body(&mir) else {
        panic!("expected a single `return`");
    };
    // The node carries no `ty` field: every buffer #1027 admits is an
    // `f64` view, so `ty()` answering `Ty::Float` unconditionally is the
    // contract rather than an inference result.
    assert_eq!(expr.ty(), Ty::Float);
    let MirExpr::BufferGet { base, index } = expr else {
        panic!("expected a `BufferGet`, got {expr:?}");
    };
    assert!(
        matches!(**base, MirExpr::Name { ref name, ty: Ty::MemoryView } if name == "b"),
        "{base:?}"
    );
    assert!(
        matches!(**index, MirExpr::Name { ref name, ty: Ty::Int } if name == "i"),
        "{index:?}"
    );
}

#[test]
fn a_walrus_in_a_buffer_index_binds_for_the_next_statement() {
    // PEP 572 (#774), the same requirement `ObjSubscript` carries:
    // `MirExpr::collect_named_expr_bindings` has to recurse into *both*
    // sides of the new node. `stmt.rs`'s `ExprStmt` arm binds whatever
    // that walk finds, so an empty arm would lower the following
    // `n` against an unbound name and panic.
    let hir = module_with_buffer_fn(
        Ty::None,
        vec![
            HirStmt::ExprStmt(HirExpr::Subscript {
                base: Box::new(HirExpr::Name("b".to_string())),
                index: Box::new(HirExpr::NamedExpr {
                    name: "n".to_string(),
                    value: Box::new(HirExpr::IntLiteral(0)),
                }),
            }),
            HirStmt::ExprStmt(HirExpr::Name("n".to_string())),
        ],
    );
    let mir = build(&hir);
    let [MirStmt::ExprStmt(first), MirStmt::ExprStmt(second)] = function_body(&mir) else {
        panic!("expected two expression statements");
    };
    assert!(matches!(first, MirExpr::BufferGet { .. }), "{first:?}");
    assert!(
        matches!(second, MirExpr::Name { name, ty: Ty::Int } if name == "n"),
        "{second:?}"
    );
}

/// `len(b)` routes away from the scalar `len` lowering -- which stays a
/// `MirExpr::Call` whose codegen reaches `expect_list_pointer` -- into a node
/// of its own whose `ty()` is unconditionally `Ty::Int`.
#[test]
fn a_len_of_a_memoryview_lowers_to_a_buffer_len_typed_int() {
    let hir = module_with_buffer_fn(
        Ty::Int,
        vec![HirStmt::Return(Some(HirExpr::Call {
            callee: "len".to_string(),
            args: vec![HirExpr::Name("b".to_string())],
        }))],
    );
    let mir = build(&hir);
    let [MirStmt::Return(Some(expr))] = function_body(&mir) else {
        panic!("expected a single `return`");
    };
    // No `ty` field here either: a buffer's element count is an `int`
    // unconditionally, exactly as `ObjLen`'s is.
    assert_eq!(expr.ty(), Ty::Int);
    let MirExpr::BufferLen { base } = expr else {
        panic!("expected a `BufferLen`, got {expr:?}");
    };
    assert!(
        matches!(base.as_ref(), MirExpr::Name { name, ty: Ty::MemoryView } if name == "b"),
        "{base:?}"
    );
}

/// `len` on any other operand is untouched by the new interception: a
/// `list` argument still lowers to the ordinary `MirExpr::Call`, whose
/// codegen re-tags `pycc_rt_int_list_len`'s raw count.
#[test]
fn a_len_of_a_list_still_lowers_to_the_scalar_call() {
    let hir = module_with_buffer_fn(
        Ty::Int,
        vec![
            HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            },
            HirStmt::Return(Some(HirExpr::Call {
                callee: "len".to_string(),
                args: vec![HirExpr::Name("xs".to_string())],
            })),
        ],
    );
    let mir = build(&hir);
    let [_, MirStmt::Return(Some(expr))] = function_body(&mir) else {
        panic!("expected an assignment and a `return`");
    };
    assert!(
        matches!(expr, MirExpr::Call { callee, .. } if callee == "len"),
        "{expr:?}"
    );
}

/// The walrus requirement again, for this node's one child: `len((n := b))`
/// is not expressible -- a walrus cannot rebind a `memoryview` -- so the
/// shape that reaches `BufferLen`'s `collect_named_expr_bindings` arm is a
/// length read whose base carries one. This asserts the arm exists at all by
/// driving the node through the same `ExprStmt` seam the index test uses.
#[test]
fn a_buffer_length_read_walks_its_base_for_walrus_bindings() {
    let hir = module_with_buffer_fn(
        Ty::None,
        vec![HirStmt::ExprStmt(HirExpr::Call {
            callee: "len".to_string(),
            args: vec![HirExpr::Name("b".to_string())],
        })],
    );
    let mir = build(&hir);
    let [MirStmt::ExprStmt(only)] = function_body(&mir) else {
        panic!("expected one expression statement");
    };
    assert!(matches!(only, MirExpr::BufferLen { .. }), "{only:?}");
}

/// Part 1 of #1142: `b[i] = v` arrives as `HirStmt::DictSet` -- `pycc_hir`
/// lowers every `<bare name>[k] = v` there -- and a `memoryview`-typed
/// target routes it to `MirStmt::BufferSet` instead.
///
/// The base is rebuilt as a `MirExpr::Name` carrying `Ty::MemoryView`
/// rather than left as the `String` `DictSet` holds: codegen's arm
/// evaluates it like any other expression, and the type is what selects the
/// `Scalar::MemoryView` the runtime call needs.
#[test]
fn a_subscript_store_into_a_memoryview_lowers_to_a_buffer_set() {
    let hir = module_with_buffer_fn(
        Ty::None,
        vec![HirStmt::DictSet {
            dict: "b".to_string(),
            key: HirExpr::Name("i".to_string()),
            value: HirExpr::FloatLiteral(1.5),
        }],
    );
    let mir = build(&hir);
    let [MirStmt::BufferSet { base, index, value }] = function_body(&mir) else {
        panic!("expected a single buffer store");
    };
    assert_eq!(base.ty(), Ty::MemoryView);
    assert_eq!(index.ty(), Ty::Int);
    assert_eq!(value.ty(), Ty::Float);
}

/// The dispatch keys on the target's resolved type, so a `dict` target
/// still lowers to `MirStmt::DictSet` -- every `d[k] = v` in the language
/// goes through this same arm, and routing one of them to a buffer store
/// would be a miscompile rather than a diagnostic.
#[test]
fn a_subscript_store_into_a_dict_still_lowers_to_a_dict_set() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "total".to_string(),
            params: vec![("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))))],
            return_ty: Ty::None,
            body: vec![HirStmt::DictSet {
                dict: "d".to_string(),
                key: HirExpr::StringLiteral("k".to_string()),
                value: HirExpr::IntLiteral(1),
            }],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    let [MirStmt::DictSet { dict, .. }] = function_body(&mir) else {
        panic!("expected a single dict store");
    };
    assert_eq!(dict, "d");
}

// ---------------------------------------------------------------------------
// Part 2a of #1142 (#1165): `MirExpr::BufferAlloc`, the second source of a
// `Ty::MemoryView` value and the first that is not a parameter.
// ---------------------------------------------------------------------------

/// `def f(): a = <callee>(4); <tail>`, with no buffer parameter, which is
/// the shape #1165 admits.
fn module_with_allocation(callee: &str, tail: Vec<HirStmt>) -> HirModule {
    let mut body = vec![HirStmt::Assign {
        target: "a".to_string(),
        value: HirExpr::Call {
            callee: callee.to_string(),
            args: vec![HirExpr::IntLiteral(4)],
        },
    }];
    body.extend(tail);
    HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body,
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    }
}

/// Both spellings lower to the dedicated node, whose `ty()` is
/// unconditionally `Ty::MemoryView` -- deliberately *not* a `MirExpr::Call`,
/// which would carry the buffer type into `pycc_codegen::call_result`'s
/// `Ty::MemoryView` panic.
#[test]
fn a_producer_call_lowers_to_a_buffer_alloc_typed_memoryview() {
    for callee in ["ndarray", "NDArray"] {
        let mir = build(&module_with_allocation(callee, vec![]));
        let [MirStmt::Assign { value, .. }] = function_body(&mir) else {
            panic!("expected a single assignment");
        };
        assert_eq!(value.ty(), Ty::MemoryView, "{callee}");
        let MirExpr::BufferAlloc { len } = value else {
            panic!("expected a `BufferAlloc`, got {value:?}");
        };
        assert_eq!(len.ty(), Ty::Int, "{callee}");
    }
}

/// The store into artifact-owned storage reaches `MirStmt::BufferSet`
/// through the same `HirStmt::DictSet` dispatch a parameter-bound buffer
/// uses: `bind_variable` records the assignment's `value.ty()`, so
/// `BufferAlloc`'s `Ty::MemoryView` selects that arm with no edit to it.
#[test]
fn a_store_into_allocated_storage_reaches_the_buffer_set_arm() {
    let mir = build(&module_with_allocation(
        "ndarray",
        vec![HirStmt::DictSet {
            dict: "a".to_string(),
            key: HirExpr::IntLiteral(0),
            value: HirExpr::FloatLiteral(1.5),
        }],
    ));
    let [_, MirStmt::BufferSet { base, .. }] = function_body(&mir) else {
        panic!("expected an allocation and a buffer store");
    };
    assert_eq!(base.ty(), Ty::MemoryView);
}

/// PEP 572 (#774): `collect_named_expr_bindings` has to recurse into the
/// length argument, or a later statement lowers against an unbound name and
/// panics.
///
/// Driven through the `ExprStmt` seam for the same reason the `BufferLen`
/// test above is: that arm is the one `stmt.rs` runs the walk from
/// (`HirStmt::Assign` never has, for any node), and `pycc_hir`'s own
/// `contains_named_expr` restriction admits a walrus in exactly the three
/// placements that seam covers. The node under test is the same either way.
#[test]
fn a_walrus_in_the_length_argument_binds_for_the_next_statement() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![
                HirStmt::ExprStmt(HirExpr::Call {
                    callee: "ndarray".to_string(),
                    args: vec![HirExpr::NamedExpr {
                        name: "n".to_string(),
                        value: Box::new(HirExpr::IntLiteral(4)),
                    }],
                }),
                HirStmt::ExprStmt(HirExpr::Name("n".to_string())),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    let [MirStmt::ExprStmt(value), MirStmt::ExprStmt(second)] = function_body(&mir) else {
        panic!("expected two expression statements");
    };
    assert!(matches!(value, MirExpr::BufferAlloc { .. }), "{value:?}");
    assert!(
        matches!(second, MirExpr::Name { name, ty: Ty::Int } if name == "n"),
        "{second:?}"
    );
}

/// D-244 #1129 statement (h) at the lowering seam, which has its own shadow
/// guard rather than inheriting the checker's: a program's own `def
/// ndarray` keeps its meaning, and the call stays a `MirExpr::Call`.
#[test]
fn a_program_that_defines_the_spelling_itself_still_lowers_to_a_call() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "ndarray".to_string(),
                params: vec![("n".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("n".to_string())))],
            },
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![HirStmt::Assign {
                    target: "a".to_string(),
                    value: HirExpr::Call {
                        callee: "ndarray".to_string(),
                        args: vec![HirExpr::IntLiteral(4)],
                    },
                }],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    let MirItem::Function { body, .. } = &mir.items[1] else {
        panic!("expected the second item to be a function");
    };
    let [MirStmt::Assign { value, .. }] = &body[..] else {
        panic!("expected a single assignment");
    };
    assert!(
        matches!(value, MirExpr::Call { callee, .. } if callee == "ndarray"),
        "{value:?}"
    );
}

// ---------------------------------------------------------------------------
// Part 2 of #1175 (#1179): `return b[start:stop]` lowers to
// `MirStmt::ReturnBufferSlice`, never to a `MirStmt::Return` of a
// `MirExpr::Slice`.
// ---------------------------------------------------------------------------

/// `def total(b: memoryview, i: int) -> memoryview: return b[<start>:<stop>]`.
fn returning_slice_of_b(start: Option<HirExpr>, stop: Option<HirExpr>) -> MirModule {
    build(&module_with_buffer_fn(
        Ty::MemoryView,
        vec![HirStmt::Return(Some(HirExpr::Slice {
            base: Box::new(HirExpr::Name("b".to_string())),
            start: start.map(Box::new),
            stop: stop.map(Box::new),
            step: None,
        }))],
    ))
}

/// The dedicated node, with both bounds lowered in place.
#[test]
fn returning_a_slice_of_a_buffer_lowers_to_the_dedicated_node() {
    let mir = returning_slice_of_b(
        Some(HirExpr::IntLiteral(1)),
        Some(HirExpr::Name("i".to_string())),
    );
    let [MirStmt::ReturnBufferSlice { name, start, stop }] = function_body(&mir) else {
        panic!(
            "expected a single `ReturnBufferSlice`, got {:?}",
            function_body(&mir)
        );
    };
    assert_eq!(name, "b");
    assert!(matches!(start, Some(MirExpr::IntLiteral(1))), "{start:?}");
    assert!(
        matches!(stop, Some(MirExpr::Name { name, ty: Ty::Int }) if name == "i"),
        "{stop:?}"
    );
}

/// An absent bound stays absent in the MIR. Codegen, not lowering, is what
/// substitutes `0` and `i64::MAX` for it -- the node must not pretend the
/// source wrote a bound it did not.
#[test]
fn an_absent_slice_bound_stays_absent_in_the_lowered_node() {
    let mir = returning_slice_of_b(None, None);
    let [
        MirStmt::ReturnBufferSlice {
            start: None,
            stop: None,
            ..
        },
    ] = function_body(&mir)
    else {
        panic!("expected both bounds absent, got {:?}", function_body(&mir));
    };
}

/// The guarantee the dedicated node exists for. `MirExpr::Slice::ty()`
/// answers from its *base*, and codegen's `Slice` arm lowers to
/// `pycc_rt_int_list_slice` -- a list-only runtime call. A buffer base
/// reaching that arm would be a silent miscompile rather than a diagnostic,
/// so this asserts the absence rather than documenting it in a comment.
#[test]
fn a_buffer_base_never_reaches_a_mir_slice_node() {
    for (start, stop) in [
        (Some(HirExpr::IntLiteral(1)), Some(HirExpr::IntLiteral(3))),
        (None, None),
    ] {
        let mir = returning_slice_of_b(start, stop);
        for stmt in function_body(&mir) {
            assert!(
                !matches!(stmt, MirStmt::Return(Some(MirExpr::Slice { .. }))),
                "{stmt:?}"
            );
        }
    }
}

/// A `step` never reaches lowering at all -- `pycc_types` refuses it -- so
/// the lowering guard keys on `step: None` and leaves any other shape on the
/// ordinary `Return` path, where `MirExpr::Slice`'s own arm would report it.
/// Lowering must not silently discard a `step` it was handed.
#[test]
fn a_slice_with_a_step_is_not_lowered_to_the_dedicated_node() {
    let mir = build(&module_with_buffer_fn(
        Ty::MemoryView,
        vec![HirStmt::Return(Some(HirExpr::Slice {
            base: Box::new(HirExpr::Name("b".to_string())),
            start: Some(Box::new(HirExpr::IntLiteral(1))),
            stop: Some(Box::new(HirExpr::IntLiteral(3))),
            step: Some(Box::new(HirExpr::IntLiteral(2))),
        }))],
    ));
    assert!(
        !matches!(function_body(&mir), [MirStmt::ReturnBufferSlice { .. }]),
        "{:?}",
        function_body(&mir)
    );
}

/// A slice of a *list* is untouched by the new guard: it keeps lowering to
/// `MirExpr::Slice` on the ordinary `Return` path.
#[test]
fn a_slice_of_a_list_still_lowers_to_a_mir_slice() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "total".to_string(),
            params: vec![("xs".to_string(), Ty::List(Box::new(Ty::Int)))],
            return_ty: Ty::List(Box::new(Ty::Int)),
            body: vec![HirStmt::Return(Some(HirExpr::Slice {
                base: Box::new(HirExpr::Name("xs".to_string())),
                start: Some(Box::new(HirExpr::IntLiteral(1))),
                stop: None,
                step: None,
            }))],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    let [MirStmt::Return(Some(MirExpr::Slice { .. }))] = function_body(&mir) else {
        panic!("expected a `Slice`, got {:?}", function_body(&mir));
    };
}
