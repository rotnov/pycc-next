use super::*;
use pycc_hir::HirClassDef;

fn class(name: &str, mro: &[&str], methods: &[&str]) -> HirClassDef {
    HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: name.to_string(),
        bases: Vec::new(),
        mro: mro.iter().map(|m| (*m).to_string()).collect(),
        attrs: Vec::new(),
        methods: methods
            .iter()
            .map(|m| ((*m).to_string(), format!("{name}.{m}")))
            .collect(),
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
    }
}

/// `A` defines `m`, `g` and `__len__`; `B(A)` overrides `m`.
fn classes() -> HashMap<String, HirClassDef> {
    [
        class("A", &["A"], &["m", "g", "__len__"]),
        class("B", &["B", "A"], &["m"]),
    ]
    .into_iter()
    .map(|d| (d.name.clone(), d))
    .collect()
}

fn instance(class: &str) -> Ty {
    Ty::Instance(Box::new(class.to_string()))
}

fn recv(class: &str) -> MirExpr {
    MirExpr::Name {
        name: "self".to_string(),
        ty: instance(class),
    }
}

fn call(callee: &str, receiver: MirExpr) -> MirExpr {
    MirExpr::Call {
        callee: callee.to_string(),
        args: vec![receiver],
        ty: Ty::Int,
    }
}

fn function(name: &str, body: Vec<MirStmt>) -> MirItem {
    MirItem::Function {
        name: name.to_string(),
        params: Vec::new(),
        return_ty: Ty::Int,
        body,
    }
}

fn module(items: Vec<MirItem>) -> MirModule {
    MirModule {
        items,
        class_defs: Vec::new(),
    }
}

#[test]
fn a_call_resolved_for_its_receiver_passes() {
    let body = vec![
        MirStmt::ExprStmt(call("B.m", recv("B"))),
        MirStmt::ExprStmt(call("A.g", recv("B"))),
        MirStmt::ExprStmt(call("A.m", recv("A"))),
    ];
    verify(&module(vec![function("f", body)]), &classes());
}

#[test]
#[should_panic(expected = "receiver-exact dispatch violated")]
fn a_call_that_skips_the_receivers_override_panics() {
    let body = vec![MirStmt::Return(Some(call("A.m", recv("B"))))];
    verify(&module(vec![function("f", body)]), &classes());
}

#[test]
fn an_existing_copy_is_the_only_accepted_callee() {
    // With `B.g` (a copy of `A.g` for `B`) lowered, `A.g` on a `B` is a
    // violation and `B.g` is accepted.
    let items = vec![
        function("B.g", Vec::new()),
        function("f", vec![MirStmt::ExprStmt(call("B.g", recv("B")))]),
    ];
    verify(&module(items), &classes());
    let bad = vec![
        function("B.g", Vec::new()),
        function("f", vec![MirStmt::ExprStmt(call("A.g", recv("B")))]),
    ];
    let result = std::panic::catch_unwind(|| verify(&module(bad), &classes()));
    assert!(result.is_err());
}

#[test]
fn a_super_call_from_the_anchor_is_accepted() {
    // Inside `B.m`, `super().m()` runs `A.m` on a `B` receiver.
    let items = vec![function(
        "B.m",
        vec![MirStmt::ExprStmt(call("A.m", recv("B")))],
    )];
    verify(&module(items), &classes());
}

#[test]
fn a_super_target_is_accepted_for_an_ordinary_call_too() {
    // The verifier cannot tell `self.m()` from `super().m()` in lowered
    // MIR (module doc, "Strength"): inside `B.m`, both `B.m` and `A.m` on a
    // `B` receiver are accepted. Outside a method body only the resolved
    // member is (`a_call_that_skips_the_receivers_override_panics`).
    let items = vec![function(
        "B.m",
        vec![
            MirStmt::ExprStmt(call("B.m", recv("B"))),
            MirStmt::ExprStmt(call("A.m", recv("B"))),
        ],
    )];
    verify(&module(items), &classes());
}

#[test]
fn a_super_target_setter_copy_is_accepted_by_its_kind_suffix() {
    // `A` and `B(A)` both define a property `p` with a setter; `C(B)`
    // inherits `B`'s. Inside `C.p.setter` (the copy of `B.p.setter`),
    // `super().p = v` runs `A.p.setter` for a `C`: the super-target copy
    // `C.p.0super_A.setter`, whose kind is its last segment.
    let with_p = |name: &str, mro: &[&str]| {
        let mut def = class(name, mro, &[]);
        def.properties.push(pycc_hir::PropertyDef {
            name: "p".to_string(),
            getter: format!("{name}.p"),
            setter: Some(format!("{name}.p.setter")),
        });
        def
    };
    let classes: HashMap<String, HirClassDef> = [
        with_p("A", &["A"]),
        with_p("B", &["B", "A"]),
        class("C", &["C", "B", "A"], &[]),
    ]
    .into_iter()
    .map(|d| (d.name.clone(), d))
    .collect();
    let items = vec![
        function("C.p.0super_A.setter", Vec::new()),
        function(
            "C.p.setter",
            vec![MirStmt::ExprStmt(call("C.p.0super_A.setter", recv("C")))],
        ),
    ];
    verify(&module(items), &classes);
}

#[test]
fn unrelated_callees_and_receivers_are_skipped() {
    let body = vec![
        // Not an instance receiver.
        MirStmt::ExprStmt(call("A.m", MirExpr::IntLiteral(1))),
        // An unknown receiver class.
        MirStmt::ExprStmt(call("A.m", recv("Z"))),
        // A top-level function callee, and a dotted non-class callee.
        MirStmt::ExprStmt(call("helper", recv("B"))),
        MirStmt::ExprStmt(call("mod.helper", recv("B"))),
        // A static method takes no receiver.
        MirStmt::ExprStmt(call("A.s.static", recv("B"))),
        // No arguments at all.
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "A.m".to_string(),
            args: Vec::new(),
            ty: Ty::Int,
        }),
    ];
    verify(
        &module(vec![
            function("f", body),
            MirItem::TopLevelStmt(MirStmt::NoOp),
        ]),
        &classes(),
    );
}

#[test]
fn a_protocol_specialization_is_checked_through_its_prefix() {
    let ok = vec![MirStmt::ExprStmt(call("0gen_B.m__P_B", recv("B")))];
    verify(&module(vec![function("f", ok)]), &classes());
    let dunder = vec![MirStmt::ExprStmt(call("0gen_A.__len____P_B", recv("B")))];
    verify(&module(vec![function("f", dunder)]), &classes());
    let bad = vec![MirStmt::ExprStmt(call("0gen_A.m__P_B", recv("B")))];
    let result = std::panic::catch_unwind(|| verify(&module(vec![function("f", bad)]), &classes()));
    assert!(result.is_err());
}

#[test]
fn split_generic_tail_prefers_a_bound_member() {
    let bound = |m: &str| m == "__len__" || m == "m";
    assert_eq!(
        split_generic_tail("__len____P_B", bound),
        ("__len__", "__P_B")
    );
    assert_eq!(split_generic_tail("m__P_B", bound), ("m", "__P_B"));
    assert_eq!(split_generic_tail("m", bound), ("m", ""));
    assert_eq!(split_generic_tail("zz__P_B", bound), ("zz__P_B", ""));
}

#[test]
fn a_specialized_copy_is_found_by_its_specialized_name() {
    // A protocol-parameter method exists only specialized: the copy of
    // `A.g` for `B` is `0gen_B.g__P_B`, and `B.g` itself is not an item.
    let items = vec![
        function("0gen_B.g__P_B", Vec::new()),
        function(
            "f",
            vec![MirStmt::ExprStmt(call("0gen_B.g__P_B", recv("B")))],
        ),
    ];
    verify(&module(items), &classes());
}

// -- #1344: set-comprehension element ops --

/// Element ops whose `__hash__` is `hash` and whose `__eq__` is `eq`.
fn set_ops(hash: &str, eq: &str) -> Box<SetElementOps> {
    Box::new(SetElementOps {
        hash: SetHashOp::Method {
            callee: hash.to_string(),
            ret: Ty::Int,
        },
        eq: SetEqOp::Method {
            callee: eq.to_string(),
        },
    })
}

/// `t = {self for _ in s}` over a `B` receiver, as a statement.
fn set_comp_stmt(ops: Box<SetElementOps>) -> MirStmt {
    MirStmt::SetCompAssign {
        ops: Some(ops),
        target: "t".to_string(),
        var: "v".to_string(),
        var_ty: Ty::Int,
        source: CompSource::Set("s".to_string()),
        cond: None,
        elt: Box::new(recv("B")),
    }
}

/// `{self for _ in s}` over a `B` receiver, as an expression.
fn set_comp_expr(ops: Box<SetElementOps>) -> MirStmt {
    MirStmt::ExprStmt(MirExpr::Comprehension(Box::new(crate::MirComprehension {
        var: "v".to_string(),
        var_ty: Ty::Int,
        source: CompSource::Set("s".to_string()),
        cond: None,
        elt: MirCompElt::Set(recv("B"), Some(ops)),
    })))
}

#[test]
fn set_comprehension_ops_resolved_for_the_element_pass() {
    let body = vec![
        set_comp_stmt(set_ops("B.m", "A.g")),
        set_comp_expr(set_ops("B.m", "A.g")),
    ];
    verify(&module(vec![function("f", body)]), &classes());
}

#[test]
#[should_panic(expected = "receiver-exact dispatch violated")]
fn a_set_comprehension_statement_hash_skipping_the_override_panics() {
    let body = vec![set_comp_stmt(set_ops("A.m", "A.g"))];
    verify(&module(vec![function("f", body)]), &classes());
}

#[test]
#[should_panic(expected = "receiver-exact dispatch violated")]
fn a_set_comprehension_expression_eq_skipping_the_override_panics() {
    let body = vec![set_comp_expr(set_ops("B.m", "A.m"))];
    verify(&module(vec![function("f", body)]), &classes());
}
