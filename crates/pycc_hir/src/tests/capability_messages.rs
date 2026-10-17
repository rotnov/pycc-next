//! Capability-error messages naming the refused construct.

use super::*;

// Issue #890: every rewritten `C0001` message names the rejected construct
// in Python terms. One exact-wording assertion per rewritten site (and per
// special-cased branch inside a site), so each is a covered region and a
// wording regression is caught here before the byte-exact fixtures in
// `tests/diagnostics` are.

#[test]
fn an_attribute_annotation_names_its_kind() {
    assert_capability_error_message(
        "import typing\ndef f(x: typing.Any) -> int:\n    return 1\n",
        "only a bare name type annotation is supported so far, got an attribute expression (`obj.attr`)",
    );
}

#[test]
fn a_bytes_annotation_names_its_kind() {
    // A *string* annotation no longer reaches the catch-all: it resolves
    // like its unquoted spelling (Part 1 of #889). A bytes literal still
    // does.
    assert_capability_error_message(
        "def f(x: b\"int\") -> int:\n    return x\n",
        "only a bare name type annotation is supported so far, got a bytes literal",
    );
}

#[test]
fn a_tuple_target_inside_a_chained_assignment_keeps_the_unpacking_refusals() {
    // #1213: each piece of a chain is lowered on its own, so a tuple piece
    // goes through the unpacking lowering (Part 1 of #891) and keeps its
    // refusals, spanned on the refused element itself.
    assert_capability_error(
        "a = b, c.d = t\n",
        "only bare-name targets in a tuple-unpacking assignment are supported so far, \
         got an attribute expression (`obj.attr`)",
        Span::new(7, 10),
    );
}

#[test]
fn a_nested_unpacking_target_names_its_kind() {
    assert_capability_error_message(
        "a, (b, c) = 1, (2, 3)\n",
        "a nested target (`a, (b, c) = ...`) in a tuple-unpacking assignment is not supported yet",
    );
}

#[test]
fn a_subscript_unpacking_target_names_its_kind() {
    assert_capability_error_message(
        "[a, d[0]] = [1, 2]\n",
        "only bare-name targets in a tuple-unpacking assignment are supported so far, got",
    );
}

#[test]
fn an_annotated_scalar_attribute_target_is_refused_naming_its_annotation() {
    assert_capability_error_message(
        "class C:\n    def __init__(self) -> None:\n        self.x: int = 1\n",
        "an annotated attribute target annotated `int` with this value is not supported yet",
    );
}

#[test]
fn an_annotated_subscript_target_names_its_kind() {
    assert_capability_error_message(
        "d = {1: 2}\nd[1]: int = 3\n",
        "only assigning to a bare name is supported so far, got a subscript expression (`obj[key]`)",
    );
}

#[test]
fn a_tuple_for_target_names_its_kind() {
    assert_capability_error_message(
        "for a, b in pairs:\n    pass\n",
        "only a bare name for-target is supported so far, got a tuple",
    );
}

#[test]
fn a_literal_for_iterable_names_its_kind() {
    assert_capability_error_message(
        "for x in [1, 2, 3]:\n    pass\n",
        "only `for x in range(...)` or `for x in <list>` is supported so far, got a list display (`[...]`) as the iterable",
    );
}

#[test]
fn a_for_call_with_a_non_bare_name_callee_names_the_callee_kind() {
    assert_capability_error_message(
        "xs = [1, 2]\nfor k in xs[0]():\n    pass\n",
        "only `for x in range(...)` is supported so far, got a call whose callee is a subscript expression (`obj[key]`)",
    );
}

#[test]
fn calling_a_non_name_callee_names_the_callee_kind() {
    assert_capability_error_message(
        "def f() -> int:\n    return (g or h)()\n",
        "only calling a bare name is supported so far, got a call whose callee is an `and`/`or` \
         boolean expression",
    );
}

#[test]
fn a_protocol_body_assignment_names_its_statement_kind() {
    assert_capability_error_message(
        "from typing import Protocol\nclass P(Protocol):\n    x = 1\n",
        "a protocol class body must contain only method definitions (`def ...`) and annotated assignments (`x: int`) -- an assignment statement is not supported yet",
    );
}

#[test]
fn a_protocol_body_ellipsis_names_the_expression_inside_the_statement() {
    assert_capability_error_message(
        "from typing import Protocol\nclass P(Protocol): ...\n",
        "a protocol class body must contain only method definitions (`def ...`) and annotated assignments (`x: int`) -- an expression statement (an `...` ellipsis literal) is not supported yet",
    );
}

#[test]
fn a_lambda_receiver_names_its_expression_kind() {
    assert_capability_error_message(
        "def f(a: str) -> str:\n    return (lambda: a).upper()\n",
        "expression kind not supported yet: a `lambda`",
    );
}

#[test]
fn an_unsupported_statement_names_its_kind() {
    assert_capability_error_message(
        "with open(\"f\") as fh:\n    pass\n",
        "statement kind not supported yet: a `with` statement",
    );
}

#[test]
fn a_nested_def_is_qualified_by_its_position() {
    assert_capability_error_message(
        "def outer() -> int:\n    def inner() -> int:\n        return 1\n    return 1\n",
        "statement kind not supported yet: a `def` nested inside a function or block body (only a module-level `def` or a method in a class body is supported)",
    );
}

#[test]
fn a_def_under_a_module_level_if_is_qualified_by_its_position() {
    assert_capability_error_message(
        "flag = True\nif flag:\n    def f() -> int:\n        return 1\n",
        "statement kind not supported yet: a `def` nested inside a function or block body (only a module-level `def` or a method in a class body is supported)",
    );
}

#[test]
fn a_nested_class_is_qualified_by_its_position() {
    assert_capability_error_message(
        "def outer() -> int:\n    class Inner:\n        def __init__(self) -> None:\n            return\n    return 1\n",
        "statement kind not supported yet: a `class` nested inside a function or block body (only a module-level `class` is supported)",
    );
}

#[test]
fn a_function_local_import_is_qualified_by_its_position() {
    assert_capability_error_message(
        "def f() -> int:\n    import os\n    return 1\n",
        "statement kind not supported yet: an `import` inside a function or block body (only a module-level import, one inside an `if TYPE_CHECKING:` guard, or an import of a CPython module, is supported)",
    );
}

#[test]
fn a_function_local_from_import_is_qualified_by_its_position() {
    assert_capability_error_message(
        "def f() -> int:\n    from os import path\n    return 1\n",
        "statement kind not supported yet: an `import` inside a function or block body (only a module-level import, one inside an `if TYPE_CHECKING:` guard, or an import of a CPython module, is supported)",
    );
}

#[test]
fn break_and_continue_inside_a_loop_lower() {
    let module = pycc_parser_test_helper::parse(
        "for i in range(3):\n    if i:\n        break\n    continue\n",
    );
    let hir = lower_checked(&module).unwrap();
    let [HirItem::TopLevelStmt(HirStmt::ForRange { body, .. })] = hir.items.as_slice() else {
        panic!("expected one range loop, got {:?}", hir.items);
    };
    assert!(matches!(&body[0], HirStmt::If { body, .. } if body == &vec![HirStmt::Break]));
    assert_eq!(body[1], HirStmt::Continue);
}

#[test]
fn no_capability_message_renders_an_ast_debug_dump() {
    // Issue #890: every `C0001` HIR lowering emits names the construct in
    // Python terms; the `Debug` form of a `ruff_python_ast` node always
    // carries `node_index: NodeIndex(`, so its absence proves no dump
    // leaked through any of the rewritten sites.
    for source in [
        "import typing\ndef f(x: typing.Any) -> int:\n    return 1\n",
        "class C:\n    a = b = 1\n",
        "a, o.x = 1, 2\n",
        "class C:\n    def __init__(self) -> None:\n        self.x: int = 1\n",
        "for a, b in pairs:\n    pass\n",
        "for x in [1]:\n    pass\n",
        "xs = [1, 2]\nfor k in xs[0]():\n    pass\n",
        "def f() -> int:\n    return (g or h)()\n",
        "xs = [k for k in (g or h)()]\n",
        "from typing import Protocol\nclass P(Protocol):\n    x = 1\n",
    ] {
        let module = pycc_parser_test_helper::parse(source);
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001", "{source:?}");
        assert!(
            !diagnostic.message.contains("NodeIndex("),
            "{source:?}: {}",
            diagnostic.message
        );
    }
}
