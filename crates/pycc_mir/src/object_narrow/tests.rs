//! #1476 (Part 3 of #1387): `isinstance` narrowing of an `object` name at
//! the MIR layer, beside the `Optional` narrowing `tests/narrow.rs` covers.

use super::*;
use crate::{MirItem, MirStmt, ObjBuiltinClass, ObjIsInstanceClass, build};
use pycc_hir::{CmpOpKind, HirItem, HirModule, HirStmt, UnaryOpKind};

fn name(n: &str) -> HirExpr {
    HirExpr::Name(n.to_string())
}

fn isinstance(operand: &str, class: &str) -> HirExpr {
    HirExpr::Call {
        callee: "isinstance".to_string(),
        args: vec![name(operand), name(class)],
    }
}

fn not(operand: HirExpr) -> HirExpr {
    HirExpr::UnaryOp {
        op: UnaryOpKind::Not,
        operand: Box::new(operand),
    }
}

fn scope(entries: &[(&str, Ty)]) -> Vec<HashMap<String, Ty>> {
    vec![
        entries
            .iter()
            .map(|(name, ty)| (name.to_string(), ty.clone()))
            .collect(),
    ]
}

fn object_name(n: &str) -> MirExpr {
    MirExpr::Name {
        name: n.to_string(),
        ty: Ty::Object,
    }
}

fn plain_class(class: &str) -> HirClassDef {
    HirClassDef {
        name: class.to_string(),
        bases: Vec::new(),
        mro: vec![class.to_string()],
        attrs: Vec::new(),
        methods: Vec::new(),
        properties: Vec::new(),
        static_methods: Vec::new(),
        class_methods: Vec::new(),
        type_param: None,
        is_enum: false,
        implicit_object_init: false,
        method_defaults: Vec::new(),
        enum_members: Vec::new(),
        class_attrs: Vec::new(),
        is_dataclass: false,
        dataclass_fields: Vec::new(),
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: Vec::new(),
        is_abstract: false,
        exception_type_tag: None,
    }
}

#[test]
fn an_isinstance_guard_on_an_object_name_narrows_by_polarity() {
    let scopes = scope(&[("o", Ty::Object)]);
    let classes = HashMap::new();
    assert_eq!(
        narrowing_target(&isinstance("o", "int"), &scopes, &classes),
        Some(("o".to_string(), Ty::Int, NarrowSide::Body))
    );
    assert_eq!(
        narrowing_target(&not(isinstance("o", "str")), &scopes, &classes),
        Some(("o".to_string(), Ty::Str, NarrowSide::Orelse))
    );
}

#[test]
fn a_compiled_class_guard_narrows_to_its_instance() {
    let scopes = scope(&[("o", Ty::Object)]);
    let classes = HashMap::from([("C".to_string(), plain_class("C"))]);
    assert_eq!(
        narrowing_target(&isinstance("o", "C"), &scopes, &classes),
        Some((
            "o".to_string(),
            Ty::Instance(Box::new("C".to_string())),
            NarrowSide::Body
        ))
    );
}

/// A class another compiled class derives from keeps the operand an
/// object; the leaf narrows.
#[test]
fn a_class_with_a_compiled_subclass_does_not_narrow() {
    let scopes = scope(&[("o", Ty::Object)]);
    let mut derived = plain_class("D");
    derived.mro.push("B".to_string());
    let classes = HashMap::from([
        ("B".to_string(), plain_class("B")),
        ("D".to_string(), derived),
    ]);
    assert_eq!(
        narrowing_target(&isinstance("o", "B"), &scopes, &classes),
        None
    );
    assert_eq!(
        narrowing_target(&isinstance("o", "D"), &scopes, &classes),
        Some((
            "o".to_string(),
            Ty::Instance(Box::new("D".to_string())),
            NarrowSide::Body
        ))
    );
}

#[test]
fn a_shadowed_builtin_a_shadowed_class_or_a_native_operand_does_not_narrow() {
    let classes = HashMap::new();
    let test = isinstance("o", "int");
    // A user function named `isinstance`.
    let shadowed_fn = scope(&[("o", Ty::Object), ("$fn:isinstance", Ty::Bool)]);
    assert_eq!(narrowing_target(&test, &shadowed_fn, &classes), None);
    // A binding named like the class.
    let shadowed_class = scope(&[("o", Ty::Object), ("int", Ty::Int)]);
    assert_eq!(narrowing_target(&test, &shadowed_class, &classes), None);
    // A module function named like the class.
    let shadowed_class_fn = scope(&[("o", Ty::Object), ("$fn:int", Ty::Str)]);
    assert_eq!(narrowing_target(&test, &shadowed_class_fn, &classes), None);
    // A native operand is folded, never narrowed.
    let native = scope(&[("o", Ty::Int)]);
    assert_eq!(narrowing_target(&test, &native, &classes), None);
    // An unbound operand.
    assert_eq!(narrowing_target(&test, &scope(&[]), &classes), None);
    // A class this part does not narrow to.
    let object = scope(&[("o", Ty::Object)]);
    assert_eq!(
        narrowing_target(&isinstance("o", "list"), &object, &classes),
        None
    );
}

#[test]
fn the_optional_none_test_keeps_its_own_sides() {
    let scopes = scope(&[("x", Ty::Optional(Box::new(Ty::Int))), ("o", Ty::Object)]);
    let classes = HashMap::new();
    let test = |op| HirExpr::Compare {
        op,
        left: Box::new(name("x")),
        right: Box::new(HirExpr::NoneLiteral),
    };
    assert_eq!(
        narrowing_target(&test(CmpOpKind::IsNot), &scopes, &classes),
        Some(("x".to_string(), Ty::Int, NarrowSide::Body))
    );
    assert_eq!(
        narrowing_target(&test(CmpOpKind::Is), &scopes, &classes),
        Some(("x".to_string(), Ty::Int, NarrowSide::Orelse))
    );
    // `o is None` on an `object` name is no `Optional` narrowing.
    let object_test = HirExpr::Compare {
        op: CmpOpKind::Is,
        left: Box::new(name("o")),
        right: Box::new(HirExpr::NoneLiteral),
    };
    assert_eq!(narrowing_target(&object_test, &scopes, &classes), None);
}

#[test]
fn a_narrowed_read_unboxes_an_object_slot_and_unwraps_an_optional_one() {
    assert_eq!(
        narrowed_read("o", Ty::Object, Ty::Str),
        MirExpr::ObjectUnbox(Box::new(object_name("o")), Box::new(Ty::Str))
    );
    let optional = Ty::Optional(Box::new(Ty::Int));
    assert_eq!(
        narrowed_read("x", optional.clone(), Ty::Int),
        MirExpr::OptionalUnwrap(
            Box::new(MirExpr::Name {
                name: "x".to_string(),
                ty: optional,
            }),
            Box::new(Ty::Int)
        )
    );
    assert_eq!(
        MirExpr::ObjectUnbox(Box::new(object_name("o")), Box::new(Ty::Float)).ty(),
        Ty::Float
    );
}

#[test]
fn object_operand_strips_only_a_narrowed_object_read() {
    let unboxed = MirExpr::ObjectUnbox(Box::new(object_name("o")), Box::new(Ty::Int));
    assert_eq!(object_operand(unboxed), object_name("o"));
    assert_eq!(
        object_operand(MirExpr::IntLiteral(1)),
        MirExpr::IntLiteral(1)
    );
}

#[test]
fn a_bare_narrowed_object_read_is_recognized() {
    let mut scopes = scope(&[("o", Ty::Object), ("x", Ty::Optional(Box::new(Ty::Int)))]);
    assert!(!is_bare_narrowed_object_read(&name("o"), &scopes));
    crate::push_narrowing(&mut scopes, "o", Ty::Int);
    crate::push_narrowing(&mut scopes, "x", Ty::Int);
    assert!(is_bare_narrowed_object_read(&name("o"), &scopes));
    // An `Optional` narrowing is not an `object` one.
    assert!(!is_bare_narrowed_object_read(&name("x"), &scopes));
    // Only a bare read counts.
    assert!(!is_bare_narrowed_object_read(
        &HirExpr::IntLiteral(1),
        &scopes
    ));
}

fn module(items: Vec<HirItem>) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items,
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    }
}

fn print(value: HirExpr) -> HirStmt {
    HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![value],
    })
}

fn print_mir(value: MirExpr) -> MirStmt {
    MirStmt::ExprStmt(MirExpr::Call {
        callee: "print".to_string(),
        args: vec![value],
        ty: Ty::None,
    })
}

/// `def f(o: object, p: object)` around `body`; answers the lowered body.
fn function_body(body: Vec<HirStmt>) -> Vec<MirStmt> {
    let mir = build(&module(vec![HirItem::Function {
        name: "f".to_string(),
        params: vec![("o".to_string(), Ty::Object), ("p".to_string(), Ty::Object)],
        return_ty: Ty::None,
        body,
    }]));
    let MirItem::Function { body, .. } = mir.items.into_iter().next().expect("one item") else {
        panic!("expected the lowered function");
    };
    body
}

fn unboxed(n: &str, ty: Ty) -> MirExpr {
    MirExpr::ObjectUnbox(Box::new(object_name(n)), Box::new(ty))
}

#[test]
fn the_guarded_body_reads_the_unboxed_value_and_the_else_branch_the_object() {
    let body = function_body(vec![HirStmt::If {
        test: isinstance("o", "float"),
        body: vec![print(name("o"))],
        orelse: vec![print(name("o"))],
    }]);
    let MirStmt::If { body, orelse, .. } = &body[0] else {
        panic!("expected the lowered `if`");
    };
    assert_eq!(body, &vec![print_mir(unboxed("o", Ty::Float))]);
    assert_eq!(orelse, &vec![print_mir(object_name("o"))]);
}

#[test]
fn a_negated_guard_narrows_the_else_branch_and_a_terminating_body_the_continuation() {
    let body = function_body(vec![
        HirStmt::If {
            test: not(isinstance("o", "int")),
            body: vec![HirStmt::Return(None)],
            orelse: vec![],
        },
        print(name("o")),
    ]);
    assert_eq!(body[1], print_mir(unboxed("o", Ty::Int)));
}

/// An `else` that rebinds the name leaves the continuation reading the
/// object (#1476 review): unboxing it there would read a rebound `float`
/// as an `int`.
#[test]
fn an_else_that_rebinds_the_name_leaves_the_continuation_an_object() {
    let body = function_body(vec![
        HirStmt::If {
            test: not(isinstance("o", "int")),
            body: vec![HirStmt::Return(None)],
            orelse: vec![HirStmt::Assign {
                target: "o".to_string(),
                value: name("p"),
            }],
        },
        print(name("o")),
    ]);
    assert_eq!(body[1], print_mir(object_name("o")));
}

#[test]
fn a_first_binding_from_a_narrowed_name_keeps_the_object() {
    let body = function_body(vec![HirStmt::If {
        test: isinstance("o", "str"),
        body: vec![HirStmt::Assign {
            target: "y".to_string(),
            value: name("o"),
        }],
        orelse: vec![],
    }]);
    let MirStmt::If { body, .. } = &body[0] else {
        panic!("expected the lowered `if`");
    };
    assert_eq!(
        body,
        &vec![MirStmt::Assign {
            target: "y".to_string(),
            value: object_name("o"),
        }]
    );
}

/// Part 11 of #1371: `type(o)` under a guard asks CPython for the object's
/// own class, so the narrowed read's unbox is dropped.
#[test]
fn type_of_a_narrowed_name_reads_the_object_itself() {
    let body = function_body(vec![HirStmt::If {
        test: isinstance("o", "int"),
        body: vec![print(HirExpr::Call {
            callee: "type".to_string(),
            args: vec![name("o")],
        })],
        orelse: vec![],
    }]);
    let MirStmt::If { body, .. } = &body[0] else {
        panic!("expected the lowered `if`");
    };
    assert_eq!(
        body[0],
        print_mir(MirExpr::ObjType {
            base: Box::new(object_name("o")),
        })
    );
}

#[test]
fn identity_and_a_nested_guard_test_the_object_itself() {
    let body = function_body(vec![HirStmt::If {
        test: isinstance("o", "int"),
        body: vec![
            print(HirExpr::Compare {
                op: CmpOpKind::Is,
                left: Box::new(name("o")),
                right: Box::new(name("p")),
            }),
            print(isinstance("o", "bool")),
        ],
        orelse: vec![],
    }]);
    let MirStmt::If { body, .. } = &body[0] else {
        panic!("expected the lowered `if`");
    };
    assert_eq!(
        body[0],
        print_mir(MirExpr::ObjCompare {
            op: CmpOpKind::Is,
            left: Box::new(object_name("o")),
            right: Box::new(object_name("p")),
        })
    );
    assert_eq!(
        body[1],
        print_mir(MirExpr::ObjIsInstance {
            value: Box::new(object_name("o")),
            class: ObjIsInstanceClass::Builtin(ObjBuiltinClass::Bool),
        })
    );
}

#[test]
fn rebinding_the_narrowed_name_ends_the_narrowing() {
    let body = function_body(vec![HirStmt::If {
        test: isinstance("o", "int"),
        body: vec![
            HirStmt::Assign {
                target: "o".to_string(),
                value: name("p"),
            },
            print(name("o")),
        ],
        orelse: vec![],
    }]);
    let MirStmt::If { body, .. } = &body[0] else {
        panic!("expected the lowered `if`");
    };
    assert_eq!(body[1], print_mir(object_name("o")));
}

/// `if isinstance(o, int): if isinstance(o, bool): <inner>; print(o)` --
/// the lowered outer body's trailing `print`.
fn after_a_nested_bool_guard(inner: Vec<HirStmt>) -> MirStmt {
    let body = function_body(vec![HirStmt::If {
        test: isinstance("o", "int"),
        body: vec![
            HirStmt::If {
                test: isinstance("o", "bool"),
                body: inner,
                orelse: vec![],
            },
            print(name("o")),
        ],
        orelse: vec![],
    }]);
    let MirStmt::If { body, .. } = &body[0] else {
        panic!("expected the lowered `if`");
    };
    body[1].clone()
}

#[test]
fn a_nested_guard_joins_back_to_the_outer_narrowing() {
    // The inner guard's `bool` ends with its branch; the outer `int` holds
    // after it.
    assert_eq!(
        after_a_nested_bool_guard(vec![HirStmt::Return(None)]),
        print_mir(unboxed("o", Ty::Int))
    );
    // A nested negated guard's narrowed `else` joins back the same way.
    let body = function_body(vec![HirStmt::If {
        test: isinstance("o", "int"),
        body: vec![
            HirStmt::If {
                test: not(isinstance("o", "bool")),
                body: vec![],
                orelse: vec![HirStmt::Return(None)],
            },
            print(name("o")),
        ],
        orelse: vec![],
    }]);
    let MirStmt::If { body, .. } = &body[0] else {
        panic!("expected the lowered `if`");
    };
    assert_eq!(body[1], print_mir(unboxed("o", Ty::Int)));
    // A nested guard with no outer narrowing joins back to none.
    let body = function_body(vec![
        HirStmt::If {
            test: isinstance("o", "bool"),
            body: vec![HirStmt::Return(None)],
            orelse: vec![],
        },
        print(name("o")),
    ]);
    assert_eq!(body[1], print_mir(object_name("o")));
}

#[test]
fn a_nested_guard_whose_branch_rebinds_the_name_ends_the_narrowing() {
    assert_eq!(
        after_a_nested_bool_guard(vec![HirStmt::Assign {
            target: "o".to_string(),
            value: name("p"),
        }]),
        print_mir(object_name("o"))
    );
}
