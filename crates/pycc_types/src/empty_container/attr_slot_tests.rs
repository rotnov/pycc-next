//! The #1265 class phase: slot resolution, the value rewrite, and the gate.

use super::*;

/// Lowers source to HIR without type-checking it.
fn lower(source: &str) -> HirModule {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    pycc_hir::lower_checked(&module).expect("test fixture must lower")
}

/// Runs the whole empty-container pass and returns the module it produced.
fn resolve(source: &str) -> HirModule {
    let hir = lower(source);
    crate::empty_container::resolve_empty_containers(&hir)
        .expect("a provisional slot always triggers the class phase")
}

/// The slot `class.attr` carries after the pass.
fn slot(hir: &HirModule, class: &str, attr: &str) -> Ty {
    let (_, def) = hir
        .class_defs
        .iter()
        .find(|(name, _)| name == class)
        .expect("class exists");
    def.attrs
        .iter()
        .find(|(name, _)| name == attr)
        .expect("attr exists")
        .1
        .clone()
}

fn list_of(element: Ty) -> Ty {
    Ty::List(Box::new(element))
}

fn provisional() -> Ty {
    list_of(Ty::Infer)
}

/// The value of the first `AttrSet` storing into `attr` in function `func`.
fn stored_value<'a>(hir: &'a HirModule, func: &str, attr: &str) -> &'a HirExpr {
    let body = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Function { name, body, .. } if name == func => Some(body),
            _ => None,
        })
        .expect("function exists");
    body.iter()
        .find_map(|stmt| match stmt {
            HirStmt::AttrSet {
                attr: stored,
                value,
                ..
            } if stored == attr => Some(value),
            _ => None,
        })
        .expect("attr store exists")
}

const INIT: &str = "class B:\n    def __init__(self) -> None:\n        self.xs = []\n";

#[test]
fn a_producer_in_another_method_resolves_the_slot_and_types_the_initialiser() {
    let hir = resolve(&format!(
        "{INIT}    def add(self, v: int) -> None:\n        self.xs.append(v)\n"
    ));
    assert_eq!(slot(&hir, "B", "xs"), list_of(Ty::Int));
    assert!(matches!(
        stored_value(&hir, "B.__init__", "xs"),
        HirExpr::EmptyList(Ty::Int)
    ));
    assert!(reject_unresolved_attr_slots(&hir).is_ok());
}

#[test]
fn a_producer_in_init_itself_resolves_the_slot() {
    let hir = resolve(
        "class B:\n    def __init__(self) -> None:\n        self.xs = []\n        \
         self.xs.append('a')\n",
    );
    assert_eq!(slot(&hir, "B", "xs"), list_of(Ty::Str));
}

#[test]
fn a_nested_producer_in_a_block_body_resolves_the_slot() {
    let hir = resolve(&format!(
        "{INIT}    def add(self, v: float) -> None:\n        if v > 0.0:\n            \
         self.xs.append(v)\n"
    ));
    assert_eq!(slot(&hir, "B", "xs"), list_of(Ty::Float));
}

#[test]
fn a_this_spelled_receiver_is_recognised_in_both_the_store_and_the_producer() {
    let hir = resolve(
        "class B:\n    def __init__(this) -> None:\n        this.xs = []\n    \
         def add(this, v: int) -> None:\n        this.xs.append(v)\n    \
         def reset(this) -> None:\n        this.xs = []\n",
    );
    assert_eq!(slot(&hir, "B", "xs"), list_of(Ty::Int));
    assert!(matches!(
        stored_value(&hir, "B.reset", "xs"),
        HirExpr::EmptyList(Ty::Int)
    ));
}

#[test]
fn a_reset_in_another_method_is_typed_from_the_resolved_slot() {
    let hir = resolve(&format!(
        "{INIT}    def add(self, v: int) -> None:\n        self.xs.append(v)\n    \
         def clear(self) -> None:\n        self.xs = []\n"
    ));
    assert!(matches!(
        stored_value(&hir, "B.clear", "xs"),
        HirExpr::EmptyList(Ty::Int)
    ));
}

#[test]
fn a_reset_of_an_annotated_slot_is_typed_even_without_a_provisional_slot() {
    let hir = resolve(
        "class B:\n    def __init__(self) -> None:\n        self.d: dict[str, int] = {}\n    \
         def clear(self) -> None:\n        self.d = {}\n",
    );
    assert!(matches!(
        stored_value(&hir, "B.clear", "d"),
        HirExpr::EmptyDict(pair) if **pair == (Ty::Str, Ty::Int)
    ));
}

#[test]
fn a_shape_mismatched_reset_is_left_for_the_checker() {
    let hir = resolve(&format!(
        "{INIT}    def add(self, v: int) -> None:\n        self.xs.append(v)\n    \
         def clear(self) -> None:\n        self.xs = {{}}\n"
    ));
    assert!(matches!(
        stored_value(&hir, "B.clear", "xs"),
        HirExpr::DictLiteral(_)
    ));
}

#[test]
fn a_store_on_another_receiver_is_not_rewritten() {
    let hir = resolve(&format!(
        "{INIT}    def add(self, v: int) -> None:\n        self.xs.append(v)\n    \
         def copy_into(self, other: B) -> None:\n        other.xs = []\n"
    ));
    assert!(matches!(
        stored_value(&hir, "B.copy_into", "xs"),
        HirExpr::ListLiteral(_)
    ));
}

#[test]
fn an_inherited_concrete_slot_resolves_a_subclass_redeclaration() {
    let hir = resolve(
        "class A:\n    def __init__(self) -> None:\n        self.xs: list[int] = []\n\
         class B(A):\n    def __init__(self) -> None:\n        self.xs = []\n",
    );
    assert_eq!(slot(&hir, "B", "xs"), list_of(Ty::Int));
    assert!(matches!(
        stored_value(&hir, "B.__init__", "xs"),
        HirExpr::EmptyList(Ty::Int)
    ));
}

#[test]
fn a_base_resolved_by_its_own_producer_resolves_its_subclass() {
    let hir = resolve(
        "class A:\n    def __init__(self) -> None:\n        self.xs = []\n    \
         def add(self, v: bool) -> None:\n        self.xs.append(v)\n\
         class B(A):\n    def __init__(self) -> None:\n        self.xs = []\n",
    );
    assert_eq!(slot(&hir, "A", "xs"), list_of(Ty::Bool));
    assert_eq!(slot(&hir, "B", "xs"), list_of(Ty::Bool));
}

#[test]
fn an_inherited_slot_of_another_shape_is_not_a_source() {
    let hir = resolve(
        "class A:\n    def __init__(self) -> None:\n        self.xs: dict[str, int] = {}\n\
         class B(A):\n    def __init__(self) -> None:\n        self.xs = []\n",
    );
    assert_eq!(slot(&hir, "B", "xs"), provisional());
}

#[test]
fn a_producer_reading_an_earlier_class_slot_resolves() {
    let hir = resolve(
        "class A:\n    def __init__(self) -> None:\n        self.ys = []\n    \
         def add(self, v: int) -> None:\n        self.ys.append(v)\n\
         class B(A):\n    def __init__(self) -> None:\n        self.xs = []\n    \
         def fill(self) -> None:\n        self.xs.append(self.ys[0])\n",
    );
    assert_eq!(slot(&hir, "B", "xs"), list_of(Ty::Int));
}

#[test]
fn a_producer_reading_a_same_class_provisional_slot_stays_a_miss() {
    let hir = resolve(
        "class B:\n    def __init__(self) -> None:\n        self.ys = []\n        \
         self.xs = []\n    def fill(self) -> None:\n        self.xs.append(self.ys[0])\n        \
         self.ys.append(1)\n",
    );
    assert_eq!(slot(&hir, "B", "ys"), list_of(Ty::Int));
    assert_eq!(slot(&hir, "B", "xs"), provisional());
}

#[test]
fn a_non_inferring_first_producer_ends_the_whole_class_scan() {
    let hir = resolve(&format!(
        "{INIT}    def a(self) -> None:\n        self.xs.append(undefined_name)\n    \
         def b(self, v: int) -> None:\n        self.xs.append(v)\n"
    ));
    assert_eq!(slot(&hir, "B", "xs"), provisional());
}

#[test]
fn a_placeholder_element_type_does_not_resolve_the_slot() {
    let hir = resolve(&format!(
        "{INIT}    def _add(self, v) -> None:\n        self.xs.append(v)\n"
    ));
    assert_eq!(slot(&hir, "B", "xs"), provisional());
}

#[test]
fn an_optional_element_type_does_not_resolve_the_slot() {
    let hir = resolve(&format!(
        "{INIT}    def add(self, v: int | None) -> None:\n        self.xs.append(v)\n"
    ));
    assert_eq!(slot(&hir, "B", "xs"), provisional());
}

#[test]
fn a_property_getter_and_setter_are_own_methods() {
    let hir = resolve(&format!(
        "{INIT}    @property\n    def last(self) -> int:\n        return 0\n    \
         @last.setter\n    def last(self, v: int) -> None:\n        self.xs.append(v)\n"
    ));
    assert_eq!(slot(&hir, "B", "xs"), list_of(Ty::Int));
}

#[test]
fn a_static_method_whose_first_parameter_is_named_self_is_not_a_source() {
    let hir = resolve(&format!(
        "{INIT}    @staticmethod\n    def add(self: B, v: int) -> None:\n        \
         self.xs.append(v)\n"
    ));
    assert_eq!(slot(&hir, "B", "xs"), provisional());
}

#[test]
fn another_class_producer_is_not_a_source() {
    let hir = resolve(&format!(
        "{INIT}class C:\n    def __init__(self) -> None:\n        self.xs = []\n    \
         def add(self, v: int) -> None:\n        self.xs.append(v)\n"
    ));
    assert_eq!(slot(&hir, "C", "xs"), list_of(Ty::Int));
    assert_eq!(slot(&hir, "B", "xs"), provisional());
}

#[test]
fn a_value_position_append_is_not_a_producer() {
    let hir = resolve(&format!(
        "{INIT}    def add(self, v: int) -> None:\n        r = self.xs.append(v)\n"
    ));
    assert_eq!(slot(&hir, "B", "xs"), provisional());
}

#[test]
fn the_gate_reports_the_first_provisional_slot_keyed_to_its_init() {
    let hir = resolve(INIT);
    let errors = reject_unresolved_attr_slots(&hir).expect_err("provisional slot is refused");
    assert_eq!(errors.len(), 1);
    let (key, diagnostic) = &errors[0];
    let init = hir
        .items
        .iter()
        .position(|item| matches!(item, HirItem::Function { name, .. } if name == "B.__init__"))
        .expect("init item");
    assert_eq!(*key, DiagnosticKey::Function(init));
    assert_eq!(diagnostic.code, "T0003");
    assert_eq!(
        diagnostic.message,
        "an empty list literal has no inferable element type for `self.xs` in class `B`"
    );
    assert_eq!(
        diagnostic.label.as_deref(),
        Some(diagnostic.message.as_str())
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some(
            "annotate the attribute (`self.xs: list[int] = []`) or append a value to it in one \
             of `B`'s own methods (`self.xs.append(...)`)"
        )
    );
}

#[test]
fn the_gate_falls_back_to_the_module_key_without_an_init_item() {
    let mut hir = resolve(INIT);
    hir.items
        .retain(|item| !matches!(item, HirItem::Function { name, .. } if name == "B.__init__"));
    let errors = reject_unresolved_attr_slots(&hir).expect_err("provisional slot is refused");
    assert_eq!(errors[0].0, DiagnosticKey::Module);
}

#[test]
fn the_class_phase_trigger_covers_both_of_its_sources() {
    assert!(needs_class_phase(&lower(INIT)));
    assert!(needs_class_phase(&lower(
        "class B:\n    def __init__(self) -> None:\n        self.xs: list[int] = []\n    \
         def clear(self) -> None:\n        self.xs = []\n"
    )));
    assert!(!needs_class_phase(&lower(
        "class B:\n    def __init__(self) -> None:\n        self.n = 0\n"
    )));
}

#[test]
fn receiver_spellings_adds_only_an_exact_alias_of_self() {
    let alias = HirStmt::Assign {
        target: "this".to_string(),
        value: HirExpr::Name("self".to_string()),
    };
    assert_eq!(receiver_spellings(&[alias]), vec!["self", "this"]);
    let other = HirStmt::Assign {
        target: "this".to_string(),
        value: HirExpr::Name("other".to_string()),
    };
    assert_eq!(receiver_spellings(&[other]), vec!["self"]);
    assert_eq!(receiver_spellings(&[]), vec!["self"]);
}

#[test]
fn container_ty_maps_both_shapes() {
    assert_eq!(container_ty(Resolution::List(Ty::Int)), list_of(Ty::Int));
    assert_eq!(
        container_ty(Resolution::Dict(Ty::Str, Ty::Int)),
        Ty::Dict(Box::new((Ty::Str, Ty::Int)))
    );
}

#[test]
fn a_scalar_slot_beside_a_provisional_one_is_left_as_declared() {
    let hir = resolve(
        "class B:\n    def __init__(self) -> None:\n        self.n = 0\n        \
         self.xs = []\n    def add(self, v: int) -> None:\n        self.xs.append(v)\n",
    );
    assert_eq!(slot(&hir, "B", "n"), Ty::Int);
    assert_eq!(slot(&hir, "B", "xs"), list_of(Ty::Int));
}

#[test]
fn a_local_name_append_is_not_an_attribute_producer_nor_the_reverse() {
    let hir = resolve(&format!(
        "{INIT}    def add(self, v: int) -> None:\n        ys = []\n        \
         ys.append(v)\n        self.xs.append(v)\n        zs = []\n        \
         self.xs.append(v)\n        zs.append(v)\n"
    ));
    assert_eq!(slot(&hir, "B", "xs"), list_of(Ty::Int));
}

/// The value of the first plain `Assign` to local `target` in function `func`.
fn assigned_value<'a>(hir: &'a HirModule, func: &str, target: &str) -> &'a HirExpr {
    hir.items
        .iter()
        .find_map(|item| match item {
            HirItem::Function { name, body, .. } if name == func => Some(body),
            _ => None,
        })
        .expect("function exists")
        .iter()
        .find_map(|stmt| match stmt {
            HirStmt::Assign {
                target: bound,
                value,
            } if bound == target => Some(value),
            _ => None,
        })
        .expect("local assignment exists")
}

#[test]
fn a_producer_nested_in_a_for_and_a_try_resolves_the_slot() {
    let in_for = resolve(&format!(
        "{INIT}    def add(self, n: int) -> None:\n        for i in range(n):\n            \
         self.xs.append(i)\n"
    ));
    assert_eq!(slot(&in_for, "B", "xs"), list_of(Ty::Int));
    let in_try = resolve(&format!(
        "{INIT}    def add(self, v: str) -> None:\n        try:\n            \
         self.xs.append(v)\n        except ValueError:\n            pass\n"
    ));
    assert_eq!(slot(&in_try, "B", "xs"), list_of(Ty::Str));
}

#[test]
fn the_receiver_dispatched_form_is_a_producer() {
    // Another class defining `append` turns `self.xs.append(v)` into #1188's
    // receiver-dispatched call; its admitted container reading still counts.
    let source = format!(
        "class Sink:\n    def append(self, v: int) -> None:\n        pass\n\
         {INIT}    def add(self, v: int) -> None:\n        self.xs.append(v)\n"
    );
    let dispatched = lower(&source).items.iter().any(|item| {
        matches!(item, HirItem::Function { name, body, .. } if name == "B.add"
            && matches!(body.first(), Some(HirStmt::ExprStmt(HirExpr::ReceiverDispatchedCall { .. }))))
    });
    assert!(dispatched, "the fixture must lower to the dispatched form");
    let hir = resolve(&source);
    assert_eq!(slot(&hir, "B", "xs"), list_of(Ty::Int));
    assert!(matches!(
        stored_value(&hir, "B.__init__", "xs"),
        HirExpr::EmptyList(Ty::Int)
    ));
}

#[test]
fn a_later_method_local_reading_the_slot_resolves_after_the_class_phase() {
    let hir = resolve(&format!(
        "{INIT}    def add(self, v: int) -> None:\n        self.xs.append(v)\n    \
         def firsts(self) -> None:\n        ys = []\n        ys.append(self.xs[0])\n"
    ));
    assert!(matches!(
        assigned_value(&hir, "B.firsts", "ys"),
        HirExpr::EmptyList(Ty::Int)
    ));
}

#[test]
fn a_producer_only_in_a_subclass_method_leaves_the_base_slot_refused() {
    let hir = resolve(&format!(
        "{INIT}class D(B):\n    def add(self, v: int) -> None:\n        self.xs.append(v)\n"
    ));
    assert_eq!(slot(&hir, "B", "xs"), provisional());
    let errors = reject_unresolved_attr_slots(&hir).expect_err("provisional slot is refused");
    assert!(errors[0].1.message.contains("`self.xs` in class `B`"));
}

#[test]
fn a_producer_only_in_a_free_function_leaves_the_slot_refused() {
    let hir = resolve(&format!(
        "{INIT}def fill(b: B) -> None:\n    b.xs.append(1)\n"
    ));
    assert_eq!(slot(&hir, "B", "xs"), provisional());
    assert!(reject_unresolved_attr_slots(&hir).is_err());
}

#[test]
fn the_gate_names_a_renamed_receiver_in_the_message_and_the_help() {
    let hir = resolve("class B:\n    def __init__(this) -> None:\n        this.xs = []\n");
    let errors = reject_unresolved_attr_slots(&hir).expect_err("provisional slot is refused");
    let diagnostic = &errors[0].1;
    assert_eq!(
        diagnostic.message,
        "an empty list literal has no inferable element type for `this.xs` in class `B`"
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some(
            "annotate the attribute (`this.xs: list[int] = []`) or append a value to it in one \
             of `B`'s own methods (`this.xs.append(...)`)"
        )
    );
}

#[test]
fn a_whole_attribute_rebinding_is_not_a_source() {
    let hir = resolve(&format!(
        "{INIT}    def load(self, other: list[int]) -> None:\n        self.xs = other\n"
    ));
    assert_eq!(slot(&hir, "B", "xs"), provisional());
    assert!(reject_unresolved_attr_slots(&hir).is_err());
}

#[test]
fn the_gate_keys_an_init_that_is_not_the_first_item() {
    let hir = resolve(&format!("def first() -> None:\n    pass\n{INIT}"));
    let init = hir
        .items
        .iter()
        .position(|item| matches!(item, HirItem::Function { name, .. } if name == "B.__init__"))
        .expect("init item");
    assert_ne!(init, 0);
    let errors = reject_unresolved_attr_slots(&hir).expect_err("provisional slot is refused");
    assert_eq!(errors[0].0, DiagnosticKey::Function(init));
}

#[test]
fn the_gate_names_the_establishing_receiver_not_a_local_alias() {
    let hir = resolve(
        "class B:\n    def __init__(self) -> None:\n        me = self\n        \
         self.xs = []\n",
    );
    let errors = reject_unresolved_attr_slots(&hir).expect_err("provisional slot is refused");
    let diagnostic = &errors[0].1;
    assert!(diagnostic.message.contains("`self.xs` in class `B`"));
    let help = diagnostic.help.as_deref().expect("help");
    assert!(help.contains("`self.xs: list[int] = []`"), "{help}");
    assert!(!diagnostic.message.contains("me.xs") && !help.contains("me.xs"));
}

#[test]
fn establishing_receiver_skips_other_statements() {
    let store = |attr: &str| HirStmt::AttrSet {
        base: HirExpr::Name("self".to_string()),
        attr: attr.to_string(),
        value: HirExpr::IntLiteral(0),
    };
    let body = [store("n"), store("xs")];
    assert_eq!(establishing_receiver(&body, "xs"), Some("self"));
    assert_eq!(establishing_receiver(&body, "ys"), None);
}

#[test]
fn the_gate_never_names_a_foreign_base_storing_the_same_attribute() {
    let hir = resolve(
        "class A:\n    def __init__(self) -> None:\n        self.xs = 0\n\
         class B:\n    def __init__(self) -> None:\n        a = A()\n        a.xs = 1\n        \
         self.xs = []\n",
    );
    let errors = reject_unresolved_attr_slots(&hir).expect_err("provisional slot is refused");
    let diagnostic = &errors[0].1;
    let help = diagnostic.help.as_deref().expect("help");
    assert!(diagnostic.message.contains("`self.xs` in class `B`"));
    assert!(help.contains("`self.xs: list[int] = []`"), "{help}");
    assert!(!diagnostic.message.contains("a.xs") && !help.contains("`a.xs"));
}

#[test]
fn the_gate_names_a_renamed_receiver_past_a_foreign_base_store() {
    let hir = resolve(
        "class A:\n    def __init__(this) -> None:\n        this.xs = 0\n\
         class B:\n    def __init__(this) -> None:\n        a = A()\n        a.xs = 1\n        \
         this.xs = []\n",
    );
    let errors = reject_unresolved_attr_slots(&hir).expect_err("provisional slot is refused");
    let diagnostic = &errors[0].1;
    assert!(diagnostic.message.contains("`this.xs` in class `B`"));
    assert!(!diagnostic.message.contains("a.xs"));
}

#[test]
fn establishing_receiver_returns_only_a_receiver_spelling() {
    let store = |base: &str| HirStmt::AttrSet {
        base: HirExpr::Name(base.to_string()),
        attr: "xs".to_string(),
        value: HirExpr::IntLiteral(0),
    };
    let alias = HirStmt::Assign {
        target: "this".to_string(),
        value: HirExpr::Name("self".to_string()),
    };
    assert_eq!(
        establishing_receiver(&[alias.clone(), store("a"), store("this")], "xs"),
        Some("this")
    );
    assert_eq!(
        establishing_receiver(&[store("a"), store("self")], "xs"),
        Some("self")
    );
    assert_eq!(establishing_receiver(&[store("a")], "xs"), None);
}
