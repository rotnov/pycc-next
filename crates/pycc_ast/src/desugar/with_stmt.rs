//! Non-strict mode: the `with` statement (v0.7 accept 2), as the statements
//! PEP 343 defines it by.
//!
//! ```python
//!                                   m = EXPR
//!                                   TARGET = m.__enter__()
//!                                   ok = True
//! with EXPR as TARGET:              try:
//!     BODY                  ==>         BODY
//!                                   except Exception as e:
//!                                       ok = False
//!                                       x = __pycc_exc_object(e)
//!                                       if not m.__exit__(type(x), x, x.__traceback__):
//!                                           raise
//!                                   finally:
//!                                       if ok:
//!                                           m.__exit__(None, None, None)
//! ```
//!
//! Several items nest, the first outermost, as CPython runs them. The
//! manager is any value: a CPython object (`open(...)`, a
//! `contextlib.contextmanager`), or a compiled instance whose class defines
//! `__enter__` and `__exit__`. `__exit__` receives the CPython exception
//! object (`pycc_hir::builtins::EXC_OBJECT`); a true result suppresses it.
//!
//! Two differences from CPython, recorded in `docs/DEVIATIONS.md`:
//! `__enter__` and `__exit__` are looked up on the manager (an instance
//! attribute shadowing the class's is used, CPython uses the type's), and a
//! `BaseException` that is not an `Exception` (`KeyboardInterrupt`,
//! `SystemExit`) leaving the body calls `__exit__(None, None, None)` and
//! cannot be suppressed. An `async with` stays refused (no `async def`
//! compiles).

use ruff_python_ast::{Stmt, StmtWith};
use ruff_text_size::{Ranged, TextRange};

/// The statements `with` means (module doc), or `None` for an `async
/// with`.
pub fn desugar_with(with: &StmtWith) -> Option<Vec<Stmt>> {
    if with.is_async {
        return None;
    }
    Some(items(with, 0))
}

/// The statements of items `index..` of `with`, around its body.
fn items(with: &StmtWith, index: usize) -> Vec<Stmt> {
    let Some(item) = with.items.get(index) else {
        return with.body.iter().cloned().collect();
    };
    let n = item.context_expr.range().start().to_u32();
    let m = format!("__pycc_with{n}");
    let enter = match &item.optional_vars {
        Some(_) => format!("__pycc_with_target{n} = {m}.__enter__()"),
        None => format!("{m}.__enter__()"),
    };
    let source = format!(
        "{m} = None\n{enter}\n__pycc_withok{n} = True\ntry:\n    pass\n\
         except Exception as __pycc_withexc{n}:\n    __pycc_withok{n} = False\n    \
         __pycc_withobj{n} = __pycc_exc_object(__pycc_withexc{n})\n    \
         if not {m}.__exit__(type(__pycc_withobj{n}), __pycc_withobj{n}, \
         __pycc_withobj{n}.__traceback__):\n        raise\n\
         finally:\n    if __pycc_withok{n}:\n        {m}.__exit__(None, None, None)\n"
    );
    let range = TextRange::new(item.range().start(), with.range().end());
    let mut stmts = super::signatures::parse_helper(&source, range);
    if let Stmt::Assign(assign) = &mut stmts[0] {
        *assign.value = item.context_expr.clone();
    }
    if let (Some(target), Stmt::Assign(assign)) = (&item.optional_vars, &mut stmts[1]) {
        assign.targets = vec![(**target).clone()];
    }
    if let Stmt::Try(try_stmt) = &mut stmts[3] {
        try_stmt.body = items(with, index + 1).into();
    }
    stmts
}
