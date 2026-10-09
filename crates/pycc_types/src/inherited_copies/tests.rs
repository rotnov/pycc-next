use super::*;

use pycc_diag::Diagnostic;

fn lower(source: &str) -> HirModule {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    pycc_hir::lower_checked(&module).expect("test fixture must lower")
}

fn item_params<'m>(module: &'m HirModule, wanted: &str) -> Option<&'m [(String, Ty)]> {
    module.items.iter().find_map(|item| match item {
        HirItem::Function { name, params, .. } if name == wanted => Some(params.as_slice()),
        _ => None,
    })
}

const TEMPLATE: &str = "class A:\n    def m(self) -> int:\n        return 1\n    \
    def g(self) -> int:\n        return self.m()\nclass B(A):\n    def m(self) -> int:\n        \
    return 2\nprint(B().g())\n";

#[test]
fn a_body_that_cannot_differ_is_not_copied() {
    let hir = lower(
        "class A:\n    def m(self) -> int:\n        return 1\n    def g(self) -> int:\n        \
         return self.m()\nclass B(A):\n    pass\nprint(B().g())\n",
    );
    assert!(add_inherited_copies(&hir).is_none());
}

#[test]
fn a_template_method_is_copied_for_the_overriding_subclass() {
    let hir = lower(TEMPLATE);
    let copied = add_inherited_copies(&hir).expect("B needs a copy of A.g");
    let params = item_params(&copied.module, "B.g").expect("the copy is spelled like B's own");
    assert_eq!(params[0].1, Ty::Instance(Box::new("B".to_string())));
    // The origin keeps its own receiver type.
    let origin = item_params(&copied.module, "A.g").expect("the origin stays");
    assert_eq!(origin[0].1, Ty::Instance(Box::new("A".to_string())));
    assert_eq!(copied.module.items.len(), hir.items.len() + 1);
}

#[test]
fn a_copy_diagnostic_is_rekeyed_noted_and_deduplicated() {
    let hir = lower(TEMPLATE);
    let copied = add_inherited_copies(&hir).expect("B needs a copy of A.g");
    let copy_index = hir.items.len();
    let origin_index = hir
        .items
        .iter()
        .position(|item| matches!(item, HirItem::Function { name, .. } if name == "A.g"))
        .expect("A.g is an item");
    let span = Span::new(3, 4);
    let mut with_help = Diagnostic::error("T0021", "copy only", Span::new(9, 10));
    with_help.help = Some("existing".to_string());
    let rekeyed = copied.rekey(vec![
        (
            DiagnosticKey::Function(origin_index),
            Diagnostic::error("T0021", "same", span),
        ),
        (
            DiagnosticKey::Function(origin_index),
            Diagnostic::error("T0021", "same", span),
        ),
        (
            DiagnosticKey::Function(copy_index),
            Diagnostic::error("T0021", "same", span),
        ),
        (DiagnosticKey::Function(copy_index), with_help),
        (
            DiagnosticKey::TopLevel(0),
            Diagnostic::error("C0001", "top", span),
        ),
    ]);
    // Both identical origin diagnostics stay, the repeated copy one is
    // dropped, the copy-only one moves to the origin with the note appended.
    assert_eq!(rekeyed.len(), 4, "{rekeyed:?}");
    assert_eq!(rekeyed[2].0, DiagnosticKey::Function(origin_index));
    assert_eq!(
        rekeyed[2].1.help.as_deref(),
        Some("existing\nwhile compiling `A.g` inherited by subclass `B`")
    );
    assert_eq!(rekeyed[3].0, DiagnosticKey::TopLevel(0));
}

#[test]
fn a_copy_without_help_gets_the_note_alone() {
    let hir = lower(TEMPLATE);
    let copied = add_inherited_copies(&hir).expect("B needs a copy of A.g");
    let rekeyed = copied.rekey(vec![(
        DiagnosticKey::Function(hir.items.len()),
        Diagnostic::error("T0022", "x", Span::new(1, 2)),
    )]);
    assert_eq!(
        rekeyed[0].1.help.as_deref(),
        Some("while compiling `A.g` inherited by subclass `B`")
    );
}

#[test]
fn a_dataclass_eq_copy_retypes_other_and_repr_prints_the_subclass() {
    let hir = lower(
        "from dataclasses import dataclass\n@dataclass\nclass A:\n    x: int\n\
         class B(A):\n    pass\nb = B(1)\n",
    );
    let copied = add_inherited_copies(&hir).expect("B inherits A's synthesized members");
    // The synthesized `__repr__` observes the exact class, so `B` gets a
    // copy whose body is regenerated with its own name.
    assert!(item_params(&copied.module, "B.__repr__").is_some());
    assert!(item_params(&copied.module, "B.__eq__").is_some());
    let copy = PlannedCopy {
        receiver: "B".to_string(),
        origin_class: "A".to_string(),
        origin_name: "A.__eq__".to_string(),
    };
    let a = Ty::Instance(Box::new("A".to_string()));
    let b = Ty::Instance(Box::new("B".to_string()));
    let params = vec![
        ("self".to_string(), a.clone()),
        ("other".to_string(), a.clone()),
        ("n".to_string(), Ty::Int),
    ];
    let retyped = retype_receiver(&params, &copy);
    assert_eq!(retyped[0].1, b);
    assert_eq!(retyped[1].1, b);
    assert_eq!(retyped[2].1, Ty::Int);
    let plain = PlannedCopy {
        origin_name: "A.same".to_string(),
        ..copy
    };
    assert_eq!(retype_receiver(&params, &plain)[1].1, a);
}

#[test]
fn a_seeded_builtin_exception_is_never_a_receiver() {
    let hir = lower(
        "class E(ValueError):\n    def m(self) -> int:\n        return 1\n\
         try:\n    raise E(\"x\")\nexcept E:\n    print(1)\n",
    );
    assert!(plan_copies(&hir).is_empty());
}
