//! Issue #1095: `append`, `pop`, `get` and `add` on a CPython object are the
//! foreign method call, whatever their arity, while a native `list`, `dict`
//! or `set` receiver in the same module keeps its container reading and its
//! diagnostics.
//!
//! The fixture helper is copied from `function_local_tests.rs` (with an
//! `ext` switch added) because a sibling module cannot reach it.

/// Lowers `source` with every `import` request answered `Foreign`, as an
/// `ext` module when `ext` is set.
fn lower(source: &str, ext: bool) -> pycc_hir::HirModule {
    let module = pycc_parser::parse(source).expect("test source must parse");
    let mut resolved = pycc_hir::ResolvedImports::default();
    resolved.set_ext_module(ext);
    for request in pycc_hir::project_import_requests(&module) {
        resolved.insert(request.span, pycc_hir::ResolvedImport::Foreign);
    }
    pycc_hir::lower_module(&module, &resolved, None)
        .unwrap_or_else(|diagnostics| panic!("{source:?} must lower: {diagnostics:#?}"))
        .hir
}

fn admitted(source: &str, ext: bool) {
    crate::check_all(&lower(source, ext))
        .unwrap_or_else(|diagnostics| panic!("{source:?}: {diagnostics:#?}"));
}

/// The single diagnostic `source` is refused with, asserted by code and a
/// message phrase.
fn refused(source: &str, ext: bool, code: &str, phrase: &str) {
    let diagnostics = crate::check_all(&lower(source, ext)).expect_err(source);
    assert_eq!(diagnostics.len(), 1, "{source:?}: {diagnostics:#?}");
    assert_eq!(diagnostics[0].code, code, "{source:?}: {diagnostics:#?}");
    assert!(
        diagnostics[0].message.contains(phrase),
        "{source:?}: {diagnostics:#?}"
    );
}

/// The six calls the issue measured as refused, each on a module object.
#[test]
fn every_container_name_on_a_foreign_module_is_admitted() {
    for call in [
        "gc.get(1, 2)",
        "gc.pop()",
        "gc.append(1)",
        "gc.add(1)",
        "gc.get(1)",
        "gc.pop(1)",
    ] {
        admitted(&format!("import gc\n\n{call}\n"), false);
    }
}

/// A module-level and a function-local object binding, with every result
/// used as an object.
#[test]
fn a_bound_object_receiver_is_admitted_in_both_scopes() {
    admitted(
        "import json\n\no = json.loads(\"[]\")\no.append(1)\nprint(o.pop())\n\
         d = json.loads(\"{}\")\nprint(d.get(\"k\"), d.get(\"k\", 2))\n",
        false,
    );
    admitted(
        "import json\n\n\ndef f(s: str) -> None:\n    o = json.loads(s)\n    \
         o.append(o)\n    print(len(o.pop()))\n    print(o.get(1, 2))\n",
        false,
    );
}

/// In an `ext` module an `Any` parameter, a bare `list`/`dict`/`set`
/// parameter and a slot holding one are all objects.
#[test]
fn an_ext_parameter_and_slot_receiver_is_admitted() {
    admitted(
        "from typing import Any\n\n\ndef f(o: Any, xs: list, d: dict, s: set) -> Any:\n    \
         xs.append(o)\n    s.add(1)\n    o.add(d.get(\"k\"))\n    print(d.get(\"k\", 2.5))\n    \
         return xs.pop()\n",
        true,
    );
    admitted(
        "class Stack:\n    def __init__(self, items: list) -> None:\n        self.items = items\n\n    \
         def push(self, x: int) -> None:\n        self.items.append(x)\n\n    \
         def top(self) -> object:\n        return self.items.pop()\n",
        true,
    );
}

/// The foreign call's own argument rule still applies to these names.
#[test]
fn an_unpackable_argument_is_refused_as_for_any_method() {
    refused(
        "import gc\n\ngc.append([1])\n",
        false,
        "I0404",
        "passing a `list[int]` argument to a CPython object's method",
    );
}

/// A native container in a module whose gate is on keeps its container
/// reading, so its own diagnostics are unchanged.
#[test]
fn a_native_receiver_keeps_its_container_diagnostics() {
    admitted(
        "import gc\n\nxs = [1]\nxs.append(2)\nprint(xs.pop())\nd = {\"a\": 1}\n\
         print(d.get(\"a\", 2))\ns = {1}\ns.add(2)\n\n\ndef f() -> None:\n    \
         ys = []\n    ys.append(1)\n    print(ys.pop())\n",
        false,
    );
    refused(
        "import gc\n\nxs = [1]\nxs.append(\"s\")\n",
        false,
        "T0021",
        "cannot append `str` to a list of `int`",
    );
    refused(
        "import gc\n\nd = {\"a\": 1}\nprint(d.get(\"a\"))\n",
        false,
        "C0001",
        "`.get()` is only supported as `dict.get(key, default)`",
    );
}

/// Issue #1482: no source program reaches the container reading of a call
/// on a CPython object's attribute any more, so the node is built by hand
/// from the admitted reading. The type checker still refuses it with
/// `I0404` rather than the misleading `T0033`.
#[test]
fn a_container_node_on_an_object_attribute_is_refused_with_i0404() {
    let mut module = lower("import gc\n\ngc.garbage.append(1)\n", false);
    let rewritten = module.items.iter_mut().find_map(|item| match item {
        pycc_hir::HirItem::TopLevelStmt(pycc_hir::HirStmt::ExprStmt(expr)) => match expr {
            pycc_hir::HirExpr::ReceiverDispatchedCall { call, .. } => {
                *expr = call.container_form().expect("an admitted reading");
                Some(())
            }
            _ => None,
        },
        _ => None,
    });
    assert_eq!(rewritten, Some(()));
    let diagnostics = crate::check_all(&module).expect_err("the container node is refused");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].code, "I0404", "{diagnostics:#?}");
    assert!(
        diagnostics[0]
            .message
            .contains("on a CPython object's attribute"),
        "{diagnostics:#?}"
    );
}

/// A private helper's `o.pop()` on a local object resolves to `object`
/// in the constraint solver, as any other method call on it does:
/// `object + int` is refused naming `object`.
#[test]
fn a_helper_returning_pop_on_an_object_resolves_to_object() {
    let helper = "def _p(s):\n    o = json.loads(s)\n    return o.pop()\n\n\n";
    admitted(
        &format!("import json\n\n\n{helper}print(len(_p(\"[[1]]\")))\n"),
        false,
    );
    refused(
        &format!("import json\n\n\n{helper}print(_p(\"[1]\") + 1)\n"),
        false,
        "T0021",
        "operator Add is not defined for `object` and `int`",
    );
}
