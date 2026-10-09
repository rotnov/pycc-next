//! The PEP 562 module hooks at the `--ext` boundary (#1467): which hooks
//! the entry source defines, which import bindings are refused, the export
//! the collector builds for a hook, and the generated table the shim adds
//! after the module body has run. `tests/issue_1467_module_getattr.rs` runs
//! the same shapes against CPython.

use super::super::{
    ExtReceiver, SHIM_C, collect_exports, collect_exports_with_hooks, generate_exports_inc,
};
use super::*;
use pycc_hir::{HirModule, Ty};

/// Parses, lowers and type-checks `source`, returning the resolved module.
fn resolved(source: &str) -> HirModule {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
    pycc_types::check_and_resolve(&hir).expect("test fixture must check")
}

fn hooks_of(source: &str) -> Result<EntryHooks, Vec<Diagnostic>> {
    EntryHooks::from_source(source)
}

fn defined(source: &str) -> Vec<String> {
    hooks_of(source)
        .expect("no refusal")
        .defined()
        .iter()
        .cloned()
        .collect()
}

fn refusals(source: &str) -> Vec<Diagnostic> {
    hooks_of(source).expect_err("an import binding is refused")
}

#[test]
fn exactly_the_two_pep_562_names_are_hooks() {
    assert!(is_module_hook("__getattr__"));
    assert!(is_module_hook("__dir__"));
    assert!(!is_module_hook("__getattribute__"));
    assert!(!is_module_hook("getattr"));
    assert!(!is_module_hook("__getattr__x"));
}

#[test]
fn a_top_level_def_of_either_hook_defines_it() {
    assert_eq!(
        defined(
            "def __dir__() -> str:\n    return 'a'\n\n\n\
             def __getattr__(name: str) -> int:\n    return 1\n\n\n\
             def f() -> int:\n    return 1\n"
        ),
        ["__dir__", "__getattr__"]
    );
}

#[test]
fn a_def_that_is_not_at_the_top_level_defines_nothing() {
    // A method and a nested function are not module attributes.
    assert!(
        defined(
            "class C:\n    def __getattr__(self, name: str) -> int:\n        return 1\n\n\n\
             def f() -> int:\n    def __dir__() -> str:\n        return 'a'\n    return 1\n"
        )
        .is_empty()
    );
    assert!(defined("x = 1\n").is_empty());
}

#[test]
fn every_import_spelling_that_binds_a_hook_name_is_refused() {
    for source in [
        "from lib import __getattr__\n",
        "from lib import g as __dir__\n",
        "import __getattr__\n",
        "import __dir__.sub\n",
        "import os.path as __getattr__\n",
        "import os\nif os.name:\n    from lib import __getattr__\n",
        "try:\n    pass\nexcept ImportError:\n    from lib import __dir__\n",
        // Deliberately over-inclusive: the guard is not evaluated.
        "from typing import TYPE_CHECKING\nif TYPE_CHECKING:\n    from lib import __getattr__\n",
    ] {
        let refused = refusals(source);
        assert_eq!(refused.len(), 1, "{source}");
        assert_eq!(refused[0].code, "C0001", "{source}");
        assert!(
            refused[0]
                .message
                .contains("bound by an import -- CPython calls a module's own"),
            "{source}"
        );
        assert!(refused[0].message.contains("(#1467)"), "{source}");
    }
}

#[test]
fn every_other_module_scope_binding_of_a_hook_name_is_refused() {
    for (source, how) in [
        ("__getattr__ = 5\n", "bound by an assignment"),
        ("__dir__: int = 5\n", "bound by an assignment"),
        ("__dir__ += 1\n", "bound by an assignment"),
        ("a, __getattr__ = 1, 2\n", "bound by an assignment"),
        (
            "for __dir__ in range(3):\n    pass\n",
            "bound by an assignment",
        ),
        (
            "with open('f') as __getattr__:\n    pass\n",
            "bound by an assignment",
        ),
        ("if (__dir__ := 1):\n    pass\n", "bound by an assignment"),
        ("type __getattr__ = int\n", "bound by an assignment"),
        ("del __getattr__\n", "deleted by a `del` statement"),
        ("class __dir__:\n    pass\n", "bound by a class statement"),
        (
            "try:\n    pass\nexcept ValueError as __getattr__:\n    pass\n",
            "bound by an except clause",
        ),
        (
            "try:\n    pass\nexcept* ValueError as __dir__:\n    pass\n",
            "bound by an except clause",
        ),
        // Deliberately over-inclusive: a lambda body is its own scope.
        ("f = lambda: (__dir__ := 1)\n", "bound by an assignment"),
        // A `def` or `class` header runs at module scope.
        (
            "def f(a: int = (__dir__ := 1)) -> int:\n    return a\n",
            "bound by an assignment",
        ),
        (
            "@(__getattr__ := staticmethod)\ndef f() -> None:\n    pass\n",
            "bound by an assignment",
        ),
        (
            "def f() -> (__dir__ := int):\n    return 1\n",
            "bound by an assignment",
        ),
        (
            "@(__dir__ := dataclass)\nclass C:\n    pass\n",
            "bound by an assignment",
        ),
        (
            "class C((__getattr__ := object)):\n    pass\n",
            "bound by an assignment",
        ),
        (
            "match 1:\n    case __getattr__:\n        pass\n",
            "bound by a match pattern",
        ),
        (
            "match [1]:\n    case [*__dir__]:\n        pass\n",
            "bound by a match pattern",
        ),
        (
            "match {}:\n    case {**__dir__}:\n        pass\n",
            "bound by a match pattern",
        ),
    ] {
        let refused = refusals(source);
        assert_eq!(refused.len(), 1, "{source}");
        assert_eq!(refused[0].code, "C0001", "{source}");
        assert!(
            refused[0].message.contains(how),
            "{source}: {}",
            refused[0].message
        );
    }
}

#[test]
fn a_hook_def_inside_any_module_level_block_is_refused() {
    let def = "def __getattr__(name: str) -> int:\n        return 1\n";
    for source in [
        format!("import sys\nif sys.version_info >= (3, 7):\n    {def}"),
        format!("if True:\n    pass\nelse:\n    {def}"),
        format!("try:\n    {def}except ImportError:\n    pass\n"),
        format!("try:\n    pass\nexcept ImportError:\n    {def}"),
        format!("try:\n    pass\nfinally:\n    {def}"),
        format!("for i in range(1):\n    {def}"),
        format!("while False:\n    {def}"),
        format!("with open('f') as fh:\n    {def}"),
        "match 1:\n    case 1:\n        def __dir__() -> str:\n            return 'a'\n"
            .to_string(),
    ] {
        let refused = refusals(&source);
        assert_eq!(refused.len(), 1, "{source}");
        assert_eq!(refused[0].code, "C0001", "{source}");
        assert!(
            refused[0]
                .message
                .contains("defined by a `def` inside a module-level block"),
            "{source}: {}",
            refused[0].message
        );
    }
}

#[test]
fn a_nested_hook_def_is_located_at_its_name() {
    let refused = refusals("if True:\n    def __dir__() -> str:\n        return 'a'\n");
    assert_eq!(refused[0].span, Some(Span::new(17, 24)));
}

#[test]
fn a_hook_def_at_the_top_level_beside_a_block_is_still_published() {
    assert_eq!(
        defined(
            "import sys\nif sys.version_info >= (3, 7):\n    x = 1\n\n\n\
             def __getattr__(name: str) -> int:\n    return 1\n"
        ),
        ["__getattr__"]
    );
}

#[test]
fn a_store_is_located_at_its_target_name() {
    let refused = refusals("x = 1\n__getattr__ = 5\n");
    assert_eq!(refused[0].span, Some(Span::new(6, 17)));
}

#[test]
fn a_read_of_a_hook_name_and_a_non_hook_store_are_not_refused() {
    assert!(
        defined(
            "def __getattr__(name: str) -> int:\n    return 1\n\n\n\
             g = __getattr__\nx = 1\nmatch 1:\n    case y:\n        pass\n\
             try:\n    pass\nexcept ValueError as e:\n    pass\nexcept TypeError:\n    pass\n\n\n\
             def h(a: int = 1, *, __dir__: int = 2) -> int:\n    return a\n"
        )
        .contains(&"__getattr__".to_string())
    );
}

#[test]
fn the_refusal_is_located_at_the_binding_alias() {
    let refused = refusals("import os\nfrom lib import f, __getattr__ as __dir__\n");
    let span = refused[0].span.expect("a located refusal");
    assert_eq!(span, Span::new(29, 51));
    assert!(refused[0].message.contains("module `__dir__`"));
}

#[test]
fn every_offending_alias_is_reported_at_once() {
    assert_eq!(refusals("from lib import __getattr__, __dir__\n").len(), 2);
}

#[test]
fn an_import_that_binds_another_name_or_a_local_one_is_not_refused() {
    assert!(
        defined(
            "from lib import __getattr__ as g\nimport os.path\n\n\n\
             def f() -> int:\n    from lib import __dir__\n    return 1\n\n\n\
             class C:\n    from lib import __getattr__\n"
        )
        .is_empty()
    );
}

#[test]
fn an_unparseable_source_defines_no_hook_and_refuses_nothing() {
    assert_eq!(hooks_of("def (:\n"), Ok(EntryHooks::default()));
}

#[test]
fn a_defined_hook_is_a_module_level_export_and_a_helper_one_is_not() {
    let source = "def __getattr__(name: str) -> int:\n    return 1\n\n\n\
                  def __dir__() -> str:\n    return 'a'\n";
    let module = resolved(source);
    let hooks = EntryHooks::from_source(source).expect("no refusal");
    let exports = collect_exports_with_hooks(&module, hooks.defined()).expect("carriable");
    let getattr = exports
        .iter()
        .find(|export| export.name == "__getattr__")
        .expect("published");
    assert_eq!(getattr.class, None);
    assert_eq!(getattr.receiver, ExtReceiver::None);
    assert_eq!(getattr.params, [Ty::Str]);
    assert_eq!(getattr.return_ty, Ty::Int);
    assert!(exports.iter().any(|export| export.name == "__dir__"));
    // The same compiled items with no entry-module hook -- a helper's
    // `def __getattr__` in the linked program -- are not exported.
    assert!(collect_exports(&module).expect("no gap").is_empty());
}

#[test]
fn a_redefined_hook_is_exported_once() {
    let source = "def __getattr__(name: str) -> int:\n    return 1\n\n\n\
                  def __getattr__(name: str) -> int:\n    return 2\n";
    let module = resolved(source);
    let hooks = EntryHooks::from_source(source).expect("no refusal");
    let exports = collect_exports_with_hooks(&module, hooks.defined()).expect("carriable");
    assert_eq!(exports.len(), 1);
    assert_eq!(exports[0].name, "__getattr__");
}

#[test]
fn an_uncarriable_hook_is_a_gap_whose_remedy_is_not_renaming_it() {
    let source = "def __getattr__(name: str) -> list[int]:\n    return [1]\n";
    let module = resolved(source);
    let hooks = EntryHooks::from_source(source).expect("no refusal");
    let gaps = collect_exports_with_hooks(&module, hooks.defined()).expect_err("a gap");
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].code, "C0003");
    assert!(gaps[0].span.is_none());
    let message = &gaps[0].message;
    assert!(
        message.starts_with(
            "--ext cannot publish the module hook `__getattr__`: its return type `-> list`"
        ),
        "{message}"
    );
    assert!(message.contains("change its signature"), "{message}");
    assert!(!message.contains("rename"), "{message}");
}

#[test]
fn a_hook_is_tabled_for_the_post_exec_publication_only() {
    let source = "def __getattr__(name: str) -> int:\n    return 1\n\n\n\
                  def f() -> int:\n    return 1\n";
    let module = resolved(source);
    let hooks = EntryHooks::from_source(source).expect("no refusal");
    let exports = collect_exports_with_hooks(&module, hooks.defined()).expect("carriable");
    let inc = generate_exports_inc("m", &exports, &[], &[], &[], &[]);
    let (methods, rest) = inc
        .split_once("static PyMethodDef pycc_ext_module_hooks[] = {\n")
        .expect("the hook table is emitted");
    let methods = methods
        .split_once("static PyMethodDef pycc_ext_methods[] = {\n")
        .expect("the method table precedes it")
        .1;
    let hooks_table = rest.split_once("};\n").expect("terminated").0;
    assert!(methods.contains("{\"f\", "), "{inc}");
    assert!(!methods.contains("__getattr__"), "{inc}");
    assert!(
        hooks_table.starts_with(
            "    {\"__getattr__\", (PyCFunction)(void (*)(void))pycc_ext_wrap___getattr__, "
        ),
        "{inc}"
    );
    assert!(
        hooks_table.ends_with("    {NULL, NULL, 0, NULL},\n"),
        "{inc}"
    );
    // The wrapper itself is emitted as for any export.
    assert!(inc.contains("pycc_ext_wrap___getattr__("), "{inc}");
}

#[test]
fn a_module_without_hooks_still_emits_the_terminated_hook_table() {
    let inc = generate_exports_inc("m", &[], &[], &[], &[], &[]);
    assert!(
        inc.contains(
            "static PyMethodDef pycc_ext_module_hooks[] = {\n    {NULL, NULL, 0, NULL},\n};\n"
        ),
        "{inc}"
    );
}

#[test]
fn the_shim_adds_the_hook_table_only_after_the_body_succeeds() {
    let shim = SHIM_C.replace("\r\n", "\n");
    let exec = shim
        .find("exec_status = pycc_ext_module_exec();")
        .expect("the body runs");
    let add = shim
        .find("if (PyModule_AddFunctions(module, pycc_ext_module_hooks) != 0) {")
        .expect("the hooks are added");
    let failure = shim
        .find("PyErr_SetString(PyExc_ImportError, \"pycc module body failed\");")
        .expect("the failure arm");
    assert!(exec < failure && failure < add);
    // The module has no creation-time method table at all (#1199), so the
    // hook table is never it.
    assert!(shim.contains("    NULL,\n    pycc_ext_slots,\n"));
    assert!(!shim.contains("    pycc_ext_module_hooks,\n    pycc_ext_slots,\n"));
}
