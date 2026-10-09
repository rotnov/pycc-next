//! A list display in a value position of an object slot (#1421): an
//! attribute store into an object-declared slot, and an `and`/`or` or
//! conditional-expression operand under an object annotation.

use super::*;
use pycc_diag::Span;
use pycc_hir::{BoolOpKind, ImportBinding, ResolvedImports};

/// Lowers `source` as an `ext` module (`ext == true`, where `Any`, `object`
/// and a bare `list` are the object) or a `native` one, with `numpy` bound
/// as a foreign import.
fn lower(source: &str, ext: bool) -> HirModule {
    let module = pycc_parser::parse(source).expect("test source must parse");
    let mut resolved = ResolvedImports::default();
    resolved.set_ext_module(ext);
    let lowered = pycc_hir::lower_module(&module, &resolved, None).expect("test source must lower");
    let mut hir = pycc_hir::finalize(lowered.hir).expect("test source must finalize");
    hir.imports.push(ImportBinding::Foreign {
        local_name: "numpy".to_string(),
        module_path: "numpy".to_string(),
        from: None,
        site: pycc_hir::ForeignImportSite::Item(0),
        span: Span::new(0, 0),
    });
    hir
}

fn check(source: &str) -> Result<(), Vec<pycc_diag::Diagnostic>> {
    crate::check_all(&lower(source, true)).map(|_| ())
}

fn assert_accepted(source: &str) {
    if let Err(diagnostics) = check(source) {
        panic!("must type-check: {source}\n{diagnostics:?}");
    }
}

fn assert_refused(source: &str, code: &str, phrase: &str) {
    let diagnostics = check(source).expect_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?} for {source}");
    assert_eq!(diagnostics[0].code, code, "{diagnostics:?} for {source}");
    assert!(
        diagnostics[0].message.contains(phrase),
        "{diagnostics:?} for {source}"
    );
}

/// The value of the first `AttrSet` storing into `attr` in function `func`
/// after the pass.
fn stored_value(hir: &HirModule, func: &str, attr: &str) -> HirExpr {
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
            } if stored == attr => Some(value.clone()),
            _ => None,
        })
        .expect("attr store exists")
}

/// The lark `ParserState.__init__` shape (`lalr_parser_state.py` lines
/// 40-44), with the slot declarations D-258 lowers to the object.
const LARK: &str = "from typing import Any, List\n\n\n\
    class ParserState:\n    \
    start: Any\n    state_stack: List[Any]\n    value_stack: list\n\n    \
    def __init__(self, start: Any, state_stack=None, value_stack=None):\n        \
    self.start = start\n        \
    self.state_stack = state_stack or [self.start]\n        \
    self.value_stack = value_stack or []\n";

/// Lines 43-44: the `or`'s list-display operand, non-empty and empty, is
/// rewritten into a CPython list, and the module type-checks.
#[test]
fn the_lark_init_shape_builds_both_operands_as_cpython_lists() {
    let hir = resolve_empty_containers(&lower(LARK, true)).expect("an object slot store");
    let HirExpr::BoolOp { left, right, .. } =
        stored_value(&hir, "ParserState.__init__", "state_stack")
    else {
        panic!("the `or` survives the rewrite");
    };
    assert_eq!(*left, HirExpr::Name("state_stack".to_string()));
    assert!(matches!(*right, HirExpr::ObjectList(ref items) if items.len() == 1));
    let HirExpr::BoolOp { right, .. } = stored_value(&hir, "ParserState.__init__", "value_stack")
    else {
        panic!("the `or` survives the rewrite");
    };
    assert_eq!(*right, HirExpr::ObjectList(Vec::new()));
    assert_accepted(LARK);
}

/// Every value position of an object slot takes a display: a bare
/// display (empty or with mixed elements), either operand of `and`/`or`,
/// either branch of a conditional expression, nested; in `__init__` or a
/// later method; through a renamed receiver; into a slot inherited from a
/// base; and into a slot established from an object parameter rather than
/// declared.
#[test]
fn every_value_position_of_an_object_slot_takes_a_display() {
    let class = "from typing import Any\n\n\nclass A:\n    s: Any\n\n    \
        def __init__(self, n: int) -> None:\n        self.s = []\n\n    ";
    for method in [
        "def m(self, n: int) -> None:\n        self.s = [n, 'a', 2.5, True, numpy.pi]\n",
        "def m(self, n: int) -> None:\n        self.s = [n] and self.s\n",
        "def m(self, n: int) -> None:\n        self.s = [n] if n else []\n",
        "def m(self, n: int) -> None:\n        self.s = self.s or ([n] if n else [])\n",
        "def m(self, n: int) -> None:\n        if n:\n            self.s = [n]\n",
        "def m(this, n: int) -> None:\n        this.s = [n]\n",
    ] {
        assert_accepted(&format!("{class}{method}"));
    }
    assert_accepted(
        "class B:\n    s: list\n\n    def __init__(self, o: list) -> None:\n        \
         self.s = o\n\n\nclass C(B):\n    \
         def m(self, n: int) -> None:\n        self.s = [n]\n",
    );
    assert_accepted(
        "from typing import Any\n\n\nclass A:\n    \
         def __init__(self, o: Any) -> None:\n        self.s = o\n\n    \
         def m(self, n: int) -> None:\n        self.s = [n]\n",
    );
}

/// The same value positions under an object-annotated local: the
/// declaration states the slot, as an attribute's does.
#[test]
fn an_object_annotated_local_takes_a_display_operand() {
    for source in [
        "def f(v: object) -> object:\n    x: object = v or [1]\n    return x\n",
        "def f(v: object) -> object:\n    x: object = v or []\n    return x\n",
        "def f(n: int) -> object:\n    x: object = [n] if n else []\n    return x\n",
        "def f(n: int) -> object:\n    if n:\n        x: object = numpy.pi and [n]\n        return x\n    return numpy.pi\n",
    ] {
        assert_accepted(source);
    }
}

/// What stays outside: a dict display into an object slot keeps `T0003`;
/// an unpackable element is `I0404`; a store through a name that is not
/// the receiver is not an object slot here and keeps `T0034`; and a
/// display operand into an *unannotated* local whose other binding is the
/// object stays a native list, so the `or` keeps its `T0021`.
#[test]
fn the_shapes_outside_the_object_slot_keep_their_diagnostics() {
    let class = "from typing import Any\n\n\nclass A:\n    s: Any\n\n    \
        def __init__(self) -> None:\n        self.s = [1]\n\n    ";
    assert_refused(
        &format!("{class}def m(self) -> None:\n        self.s = {{}}\n"),
        "T0003",
        "an empty dict literal",
    );
    assert_refused(
        &format!("{class}def m(self) -> None:\n        self.s = self.s or [[1]]\n"),
        "I0404",
        "a `list[int]` element in a list display bound to a CPython object",
    );
    assert_refused(
        &format!("{class}def m(self, n: int) -> None:\n        self.s = [None] if n else []\n"),
        "I0404",
        "a `None` element in a list display bound to a CPython object",
    );
    assert_refused(
        &format!("{class}\n\ndef f(a: A, o: Any) -> None:\n    a.s = [o]\n"),
        "T0034",
        "list[object]",
    );
    assert_refused(
        "def f(v: object) -> int:\n    s = numpy.pi[1:]\n    s = v or [1]\n    return len(s)\n",
        "T0021",
        "`or` operand of type `list[int]`",
    );
}

/// A native container slot is untouched: a non-empty display into a
/// `list[int]` attribute stays a native list, and an empty one is still
/// typed from the slot.
#[test]
fn a_native_container_slot_keeps_its_native_display() {
    let source = "class A:\n    xs: list[int]\n\n    \
        def __init__(self, n: int) -> None:\n        self.xs = [n]\n\n    \
        def clear(self) -> None:\n        self.xs = []\n";
    for ext in [true, false] {
        let hir = lower(source, ext);
        let resolved = resolve_empty_containers(&hir).expect("an empty reset");
        assert!(matches!(
            stored_value(&resolved, "A.__init__", "xs"),
            HirExpr::ListLiteral(_)
        ));
        assert_eq!(
            stored_value(&resolved, "A.clear", "xs"),
            HirExpr::EmptyList(Ty::Int)
        );
        crate::check_all(&resolved).expect("a native list slot type-checks");
    }
}

/// The class-phase trigger fires on a display stored into an attribute
/// only when some class has an object slot, so a native module with
/// `self.xs = [1]` keeps the pass's no-clone fast path.
#[test]
fn the_display_trigger_needs_an_object_slot() {
    let native = lower(
        "class A:\n    xs: list[int]\n\n    def __init__(self) -> None:\n        \
         self.xs = [1]\n",
        false,
    );
    assert!(!attr_slot::needs_class_phase(&native));
    assert!(resolve_empty_containers(&native).is_none());
    let object = lower(
        "from typing import Any\n\n\nclass A:\n    s: Any\n\n    \
         def __init__(self) -> None:\n        self.s = [1]\n",
        true,
    );
    assert!(attr_slot::needs_class_phase(&object));
}

/// Only value positions are rewritten: the test of a conditional
/// expression, the operands of a truth-only `and`/`or`, a display's own
/// elements and any other expression are left alone, and the two walks
/// agree on every shape.
#[test]
fn only_value_positions_are_rewritten() {
    let list = |items: Vec<HirExpr>| HirExpr::ListLiteral(items);
    let bool_op = |left: HirExpr, right: HirExpr, truth_only: bool| HirExpr::BoolOp {
        op: BoolOpKind::Or,
        left: Box::new(left),
        right: Box::new(right),
        truth_only,
    };
    let if_exp = |test: HirExpr, body: HirExpr, orelse: HirExpr| HirExpr::IfExp {
        test: Box::new(test),
        body: Box::new(body),
        orelse: Box::new(orelse),
    };
    let name = || HirExpr::Name("v".to_string());

    let mut nested = list(vec![list(Vec::new())]);
    assert!(has_value_display(&nested));
    assert!(rewrite_value_displays(&mut nested));
    assert_eq!(nested, HirExpr::ObjectList(vec![list(Vec::new())]));

    let mut truth = bool_op(list(Vec::new()), list(Vec::new()), true);
    let before = truth.clone();
    assert!(!has_value_display(&truth));
    assert!(!rewrite_value_displays(&mut truth));
    assert_eq!(truth, before);

    let mut test_only = if_exp(list(Vec::new()), name(), name());
    let before = test_only.clone();
    assert!(!has_value_display(&test_only));
    assert!(!rewrite_value_displays(&mut test_only));
    assert_eq!(test_only, before);

    let mut orelse_only = if_exp(name(), name(), list(Vec::new()));
    assert!(has_value_display(&orelse_only));
    assert!(rewrite_value_displays(&mut orelse_only));
    assert_eq!(
        orelse_only,
        if_exp(name(), name(), HirExpr::ObjectList(Vec::new()))
    );

    let mut right_only = bool_op(name(), list(Vec::new()), false);
    assert!(has_value_display(&right_only));
    assert!(rewrite_value_displays(&mut right_only));
    assert_eq!(
        right_only,
        bool_op(name(), HirExpr::ObjectList(Vec::new()), false)
    );

    let mut plain = name();
    assert!(!has_value_display(&plain));
    assert!(!rewrite_value_displays(&mut plain));
}
