//! Exception lowering (`exception::lower_raise` and handler lowering).
//!
//! Covers `try`/`except`/`else`/`finally`, bare and typed handlers, `raise`,
//! `raise ... from ...`, bare re-raise, `resolve_exception_tag` over every
//! builtin type, and the test helpers' own panic arms.

use crate::exception::{handler_type_tags, lower_exception_value, resolve_exception_tag};
use crate::*;
use pycc_hir::{HirExpr, HirItem, HirModule, HirStmt, Ty};

// #382 (PR-22 Part 1): MIR lowering tests for exception handling.

fn try_module(
    body: Vec<HirStmt>,
    handlers: Vec<pycc_hir::HirExceptHandler>,
    orelse: Vec<HirStmt>,
    finalbody: Vec<HirStmt>,
) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    }
}

/// Test helper: extract a `Try` statement from a top-level MIR item,
/// panicking if the item is not a `Try`.  The panic arm is covered by
/// `expect_top_level_try_panics_on_non_try`.
fn expect_top_level_try(
    item: &MirItem,
) -> (&[MirStmt], &[MirExceptHandler], &[MirStmt], &[MirStmt]) {
    match item {
        MirItem::TopLevelStmt(MirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }) => (body, handlers, orelse, finalbody),
        _ => panic!("expected Try"),
    }
}

/// Part 3 of #382 (#542, PEP 654): a `HirModule` wrapping a single
/// top-level `HirStmt::TryStar`, mirroring `try_module` above for the
/// plain `Try` case.
fn try_star_module(
    body: Vec<HirStmt>,
    handlers: Vec<pycc_hir::HirExceptHandler>,
    orelse: Vec<HirStmt>,
    finalbody: Vec<HirStmt>,
) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    }
}

/// Test helper: extract a `TryStar` statement from a top-level MIR item,
/// panicking if the item is not a `TryStar`.
fn expect_top_level_try_star(
    item: &MirItem,
) -> (&[MirStmt], &[MirExceptHandler], &[MirStmt], &[MirStmt]) {
    match item {
        MirItem::TopLevelStmt(MirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        }) => (body, handlers, orelse, finalbody),
        _ => panic!("expected TryStar"),
    }
}

/// Test helper: extract a `Raise` statement from a top-level MIR item,
/// panicking if the item is not a `Raise`.  The panic arm is covered by
/// `expect_top_level_raise_panics_on_non_raise`.
fn expect_top_level_raise(item: &MirItem) -> &MirExceptionValue {
    match item {
        MirItem::TopLevelStmt(MirStmt::Raise { exception, .. }) => exception,
        _ => panic!("expected Raise"),
    }
}

/// Test helper: extract a `RaiseFrom` statement from a top-level MIR item,
/// panicking if the item is not a `RaiseFrom`.  The panic arm is covered
/// by `expect_top_level_raise_from_panics_on_non_raise_from`.
fn expect_top_level_raise_from(item: &MirItem) -> (&MirExceptionValue, &MirExceptionValue) {
    match item {
        MirItem::TopLevelStmt(MirStmt::RaiseFrom {
            exception, cause, ..
        }) => (exception, cause),
        _ => panic!("expected RaiseFrom"),
    }
}

fn expect_constructed_exception(value: &MirExceptionValue) -> (&u8, &MirExpr) {
    match value {
        MirExceptionValue::Constructed {
            type_tag, message, ..
        } => (type_tag, message),
        MirExceptionValue::Existing(_) | MirExceptionValue::ConstructedGroup { .. } => {
            panic!("expected constructed exception")
        }
    }
}

/// Test helper: assert a top-level MIR item is a `Reraise`, panicking
/// otherwise.  The panic arm is covered by
/// `expect_top_level_reraise_panics_on_non_reraise`.
fn expect_top_level_reraise(item: &MirItem) {
    match item {
        MirItem::TopLevelStmt(MirStmt::Reraise) => {}
        _ => panic!("expected Reraise"),
    }
}

#[test]
#[should_panic(expected = "expected Try")]
fn expect_top_level_try_panics_on_non_try() {
    expect_top_level_try(&MirItem::TopLevelStmt(MirStmt::NoOp));
}

#[test]
#[should_panic(expected = "expected Raise")]
fn expect_top_level_raise_panics_on_non_raise() {
    expect_top_level_raise(&MirItem::TopLevelStmt(MirStmt::NoOp));
}

#[test]
#[should_panic(expected = "expected RaiseFrom")]
fn expect_top_level_raise_from_panics_on_non_raise_from() {
    expect_top_level_raise_from(&MirItem::TopLevelStmt(MirStmt::NoOp));
}

#[test]
#[should_panic(expected = "expected constructed exception")]
fn expect_constructed_exception_panics_on_an_existing_exception() {
    expect_constructed_exception(&MirExceptionValue::Existing(MirExpr::IntLiteral(0)));
}

/// Part 3 of #382 (#542, PEP 654, D-202): the same panic arm, reached via
/// the other non-`Constructed` variant -- a `ConstructedGroup` is not a
/// single-exception `Constructed` value either.
#[test]
#[should_panic(expected = "expected constructed exception")]
fn expect_constructed_exception_panics_on_a_constructed_group() {
    expect_constructed_exception(&MirExceptionValue::ConstructedGroup {
        type_tag: 24,
        class_name: "ExceptionGroup".to_string(),
        message: MirExpr::StringLiteral("msg".to_string()),
        members: Vec::new(),
    });
}

#[test]
#[should_panic(expected = "expected Reraise")]
fn expect_top_level_reraise_panics_on_non_reraise() {
    expect_top_level_reraise(&MirItem::TopLevelStmt(MirStmt::NoOp));
}

/// Test helper: assert a `MirExpr` is a `StringLiteral`, returning
/// the inner string.  Panicking otherwise.  The panic arm is covered
/// by `expect_string_literal_panics_on_non_string`.
fn expect_string_literal(expr: &MirExpr) -> &str {
    match expr {
        MirExpr::StringLiteral(s) => s,
        _ => panic!("expected StringLiteral"),
    }
}

#[test]
#[should_panic(expected = "expected StringLiteral")]
fn expect_string_literal_panics_on_non_string() {
    expect_string_literal(&MirExpr::IntLiteral(0));
}

#[test]
fn lowers_try_with_value_error_handler_to_mir() {
    let hir = try_module(
        vec![HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::StringLiteral("hello".to_string())],
        })],
        vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: None,
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::StringLiteral("caught".to_string())],
            })],
        }],
        Vec::new(),
        Vec::new(),
    );
    let mir = build(&hir);
    assert_eq!(mir.items.len(), 1);
    let (body, handlers, orelse, finalbody) = expect_top_level_try(&mir.items[0]);
    assert_eq!(body.len(), 1);
    assert_eq!(handlers.len(), 1);
    assert_eq!(handlers[0].exc_type_tag, Some(vec![1])); // ValueError = 1
    assert!(orelse.is_empty());
    assert!(finalbody.is_empty());
}

#[test]
fn lowers_try_with_bare_except_to_mir() {
    let hir = try_module(
        vec![HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::StringLiteral("body".to_string())],
        })],
        vec![pycc_hir::HirExceptHandler {
            exc_type: None,
            name: None,
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::StringLiteral("caught".to_string())],
            })],
        }],
        Vec::new(),
        Vec::new(),
    );
    let mir = build(&hir);
    let (_, handlers, _, _) = expect_top_level_try(&mir.items[0]);
    assert_eq!(handlers[0].exc_type_tag, None); // bare except
}

#[test]
fn lowers_try_with_else_and_finally_to_mir() {
    let hir = try_module(
        vec![HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::StringLiteral("body".to_string())],
        })],
        vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["Exception".to_string()]),
            name: None,
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::StringLiteral("handler".to_string())],
            })],
        }],
        vec![HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::StringLiteral("else".to_string())],
        })],
        vec![HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::StringLiteral("finally".to_string())],
        })],
    );
    let mir = build(&hir);
    let (body, handlers, orelse, finalbody) = expect_top_level_try(&mir.items[0]);
    assert_eq!(body.len(), 1);
    assert_eq!(handlers.len(), 1);
    assert_eq!(handlers[0].exc_type_tag, Some(vec![0])); // Exception = 0
    assert_eq!(orelse.len(), 1);
    assert_eq!(finalbody.len(), 1);
}

/// Part 3 of #382 (#542, PEP 654): `lower_stmt`'s own unit-test binary
/// never previously built a `HirStmt::TryStar` at all -- every existing
/// `except*` test drove it end-to-end through `pycc_types`/`pycc_codegen`,
/// a *different* compiled instance of this crate under `cargo llvm-cov
/// --workspace`. Both instances are tracked separately for the D-014
/// region/line gate (`llvm-cov export`/`report` counts each monomorphized
/// compilation unit's regions independently, unlike `llvm-cov show`'s
/// per-source-line merge), so the end-to-end exercise of `HirStmt::TryStar`
/// left this crate's *own* instance of `lower_stmt`'s `TryStar` arm --
/// its body/handlers/orelse/finalbody closures -- literally unexecuted.
/// A named, typed handler with a non-empty body, plus non-empty orelse
/// and finalbody, exercises every closure in the arm: the body map, the
/// handlers map, its nested `exc_type` tag-collection map/flat_map, the
/// handler-body map, the `binding_ty` map (only entered when `h.name` is
/// `Some`), the orelse map, and the finalbody map.
#[test]
fn lowers_try_star_with_named_handler_else_and_finally_to_mir() {
    let hir = try_star_module(
        vec![HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::StringLiteral("body".to_string())],
        })],
        vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: Some("eg".to_string()),
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::StringLiteral("handler".to_string())],
            })],
        }],
        vec![HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::StringLiteral("else".to_string())],
        })],
        vec![HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::StringLiteral("finally".to_string())],
        })],
    );
    let mir = build(&hir);
    assert_eq!(mir.items.len(), 1);
    let (body, handlers, orelse, finalbody) = expect_top_level_try_star(&mir.items[0]);
    assert_eq!(body.len(), 1);
    assert_eq!(handlers.len(), 1);
    assert_eq!(handlers[0].exc_type_tag, Some(vec![1])); // ValueError = 1
    assert_eq!(handlers[0].binding_name.as_deref(), Some("eg"));
    // `except*` always binds the handler name to `ExceptionGroup`, never
    // the named handler type (matching `pycc_types::exception::
    // check_try_star_stmt`'s identical rule at type-checking time).
    assert_eq!(
        handlers[0].binding_ty,
        Some(Ty::Instance(Box::new("ExceptionGroup".to_string())))
    );
    assert_eq!(handlers[0].body.len(), 1);
    assert_eq!(orelse.len(), 1);
    assert_eq!(finalbody.len(), 1);
}

#[test]
fn lowers_raise_value_error_to_mir() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Raise {
            exc: Some(HirExpr::Call {
                callee: "ValueError".to_string(),
                args: vec![HirExpr::StringLiteral("bad value".to_string())],
            }),
            cause: None,
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    let (exc_type_tag, message) =
        expect_constructed_exception(expect_top_level_raise(&mir.items[0]));
    assert_eq!(*exc_type_tag, 1); // ValueError = 1
    expect_string_literal(message);
}

#[test]
fn lowers_raise_from_to_mir() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Raise {
            exc: Some(HirExpr::Call {
                callee: "ValueError".to_string(),
                args: vec![HirExpr::StringLiteral("bad".to_string())],
            }),
            cause: Some(HirExpr::Call {
                callee: "TypeError".to_string(),
                args: vec![HirExpr::StringLiteral("cause".to_string())],
            }),
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    let (exception, cause) = expect_top_level_raise_from(&mir.items[0]);
    let (exc_type_tag, _) = expect_constructed_exception(exception);
    let (cause_type_tag, _) = expect_constructed_exception(cause);
    assert_eq!(*exc_type_tag, 1); // ValueError = 1
    assert_eq!(*cause_type_tag, 2); // TypeError = 2
}

#[test]
fn lowers_bare_reraise_to_mir() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Raise {
            exc: None,
            cause: None,
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    expect_top_level_reraise(&mir.items[0]);
}

#[test]
fn resolve_exception_tag_maps_all_builtin_types() {
    assert_eq!(resolve_exception_tag("Exception"), Some(0));
    assert_eq!(resolve_exception_tag("ValueError"), Some(1));
    assert_eq!(resolve_exception_tag("TypeError"), Some(2));
    assert_eq!(resolve_exception_tag("KeyError"), Some(3));
    assert_eq!(resolve_exception_tag("IndexError"), Some(4));
    assert_eq!(resolve_exception_tag("ZeroDivisionError"), Some(5));
    assert_eq!(resolve_exception_tag("RuntimeError"), Some(6));
    assert_eq!(resolve_exception_tag("UnknownError"), None);
}

/// Part 2 of #543 (#739), D-194: `resolve_exception_tag`'s hand-written
/// `match` is documented as implementing exactly the name set
/// `pycc_hir::is_flat_builtin_exception_class` recognizes, "kept in sync by
/// construction, not by a shared constant" (see this function's own doc
/// comment). That comment is a promise with nothing enforcing it: the two
/// are independent artifacts in different crates. This test turns the
/// promise into an assertion, over the full `BUILTIN_EXCEPTION_CLASSES`
/// array rather than a hardcoded name list, so a future edit that widens or
/// reorders the flat-name set in one place without the other fails loudly
/// here instead of silently reopening the `.expect()` panic risk D-194's
/// shadow-gate fix was written to eliminate (see
/// `pycc_types::exception::is_unshadowed_builtin_exception`, which trusts
/// `is_flat_builtin_exception_class` to mean exactly "resolvable by
/// `resolve_exception_tag`").
#[test]
fn resolve_exception_tag_agrees_with_is_flat_builtin_exception_class() {
    for (index, name) in pycc_hir::BUILTIN_EXCEPTION_CLASSES.iter().enumerate() {
        let flat = pycc_hir::is_flat_builtin_exception_class(name);
        let resolved = resolve_exception_tag(name);
        if flat {
            assert_eq!(
                resolved,
                Some(index as u8),
                "`{name}` is flat but resolve_exception_tag disagrees on its tag"
            );
        } else {
            assert_eq!(
                resolved, None,
                "`{name}` is not flat but resolve_exception_tag still resolves it by name"
            );
        }
    }
}

#[test]
fn lowers_raise_with_no_args_uses_fallback_message() {
    // `raise ValueError()` — no args, should use "unknown" fallback.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Raise {
            exc: Some(HirExpr::Call {
                callee: "ValueError".to_string(),
                args: vec![],
            }),
            cause: None,
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    let (_, message) = expect_constructed_exception(expect_top_level_raise(&mir.items[0]));
    assert_eq!(expect_string_literal(message), "unknown");
}

#[test]
fn lowers_existing_exception_value_without_replacing_it() {
    let mut scopes = vec![HashMap::from([(
        "some_exc".to_string(),
        Ty::Instance(Box::new("ValueError".to_string())),
    )])];
    let value = lower_exception_value(
        &HirExpr::Name("some_exc".to_string()),
        &mut scopes,
        &HashMap::new(),
        None,
    );
    assert!(matches!(
        value,
        MirExceptionValue::Existing(MirExpr::Name {
            name,
            ty: Ty::Instance(class_name),
        }) if name == "some_exc" && class_name.as_str() == "ValueError"
    ));
}

#[test]
#[should_panic(expected = "pycc_types rejects unknown exception handler types before MIR")]
fn an_unknown_exception_handler_cannot_silently_become_a_bare_handler() {
    let hir = try_module(
        vec![HirStmt::ExprStmt(HirExpr::IntLiteral(0))],
        vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["TypoError".to_string()]),
            name: None,
            body: vec![HirStmt::ExprStmt(HirExpr::IntLiteral(0))],
        }],
        Vec::new(),
        Vec::new(),
    );
    let _ = build(&hir);
}

// Part 2 of #541 (D-189): user-defined exception classes.

/// A minimal raisable user exception class: `class <name>(<mro[1]>): pass`,
/// carrying the tag HIR lowering would have assigned it.
fn user_exception_class(name: &str, mro: &[&str], tag: Option<u8>) -> (String, HirClassDef) {
    (
        name.to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: tag,
            name: name.to_string(),
            bases: mro
                .get(1)
                .map(|base| vec![base.to_string()])
                .unwrap_or_default(),
            mro: mro.iter().map(|entry| entry.to_string()).collect(),
            attrs: Vec::new(),
            methods: vec![(
                "__init__".to_string(),
                pycc_hir::EXCEPTION_INIT_MANGLED_NAME.to_string(),
            )],
            properties: Vec::new(),
            static_methods: Vec::new(),
            class_methods: Vec::new(),
            type_param: None,
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
        },
    )
}

fn exception_hierarchy() -> HashMap<String, HirClassDef> {
    HashMap::from([
        user_exception_class("AppError", &["AppError", "Exception"], Some(7)),
        user_exception_class(
            "DatabaseError",
            &["DatabaseError", "AppError", "Exception"],
            Some(9),
        ),
        user_exception_class(
            "ConfigError",
            &["ConfigError", "AppError", "Exception"],
            Some(8),
        ),
        // Rooted at a builtin other than `Exception`, so a `ValueError`
        // handler has to widen to it (#702).
        user_exception_class(
            "ParseError",
            &["ParseError", "ValueError", "Exception"],
            Some(10),
        ),
        user_exception_class("Unrelated", &["Unrelated"], None),
    ])
}

#[test]
fn a_user_exception_handler_accepts_its_own_tag_and_every_subclass_tag_sorted() {
    let classes = exception_hierarchy();
    // Sorted ascending even though `ConfigError` (8) was declared after
    // `DatabaseError` (9) and `classes` iterates in hash-map order.
    assert_eq!(handler_type_tags("AppError", &classes), vec![7, 8, 9]);
}

#[test]
fn a_leaf_user_exception_handler_accepts_only_its_own_tag() {
    let classes = exception_hierarchy();
    assert_eq!(handler_type_tags("DatabaseError", &classes), vec![9]);
}

#[test]
fn a_builtin_handler_also_accepts_its_user_defined_subclasses() {
    let classes = exception_hierarchy();
    // `ParseError` derives from `ValueError`, not from `Exception`, so the
    // handler widens from `ValueError`'s own tag 1 to include it.
    assert_eq!(handler_type_tags("ValueError", &classes), vec![1, 10]);
}

#[test]
fn a_builtin_handler_without_user_subclasses_stays_a_single_tag() {
    let classes = exception_hierarchy();
    // No class in the fixture reaches `TypeError`.
    assert_eq!(handler_type_tags("TypeError", &classes), vec![2]);
}

#[test]
fn an_exception_handler_stays_a_single_catch_all_tag() {
    // Tag 0 is `pycc_rt_exception_type_matches`'s own catch-all, so listing
    // every user tag alongside it would emit dead comparisons.
    let classes = exception_hierarchy();
    assert_eq!(handler_type_tags("Exception", &classes), vec![0]);
}

#[test]
fn raising_a_user_exception_class_lowers_to_a_constructed_value_with_its_name() {
    let classes = exception_hierarchy();
    let mut scopes = vec![HashMap::new()];
    let value = lower_exception_value(
        &HirExpr::Call {
            callee: "DatabaseError".to_string(),
            args: vec![HirExpr::StringLiteral("boom".to_string())],
        },
        &mut scopes,
        &classes,
        None,
    );
    assert!(matches!(
        value,
        MirExceptionValue::Constructed {
            type_tag: 9,
            ref class_name,
            message: MirExpr::StringLiteral(ref message),
        } if class_name == "DatabaseError" && message == "boom"
    ));
}

#[test]
fn a_pep_758_multi_type_handlers_tag_set_is_the_union_deduped() {
    // PEP 758 (#740): `except (AppError, ConfigError):` unions each named
    // type's own `handler_type_tags` result. `AppError`'s set ([7, 8, 9])
    // already contains `ConfigError`'s own tag (8), since `ConfigError` is
    // one of `AppError`'s subclasses -- so a naive concatenation would
    // double-count tag 8, and the combined set must be deduped back down
    // to `AppError`'s own set (a hand-computed overlapping-family case,
    // matching the real `OSError`/`ConnectionError` overlap this change
    // exists to handle correctly).
    let classes = exception_hierarchy();
    let mut combined: Vec<u8> = handler_type_tags("AppError", &classes);
    combined.extend(handler_type_tags("ConfigError", &classes));
    assert_eq!(
        combined,
        vec![7, 8, 9, 8],
        "sanity: naive concatenation duplicates tag 8 before dedup"
    );
    combined.sort_unstable();
    combined.dedup();
    assert_eq!(combined, vec![7, 8, 9]);
}

#[test]
fn a_pep_758_multi_type_handler_lowers_through_build_with_deduped_sorted_tags() {
    // Unlike the test above (which hand-computes the union/dedup in
    // isolation), this drives the real `HirStmt::Try` -> MIR lowering path
    // in `crate::stmt` end to end, so deleting either `.sort_unstable()` or
    // `.dedup()` at that call site would fail *this* test even though
    // codegen's OR-chain dispatch is itself idempotent under a duplicate or
    // unsorted tag (deep-reviewer finding on #740).
    let classes = exception_hierarchy();
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Try {
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::StringLiteral("body".to_string())],
            })],
            handlers: vec![pycc_hir::HirExceptHandler {
                // `AppError`'s own tag set ([7, 8, 9]) already contains
                // `ConfigError`'s tag (8), so a naive union would produce
                // [7, 8, 9, 8] before sort+dedup.
                exc_type: Some(vec!["AppError".to_string(), "ConfigError".to_string()]),
                name: None,
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::StringLiteral("caught".to_string())],
                })],
            }],
            orelse: Vec::new(),
            finalbody: Vec::new(),
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: classes.into_iter().collect(),
    };
    let mir = build(&hir);
    let (_, handlers, _, _) = expect_top_level_try(&mir.items[0]);
    assert_eq!(handlers.len(), 1);
    assert_eq!(handlers[0].exc_type_tag, Some(vec![7, 8, 9]));
}

#[test]
fn calling_an_untagged_class_is_not_an_exception_construction() {
    let classes = exception_hierarchy();
    let mut scopes = vec![HashMap::new()];
    let value = lower_exception_value(
        &HirExpr::Call {
            callee: "Unrelated".to_string(),
            args: Vec::new(),
        },
        &mut scopes,
        &classes,
        None,
    );
    assert!(matches!(value, MirExceptionValue::Existing(_)));
}

// -- Part 3A of #541 (#736): render a caught exception binding ----------

#[test]
fn print_of_a_caught_exception_binding_lowers_to_exception_message() {
    let hir = try_module(
        Vec::new(),
        vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: Some("e".to_string()),
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Name("e".to_string())],
            })],
        }],
        Vec::new(),
        Vec::new(),
    );
    let mir = build(&hir);
    let (_, handlers, _, _) = expect_top_level_try(&mir.items[0]);
    assert_eq!(handlers.len(), 1);
    assert_eq!(handlers[0].body.len(), 1);
    // `print`'s argument should be rewritten from a bare `Name` read of
    // `e` to `MirExpr::ExceptionMessage(Name(e))`, matching
    // `rewrite_instance_to_repr`'s own dataclass `print` rewrite tests.
    assert!(matches!(
        &handlers[0].body[0],
        MirStmt::ExprStmt(MirExpr::Call { callee, args, .. })
            if callee == "print"
                && args.len() == 1
                && matches!(
                    &args[0],
                    MirExpr::ExceptionMessage(inner)
                        if matches!(inner.as_ref(), MirExpr::Name { name, .. } if name == "e")
                )
    ));
}

#[test]
fn fstring_interpolation_of_a_caught_exception_binding_lowers_to_exception_message() {
    let hir = try_module(
        Vec::new(),
        vec![pycc_hir::HirExceptHandler {
            exc_type: Some(vec!["ValueError".to_string()]),
            name: Some("e".to_string()),
            body: vec![HirStmt::Assign {
                target: "s".to_string(),
                value: HirExpr::FString(vec![pycc_hir::FStringPart::Interpolation(Box::new(
                    HirExpr::Name("e".to_string()),
                ))]),
            }],
        }],
        Vec::new(),
        Vec::new(),
    );
    let mir = build(&hir);
    let (_, handlers, _, _) = expect_top_level_try(&mir.items[0]);
    assert_eq!(handlers.len(), 1);
    assert!(matches!(
        &handlers[0].body[0],
        MirStmt::Assign {
            value: MirExpr::FString(parts),
            ..
        } if parts.len() == 1
            && matches!(
                &parts[0],
                MirFStringPart::Interpolation(inner)
                    if matches!(inner.as_ref(), MirExpr::ExceptionMessage(_))
            )
    ));
}

#[test]
fn print_of_a_caught_user_defined_exception_binding_lowers_to_exception_message() {
    // The rewrite must also fire for a user-defined exception subclass, not
    // just a builtin like `ValueError` -- the stated scope for #736 covers
    // "a user-defined exception class inheriting `Exception`'s constructor".
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Try {
            body: Vec::new(),
            handlers: vec![pycc_hir::HirExceptHandler {
                exc_type: Some(vec!["AppError".to_string()]),
                name: Some("e".to_string()),
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::Name("e".to_string())],
                })],
            }],
            orelse: Vec::new(),
            finalbody: Vec::new(),
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: exception_hierarchy().into_iter().collect(),
    };
    let mir = build(&hir);
    let (_, handlers, _, _) = expect_top_level_try(&mir.items[0]);
    assert_eq!(handlers.len(), 1);
    assert_eq!(handlers[0].body.len(), 1);
    assert!(matches!(
        &handlers[0].body[0],
        MirStmt::ExprStmt(MirExpr::Call { callee, args, .. })
            if callee == "print"
                && args.len() == 1
                && matches!(
                    &args[0],
                    MirExpr::ExceptionMessage(inner)
                        if matches!(inner.as_ref(), MirExpr::Name { name, .. } if name == "e")
                )
    ));
}

#[test]
fn print_of_a_non_exception_class_instance_is_not_rewritten_to_exception_message() {
    // `rewrite_exception_to_message` must be a no-op for an ordinary,
    // non-exception class instance -- covers the `exception_type_tag`
    // `None` branch for a *registered* but non-exception class (distinct
    // from an unregistered class name, already covered by
    // `print_of_an_instance_with_an_unregistered_class_passes_through` in
    // `class_dunder.rs`).
    let point_ty = Ty::Instance(Box::new("Point".to_string()));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "test".to_string(),
            params: vec![("p".to_string(), point_ty.clone())],
            return_ty: Ty::None,
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Name("p".to_string())],
            })],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![(
            "Point".to_string(),
            pycc_hir::HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "Point".to_string(),
                bases: Vec::new(),
                mro: vec!["Point".to_string()],
                attrs: Vec::new(),
                methods: Vec::new(),
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
            },
        )],
    };
    let mir = build(&hir);
    assert!(matches!(
        &mir.items[0],
        MirItem::Function { body, .. }
            if body.len() == 1
                && matches!(
                    &body[0],
                    MirStmt::ExprStmt(MirExpr::Call { args, .. })
                        if args.len() == 1 && args[0].ty() == point_ty
                )
    ));
}
