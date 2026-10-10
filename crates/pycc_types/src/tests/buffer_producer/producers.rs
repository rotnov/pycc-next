//! Buffer producers: admission, refusals, provenance joins and exported spellings.

use super::*;

/// The admitted shape, in both spellings: allocate, store one element, read
/// one back. This is the whole of what Part 2a opens, so it is also the one
/// test that proves the producer is not refused outright.
#[test]
fn allocating_a_buffer_and_touching_one_element_type_checks() {
    for callee in ["ndarray", "NDArray"] {
        let hir = func(
            vec![],
            Ty::Float,
            vec![
                alloc_four(callee),
                HirStmt::DictSet {
                    dict: "a".to_string(),
                    key: HirExpr::IntLiteral(0),
                    value: HirExpr::FloatLiteral(1.5),
                },
                HirStmt::Return(Some(element())),
            ],
        );
        assert!(check(&hir).is_ok(), "{callee}");
    }
}

/// `len(a)` on artifact-owned storage, the sweep bound #1116 admitted for a
/// parameter: the length read keys on the *type*, so it reaches the owned
/// binding unchanged.
#[test]
fn reading_the_length_of_owned_storage_type_checks() {
    let hir = func(
        vec![],
        Ty::Int,
        vec![
            alloc_four("ndarray"),
            HirStmt::Return(Some(call("len", vec![HirExpr::Name("a".to_string())]))),
        ],
    );
    assert!(check(&hir).is_ok());
}

/// The annotated form of the same binding. `reject_memoryview_declaration`
/// refuses a bare `a: memoryview`, and Part 2a gates that refusal on the
/// initializer rather than changing its own message: with a producer on the
/// right-hand side the declaration is the admitted shape.
#[test]
fn an_annotated_buffer_binding_with_a_producer_is_admitted() {
    let hir = func(
        vec![],
        Ty::Float,
        vec![
            HirStmt::AnnAssign {
                target: "a".to_string(),
                annotation: Ty::MemoryView,
                value: Some(call("ndarray", vec![HirExpr::IntLiteral(4)])),
                is_final: false,
            },
            HirStmt::Return(Some(element())),
        ],
    );
    assert!(check(&hir).is_ok());
}

/// ...and the declaration with no initializer keeps the refusal it had.
#[test]
fn an_annotated_buffer_binding_without_a_producer_is_still_refused() {
    let hir = func(
        vec![],
        Ty::None,
        vec![HirStmt::AnnAssign {
            target: "a".to_string(),
            annotation: Ty::MemoryView,
            value: None,
            is_final: false,
        }],
    );
    assert_eq!(check(&hir).unwrap_err().code, "C0001");
}

/// Every position that is *not* an assignment's right-hand side gets the one
/// named producer-position refusal, rather than falling out incidentally as
/// a type mismatch somewhere downstream. The bare-statement row is the one
/// that matters for memory safety: without it the value is allocated,
/// never slotted, and leaked once per call.
#[test]
fn a_producer_outside_an_assignment_is_the_named_position_refusal() {
    for body in [
        vec![HirStmt::ExprStmt(call(
            "ndarray",
            vec![HirExpr::IntLiteral(4)],
        ))],
        vec![HirStmt::Assign {
            target: "n".to_string(),
            value: call("len", vec![call("ndarray", vec![HirExpr::IntLiteral(4)])]),
        }],
        vec![HirStmt::Assign {
            target: "a".to_string(),
            value: HirExpr::ListLiteral(vec![call("ndarray", vec![HirExpr::IntLiteral(4)])]),
        }],
        vec![HirStmt::Return(Some(call(
            "ndarray",
            vec![HirExpr::IntLiteral(4)],
        )))],
        vec![HirStmt::Assign {
            target: "a".to_string(),
            value: HirExpr::Subscript {
                base: Box::new(call("ndarray", vec![HirExpr::IntLiteral(4)])),
                index: Box::new(HirExpr::IntLiteral(0)),
            },
        }],
    ] {
        let hir = func(vec![], Ty::None, body.clone());
        let err = check(&hir).unwrap_err();
        assert_eq!(err.code, "C0001", "{body:?}");
        assert!(
            err.message
                .contains("whole right-hand side of an assignment"),
            "{}",
            err.message
        );
    }
}

/// A wrong arity is not a producer at all, so it reaches the same
/// position refusal -- whose message names the shape that *is* admitted.
#[test]
fn a_producer_with_the_wrong_arity_is_the_position_refusal() {
    let hir = func(
        vec![],
        Ty::None,
        vec![HirStmt::Assign {
            target: "a".to_string(),
            value: call(
                "ndarray",
                vec![HirExpr::IntLiteral(4), HirExpr::IntLiteral(5)],
            ),
        }],
    );
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message
            .contains("whole right-hand side of an assignment"),
        "{}",
        err.message
    );
}

/// Module scope gets its own refusal, because its ground is different: a
/// module-level frame gets no owned-slot epilogue, so free-at-exit has no
/// exit to run at. Part 2b lifts one of the two, not both.
#[test]
fn a_producer_at_module_scope_names_the_missing_frame() {
    for stmt in [
        HirStmt::Assign {
            target: "a".to_string(),
            value: call("ndarray", vec![HirExpr::IntLiteral(4)]),
        },
        HirStmt::AnnAssign {
            target: "a".to_string(),
            annotation: Ty::MemoryView,
            value: Some(call("ndarray", vec![HirExpr::IntLiteral(4)])),
            is_final: false,
        },
    ] {
        let hir = HirModule {
            seeded_builtin_exception_classes: false,
            items: vec![HirItem::TopLevelStmt(stmt)],
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: Vec::new(),
        };
        let err = check(&hir).unwrap_err();
        assert_eq!(err.code, "C0001");
        assert!(err.message.contains("at module scope"), "{}", err.message);
    }
}

/// The length argument is an `int` element count, and a non-`int` says so in
/// the producer's own words rather than as a generic call mismatch.
#[test]
fn a_non_int_length_is_a_typed_refusal() {
    let hir = func(
        vec![],
        Ty::None,
        vec![HirStmt::Assign {
            target: "a".to_string(),
            value: call("ndarray", vec![HirExpr::StringLiteral("four".to_string())]),
        }],
    );
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("element count"), "{}", err.message);
}

/// A refusal raised while inferring the length argument is reported as
/// itself, not swallowed into the producer's own message.
#[test]
fn a_refusal_inside_the_length_argument_is_reported_unchanged() {
    let hir = func(
        vec![],
        Ty::None,
        vec![HirStmt::Assign {
            target: "a".to_string(),
            value: call("ndarray", vec![HirExpr::Name("missing".to_string())]),
        }],
    );
    let err = check(&hir).unwrap_err();
    assert!(err.message.contains("missing"), "{}", err.message);
}

/// Any use of owned storage beyond the admitted operations is refused in
/// the *owned* wording, not the buffer-parameter one. The probe is an alias
/// `z = a`, not `return a`: Part 2b of #1142 (#1164) admits the return of an
/// owned name, so the statement that used to carry this assertion now type
/// checks. Aliasing is still refused, and it is refused through the same
/// `crate::expr::reject_memoryview_read` owned arm.
#[test]
fn any_other_use_of_owned_storage_names_the_artifact_as_its_owner() {
    let hir = func(
        vec![],
        Ty::None,
        vec![
            alloc_four("ndarray"),
            HirStmt::Assign {
                target: "z".to_string(),
                value: HirExpr::Name("a".to_string()),
            },
        ],
    );
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message
            .contains("this `pycc build --ext` artifact allocated"),
        "{}",
        err.message
    );
}

/// ...and the one use Part 2b of #1142 (#1164) *does* admit: returning the
/// owned name from a function whose declared return type is the buffer type.
/// Two-directional, so a future change that reinstates the refusal cannot
/// pass by merely changing its wording.
#[test]
fn returning_owned_storage_is_admitted_by_egress() {
    for callee in ["ndarray", "NDArray"] {
        let hir = func(
            vec![],
            Ty::MemoryView,
            vec![
                alloc_four(callee),
                HirStmt::Return(Some(HirExpr::Name("a".to_string()))),
            ],
        );
        assert!(check(&hir).is_ok(), "{callee}");
    }
}

/// Part 1 of #1175 admits the second provenance through the same egress:
/// returning the name a buffer *parameter* binds. The refusal this replaces
/// reasoned that handing back a view the host lent for one call is a
/// use-after-free; that reason was falsified. The wrapper acquires an
/// independent `PyMemoryView_FromObject` export on the host object before it
/// releases its own `Py_buffer`, so the storage the returned view spans stays
/// pinned by an export the caller owns -- pycc frees nothing and the host
/// object outlives the call.
#[test]
fn returning_a_buffer_parameter_is_admitted_as_the_caller_owned_egress() {
    let hir = func(
        vec![("b".to_string(), Ty::MemoryView)],
        Ty::MemoryView,
        vec![HirStmt::Return(Some(HirExpr::Name("b".to_string())))],
    );
    assert!(check(&hir).is_ok());
}

/// Egress keys on the declared return type, not on the operand alone: an
/// owned name returned from a function declared to return something else is
/// still the ordinary return-type mismatch, not a silent admission.
#[test]
fn returning_owned_storage_from_a_non_buffer_signature_is_a_type_error() {
    let hir = func(
        vec![],
        Ty::Float,
        vec![
            alloc_four("ndarray"),
            HirStmt::Return(Some(HirExpr::Name("a".to_string()))),
        ],
    );
    let err = check(&hir).unwrap_err();
    assert_ne!(err.code, "C0003", "{}", err.message);
}

/// ...and the symmetric statement for the caller-owned provenance: a buffer
/// parameter returned from a signature declared to return something else is
/// the ordinary mismatch, not the admission. Egress keys on the *pair*
/// (operand, declared return type) for both provenances, so neither one can
/// be admitted by the operand alone.
#[test]
fn returning_a_buffer_parameter_from_a_non_buffer_signature_is_a_type_error() {
    let hir = func(
        vec![("b".to_string(), Ty::MemoryView)],
        Ty::Float,
        vec![HirStmt::Return(Some(HirExpr::Name("b".to_string())))],
    );
    let err = check(&hir).unwrap_err();
    assert_ne!(err.code, "C0003", "{}", err.message);
}

/// Rebinding a buffer parameter is refused, which is what keeps the flat
/// provenance model sound: no name is parameter-bound in one half of a
/// function and artifact-owned in the other.
#[test]
fn rebinding_a_buffer_parameter_is_refused() {
    let hir = func(
        vec![("b".to_string(), Ty::MemoryView)],
        Ty::None,
        vec![HirStmt::Assign {
            target: "b".to_string(),
            value: call("ndarray", vec![HirExpr::IntLiteral(4)]),
        }],
    );
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message
            .contains("a buffer parameter of a `pycc build --ext` export"),
        "{}",
        err.message
    );
}

/// D-244 #1129 statement (h) in call position: a program's own `def
/// ndarray` wins, and its result is that function's return type.
#[test]
fn a_program_that_defines_the_spelling_itself_keeps_its_own_meaning() {
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
                return_ty: Ty::Int,
                body: vec![
                    alloc_four("ndarray"),
                    HirStmt::Return(Some(HirExpr::Name("a".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

/// The same statement through the class table rather than the function
/// table: `class NDArray` makes `NDArray(...)` an instantiation.
#[test]
fn a_program_that_defines_the_spelling_as_a_class_keeps_its_own_meaning() {
    let class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "NDArray".to_string(),
        bases: Vec::new(),
        mro: vec!["NDArray".to_string()],
        attrs: Vec::new(),
        methods: vec![("__init__".to_string(), "NDArray.__init__".to_string())],
        type_param: None,
        properties: Vec::new(),
        static_methods: Vec::new(),
        class_methods: Vec::new(),
        is_enum: false,
        implicit_object_init: false,
        method_defaults: Vec::new(),
        enum_members: Vec::new(),
        is_dataclass: false,
        dataclass_fields: Vec::new(),
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: Vec::new(),
        is_abstract: false,
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "NDArray.__init__".to_string(),
                params: vec![
                    (
                        "self".to_string(),
                        Ty::Instance(Box::new("NDArray".to_string())),
                    ),
                    ("n".to_string(), Ty::Int),
                ],
                return_ty: Ty::None,
                body: vec![],
            },
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Instance(Box::new("NDArray".to_string())),
                body: vec![
                    alloc_four("NDArray"),
                    HirStmt::Return(Some(HirExpr::Name("a".to_string()))),
                ],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("NDArray".to_string(), class_def)],
    };
    assert!(check(&hir).is_ok());
}

/// A module-level binding of the spelling shadows it too -- and then the
/// call is refused as a call to a non-callable value, which is what CPython
/// would raise, rather than admitted as a producer.
#[test]
fn a_module_level_binding_of_the_spelling_shadows_the_producer() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "ndarray".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![alloc_four("ndarray")],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert!(
        !err.message.contains("right-hand side of an assignment"),
        "{}",
        err.message
    );
}

/// Provenance survives a branch join in every joining shape: a name bound to
/// owned storage on one arm is still owned storage after the join, so the
/// use refusal that follows is the owned one rather than the parameter one.
/// The post-join probe is an alias rather than `return a`, which #1164 now
/// admits; `owned_storage_returned_after_a_branch_join_is_admitted` covers
/// the joining shape on the admitted side.
#[test]
fn owned_provenance_survives_a_branch_join() {
    let hir = func(
        vec![("c".to_string(), Ty::Bool)],
        Ty::None,
        vec![
            HirStmt::If {
                test: HirExpr::Name("c".to_string()),
                body: vec![alloc_four("ndarray")],
                orelse: vec![alloc_four("NDArray")],
            },
            HirStmt::Assign {
                target: "z".to_string(),
                value: HirExpr::Name("a".to_string()),
            },
        ],
    );
    let err = check(&hir).unwrap_err();
    assert!(
        err.message
            .contains("this `pycc build --ext` artifact allocated"),
        "{}",
        err.message
    );
}

/// The same, through a loop body's join. The pre-loop allocation is not
/// decoration: a name bound *only* inside a loop body is `Maybe` after it
/// (D-147), so reading it is refused for that reason before provenance is
/// ever consulted -- the reachable shape is a reallocation inside the loop
/// of a name the frame already owns.
#[test]
fn owned_provenance_survives_a_loop_join() {
    let hir = func(
        vec![("c".to_string(), Ty::Bool)],
        Ty::None,
        vec![
            alloc_four("ndarray"),
            HirStmt::While {
                test: HirExpr::Name("c".to_string()),
                body: vec![alloc_four("NDArray")],
            },
            HirStmt::Assign {
                target: "z".to_string(),
                value: HirExpr::Name("a".to_string()),
            },
        ],
    );
    let err = check(&hir).unwrap_err();
    assert!(
        err.message
            .contains("this `pycc build --ext` artifact allocated"),
        "{}",
        err.message
    );
}

/// The joining shapes on the admitted side: a name bound to owned storage on
/// both arms of a branch, and one reallocated inside a loop body, are each
/// still returnable after the join. Egress reads the same `owned_buffers`
/// set the refusal above reads, so the two tests move together.
#[test]
fn owned_storage_returned_after_a_branch_join_is_admitted() {
    let branch = func(
        vec![("c".to_string(), Ty::Bool)],
        Ty::MemoryView,
        vec![
            HirStmt::If {
                test: HirExpr::Name("c".to_string()),
                body: vec![alloc_four("ndarray")],
                orelse: vec![alloc_four("NDArray")],
            },
            HirStmt::Return(Some(HirExpr::Name("a".to_string()))),
        ],
    );
    assert!(check(&branch).is_ok());

    let loop_join = func(
        vec![("c".to_string(), Ty::Bool)],
        Ty::MemoryView,
        vec![
            alloc_four("ndarray"),
            HirStmt::While {
                test: HirExpr::Name("c".to_string()),
                body: vec![alloc_four("NDArray")],
            },
            HirStmt::Return(Some(HirExpr::Name("a".to_string()))),
        ],
    );
    assert!(check(&loop_join).is_ok());
}

/// The public spelling predicate `src/memoryview_mode.rs`'s native-mode gate
/// consumes: exactly the two spellings, and `memoryview` deliberately not
/// among them.
#[test]
fn the_exported_spelling_predicate_names_exactly_the_two_producers() {
    assert!(crate::is_buffer_producer_spelling("ndarray"));
    assert!(crate::is_buffer_producer_spelling("NDArray"));
    assert!(!crate::is_buffer_producer_spelling("memoryview"));
}

/// The second export the native-mode gate consumes: the producer spellings a
/// function binds locally, filtered out of `function_local_names` so the gate
/// tests locality with the same computation
/// `buffer::producer_assignment_ty`'s statement-(h) block does. A parameter
/// and a body binding are both locals; a non-producer local is not returned,
/// and a function that binds neither returns nothing.
#[test]
fn the_exported_local_spellings_are_the_producers_a_function_binds() {
    let params = vec![("ndarray".to_string(), Ty::Int)];
    assert_eq!(
        crate::function_local_producer_spellings(&params, &[]),
        vec!["ndarray"]
    );

    let body = vec![
        HirStmt::Assign {
            target: "NDArray".to_string(),
            value: HirExpr::IntLiteral(1),
        },
        HirStmt::Assign {
            target: "other".to_string(),
            value: HirExpr::IntLiteral(2),
        },
    ];
    assert_eq!(
        crate::function_local_producer_spellings(&[], &body),
        vec!["NDArray"]
    );
    assert_eq!(
        crate::function_local_producer_spellings(&[], &body[1..]),
        Vec::<&str>::new()
    );
}
