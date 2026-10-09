//! Comprehensions in expression position (#1254, D-250): the lowered
//! [`HirComprehension`] node, the synthesized loop-variable rename through
//! a nested comprehension, the walrus refusal, and the sub-expression views.

use super::*;

/// Lowers `source` and returns the single comprehension found as the first
/// argument of the first argument of the module's first `print(...)` call.
fn inner_comprehension(source: &str) -> HirComprehension {
    let module = pycc_parser_test_helper::parse(source);
    let hir = lower_checked(&module).unwrap();
    let HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call { args, .. })) = &hir.items[0] else {
        panic!("expected a call statement, got {:?}", hir.items[0]);
    };
    let HirExpr::Call { args, .. } = &args[0] else {
        panic!("expected a nested call, got {:?}", args[0]);
    };
    let HirExpr::Comprehension(comp) = &args[0] else {
        panic!("expected a comprehension, got {:?}", args[0]);
    };
    (**comp).clone()
}

#[test]
fn a_list_comprehension_argument_lowers_to_a_comprehension_node() {
    let comp = inner_comprehension("print(len([i for i in range(3) if i]))\n");
    assert_eq!(
        comp,
        HirComprehension {
            var: "0comp_17_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: Some(HirExpr::Name("0comp_17_i".to_string())),
            elt: CompElt::List(HirExpr::Name("0comp_17_i".to_string())),
        }
    );
}

#[test]
fn set_and_dict_comprehension_arguments_lower_to_their_element_kinds() {
    let set = inner_comprehension("print(len({k for k in d}))\n");
    assert_eq!(set.iter, CompIter::Name("d".to_string()));
    assert_eq!(set.elt, CompElt::Set(HirExpr::Name(set.var.clone())));
    let dict = inner_comprehension("print(len({k: 1 for k in d}))\n");
    assert_eq!(
        dict.elt,
        CompElt::Dict {
            key: HirExpr::Name(dict.var.clone()),
            value: HirExpr::IntLiteral(1),
        }
    );
}

/// An inner comprehension's iterable is evaluated in the outer one's scope,
/// so a name that is the outer loop variable resolves to it -- both a bare
/// iterable name and a `range` operand -- while the inner element keeps its
/// own variable.
#[test]
fn a_nested_comprehension_renames_the_outer_variable_in_its_own_iterable() {
    for (source, outer_var) in [
        (
            "print(len([len([y for y in x]) for x in range(3)]))\n",
            "0comp_35_x",
        ),
        (
            "print(len([len([x for x in range(x)]) for x in range(4)]))\n",
            "0comp_42_x",
        ),
        (
            "print(len([len([y for y in x.keys()]) for x in o]))\n",
            "0comp_42_x",
        ),
    ] {
        let outer = inner_comprehension(source);
        assert_eq!(outer.var, outer_var, "{source}");
        let CompElt::List(HirExpr::Call { args, .. }) = &outer.elt else {
            panic!(
                "{source}: expected a `len(...)` element, got {:?}",
                outer.elt
            );
        };
        let HirExpr::Comprehension(inner) = &args[0] else {
            panic!(
                "{source}: expected a nested comprehension, got {:?}",
                args[0]
            );
        };
        match &inner.iter {
            CompIter::Name(name) => assert_eq!(name, outer_var, "{source}"),
            CompIter::Range { stop, .. } => {
                assert_eq!(stop, &HirExpr::Name(outer_var.to_string()), "{source}")
            }
            CompIter::Iterable(iterable) => assert_eq!(
                **iterable,
                HirExpr::MethodCall {
                    base: Box::new(HirExpr::Name(outer_var.to_string())),
                    method: "keys".to_string(),
                    args: vec![],
                },
                "{source}"
            ),
        }
        assert_eq!(
            inner.elt,
            CompElt::List(HirExpr::Name(inner.var.clone())),
            "{source}"
        );
        assert_ne!(inner.var, outer.var, "{source}");
    }
}

/// A walrus anywhere inside a comprehension is refused, in the expression
/// form and in the statement form alike, spanning the whole comprehension.
#[test]
fn a_walrus_inside_a_comprehension_is_refused_in_both_forms() {
    for (source, start) in [
        ("print([(y := x) for x in range(3)])\n", 6),
        ("ys = [x for x in range(3) if (y := x)]\n", 5),
        ("ys = {x: (y := 1) for x in d}\n", 5),
        ("ys = {(y := x) for x in range(3)}\n", 5),
        ("print(len({(y := x) for x in range(3)}))\n", 10),
        ("print(len({\"a\": (y := x) for x in range(3)}))\n", 10),
    ] {
        let module = pycc_parser_test_helper::parse(source);
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001", "{source}");
        assert_eq!(
            diagnostic.message,
            "a walrus assignment (`:=`) inside a comprehension is not supported yet",
            "{source}"
        );
        assert_eq!(
            diagnostic.span.map(|span| span.start),
            Some(start),
            "{source}"
        );
    }
}

#[test]
fn the_sub_expression_views_list_every_operand_in_evaluation_order() {
    let name = |n: &str| HirExpr::Name(n.to_string());
    let mut dict = HirComprehension {
        var: "v".to_string(),
        iter: CompIter::Range {
            start: name("a"),
            stop: name("b"),
            step: name("c"),
        },
        cond: Some(name("t")),
        elt: CompElt::Dict {
            key: name("k"),
            value: name("w"),
        },
    };
    assert_eq!(
        dict.sub_exprs(),
        vec![
            &name("a"),
            &name("b"),
            &name("c"),
            &name("t"),
            &name("k"),
            &name("w")
        ]
    );
    assert_eq!(dict.body_exprs_mut().len(), 3);
    let mut set = HirComprehension {
        var: "v".to_string(),
        iter: CompIter::Name("s".to_string()),
        cond: None,
        elt: CompElt::Set(name("e")),
    };
    assert_eq!(set.sub_exprs(), vec![&name("e")]);
    for expr in set.body_exprs_mut() {
        *expr = name("f");
    }
    assert_eq!(set.elt, CompElt::Set(name("f")));
}

/// The outer loop variable is renamed inside a nested dict comprehension's
/// key and value as well as a list or set element.
#[test]
fn a_nested_dict_comprehension_renames_the_outer_variable_in_its_key_and_value() {
    let outer = inner_comprehension("print(len([len({s: x for s in d}) for x in range(3)]))\n");
    let CompElt::List(HirExpr::Call { args, .. }) = &outer.elt else {
        panic!("expected a `len(...)` element, got {:?}", outer.elt);
    };
    let HirExpr::Comprehension(inner) = &args[0] else {
        panic!("expected a nested comprehension, got {:?}", args[0]);
    };
    assert_eq!(
        inner.elt,
        CompElt::Dict {
            key: HirExpr::Name(inner.var.clone()),
            value: HirExpr::Name(outer.var.clone()),
        }
    );
    let set_outer = inner_comprehension("print(len([len({x for s in d}) for x in range(3)]))\n");
    let CompElt::List(HirExpr::Call { args, .. }) = &set_outer.elt else {
        panic!("expected a `len(...)` element, got {:?}", set_outer.elt);
    };
    let HirExpr::Comprehension(inner) = &args[0] else {
        panic!("expected a nested comprehension, got {:?}", args[0]);
    };
    assert_eq!(
        inner.elt,
        CompElt::Set(HirExpr::Name(set_outer.var.clone()))
    );
}

/// Part 2d of #1371: `HirExpr::ObjectList` is built only by `pycc_types`'
/// empty-container pre-pass, after lowering, so no source reaches the
/// rename with one. The arm still keeps the node and renames every
/// element, so a future caller cannot silently lose a loop variable.
#[test]
fn renaming_an_object_list_renames_every_element() {
    let renamed = rename_name_in_expr(
        HirExpr::ObjectList(vec![
            HirExpr::Name("i".to_string()),
            HirExpr::IntLiteral(1),
            HirExpr::Name("j".to_string()),
        ]),
        "i",
        "k",
    );
    assert_eq!(
        renamed,
        HirExpr::ObjectList(vec![
            HirExpr::Name("k".to_string()),
            HirExpr::IntLiteral(1),
            HirExpr::Name("j".to_string()),
        ])
    );
}
