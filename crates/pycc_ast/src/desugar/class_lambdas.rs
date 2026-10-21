//! Non-strict mode: a `lambda` bound in a class body becomes a method.
//!
//! `name = lambda self, x: ...` in a class body is a method CPython binds
//! like a `def`'s, `name = staticmethod(lambda x: ...)` a static method and
//! `name = classmethod(lambda cls, x: ...)` a class method; each is
//! written as the `def` (decorated `@staticmethod` / `@classmethod`) that
//! returns the lambda's body, so the class stays native. Their names
//! resolve as a lambda's do -- a class body's names are not in scope in
//! either -- and their defaults are evaluated where the class body runs.
//! Only a name the class body binds once is rewritten, and only a lambda
//! without `*args`, keyword-only parameters or `**kwargs`; the function's
//! `__name__` is then the attribute's name rather than `<lambda>`
//! (`docs/DEVIATIONS.md`).

use super::aliases::{inferred_def, rebound_names};
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::{
    AtomicNodeIndex, Decorator, Expr, ExprLambda, Parameters, Stmt, StmtClassDef,
};

/// Rewrites the lambdas of every class body (module doc).
pub(crate) fn class_lambdas(body: &mut [Stmt]) {
    if !super::dataclasses::is_non_strict() {
        return;
    }
    struct Walk;
    impl Transformer for Walk {
        fn visit_stmt(&self, stmt: &mut Stmt) {
            if let Stmt::ClassDef(class) = stmt {
                rewrite_class(class);
            }
            transformer::walk_stmt(self, stmt);
        }
    }
    for stmt in body.iter_mut() {
        Walk.visit_stmt(stmt);
    }
}

fn rewrite_class(class: &mut StmtClassDef) {
    let counts = rebound_names(&class.body);
    for stmt in class.body.iter_mut() {
        let Stmt::Assign(assign) = stmt else {
            continue;
        };
        let [Expr::Name(target)] = assign.targets.as_slice() else {
            continue;
        };
        let name = target.id.to_string();
        if counts.get(&name).copied().unwrap_or(0) != 1 {
            continue;
        }
        let (lambda, decorator) = match assign.value.as_ref() {
            Expr::Lambda(lambda) => (lambda, None),
            Expr::Call(call)
                if call.arguments.keywords.is_empty()
                    && matches!(&*call.func, Expr::Name(f)
                        if matches!(f.id.as_str(), "staticmethod" | "classmethod")) =>
            {
                let ([Expr::Lambda(lambda)], Expr::Name(f)) =
                    (&call.arguments.args[..], call.func.as_ref())
                else {
                    continue;
                };
                (lambda, Some(f.id.to_string()))
            }
            _ => continue,
        };
        if !plain(lambda) {
            continue;
        }
        let range = assign.range;
        let params = lambda.parameters.as_deref().cloned().unwrap_or_else(|| Parameters {
            range,
            node_index: AtomicNodeIndex::default(),
            posonlyargs: Vec::new().into(),
            args: Vec::new().into(),
            vararg: None,
            kwonlyargs: Vec::new().into(),
            kwarg: None,
        });
        let mut def = inferred_def(&name, params, (*lambda.body).clone(), range);
        // A method's unannotated return is inferred as written (non-strict
        // mode), with no marker.
        if let Stmt::FunctionDef(def) = &mut def {
            def.returns = None;
        }
        if let (Some(decorator), Stmt::FunctionDef(def)) = (decorator, &mut def) {
            def.decorator_list = vec![Decorator {
                node_index: AtomicNodeIndex::default(),
                range,
                expression: super::build::load(&decorator, range),
            }]
            .into();
        }
        *stmt = def;
    }
}

/// Whether `lambda`'s parameters are plain positional ones.
fn plain(lambda: &ExprLambda) -> bool {
    lambda.parameters.as_deref().is_none_or(|p| {
        p.vararg.is_none() && p.kwarg.is_none() && p.kwonlyargs.is_empty()
    })
}
