//! Per-module namespaces for multi-module programs (v0.3 accept 1).
//!
//! A program of several project modules is linked into one flat item list
//! (`pycc_hir::link`), so two modules defining the same top-level name would
//! share one global. Instead, every top-level name a *dependency* module
//! (any project module but the entry) binds is renamed to a qualified
//! spelling no Python identifier can collide with: [`module_global`] of the
//! module's dotted name and the name. `helper.py`'s `def double` becomes
//! `0m6helper_double`, `pkg/sub.py`'s `X` becomes `0m3pkg3sub_X`. The
//! leading digit keeps the spelling out of every user namespace, and the
//! length-prefixed segments make it injective.
//!
//! An importer then reaches those names directly: `helper.double(2)`,
//! `from helper import double` and `pkg.sub.X` are all rewritten to the
//! qualified name, so a call between compiled modules is a direct call.
//! The driver (`src/modules/namespace.rs`) decides what each module-scope
//! name means; [`rewrite_module_globals`] applies those decisions with
//! Python's scoping rules (function locals, `global`/`nonlocal`, class
//! bodies, comprehensions and lambdas) so only references that resolve to
//! the module's own global scope change.
//!
//! [`display_name`] and [`demangle_text`] turn a qualified spelling back
//! into the dotted form CPython shows (`helper.double`), for diagnostics
//! and runtime names.

mod scope;

pub use scope::{
    ChainRewrite, GlobalRewrite, ModuleBindings, NameRewrite, RewriteError, module_bindings,
    rewrite_module_globals, statement_bindings,
};

use ruff_python_ast::name::Name;
use ruff_python_ast::{
    Alias, Arguments, AtomicNodeIndex, Expr, ExprCall, ExprContext, ExprName, ExprNumberLiteral,
    ExprStringLiteral, ExprSubscript, Identifier, Int, Number, Stmt, StmtAssign, StmtImportFrom,
    StmtRaise, StringLiteral, StringLiteralFlags, StringLiteralValue,
};
use ruff_text_size::TextRange;

/// The prefix of every qualified module-global spelling.
const PREFIX: &str = "0m";

/// The qualified spelling of top-level `name` of the module whose dotted
/// name is `module` (`helper`, `pkg.sub`): `0m`, then each dotted segment
/// as its byte length followed by the segment, then `_` and `name`.
pub fn module_global(module: &str, name: &str) -> String {
    let mut out = String::from(PREFIX);
    for segment in module.split('.') {
        out.push_str(&segment.len().to_string());
        out.push_str(segment);
    }
    out.push('_');
    out.push_str(name);
    out
}

/// The key a synthesized import statement names its module by
/// ([`synthesized_import`]): the module's qualified prefix with no name.
pub fn module_key(module: &str) -> String {
    module_global(module, "")
}

/// Splits a [`module_global`] spelling into the dotted module name and the
/// plain name, or `None` for any other identifier.
pub fn split_module_global(name: &str) -> Option<(String, &str)> {
    let mut rest = name.strip_prefix(PREFIX)?;
    let mut module = String::new();
    loop {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            break;
        }
        let len: usize = rest[..digits].parse().ok()?;
        let segment = rest.get(digits..digits + len)?;
        if segment.is_empty() {
            return None;
        }
        if !module.is_empty() {
            module.push('.');
        }
        module.push_str(segment);
        rest = &rest[digits + len..];
    }
    if module.is_empty() {
        return None;
    }
    let plain = rest.strip_prefix('_')?;
    Some((module, plain))
}

/// The name CPython shows for a qualified spelling: `helper.double` for
/// `0m6helper_double`, or `name` unchanged when it is not one.
pub fn display_name(name: &str) -> String {
    match split_module_global(name) {
        Some((module, plain)) => format!("{module}.{plain}"),
        None => name.to_string(),
    }
}

/// The plain (unqualified) name of a qualified spelling, or `name` itself.
pub fn plain_name(name: &str) -> &str {
    match split_module_global(name) {
        Some((_, plain)) => plain,
        None => name,
    }
}

/// `text` with every qualified spelling inside it replaced by its
/// [`plain_name`], for a diagnostic message: the name as the program
/// spells it where it is defined (`Point`, not `geometry.Point`).
pub fn demangle_text(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < text.len() {
        let at_boundary =
            index == 0 || !(bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'_');
        if at_boundary && text[index..].starts_with(PREFIX) {
            let end = text[index..]
                .char_indices()
                .find(|(_, c)| !(c.is_alphanumeric() || *c == '_'))
                .map_or(text.len(), |(offset, _)| index + offset);
            if let Some((module, plain)) = split_module_global(&text[index..end]) {
                // A module key (`synthesized_import`'s module) names the module.
                out.push_str(if plain.is_empty() { &module } else { plain });
                index = end;
                continue;
            }
        }
        let ch = text[index..]
            .chars()
            .next()
            .expect("index is on a char boundary");
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

/// A compile-time `from <module_key> import a, b` statement placed at
/// `range`: binds the qualified names a module reaches in another module
/// (`pycc_hir` answers it through `ResolvedImports::insert_synthesized`).
pub fn synthesized_import(module_key: &str, names: &[String], range: TextRange) -> Stmt {
    Stmt::ImportFrom(StmtImportFrom {
        node_index: AtomicNodeIndex::default(),
        range,
        module: Some(Identifier::new(Name::new(module_key), range)),
        names: names
            .iter()
            .map(|name| Alias {
                range,
                node_index: AtomicNodeIndex::default(),
                name: Identifier::new(Name::new(name), range),
                asname: None,
            })
            .collect(),
        level: 0,
        is_lazy: false,
    })
}

/// `target = source` (with `cell`, `target = source[0]`) at `range`: the
/// value copy `from m import x` makes when `x` is a variable of `m`
/// (CPython binds the value the name holds when the import runs).
pub fn snapshot_assign(target: &str, source: &str, cell: bool, range: TextRange) -> Stmt {
    let value = if cell {
        cell_element(source, ExprContext::Load, range)
    } else {
        Expr::Name(ExprName {
            node_index: AtomicNodeIndex::default(),
            range,
            id: Name::new(source),
            ctx: ExprContext::Load,
        })
    };
    Stmt::Assign(StmtAssign {
        node_index: AtomicNodeIndex::default(),
        range,
        targets: vec![Expr::Name(ExprName {
            node_index: AtomicNodeIndex::default(),
            range,
            id: Name::new(target),
            ctx: ExprContext::Store,
        })],
        value: Box::new(value),
    })
}

/// `name[0]`: the value of a module global that lives in a one-element
/// list because a function rebinds it (`desugar::globals`).
pub(crate) fn cell_element(name: &str, ctx: ExprContext, range: TextRange) -> Expr {
    Expr::Subscript(ExprSubscript {
        node_index: AtomicNodeIndex::default(),
        range,
        value: Box::new(Expr::Name(ExprName {
            node_index: AtomicNodeIndex::default(),
            range,
            id: Name::new(name),
            ctx: ExprContext::Load,
        })),
        slice: Box::new(Expr::NumberLiteral(ExprNumberLiteral {
            node_index: AtomicNodeIndex::default(),
            range,
            value: Number::Int(Int::from(0u64)),
        })),
        ctx,
    })
}

/// The prefix `desugar::globals` gives the cell of a module global a
/// function rebinds: `_pycc_global_<name>`.
pub const CELL_PREFIX: &str = "_pycc_global_";

/// A string assignment `target = "value"` at `range` (a dependency's own
/// `__name__`).
pub fn string_assign(target: &str, value: &str, range: TextRange) -> Stmt {
    Stmt::Assign(StmtAssign {
        node_index: AtomicNodeIndex::default(),
        range,
        targets: vec![Expr::Name(ExprName {
            node_index: AtomicNodeIndex::default(),
            range,
            id: Name::new(target),
            ctx: ExprContext::Store,
        })],
        value: Box::new(Expr::StringLiteral(ExprStringLiteral {
            node_index: AtomicNodeIndex::default(),
            range,
            value: StringLiteralValue::single(StringLiteral {
                range,
                node_index: AtomicNodeIndex::default(),
                value: value.into(),
                flags: StringLiteralFlags::empty(),
            }),
        })),
    })
}

/// `raise <class>("<message>")` at `range`: what a `from m import x`
/// runs when the partially initialized module `m` (a circular import) has
/// not bound `x` yet.
pub fn raise_with_message(class: &str, message: &str, range: TextRange) -> Stmt {
    let name = Expr::Name(ExprName {
        node_index: AtomicNodeIndex::default(),
        range,
        id: Name::new(class),
        ctx: ExprContext::Load,
    });
    let message = Expr::StringLiteral(ExprStringLiteral {
        node_index: AtomicNodeIndex::default(),
        range,
        value: StringLiteralValue::single(StringLiteral {
            range,
            node_index: AtomicNodeIndex::default(),
            value: message.into(),
            flags: StringLiteralFlags::empty(),
        }),
    });
    Stmt::Raise(StmtRaise {
        node_index: AtomicNodeIndex::default(),
        range,
        exc: Some(Box::new(Expr::Call(ExprCall {
            node_index: AtomicNodeIndex::default(),
            range,
            func: Box::new(name),
            arguments: Arguments {
                range,
                node_index: AtomicNodeIndex::default(),
                args: vec![message].into_boxed_slice(),
                keywords: Default::default(),
            },
        }))),
        cause: None,
    })
}

#[cfg(test)]
mod tests;
