//! Names reserved for the compiler's own rewrites.
//!
//! Desugaring introduces names a program must not be able to reach: the
//! function-value marker [`crate::CLOSURE_MARKER`], and `_pycc_`-prefixed
//! lifted functions, cells and temporaries. A program naming one could
//! forge a function value binding more captured values than its function
//! has parameters, or alias a compiler temporary. Every binding or read of
//! such a name in user source is reported, before desugaring runs.

use crate::CLOSURE_MARKER;
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{Expr, ModModule, Pattern, Stmt};
use ruff_text_size::{Ranged, TextRange};

/// Whether `name` is reserved for the compiler.
pub fn is_reserved_name(name: &str) -> bool {
    name == CLOSURE_MARKER || name.starts_with("_pycc_")
}

/// Every use of a reserved name ([`is_reserved_name`]) in `module`, as
/// the name and its byte range, in source order.
pub fn reserved_name_uses(module: &ModModule) -> Vec<(String, std::ops::Range<u32>)> {
    struct Find(Vec<(String, std::ops::Range<u32>)>);
    impl Find {
        fn check(&mut self, name: &str, range: TextRange) {
            if is_reserved_name(name) {
                self.0
                    .push((name.to_string(), range.start().into()..range.end().into()));
            }
        }
    }
    impl<'a> Visitor<'a> for Find {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(def) => {
                    self.check(def.name.as_str(), def.name.range());
                    for param in def.parameters.iter() {
                        self.check(param.name().as_str(), param.name().range());
                    }
                }
                Stmt::ClassDef(def) => self.check(def.name.as_str(), def.name.range()),
                Stmt::Import(import) => {
                    for alias in &import.names {
                        let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                        self.check(bound.split('.').next().unwrap_or(""), bound.range());
                    }
                }
                Stmt::ImportFrom(import) => {
                    for alias in &import.names {
                        let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                        self.check(bound.as_str(), bound.range());
                    }
                }
                Stmt::Global(global) => {
                    for name in &global.names {
                        self.check(name.as_str(), name.range());
                    }
                }
                Stmt::Nonlocal(nonlocal) => {
                    for name in &nonlocal.names {
                        self.check(name.as_str(), name.range());
                    }
                }
                Stmt::Try(try_stmt) => {
                    for handler in &try_stmt.handlers {
                        let ruff_python_ast::ExceptHandler::ExceptHandler(h) = handler;
                        if let Some(name) = &h.name {
                            self.check(name.as_str(), name.range());
                        }
                    }
                }
                _ => {}
            }
            visitor::walk_stmt(self, stmt);
        }

        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Name(name) => self.check(name.id.as_str(), name.range),
                Expr::Lambda(lambda) => {
                    for param in lambda.parameters.iter().flat_map(|p| p.iter()) {
                        self.check(param.name().as_str(), param.name().range());
                    }
                }
                _ => {}
            }
            visitor::walk_expr(self, expr);
        }

        fn visit_pattern(&mut self, pattern: &'a Pattern) {
            let name = match pattern {
                Pattern::MatchAs(p) => p.name.as_ref(),
                Pattern::MatchStar(p) => p.name.as_ref(),
                Pattern::MatchMapping(p) => p.rest.as_ref(),
                _ => None,
            };
            if let Some(name) = name {
                self.check(name.as_str(), name.range());
            }
            visitor::walk_pattern(self, pattern);
        }

        fn visit_type_param(&mut self, type_param: &'a ruff_python_ast::TypeParam) {
            self.check(type_param.name().as_str(), type_param.name().range());
            visitor::walk_type_param(self, type_param);
        }
    }
    let mut find = Find(Vec::new());
    for stmt in &module.body {
        find.visit_stmt(stmt);
    }
    find.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uses(source: &str) -> Vec<String> {
        let module = ruff_python_parser::parse_module(source)
            .expect("the source parses")
            .into_syntax();
        reserved_name_uses(&module)
            .into_iter()
            .map(|(name, range)| {
                assert_eq!(&source[range.start as usize..range.end as usize], name);
                name
            })
            .collect()
    }

    #[test]
    fn every_binding_and_read_of_a_reserved_name_is_found() {
        assert_eq!(
            uses(
                "def f(_pycc_a: int) -> int:\n    return __pycc_closure__(g, 1)\n\n\
                 _pycc_x = 1\nimport os as _pycc_os\nfrom m import _pycc_y\n\
                 class _pycc_C:\n    pass\n\n\
                 try:\n    pass\nexcept E as _pycc_e:\n    pass\n\
                 match v:\n    case [_pycc_p, *_pycc_r]:\n        pass\n    case {**_pycc_m}:\n        pass\n\
                 type _pycc_T = int\n\
                 def g[_pycc_U](x: int) -> int:\n    global _pycc_z\n    return x\n\
                 h = lambda _pycc_l: 0\n"
            ),
            [
                "_pycc_a",
                "__pycc_closure__",
                "_pycc_x",
                "_pycc_os",
                "_pycc_y",
                "_pycc_C",
                "_pycc_e",
                "_pycc_p",
                "_pycc_r",
                "_pycc_m",
                "_pycc_T",
                "_pycc_U",
                "_pycc_z",
                "_pycc_l",
            ]
        );
    }

    #[test]
    fn ordinary_and_fallback_names_are_not_reserved() {
        assert!(uses("pycc_x = 1\n__pycc_fallback_ns = {}\nx._pycc_attr = 2\n").is_empty());
        assert!(!is_reserved_name("__pycc_infer__x"));
        assert!(is_reserved_name("__pycc_closure__"));
    }
}
