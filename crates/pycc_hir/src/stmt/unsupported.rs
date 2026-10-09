//! The `C0001` diagnostic `lower_stmt` reports for a statement kind none
//! of its arms lowers (issue #890), extracted from `stmt.rs` (#1291). A
//! nested `import` names its position: inside a function, or inside a
//! module-level block (#1291).

use crate::unsupported;
use pycc_ast::Stmt;
use pycc_diag::Diagnostic;

/// The diagnostic for `other`, a statement `lower_stmt` has no arm for.
/// `in_function` is `lower_stmt`'s own flag: it selects which of the two
/// nested-`import` texts applies.
pub(super) fn unsupported_statement(other: &Stmt, in_function: bool) -> Diagnostic {
    // Issue #890: name the rejected kind. Four kinds that *are*
    // supported at module level reach `lower_stmt`'s catch-all arm only
    // because they are nested, so naming them by kind alone would falsely say
    // "a function definition is not supported"; each gets a
    // position qualifier instead. Every top-level `Import`/
    // `ImportFrom` goes through `import::lower_import_stmt` before
    // `module::lower_all` ever calls `lower_stmt`, and an `if
    // TYPE_CHECKING:` body is constant-folded by `lower_stmt`, so an import
    // here is always inside a function or block body (a class-body
    // import is rejected by `class.rs` first and never gets here). A
    // CPython-backed `import` (#1291) or `from ... import` (#1383) in a
    // module-level `if`/`try` block never gets here either: `lower_stmt`
    // lowers it to `HirStmt::ForeignImport`. The function text is unchanged from before #1291.
    let kind = match other {
        Stmt::FunctionDef(_) => {
            "a `def` nested inside a function or block body \
             (only a module-level `def` or a method in a class body is supported)"
        }
        Stmt::ClassDef(_) => {
            "a `class` nested inside a function or block body \
             (only a module-level `class` is supported)"
        }
        Stmt::Import(_) | Stmt::ImportFrom(_) if in_function => {
            "an `import` inside a function or block body \
             (only a module-level import, or one inside an `if TYPE_CHECKING:` guard, is supported)"
        }
        Stmt::Import(_) | Stmt::ImportFrom(_) => {
            "an `import` inside a block body \
             (only a module-level import, a CPython-backed `import X`, `import X as Y` \
             or a CPython-backed `from X import a, b` inside a module-level `if` or `try` block, \
             or an import inside an \
             `if TYPE_CHECKING:` guard, is supported)"
        }
        _ => pycc_ast::stmt_kind_name(other),
    };
    unsupported(
        format!("statement kind not supported yet: {kind}"),
        pycc_ast::stmt_range(other),
    )
}
