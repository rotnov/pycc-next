//! A buffer-carrier from-import (#1380, Part 2 of #1138, D-244): the import
//! binds a hidden name and runs in the host, its spelling stays an
//! annotation, and every other read or binding of the spelling is refused by
//! `carrier::reject_carrier_misuse` at the offending node's own span.

use super::from_foreign::lower_foreign;
use super::*;
use crate::FromImport;
use crate::import::{reject_carrier_misuse, splice_by_item};

/// Every module a fixture may import, answered as foreign.
const FOREIGN: &[&str] = &["numpy", "numpy.typing", "other", "ndarray"];

/// Both carrier imports, as lowering records them.
const IMPORTS: &str = "from numpy import ndarray\nfrom numpy.typing import NDArray\n";

/// The post-check alone over `source`, answering every import request for
/// a module in `FOREIGN` as foreign and treating the items in `failed` as
/// already failed.
fn scan(source: &str, failed: &[usize]) -> Vec<Diagnostic> {
    let parsed = parse(source);
    let mut resolved = ResolvedImports::default();
    for request in project_import_requests(&parsed) {
        if request
            .module
            .as_deref()
            .is_some_and(|module| FOREIGN.contains(&module))
        {
            resolved.insert(request.span, ResolvedImport::Foreign);
        }
    }
    reject_carrier_misuse(&parsed.body, &resolved, failed)
}

/// The post-check alone over `IMPORTS` followed by `body`, with no item
/// failed.
fn misuse(body: &str) -> Vec<Diagnostic> {
    scan(&format!("{IMPORTS}{body}"), &[])
}

/// The span of `name` inside the first occurrence of `context` in `source`.
fn span_in(source: &str, context: &str, name: &str) -> Span {
    let start = source.find(context).expect("context occurs in the source")
        + context.find(name).expect("name occurs in its context");
    Span::new(start as u32, (start + name.len()) as u32)
}

/// `body` (after `IMPORTS`) is refused exactly once, at `name` inside
/// `context`, with the read or binding message for `name`.
fn assert_refused(body: &str, context: &str, name: &str, kind: &str) {
    let source = format!("{IMPORTS}{body}");
    let diagnostics = misuse(body);
    assert_eq!(diagnostics.len(), 1, "{body}: {diagnostics:#?}");
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.code, "C0001");
    let spelling = if name.starts_with("ndarray") {
        "ndarray"
    } else {
        "NDArray"
    };
    assert!(
        diagnostic
            .message
            .starts_with(&format!("{kind} `{spelling}`")),
        "{body}: {}",
        diagnostic.message
    );
    assert_eq!(
        diagnostic.span,
        Some(span_in(&source, context, name)),
        "{body}"
    );
}

fn assert_read_refused(body: &str, context: &str, name: &str) {
    assert_refused(body, context, name, "reading");
}

fn assert_binding_refused(body: &str, context: &str, name: &str) {
    assert_refused(body, context, name, "binding");
}

#[test]
fn each_carrier_pair_binds_a_hidden_name_and_keeps_the_real_one_in_from() {
    let source = "from numpy.typing import NDArray\nfrom numpy import ndarray, float64\n\
                  def total(a: NDArray, b: ndarray) -> float:\n    return a[0] + b[0]\n";
    let lowered = lower_foreign(source, FOREIGN).expect("must lower");
    let rows: Vec<(String, String, String, usize)> = lowered
        .hir
        .imports
        .iter()
        .map(|binding| match binding {
            ImportBinding::Foreign {
                local_name,
                module_path,
                from: Some(FromImport { name, index, .. }),
                ..
            } => (
                local_name.clone(),
                module_path.clone(),
                name.clone(),
                *index,
            ),
            other => panic!("unexpected binding {other:?}"),
        })
        .collect();
    let row = |local: &str, module: &str, name: &str, index| {
        (
            local.to_string(),
            module.to_string(),
            name.to_string(),
            index,
        )
    };
    assert_eq!(
        rows,
        vec![
            row("$carrier:NDArray", "numpy.typing", "NDArray", 0),
            row("$carrier:ndarray", "numpy", "ndarray", 0),
            row("float64", "numpy", "float64", 1),
        ]
    );
}

/// The identical import twice, and once more inside a `TYPE_CHECKING`
/// guard, is still one meaning of the name: nothing is refused.
#[test]
fn a_repeated_or_guarded_carrier_import_is_accepted() {
    let source = "from typing import TYPE_CHECKING\nfrom numpy import ndarray\n\
                  if TYPE_CHECKING:\n    from numpy import ndarray\n\
                  from numpy import ndarray\ndef f(a: ndarray) -> float:\n    return a[0]\n";
    lower_foreign(source, FOREIGN).expect("must lower");
}

#[test]
fn a_module_without_a_carrier_import_is_not_scanned() {
    let source = "ndarray = 1\nx = NDArray\n";
    assert!(scan(source, &[]).is_empty());
}

/// A carrier import only inside `if TYPE_CHECKING:` never runs in CPython,
/// so a call `ndarray(n)` there would raise `NameError`: the guarded import
/// still declares the spelling, and the call is refused like any read.
#[test]
fn a_type_checking_only_carrier_import_still_refuses_a_read() {
    let source = "from typing import TYPE_CHECKING\nif TYPE_CHECKING:\n    \
                  from numpy import ndarray\n\
                  def f(n: int) -> float:\n    a = ndarray(n)\n    return a[0]\n";
    let diagnostic = super::from_foreign::only_error(lower_foreign(source, FOREIGN));
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.span,
        Some(span_in(source, "= ndarray(n)", "ndarray"))
    );
    assert!(
        diagnostic
            .message
            .starts_with("reading `ndarray` outside a type annotation"),
        "{}",
        diagnostic.message
    );
}

/// A carrier import is found in every nested `if`/`try` body: an `elif`,
/// an `else`, a `try` body, a handler, a `try`'s `else` and `finally`.
#[test]
fn a_carrier_import_in_any_nested_module_level_body_is_found() {
    for header in [
        "if a:\n    pass\nelif b:\n    from numpy import ndarray\n",
        "if a:\n    pass\nelse:\n    if b:\n        from numpy import ndarray\n",
        "try:\n    from numpy import ndarray\nexcept ImportError:\n    pass\n",
        "try:\n    pass\nexcept ImportError:\n    from numpy import ndarray\n",
        "try:\n    pass\nexcept ImportError:\n    pass\nelse:\n    from numpy import ndarray\n",
        "try:\n    pass\nfinally:\n    from numpy import ndarray\n",
    ] {
        let source = format!("{header}x = ndarray\n");
        let diagnostics = scan(&source, &[]);
        assert_eq!(diagnostics.len(), 1, "{source}: {diagnostics:#?}");
    }
}

/// Only a foreign answer declares a carrier: a project `numpy.py` defining
/// its own `ndarray` is an ordinary project import, so calling it is valid.
#[test]
fn a_project_module_numpy_declares_no_carrier() {
    let dependency = Fixture::new("def ndarray(n: int) -> int:\n    return n\n");
    dependency.lower_ok("from numpy import ndarray\nx = ndarray(1)\n");
}

/// A guarded from-import of a project module gets no answer from the
/// driver, so it declares nothing either: the spelling keeps the meaning it
/// has without the import.
#[test]
fn an_unanswered_carrier_import_declares_nothing() {
    for source in [
        "from numpy import ndarray\nx = ndarray\n",
        "from typing import TYPE_CHECKING\nif TYPE_CHECKING:\n    \
         from numpy import ndarray\nx = ndarray\n",
    ] {
        let parsed = parse(source);
        let resolved = ResolvedImports::default();
        assert!(
            reject_carrier_misuse(&parsed.body, &resolved, &[]).is_empty(),
            "{source}"
        );
    }
}

/// A module-scope `del` of a carrier spelling is one diagnostic: the
/// imported-name `del` rule's, not also the carrier scan's binding refusal.
#[test]
fn a_module_scope_del_of_a_carrier_is_reported_once() {
    for source in [
        "from numpy import ndarray\ndel ndarray\n",
        "from numpy import ndarray\nif a:\n    del ndarray\n",
        "from numpy import ndarray\ntry:\n    del (ndarray, b)\nexcept E:\n    pass\n",
    ] {
        let diagnostic = super::from_foreign::only_error(lower_foreign(source, FOREIGN));
        assert_eq!(diagnostic.code, "C0001", "{source}");
        assert!(
            diagnostic
                .message
                .contains("a `del` of the imported name `ndarray`"),
            "{source}: {}",
            diagnostic.message
        );
    }
}

/// The module-scope `del` exemption covers only the deleted name: a read
/// inside the target is still refused.
#[test]
fn a_read_inside_a_module_scope_del_target_is_refused() {
    assert_read_refused("del x[ndarray]\n", "x[ndarray]", "ndarray");
}

/// Neither an aliased carrier pair nor a carrier import inside a function
/// declares the spelling: the first binds another name, and the second
/// already fails its own item.
#[test]
fn an_aliased_or_function_level_carrier_import_declares_nothing() {
    for source in [
        "from numpy import ndarray as nd\nx = ndarray\n",
        "def f() -> None:\n    from numpy import ndarray\nx = ndarray\n",
    ] {
        assert!(scan(source, &[]).is_empty(), "{source}");
    }
}

/// An item whose lowering already failed keeps its own diagnostic alone:
/// `from other import ndarray` is the spelling refusal, not also a second
/// binding of the carrier spelling.
#[test]
fn a_failed_import_item_is_reported_once() {
    let source = "from numpy import ndarray\nfrom other import ndarray\n";
    let diagnostic = super::from_foreign::only_error(lower_foreign(source, FOREIGN));
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        !diagnostic
            .message
            .starts_with("binding `ndarray` in a module"),
        "{}",
        diagnostic.message
    );
}

/// A failed item is neither scanned nor searched for a carrier import.
#[test]
fn a_failed_item_is_skipped_by_the_scan() {
    let source = "from numpy import ndarray\nx = ndarray\n";
    assert_eq!(scan(source, &[]).len(), 1);
    assert!(scan(source, &[1]).is_empty());
    assert!(scan(source, &[0]).is_empty());
}

#[test]
fn every_annotation_position_is_accepted() {
    let diagnostics = misuse(
        "def f(a: NDArray, b: list[ndarray], *args: NDArray, c: ndarray = None, \
         **kw: NDArray) -> NDArray:\n    x: ndarray = a\n    return a\n\
         class C:\n    field: NDArray\n    other: \"ndarray\"\n\
         type A = NDArray\ntype G[T] = list[NDArray]\nB: TypeAlias = ndarray\n\
         y: list[NDArray] = []\n",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn each_read_outside_an_annotation_is_refused() {
    for (body, context, name) in [
        ("x = NDArray\n", "x = NDArray", "NDArray"),
        ("A = NDArray\n", "A = NDArray", "NDArray"),
        (
            "def f(n: int) -> int:\n    a = ndarray(n)\n    return 0\n",
            "a = ndarray(n)",
            "ndarray",
        ),
        (
            "def f(a: object) -> bool:\n    return isinstance(a, ndarray)\n",
            "a, ndarray)",
            "ndarray",
        ),
        ("print(ndarray.__name__)\n", "print(ndarray", "ndarray"),
        (
            "def f(a: int = NDArray) -> int:\n    return 0\n",
            "= NDArray",
            "NDArray",
        ),
        (
            "@ndarray\ndef f() -> int:\n    return 0\n",
            "@ndarray",
            "ndarray",
        ),
        ("class C(ndarray):\n    pass\n", "C(ndarray", "ndarray"),
        (
            "def f[T: NDArray](a: T) -> T:\n    return a\n",
            "T: NDArray",
            "NDArray",
        ),
        (
            "class C:\n    x: NDArray = NDArray\n",
            "= NDArray",
            "NDArray",
        ),
        ("y: int = len(ndarray)\n", "len(ndarray", "ndarray"),
    ] {
        assert_read_refused(body, context, name);
    }
}

/// A use above the import is refused as well as one below it: the check
/// does not depend on where the import sits.
#[test]
fn a_producer_call_above_the_import_is_refused() {
    let source = "def f(n: int) -> float:\n    a = ndarray(n)\n    return 0.0\n\
                  from numpy import ndarray\n";
    let diagnostics = lower_foreign(source, FOREIGN).expect_err("refused");
    let expected = span_in(source, "a = ndarray", "ndarray");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.span == Some(expected)
                && diagnostic
                    .message
                    .starts_with("reading `ndarray` outside a type annotation")),
        "{diagnostics:#?}"
    );
}

#[test]
fn each_definition_and_parameter_binding_is_refused() {
    for (body, context, name) in [
        (
            "def ndarray() -> int:\n    return 0\n",
            "def ndarray",
            "ndarray",
        ),
        ("class NDArray:\n    pass\n", "class NDArray", "NDArray"),
        (
            "def f(ndarray: int) -> int:\n    return 0\n",
            "(ndarray",
            "ndarray",
        ),
        (
            "def f(a: int, /, ndarray: int) -> int:\n    return 0\n",
            ", ndarray",
            "ndarray",
        ),
        (
            "def f(*ndarray: int) -> int:\n    return 0\n",
            "*ndarray",
            "ndarray",
        ),
        (
            "def f(*, ndarray: int) -> int:\n    return 0\n",
            ", ndarray",
            "ndarray",
        ),
        (
            "def f(**ndarray: int) -> int:\n    return 0\n",
            "**ndarray",
            "ndarray",
        ),
        ("g = lambda NDArray: 0\n", "lambda NDArray", "NDArray"),
        (
            "def f[NDArray](a: int) -> int:\n    return 0\n",
            "[NDArray",
            "NDArray",
        ),
        (
            "def f[*NDArray](a: int) -> int:\n    return 0\n",
            "[*NDArray",
            "NDArray",
        ),
        (
            "def f[**NDArray](a: int) -> int:\n    return 0\n",
            "[**NDArray",
            "NDArray",
        ),
    ] {
        assert_binding_refused(body, context, name);
    }
}

#[test]
fn each_statement_binding_is_refused() {
    for (body, context, name) in [
        (
            "try:\n    pass\nexcept ValueError as ndarray:\n    pass\n",
            "as ndarray",
            "ndarray",
        ),
        (
            "def f() -> None:\n    global ndarray\n",
            "global ndarray",
            "ndarray",
        ),
        (
            "def f() -> None:\n    x = 1\n    def g() -> None:\n        nonlocal NDArray\n",
            "nonlocal NDArray",
            "NDArray",
        ),
        ("ndarray = 1\n", "ndarray = 1", "ndarray"),
        ("NDArray += 1\n", "NDArray +=", "NDArray"),
        (
            "for ndarray in range(3):\n    pass\n",
            "for ndarray",
            "ndarray",
        ),
        (
            "with open('x') as NDArray:\n    pass\n",
            "as NDArray",
            "NDArray",
        ),
        (
            "y = [1 for ndarray in range(3)]\n",
            "for ndarray",
            "ndarray",
        ),
        ("y = (NDArray := 1)\n", "(NDArray", "NDArray"),
        (
            "def f() -> None:\n    del ndarray\n",
            "del ndarray",
            "ndarray",
        ),
        ("class C:\n    del NDArray\n", "del NDArray", "NDArray"),
        ("type NDArray = int\n", "type NDArray", "NDArray"),
        ("type G[NDArray] = int\n", "[NDArray", "NDArray"),
        (
            "ndarray: TypeAlias = int\n",
            "ndarray: TypeAlias",
            "ndarray",
        ),
    ] {
        assert_binding_refused(body, context, name);
    }
}

#[test]
fn each_second_import_binding_is_refused() {
    for (body, context, name) in [
        ("import ndarray\n", "\nimport ndarray", "ndarray"),
        (
            "import ndarray.sub\n",
            "\nimport ndarray.sub",
            "ndarray.sub",
        ),
        ("import json as NDArray\n", "as NDArray", "NDArray"),
        (
            "from other import ndarray\n",
            "from other import ndarray",
            "ndarray",
        ),
        (
            "from numpy import NDArray\n",
            "from numpy import NDArray",
            "NDArray",
        ),
    ] {
        assert_binding_refused(body, context, name);
    }
}

#[test]
fn each_match_capture_binding_is_refused() {
    for (body, context, name) in [
        (
            "match 1:\n    case ndarray:\n        pass\n",
            "case ndarray",
            "ndarray",
        ),
        (
            "match [1]:\n    case [*ndarray]:\n        pass\n",
            "*ndarray",
            "ndarray",
        ),
        (
            "match {}:\n    case {**ndarray}:\n        pass\n",
            "**ndarray",
            "ndarray",
        ),
        (
            "match 1:\n    case int() as NDArray:\n        pass\n",
            "as NDArray",
            "NDArray",
        ),
    ] {
        assert_binding_refused(body, context, name);
    }
}

/// #1485's fallback shape: the `except ImportError` rebinding is a second
/// binding of the spelling, refused at the assignment.
#[test]
fn a_try_except_fallback_rebinding_is_refused() {
    let source = "try:\n    from numpy import ndarray\nexcept ImportError:\n    ndarray = None\n";
    let diagnostics = lower_foreign(source, FOREIGN).expect_err("refused");
    let expected = span_in(source, "ndarray = None", "ndarray");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.span == Some(expected)
                && diagnostic
                    .message
                    .starts_with("binding `ndarray` in a module that imports it")),
        "{diagnostics:#?}"
    );
}

/// Every offending site is reported, in source order.
#[test]
fn every_site_is_reported_in_source_order() {
    let diagnostics = misuse("x = NDArray\nndarray = 1\ny = ndarray\n");
    let messages: Vec<&str> = diagnostics
        .iter()
        .map(|diagnostic| &diagnostic.message[..9])
        .collect();
    assert_eq!(messages, ["reading `", "binding `", "reading `"]);
}

#[test]
fn the_messages_name_the_import_and_the_decision() {
    let read = &misuse("x = NDArray\n")[0];
    assert_eq!(
        read.message,
        "reading `NDArray` outside a type annotation is not supported yet: `from numpy.typing \
         import NDArray` keeps the name's buffer-annotation meaning in pycc (D-244), so it may \
         only annotate"
    );
    let binding = &misuse("ndarray = 1\n")[0];
    assert_eq!(
        binding.message,
        "binding `ndarray` in a module that imports it with `from numpy import ndarray` is not \
         supported yet: the import keeps the name's buffer-annotation meaning in pycc (D-244)"
    );
}

/// The carrier scan runs after the item loop, but its diagnostics keep the
/// per-item source order: a line-1 read precedes a later item's own
/// lowering failure.
#[test]
fn a_carrier_diagnostic_keeps_its_item_order() {
    let source = "x = NDArray\nasync def g() -> None:\n    pass\n\
                  from numpy.typing import NDArray\n";
    let diagnostics = lower_foreign(source, FOREIGN).expect_err("refused");
    let starts: Vec<u32> = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.span.expect("spanned").start)
        .collect();
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    assert!(
        diagnostics[0]
            .message
            .starts_with("reading `NDArray` outside a type annotation"),
        "{diagnostics:#?}"
    );
    assert!(starts[0] < starts[1], "{diagnostics:#?}");
}

/// Each carrier diagnostic lands after the diagnostics its item collected
/// in the loop, and every diagnostic collected after the loop stays last.
#[test]
fn splice_by_item_places_each_diagnostic_after_its_item() {
    let source = "a = 1\nb = 2\nc = 3\n";
    let body = &parse(source).body;
    let at = |message: &str, start: u32| crate::unsupported(message, start..start + 1);
    let collected = vec![at("item 0", 0), at("item 2", 12), at("after loop", 0)];
    let carrier = vec![at("carrier 0", 0), at("carrier 1", 6), at("carrier 2", 12)];
    let merged = splice_by_item(body, &[0, 1, 1], 2, collected, carrier);
    let messages: Vec<&str> = merged
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    assert_eq!(
        messages,
        [
            "item 0",
            "carrier 0",
            "carrier 1",
            "item 2",
            "carrier 2",
            "after loop"
        ]
    );
    let untouched = splice_by_item(body, &[0, 1, 1], 2, vec![at("only", 0)], Vec::new());
    assert_eq!(untouched.len(), 1);
}

/// Each refused site is reported once through the whole lowering, not by
/// both the carrier scan and the item's own check.
#[test]
fn each_module_level_refusal_is_one_diagnostic() {
    for body in [
        "ndarray = 1\n",
        "import ndarray\n",
        "global ndarray\n",
        "def ndarray() -> None:\n    pass\n",
        "class NDArray:\n    pass\n",
        "for ndarray in []:\n    pass\n",
        "with a as ndarray:\n    pass\n",
        "x = ndarray\n",
        "print(NDArray)\n",
        "type NDArray = int\n",
        "NDArray: TypeAlias = int\n",
        "from numpy import ndarray\nndarray = 2\n",
    ] {
        let source = format!("{IMPORTS}{body}");
        let diagnostic = super::from_foreign::only_error(lower_foreign(&source, FOREIGN));
        assert_eq!(diagnostic.code, "C0001", "{body}");
    }
}
