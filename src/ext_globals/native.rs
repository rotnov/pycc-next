//! Which module globals hold a native value, read from the source alone.
//!
//! A strict module gives every global one type, so one module-level store
//! whose annotation or value is evidently native settles it. Anything not
//! evident -- an import, a foreign call, an operator on unknown operands --
//! is treated as a possible CPython object and left unpublished
//! (`super`'s module documentation says why). An instance of the module's
//! own class or an enum member crosses the boundary as a CPython object
//! too, so it is not published either.

use pycc_ast::{Expr, Stmt};
use std::collections::{BTreeMap, BTreeSet};

/// Annotations naming a native scalar.
const SCALARS: &[&str] = &["int", "float", "str", "bool", "bytes", "complex", "None"];

/// Builtins whose result is native whatever their arguments are.
const NATIVE_BUILTINS: &[&str] = &[
    "int", "float", "str", "bool", "len", "repr", "ord", "chr", "hex", "oct", "bin", "ascii",
];

/// The module's own definitions an expression can evidently produce a
/// native value from.
#[derive(Default)]
struct Module {
    /// Functions annotated to return a native value.
    native_functions: BTreeSet<String>,
}

impl Module {
    fn scan(body: &[Stmt]) -> Self {
        let mut module = Module::default();
        for stmt in body {
            if let Stmt::FunctionDef(def) = stmt
                && def
                    .returns
                    .as_deref()
                    .is_some_and(|returns| module.native_annotation(returns))
            {
                module.native_functions.insert(def.name.to_string());
            }
        }
        module
    }

    /// Whether `annotation` names a native type: a scalar, a tuple or
    /// union of those.
    fn native_annotation(&self, annotation: &Expr) -> bool {
        match annotation {
            Expr::Name(name) => SCALARS.contains(&name.id.as_str()),
            Expr::NoneLiteral(_) => true,
            Expr::BinOp(op) => self.native_annotation(&op.left) && self.native_annotation(&op.right),
            Expr::Subscript(sub) => {
                matches!(&*sub.value, Expr::Name(name) if matches!(name.id.as_str(), "tuple" | "Optional"))
                    && match &*sub.slice {
                        Expr::Tuple(tuple) => tuple.elts.iter().all(|elt| {
                            matches!(elt, Expr::EllipsisLiteral(_)) || self.native_annotation(elt)
                        }),
                        other => self.native_annotation(other),
                    }
            }
            _ => false,
        }
    }

    /// Whether `expr` evidently evaluates to a native value, given the
    /// names already known to hold one.
    fn native_value(&self, expr: &Expr, known: &BTreeSet<String>) -> bool {
        match expr {
            Expr::NumberLiteral(_)
            | Expr::StringLiteral(_)
            | Expr::BytesLiteral(_)
            | Expr::BooleanLiteral(_)
            | Expr::NoneLiteral(_)
            | Expr::FString(_) => true,
            Expr::Name(name) => known.contains(name.id.as_str()),
            Expr::UnaryOp(op) => self.native_value(&op.operand, known),
            Expr::BinOp(op) => {
                self.native_value(&op.left, known) && self.native_value(&op.right, known)
            }
            Expr::BoolOp(op) => op.values.iter().all(|value| self.native_value(value, known)),
            Expr::Compare(compare) => {
                self.native_value(&compare.left, known)
                    && compare
                        .comparators
                        .iter()
                        .all(|value| self.native_value(value, known))
            }
            Expr::If(choice) => {
                self.native_value(&choice.body, known) && self.native_value(&choice.orelse, known)
            }
            Expr::Tuple(tuple) => tuple.elts.iter().all(|elt| self.native_value(elt, known)),
            Expr::Call(call) => match &*call.func {
                Expr::Name(name) => {
                    let name = name.id.as_str();
                    self.native_functions.contains(name)
                        || (NATIVE_BUILTINS.contains(&name) && !known.contains(name))
                }
                _ => false,
            },
            _ => false,
        }
    }
}

/// The module-level names some module-level store evidently binds to a
/// native value.
pub(super) fn native_names(body: &[Stmt]) -> BTreeSet<String> {
    let module = Module::scan(body);
    // Each module-level store: its targets and value, or its annotation.
    let mut stores: BTreeMap<String, Vec<Store<'_>>> = BTreeMap::new();
    collect(body, &mut stores);
    let mut known = BTreeSet::new();
    loop {
        let before = known.len();
        for (name, kinds) in &stores {
            if known.contains(name) {
                continue;
            }
            if kinds.iter().any(|store| match store {
                Store::Annotated(annotation) => module.native_annotation(annotation),
                Store::Value(value) => module.native_value(value, &known),
            }) {
                known.insert(name.clone());
            }
        }
        if known.len() == before {
            return known;
        }
    }
}

enum Store<'a> {
    Annotated(&'a Expr),
    Value(&'a Expr),
}

/// The module-level stores of `body`, compound statements' bodies
/// included (not functions' or classes').
fn collect<'a>(body: &'a [Stmt], stores: &mut BTreeMap<String, Vec<Store<'a>>>) {
    for stmt in body {
        match stmt {
            Stmt::Assign(assign) => {
                for target in &assign.targets {
                    bind(target, &assign.value, stores);
                }
            }
            Stmt::AnnAssign(assign) => {
                if let Expr::Name(name) = &*assign.target {
                    stores
                        .entry(name.id.to_string())
                        .or_default()
                        .push(Store::Annotated(&assign.annotation));
                }
            }
            Stmt::If(s) => {
                collect(&s.body, stores);
                for clause in &s.elif_else_clauses {
                    collect(&clause.body, stores);
                }
            }
            Stmt::While(s) => {
                collect(&s.body, stores);
                collect(&s.orelse, stores);
            }
            Stmt::For(s) => {
                collect(&s.body, stores);
                collect(&s.orelse, stores);
            }
            Stmt::With(s) => collect(&s.body, stores),
            Stmt::Try(s) => {
                collect(&s.body, stores);
                collect(&s.orelse, stores);
                collect(&s.finalbody, stores);
                for handler in &s.handlers {
                    let pycc_ast::ExceptHandler::ExceptHandler(h) = handler;
                    collect(&h.body, stores);
                }
            }
            Stmt::Match(s) => {
                for case in &s.cases {
                    collect(&case.body, stores);
                }
            }
            _ => {}
        }
    }
}

/// Records `target = value`, element-wise for a tuple target assigned a
/// tuple of the same length.
fn bind<'a>(target: &'a Expr, value: &'a Expr, stores: &mut BTreeMap<String, Vec<Store<'a>>>) {
    match (target, value) {
        (Expr::Name(name), value) => stores
            .entry(name.id.to_string())
            .or_default()
            .push(Store::Value(value)),
        (Expr::Tuple(targets), Expr::Tuple(values)) if targets.elts.len() == values.elts.len() => {
            for (target, value) in targets.elts.iter().zip(values.elts.iter()) {
                bind(target, value, stores);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(source: &str) -> Vec<String> {
        let module = pycc_parser::parse(source).expect("parses");
        let body: Vec<Stmt> = module.body.iter().cloned().collect();
        native_names(&body).into_iter().collect()
    }

    #[test]
    fn literals_annotations_and_own_definitions_are_native() {
        let source = "import json\n\ndef make() -> int:\n    return 1\n\n\
                      A = 1\nB: float = json.loads('1')\nC = (A + 2, 'x')\n\
                      F = make()\nG = str(json)\nH = A if A else 2\nI, J = 1, 'j'\n\
                      K = J\nL: tuple[int, str | None] = (1, None)\n";
        assert_eq!(names(source), ["A", "B", "C", "F", "G", "H", "I", "J", "K", "L"]);
    }

    #[test]
    fn imports_and_foreign_values_are_not() {
        let source = "import json\nimport m as s\nfrom enum import Enum\n\n\
                      class Color(Enum):\n    RED = 1\n\nclass Box:\n    pass\n\n\
                      X = s.Thing()\nY = json\nZ = X + 1\nW = [1, 2]\nV = s.N\nU = Box()\n\
                      T = Color.RED\nS: Box = Box()\n";
        assert!(names(source).is_empty(), "{:?}", names(source));
    }
}
