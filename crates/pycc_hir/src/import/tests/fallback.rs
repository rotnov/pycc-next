//! The `except ImportError` fallback rebinding of a foreign import (#1485):
//! which shapes form a [`FallbackGroup`], and which keep the general
//! foreign-shadowing `C0001` or the tailored native-value one.

use super::*;
use crate::import::fallback::Arm;
use crate::{HirExpr, HirItem, HirStmt};

const SHADOW: &str = "shadowing a foreign import is not supported yet";
const NATIVE: &str = "is rebound in an `except ImportError` handler to a value other than `None`";

/// Lowers `source` with every import request answered foreign, the way the
/// driver answers a non-project, non-`pycc_std` module.
fn lower_foreign(source: &str) -> Result<LoweredModule, Vec<Diagnostic>> {
    let parsed = parse(source);
    let mut resolved = ResolvedImports::default();
    for request in project_import_requests(&parsed) {
        resolved.insert(request.span, ResolvedImport::Foreign);
    }
    lower_module(&parsed, &resolved, None)
}

fn admitted(source: &str) -> LoweredModule {
    lower_foreign(source).unwrap_or_else(|diagnostics| panic!("{source:?}: {diagnostics:#?}"))
}

/// The single diagnostic `source` is refused with.
fn refused(source: &str) -> Diagnostic {
    let diagnostics = lower_foreign(source).expect_err(source);
    assert_eq!(diagnostics.len(), 1, "{source:?}: {diagnostics:#?}");
    diagnostics.into_iter().next().expect("one diagnostic")
}

fn span_of(source: &str, needle: &str) -> Span {
    let start = source.find(needle).expect("needle");
    let end = start + source[start..].find('\n').expect("a line end");
    Span::new(start as u32, end as u32)
}

/// The fallback groups of the first top-level `try` of `module`.
fn groups(module: &LoweredModule) -> Vec<FallbackGroup> {
    let stmt = module
        .hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::TopLevelStmt(stmt @ HirStmt::Try { .. }) => Some(stmt),
            _ => None,
        })
        .expect("a top-level try");
    fallback_groups(std::slice::from_ref(stmt), Span::new(0, 0)).expect("no tailored refusal")
}

/// Every admitted shape: the from and plain forms, each catcher, a `None`
/// or an import fallback, two names, and two handlers.
#[test]
fn the_fallback_shapes_are_admitted_with_no_definition() {
    for (source, names) in [
        (
            "try:\n    from itertools import product\nexcept ImportError:\n    product = None\n",
            &["product"][..],
        ),
        (
            "try:\n    from itertools import product\nexcept ModuleNotFoundError:\n    \
             product = None\n",
            &["product"],
        ),
        (
            "try:\n    from itertools import product\nexcept Exception:\n    product = None\n",
            &["product"],
        ),
        (
            "try:\n    from itertools import product\nexcept:\n    product = None\n",
            &["product"],
        ),
        (
            "try:\n    from itertools import product\nexcept (ValueError, ImportError):\n    \
             product = None\n",
            &["product"],
        ),
        (
            "try:\n    import numpy as np\nexcept ImportError:\n    np = None\n",
            &["np"],
        ),
        (
            "try:\n    import colorsys\nexcept Exception:\n    colorsys = None\n",
            &["colorsys"],
        ),
        (
            "try:\n    from nosuch import hls_to_rgb\nexcept ImportError:\n    \
             from colorsys import hls_to_rgb\n",
            &["hls_to_rgb"],
        ),
        (
            "try:\n    import nosuch as cs\nexcept ImportError:\n    import colorsys as cs\n",
            &["cs"],
        ),
        (
            "try:\n    from nosuch import a, b\nexcept ImportError:\n    a = None\n    b = None\n",
            &["a", "b"],
        ),
        (
            "try:\n    from nosuch import product\nexcept ModuleNotFoundError:\n    \
             from itertools import product\nexcept ImportError:\n    \
             from functools import product\n",
            &["product"],
        ),
        // A handler may do more than rebind, and a non-qualifying handler
        // that leaves the name alone does not matter.
        (
            "try:\n    from itertools import product\nexcept ValueError:\n    pass\n\
             except ImportError:\n    print('no itertools')\n    product = None\n",
            &["product"],
        ),
        // `N = M = None` repeats the literal for each target, so `N` is the
        // admitted `N = None` and `M` an ordinary definition.
        (
            "try:\n    from itertools import product\nexcept ImportError:\n    \
             product = other = None\n",
            &["product"],
        ),
        // Two separate `try` statements importing the identical object.
        (
            "try:\n    from itertools import product\nexcept ImportError:\n    product = None\n\
             try:\n    from itertools import product\nexcept ImportError:\n    product = None\n",
            &["product"],
        ),
    ] {
        let module = admitted(source);
        for name in names {
            assert!(
                !module.definition_spans.iter().any(|(n, _)| n == name),
                "{source:?}: {:?}",
                module.definition_spans
            );
        }
    }
}

/// The group records the body's import and each handler's fallback import
/// with its arm, and the handler's `N = None` survives lowering.
#[test]
fn a_group_records_each_import_with_its_arm() {
    let source = "try:\n    from nosuch import product\nexcept ModuleNotFoundError:\n    \
                  from itertools import product\nexcept ImportError:\n    product = None\n";
    let module = admitted(source);
    assert_eq!(
        groups(&module),
        vec![FallbackGroup {
            name: "product".to_string(),
            import_spans: vec![
                (span_of(source, "from nosuch"), Arm::Body),
                (span_of(source, "from itertools"), Arm::Handler(0)),
            ],
        }]
    );
    let HirItem::TopLevelStmt(HirStmt::Try { handlers, .. }) = &module.hir.items[0] else {
        panic!("{:#?}", module.hir.items);
    };
    assert_eq!(
        handlers[1].body,
        vec![HirStmt::Assign {
            target: "product".to_string(),
            value: HirExpr::NoneLiteral,
        }]
    );
}

/// A `try` with no fallback forms no group: a handler that only passes, a
/// body with no foreign import, and a statement that is not a `try`.
#[test]
fn a_try_without_a_fallback_forms_no_group() {
    let module =
        admitted("try:\n    from itertools import product\nexcept ImportError:\n    pass\n");
    assert_eq!(groups(&module), Vec::new());
    let module = admitted("try:\n    x = 1\nexcept ImportError:\n    x = None\n");
    assert_eq!(groups(&module), Vec::new());
    assert_eq!(
        fallback_groups(&[HirStmt::Return(None)], Span::new(0, 0)),
        Ok(Vec::new())
    );
}

/// Every other binding of the name keeps the general shadowing refusal.
#[test]
fn every_other_rebinding_keeps_the_shadowing_refusal() {
    const TRY: &str = "try:\n    from itertools import product\n";
    for tail in [
        "except ImportError:\n    product = None\nelse:\n    product = None\n",
        "except ImportError:\n    product = None\nfinally:\n    product = None\n",
        "except ValueError:\n    product = None\n",
        "except ValueError:\n    product = None\nexcept ImportError:\n    product = None\n",
        "except ImportError:\n    if True:\n        product = None\n",
        "except ImportError as product:\n    pass\n",
        "except ImportError:\n    product = None\nproduct = None\n",
        "except ImportError:\n    product = None\nfrom functools import product\n",
        "except ImportError:\n    if True:\n        from functools import product\n",
        "except ValueError:\n    from functools import product\n",
        "except ImportError:\n    product: int = 3\n",
        "except ImportError:\n    product = other = len('a')\n",
        "except ImportError:\n    product, other = None, 1\n",
        "except ImportError:\n    for product in range(2):\n        pass\n",
        "except* ImportError:\n    product = None\n",
    ] {
        let source = format!("{TRY}{tail}");
        let error = refused(&source);
        assert_eq!(error.code, "C0001", "{source:?}");
        assert!(
            error.message.contains(SHADOW),
            "{source:?}: {}",
            error.message
        );
    }
    // The body binds the name otherwise as well.
    let error = refused(
        "try:\n    from itertools import product\n    product = None\nexcept ImportError:\n    \
         product = None\n",
    );
    assert!(error.message.contains(SHADOW), "{}", error.message);
    // A fallback `try` nested in an `if` is not a module-level `try`.
    let error = refused(
        "if True:\n    try:\n        from itertools import product\n    except ImportError:\n        \
         product = None\n",
    );
    assert!(error.message.contains(SHADOW), "{}", error.message);
}

/// A `try` that forms a group can still hold a stray import of the name in
/// an arm the group does not cover: a handler that does not catch a failed
/// import, the `else`, or the `finally`. `killed_names` records no import,
/// so the group test does not see it; the shadowing check refuses it
/// because only a pair of imports both recorded in the group, in different
/// arms, is exempt. It is refused at the first import of the pair.
#[test]
fn a_stray_import_beside_a_formed_group_stays_refused() {
    const TRY: &str = "try:\n    from itertools import product\n";
    for tail in [
        "except ValueError:\n    from functools import product\nexcept ImportError:\n    \
         product = None\n",
        "except ImportError:\n    product = None\nelse:\n    from functools import product\n",
        "except ImportError:\n    product = None\nfinally:\n    from functools import product\n",
        "except ImportError:\n    from operator import product\nelse:\n    \
         from functools import product\n",
        "except ImportError:\n    from operator import product\nfinally:\n    \
         from functools import product\n",
    ] {
        let source = format!("{TRY}{tail}");
        let error = refused(&source);
        assert_eq!(error.code, "C0001", "{source:?}");
        assert!(
            error.message.contains(SHADOW),
            "{source:?}: {}",
            error.message
        );
        assert_eq!(
            error.span,
            Some(span_of(&source, "from itertools")),
            "{source:?}"
        );
    }
}

/// Two different imports of the name in the same arm are the sequential
/// rebinding #1291 refuses, so they are refused at the first of the pair;
/// so are two `try` statements binding it to different objects, which a
/// name-keyed exemption would wrongly admit.
#[test]
fn two_imports_in_one_arm_or_two_trys_stay_refused() {
    for (source, at) in [
        (
            "try:\n    from itertools import product\n    from functools import product\n\
             except ImportError:\n    product = None\n",
            "from itertools",
        ),
        (
            "try:\n    from nosuch import product\nexcept ImportError:\n    \
             from itertools import product\n    from functools import product\n",
            "from itertools",
        ),
        (
            "try:\n    from itertools import product\nexcept ImportError:\n    product = None\n\
             try:\n    from functools import product\nexcept ImportError:\n    product = None\n",
            "from itertools",
        ),
    ] {
        let error = refused(source);
        assert!(
            error.message.contains(SHADOW),
            "{source:?}: {}",
            error.message
        );
        assert_eq!(error.span, Some(span_of(source, at)), "{source:?}");
    }
}

/// A native value in a qualifying handler is the tailored refusal, at the
/// `try` statement, and leaves none of the block's imports behind.
#[test]
fn a_native_fallback_value_is_the_tailored_refusal() {
    for value in ["3", "[]", "chain", "product + 1"] {
        let source = format!(
            "from itertools import chain\ntry:\n    from itertools import product\n\
             except ImportError:\n    product = {value}\n"
        );
        let error = refused(&source);
        assert_eq!(error.code, "C0001", "{source:?}");
        assert_eq!(
            error.message,
            "`product` is rebound in an `except ImportError` handler to a value other than \
             `None`; only a `None` or a fallback `import`/`from ... import` statement is \
             supported as the fallback of a foreign import yet",
            "{source:?}"
        );
        let start = source.find("try:").unwrap() as u32;
        assert_eq!(
            error.span,
            Some(Span::new(start, source.trim_end().len() as u32)),
            "{source:?}"
        );
    }
    // The augmented form desugars to the same assignment.
    let error =
        refused("try:\n    from itertools import product\nexcept ImportError:\n    product += 1\n");
    assert!(error.message.contains(NATIVE), "{}", error.message);
}
