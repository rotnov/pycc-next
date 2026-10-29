use super::*;

fn hir(items: Vec<HirItem>) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items,
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    }
}

fn func(name: &str, params: Vec<(String, Ty)>, return_ty: Ty) -> HirItem {
    HirItem::Function {
        name: name.to_string(),
        params,
        return_ty,
        body: Vec::new(),
    }
}

/// One export-set entry, keyed on the only field
/// [`refuse_in_ext_mode`] reads. Every other field is inert here: the
/// walk asks whether a name is present, never how it is carried.
fn export(name: &str, return_ty: Ty) -> crate::ext_build::ExtExport {
    crate::ext_build::ExtExport {
        defaults: Vec::new(),
        name: name.to_string(),
        class: None,
        method: None,
        returns_buffer_slice: false,
        receiver: crate::ext_build::ExtReceiver::None,
        param_names: crate::ext_build::test_param_names(&Vec::new()),
        params: Vec::new(),
        param_writable: Vec::new(),
        return_ty,
        keyword_names: None,
        keyword_receiver: None,
    }
}

#[test]
fn a_program_without_a_memoryview_annotation_is_admitted() {
    let admitted = hir(vec![func(
        "scale",
        vec![("x".to_string(), Ty::Float)],
        Ty::Float,
    )]);
    assert!(refuse_in_native_mode(&admitted).is_ok());
}

#[test]
fn ext_mode_refuses_a_memoryview_return_type_on_a_function_no_export_walk_visits() {
    let gaps = refuse_in_ext_mode(
        &hir(vec![
            // A `memoryview` parameter is not this gate's business: the
            // read refusal in `crates/pycc_types` already closes it.
            func("total", vec![("v".to_string(), Ty::MemoryView)], Ty::Int),
            // A non-function item is skipped: only a signature can name a
            // return type at all.
            HirItem::TopLevelStmt(pycc_hir::HirStmt::Assign {
                target: "n".to_string(),
                value: pycc_hir::HirExpr::IntLiteral(1),
            }),
            func("_make", Vec::new(), Ty::MemoryView),
            func("Buf.view", Vec::new(), Ty::MemoryView),
        ]),
        // Part 2b of #1142 (#1164): an empty export set, so nothing here
        // is exempt -- including `Buf.view`, which #1174 admits when it
        // *is* exported. The sibling test below pins that direction.
        &[],
    )
    .expect_err("a `memoryview` return type is refused under --ext");
    let messages: Vec<&str> = gaps.iter().map(|gap| gap.message.as_str()).collect();
    assert_eq!(gaps.len(), 2, "{messages:?}");
    assert!(
        gaps.iter()
            .all(|gap| gap.code == "C0001" && gap.span.is_none()),
        "{messages:?}"
    );
    assert!(
        messages[0].contains("`_make`'s return type is a buffer"),
        "{messages:?}"
    );
    assert!(
        messages[1].contains("`Buf.view`'s return type is a buffer"),
        "{messages:?}"
    );
}

#[test]
fn ext_mode_admits_a_program_that_returns_no_memoryview() {
    let admitted = hir(vec![func(
        "total",
        vec![("v".to_string(), Ty::MemoryView)],
        Ty::Float,
    )]);
    assert!(refuse_in_ext_mode(&admitted, &[]).is_ok());
}

/// #1174: the exemption is *exported*, with no module-level half left.
///
/// `make` is exported and dot-free and `Buf.view` is exported and dotted;
/// both are admitted, because `crate::buffer::
/// refuse_buffer_returning_method` now closes the four method-resolution
/// exits that `pycc_types`' buffer-returning-call interception did not
/// cover, so `g.view()` inside the artifact is refused at the *call*
/// rather than the declaration. `_make` is unexported and is still
/// refused -- membership in the export set is the whole test now.
#[test]
fn ext_mode_admits_every_exported_buffer_return_and_nothing_else() {
    let exports = vec![
        export("make", Ty::MemoryView),
        export("Buf.view", Ty::MemoryView),
    ];
    let program = hir(vec![
        func("make", Vec::new(), Ty::MemoryView),
        func("_make", Vec::new(), Ty::MemoryView),
        func("Buf.view", Vec::new(), Ty::MemoryView),
    ]);
    let gaps = refuse_in_ext_mode(&program, &exports)
        .expect_err("an unexported buffer return is still refused");
    let messages: Vec<&str> = gaps.iter().map(|gap| gap.message.as_str()).collect();
    assert_eq!(gaps.len(), 1, "{messages:?}");
    assert!(
        messages[0].contains("`_make`'s return type is a buffer"),
        "{messages:?}"
    );
    // Two-directional: the admitted export is not merely absent from the
    // gap list, it is admitted when it is the only item in the program.
    assert!(
        refuse_in_ext_mode(
            &hir(vec![func("make", Vec::new(), Ty::MemoryView)]),
            &exports,
        )
        .is_ok()
    );
    // #1174's own direction: the *method* is admitted on its own too,
    // which is the half that was refused before this change.
    assert!(
        refuse_in_ext_mode(
            &hir(vec![func("Buf.view", Vec::new(), Ty::MemoryView)]),
            &exports,
        )
        .is_ok()
    );
    // And a *name* match alone is not the test: an export set holding a
    // different name leaves `make` refused.
    assert!(
        refuse_in_ext_mode(
            &hir(vec![func("make", Vec::new(), Ty::MemoryView)]),
            &[export("other", Ty::MemoryView)],
        )
        .is_err()
    );
}

#[test]
fn every_memoryview_signature_is_reported_not_only_the_first() {
    let gaps = refuse_in_native_mode(&hir(vec![
        func("total", vec![("xs".to_string(), Ty::MemoryView)], Ty::Float),
        func("plain", vec![("x".to_string(), Ty::Int)], Ty::Int),
        // The return position is the second half of the gate: it is a
        // `C0003` capability gap under `--ext`, but in native mode it
        // is the same refusal a parameter gets.
        func("view", Vec::new(), Ty::MemoryView),
    ]))
    .expect_err("a memoryview annotation is refused in native mode");
    let messages: Vec<&str> = gaps.iter().map(|(_, gap)| gap.message.as_str()).collect();
    assert_eq!(gaps.len(), 2, "{messages:?}");
    assert!(
        gaps.iter()
            .all(|(_, gap)| gap.code == NATIVE_MEMORYVIEW_CODE && gap.span.is_none()),
        "{messages:?}"
    );
    assert!(messages[0].contains("`xs: memoryview`"), "{messages:?}");
    assert!(messages[1].contains("`-> memoryview`"), "{messages:?}");
    assert!(messages[0].contains("--ext"), "{messages:?}");
    // Each gap carries its own index in `hir.items` -- not its position
    // among the offending ones -- because that is what the driver joins
    // against the program's per-file item bounds to name the owning file.
    let positions: Vec<usize> = gaps.iter().map(|(index, _)| *index).collect();
    assert_eq!(positions, vec![0, 2], "{messages:?}");
}

// ---------------------------------------------------------------------
// Part 2a of #1142 (#1165): the body-level artifact-owned buffer gate.
// ---------------------------------------------------------------------

/// A minimal non-generic, non-protocol class, for the statement-(h)
/// shadow test below.
fn plain_class_def(name: &str) -> pycc_hir::HirClassDef {
    pycc_hir::HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: name.to_string(),
        bases: Vec::new(),
        mro: vec![name.to_string()],
        attrs: Vec::new(),
        methods: Vec::new(),
        type_param: None,
        properties: Vec::new(),
        static_methods: Vec::new(),
        class_methods: Vec::new(),
        is_enum: false,
        implicit_object_init: true,
        method_defaults: Vec::new(),
        enum_members: Vec::new(),
        is_dataclass: false,
        dataclass_fields: Vec::new(),
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: Vec::new(),
        is_abstract: false,
    }
}

/// `a = <callee>(4)`.
fn alloc(callee: &str) -> HirStmt {
    HirStmt::Assign {
        target: "a".to_string(),
        value: HirExpr::Call {
            callee: callee.to_string(),
            args: vec![HirExpr::IntLiteral(4)],
        },
    }
}

/// `def <name>(): <body>`.
fn body_func(name: &str, body: Vec<HirStmt>) -> HirItem {
    HirItem::Function {
        name: name.to_string(),
        params: vec![],
        return_ty: Ty::None,
        body,
    }
}

/// Both spellings, both binding forms, and a message whose ground is the
/// missing *host* rather than the missing interpreter: the allocation
/// itself would work natively, which is exactly why the refusal needs
/// its own wording instead of [`gap`]'s.
#[test]
fn allocating_a_buffer_natively_is_refused_in_its_own_words() {
    for stmt in [
        alloc("ndarray"),
        alloc("NDArray"),
        HirStmt::AnnAssign {
            target: "a".to_string(),
            annotation: Ty::MemoryView,
            value: Some(HirExpr::Call {
                callee: "ndarray".to_string(),
                args: vec![HirExpr::IntLiteral(4)],
            }),
            is_final: false,
        },
    ] {
        let module = hir(vec![body_func("f", vec![stmt])]);
        let gaps = refuse_buffer_producers_in_native_mode(&module).unwrap_err();
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].0, 0);
        assert_eq!(gaps[0].1.code, NATIVE_MEMORYVIEW_CODE);
        assert!(gaps[0].1.message.contains("`f`"), "{:?}", gaps[0].1);
        assert!(gaps[0].1.message.contains("pycc build --ext"));
        assert!(gaps[0].1.message.contains("extension-module boundary"));
        assert!(gaps[0].1.span.is_none());
    }
}

/// A program that allocates nowhere is untouched, and so is a top-level
/// statement (module scope is `pycc_types`' own refusal, in every mode).
#[test]
fn a_program_that_allocates_no_buffer_is_admitted_natively() {
    assert!(refuse_buffer_producers_in_native_mode(&hir(vec![])).is_ok());
    assert!(
        refuse_buffer_producers_in_native_mode(&hir(vec![
            body_func("f", vec![HirStmt::Return(None)]),
            HirItem::TopLevelStmt(alloc("ndarray")),
        ]))
        .is_ok()
    );
}

/// Every nesting shape the walk recurses through reaches the producer,
/// and every shape it deliberately does not recurse through is left to
/// the checker's own `C0001`.
#[test]
fn the_walk_reaches_a_producer_at_every_nesting_depth() {
    let nested: Vec<Vec<HirStmt>> = vec![
        vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![alloc("ndarray")],
            orelse: vec![],
        }],
        vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![],
            orelse: vec![alloc("ndarray")],
        }],
        vec![HirStmt::While {
            test: HirExpr::BoolLiteral(true),
            body: vec![alloc("ndarray")],
        }],
        vec![HirStmt::Try {
            body: vec![alloc("ndarray")],
            handlers: vec![],
            orelse: vec![],
            finalbody: vec![],
        }],
        vec![HirStmt::Try {
            body: vec![],
            handlers: vec![],
            orelse: vec![alloc("ndarray")],
            finalbody: vec![],
        }],
        vec![HirStmt::Try {
            body: vec![],
            handlers: vec![],
            orelse: vec![],
            finalbody: vec![alloc("ndarray")],
        }],
        vec![HirStmt::Try {
            body: vec![],
            handlers: vec![pycc_hir::HirExceptHandler {
                exc_type: None,
                name: None,
                body: vec![alloc("ndarray")],
            }],
            orelse: vec![],
            finalbody: vec![],
        }],
        vec![HirStmt::Match {
            subject: HirExpr::IntLiteral(0),
            cases: vec![pycc_hir::HirMatchCase {
                pattern: pycc_hir::HirPattern::Wildcard,
                guard: None,
                body: vec![alloc("ndarray")],
            }],
        }],
        vec![HirStmt::ForRange {
            var: "i".to_string(),
            start: HirExpr::IntLiteral(0),
            stop: HirExpr::IntLiteral(1),
            step: HirExpr::IntLiteral(1),
            body: vec![alloc("ndarray")],
        }],
        // A value-less annotated declaration binds no producer, and a
        // nested block that holds none leaves the walk at `None`.
        vec![
            HirStmt::AnnAssign {
                target: "a".to_string(),
                annotation: Ty::Int,
                value: None,
                is_final: false,
            },
            alloc("ndarray"),
        ],
    ];
    for body in nested {
        let module = hir(vec![body_func("f", body.clone())]);
        assert!(
            refuse_buffer_producers_in_native_mode(&module).is_err(),
            "{body:?}"
        );
    }
    // Not an assignment's right-hand side, so not this gate's subject.
    let module = hir(vec![body_func(
        "f",
        vec![
            HirStmt::ExprStmt(HirExpr::Call {
                callee: "ndarray".to_string(),
                args: vec![HirExpr::IntLiteral(4)],
            }),
            HirStmt::Return(None),
        ],
    )]);
    assert!(refuse_buffer_producers_in_native_mode(&module).is_ok());
}

/// The arity the checker admits is the arity this gate claims: a
/// two-argument call is `pycc_types`' `C0001`, not an `I0405`.
#[test]
fn a_producer_with_the_wrong_arity_is_not_this_gates_subject() {
    let module = hir(vec![body_func(
        "f",
        vec![HirStmt::Assign {
            target: "a".to_string(),
            value: HirExpr::Call {
                callee: "ndarray".to_string(),
                args: vec![HirExpr::IntLiteral(4), HirExpr::IntLiteral(5)],
            },
        }],
    )]);
    assert!(refuse_buffer_producers_in_native_mode(&module).is_ok());
}

/// D-244 #1129 statement (h): the program's own binding wins, through
/// each of the three tables the checker consults. Over-refusal is the
/// failure that matters here -- it rejects a legal native program.
#[test]
fn a_program_that_shadows_the_spelling_is_not_refused() {
    let own_function = hir(vec![
        HirItem::Function {
            name: "ndarray".to_string(),
            params: vec![("n".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(None)],
        },
        body_func("f", vec![alloc("ndarray")]),
    ]);
    assert!(refuse_buffer_producers_in_native_mode(&own_function).is_ok());

    let own_binding = hir(vec![
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "ndarray".to_string(),
            value: HirExpr::IntLiteral(1),
        }),
        body_func("f", vec![alloc("ndarray")]),
    ]);
    assert!(refuse_buffer_producers_in_native_mode(&own_binding).is_ok());

    let own_annotated_binding = hir(vec![
        HirItem::TopLevelStmt(HirStmt::AnnAssign {
            target: "ndarray".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::IntLiteral(1)),
            is_final: false,
        }),
        body_func("f", vec![alloc("ndarray")]),
    ]);
    assert!(refuse_buffer_producers_in_native_mode(&own_annotated_binding).is_ok());

    let mut own_class = hir(vec![body_func("f", vec![alloc("NDArray")])]);
    own_class
        .class_defs
        .push(("NDArray".to_string(), plain_class_def("NDArray")));
    assert!(refuse_buffer_producers_in_native_mode(&own_class).is_ok());

    // A foreign import of the spelling (Part 1 of #1333): the checker
    // admits `a = ndarray(4)` as a CPython call, so this gate must not
    // report an allocation the program does not make.
    let mut foreign_import = hir(vec![body_func("f", vec![alloc("ndarray")])]);
    foreign_import
        .imports
        .push(pycc_hir::ImportBinding::Foreign {
            local_name: "ndarray".to_string(),
            module_path: "ndarray".to_string(),
            from: None,
            site: pycc_hir::ForeignImportSite::Item(0),
            span: pycc_diag::Span::new(0, 0),
        });
    assert!(refuse_buffer_producers_in_native_mode(&foreign_import).is_ok());

    // A foreign import of some *other* name shadows nothing.
    let mut other_foreign = hir(vec![body_func("f", vec![alloc("ndarray")])]);
    other_foreign
        .imports
        .push(pycc_hir::ImportBinding::Foreign {
            local_name: "numpy".to_string(),
            module_path: "numpy".to_string(),
            from: None,
            site: pycc_hir::ForeignImportSite::Item(0),
            span: pycc_diag::Span::new(0, 0),
        });
    assert!(refuse_buffer_producers_in_native_mode(&other_foreign).is_err());

    // An unrelated top-level statement contributes no shadow and stops
    // no refusal.
    let unrelated = hir(vec![
        HirItem::TopLevelStmt(HirStmt::Return(None)),
        body_func("f", vec![alloc("ndarray")]),
    ]);
    assert!(refuse_buffer_producers_in_native_mode(&unrelated).is_err());
}

/// Every function that allocates is named, not only the first: one build
/// reports the whole list, exactly as the signature walk does.
#[test]
fn every_allocating_function_is_reported() {
    let module = hir(vec![
        body_func("f", vec![alloc("ndarray")]),
        body_func("g", vec![HirStmt::Return(None)]),
        body_func("h", vec![alloc("NDArray")]),
    ]);
    let gaps = refuse_buffer_producers_in_native_mode(&module).unwrap_err();
    assert_eq!(
        gaps.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
        vec![0, 2]
    );
}
