use super::*;

fn class(name: &str, mro: &[&str], methods: &[&str]) -> HirClassDef {
    HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: name.to_string(),
        bases: mro.iter().skip(1).map(|base| base.to_string()).collect(),
        mro: mro.iter().map(|entry| entry.to_string()).collect(),
        attrs: vec![("n".to_string(), Ty::Int)],
        methods: methods
            .iter()
            .map(|method| (method.to_string(), format!("{name}.{method}")))
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

fn scopes(signatures: &[(&str, Vec<Ty>, Ty)]) -> Vec<HashMap<String, Ty>> {
    vec![
        signatures
            .iter()
            .map(|(name, params, ret)| {
                (
                    format!("$callable:{name}"),
                    Ty::Callable(Box::new((params.clone(), ret.clone()))),
                )
            })
            .collect(),
    ]
}

fn calls(stmts: &[MirStmt], out: &mut Vec<String>) {
    for stmt in stmts {
        match stmt {
            MirStmt::If { test, body, orelse } => {
                if let MirExpr::Call { callee, .. } = test {
                    out.push(callee.clone());
                }
                calls(body, out);
                calls(orelse, out);
            }
            MirStmt::Return(Some(MirExpr::Call { callee, .. }))
            | MirStmt::ExprStmt(MirExpr::Call { callee, .. }) => out.push(callee.clone()),
            _ => {}
        }
    }
}

/// A method a subclass another module compiled overrides dispatches on the
/// receiver's record (its vtable at the method's index in the class's
/// method table), registered whether or not a call routed there; one
/// nothing overrides keeps its body; a receiver typed with a sibling class
/// runs the other module's entry.
#[test]
fn a_foreign_override_dispatches_on_the_record() {
    reset();
    let shape = class("Shape", &["Shape"], &["__init__", "kind", "size"]);
    let classes: HashMap<String, HirClassDef> =
        [("Shape".to_string(), shape.clone())].into_iter().collect();
    let me = Ty::Instance(Box::new("Shape".to_string()));
    let scopes = scopes(&[
        ("Shape.kind", vec![me.clone()], Ty::Str),
        ("Shape.size", vec![me.clone()], Ty::Int),
    ]);
    set_foreign_overrides(BTreeMap::from([(
        "Shape".to_string(),
        BTreeSet::from(["kind".to_string(), "missing".to_string()]),
    )]));
    assert_eq!(
        route(&shape, "size", "Shape.size".to_string(), &scopes, &classes),
        "Shape.size"
    );
    route_planned(&scopes, &classes);
    let items = items();
    let [MirItem::Function { name, body, .. }] = items.as_slice() else {
        panic!("one dispatcher: {items:?}")
    };
    assert_eq!(name, "Shape.kind.0dispatch");
    let mut seen = Vec::new();
    calls(body, &mut seen);
    assert_eq!(
        seen,
        [
            INSTANCE_FOREIGN,
            INSTANCE_VT_HAS,
            INSTANCE_VT_CALL,
            "Shape.kind"
        ]
    );
    let stub_name = format!("{}Shape", pycc_hir::SIBLING_CLASS_PREFIX);
    let stub = class(&stub_name, &[&stub_name], &["kind"]);
    assert_eq!(
        route(&stub, "kind", format!("{stub_name}.kind"), &scopes, &classes),
        format!("{stub_name}.kind.{ENTRY_SEGMENT}")
    );
    set_foreign_overrides(BTreeMap::new());
    reset();
}
