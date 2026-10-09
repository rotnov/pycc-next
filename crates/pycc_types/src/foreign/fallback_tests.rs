//! #1485: the `except ImportError` fallback of a foreign import type-checks
//! as `object` on every path, and seeding the handler changes nothing for a
//! handler that does not rebind the name.

/// Lowers `source` with every import request answered `Foreign`.
fn lower_all_foreign(source: &str) -> pycc_hir::HirModule {
    let module = pycc_parser::parse(source).expect("test source must parse");
    let mut resolved = pycc_hir::ResolvedImports::default();
    for request in pycc_hir::project_import_requests(&module) {
        resolved.insert(request.span, pycc_hir::ResolvedImport::Foreign);
    }
    pycc_hir::lower_module(&module, &resolved, None)
        .unwrap_or_else(|diagnostics| panic!("{source:?} must lower: {diagnostics:#?}"))
        .hir
}

fn admitted(source: &str) {
    crate::check_all(&lower_all_foreign(source))
        .unwrap_or_else(|diagnostics| panic!("{source:?}: {diagnostics:#?}"));
}

/// The single diagnostic `source` is refused with, asserted by code and a
/// message phrase.
fn refused(source: &str, code: &str, phrase: &str) {
    let diagnostics = crate::check_all(&lower_all_foreign(source)).expect_err(source);
    assert_eq!(diagnostics.len(), 1, "{source:?}: {diagnostics:#?}");
    assert_eq!(diagnostics[0].code, code, "{source:?}: {diagnostics:#?}");
    assert!(
        diagnostics[0].message.contains(phrase),
        "{source:?}: {diagnostics:#?}"
    );
}

const READS: &str = "print(product is None)\n\ndef f() -> bool:\n    return product is not None\n";

#[test]
fn a_none_or_import_fallback_is_an_object_after_the_try() {
    for head in [
        "try:\n    from itertools import product\nexcept ImportError:\n    product = None\n",
        "try:\n    import itertools as product\nexcept ImportError:\n    product = None\n",
        "try:\n    from nosuch import product\nexcept ImportError:\n    \
         from itertools import product\n",
        "try:\n    from nosuch import product\nexcept ModuleNotFoundError:\n    \
         product = None\nexcept ImportError:\n    from itertools import product\n",
        // Already bound before the `try`: the handler keeps the bound
        // `object` rather than being re-seeded.
        "from itertools import product\ntry:\n    from itertools import product\n\
         except ImportError:\n    product = None\n",
    ] {
        admitted(&format!("{head}{READS}"));
    }
}

/// A handler that only reads the name is not seeded, so the read is the
/// unbound `T0021` it was before #1485.
#[test]
fn a_read_only_handler_still_reports_unbound() {
    refused(
        "try:\n    from itertools import product\nexcept ImportError:\n    print(product)\n",
        "T0021",
        "name `product` is not defined",
    );
}

/// A read before the handler's own rebinding may see the name bound by a
/// partial import or not at all, so it is possibly unbound.
#[test]
fn a_read_before_the_rebinding_is_possibly_unbound() {
    refused(
        "try:\n    from itertools import product\nexcept ImportError:\n    \
         print(product)\n    product = None\n",
        "T0041",
        "`product` may not be bound",
    );
}
