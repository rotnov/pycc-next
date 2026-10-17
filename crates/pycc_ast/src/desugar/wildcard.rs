//! `from M import *` for a standard module whose public names are known:
//! the names the module reads that `M` exports, imported explicitly.
//!
//! CPython binds every public name of `M`; a name the program never reads
//! is unobservable, so importing only the read ones is equivalent. A
//! module this table does not know keeps its wildcard (and its `C0001`).

use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{Alias, AtomicNodeIndex, Expr, ExprContext, Identifier, Stmt};
use std::collections::BTreeSet;

/// The names `from M import *` binds in CPython 3.13 (`__all__`, else the
/// public attributes).
fn exports(module: &str) -> Option<&'static [&'static str]> {
    Some(match module {
        "math" => &[
            "acos", "acosh", "asin", "asinh", "atan", "atan2", "atanh", "cbrt", "ceil", "comb",
            "copysign", "cos", "cosh", "degrees", "dist", "e", "erf", "erfc", "exp", "exp2",
            "expm1", "fabs", "factorial", "floor", "fma", "fmod", "frexp", "fsum", "gamma", "gcd",
            "hypot", "inf", "isclose", "isfinite", "isinf", "isnan", "isqrt", "lcm", "ldexp",
            "lgamma", "log", "log10", "log1p", "log2", "modf", "nan", "nextafter", "perm", "pi",
            "pow", "prod", "radians", "remainder", "sin", "sinh", "sqrt", "sumprod", "tan", "tanh",
            "tau", "trunc", "ulp",
        ],
        "sys" => &[
            "argv", "exit", "getrecursionlimit", "maxsize", "setrecursionlimit", "stderr",
            "stdin", "stdout", "version", "version_info", "platform", "path", "modules",
            "gettrace", "settrace", "getsizeof", "intern", "flags", "float_info", "int_info",
            "byteorder", "executable", "implementation", "set_int_max_str_digits",
            "get_int_max_str_digits", "exc_info", "exception", "hexversion", "maxunicode",
        ],
        "itertools" => &[
            "accumulate", "batched", "chain", "combinations", "combinations_with_replacement",
            "compress", "count", "cycle", "dropwhile", "filterfalse", "groupby", "islice",
            "pairwise", "permutations", "product", "repeat", "starmap", "takewhile", "tee",
            "zip_longest",
        ],
        "bisect" => &[
            "bisect", "bisect_left", "bisect_right", "insort", "insort_left", "insort_right",
        ],
        "heapq" => &[
            "heapify", "heappop", "heappush", "heappushpop", "heapreplace", "merge", "nlargest",
            "nsmallest",
        ],
        "collections" => &[
            "ChainMap", "Counter", "OrderedDict", "UserDict", "UserList", "UserString",
            "defaultdict", "deque", "namedtuple",
        ],
        "functools" => &[
            "WRAPPER_ASSIGNMENTS", "WRAPPER_UPDATES", "cache", "cached_property", "cmp_to_key",
            "lru_cache", "partial", "partialmethod", "reduce", "singledispatch",
            "singledispatchmethod", "total_ordering", "update_wrapper", "wraps",
        ],
        _ => return None,
    })
}

/// Every name `body` reads.
fn read_names(body: &[Stmt]) -> BTreeSet<String> {
    struct Collect(BTreeSet<String>);
    impl<'a> Visitor<'a> for Collect {
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Name(name) = expr
                && name.ctx == ExprContext::Load
            {
                self.0.insert(name.id.to_string());
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut collect = Collect(BTreeSet::new());
    for stmt in body {
        collect.visit_stmt(stmt);
    }
    collect.0
}

/// Every name bound at module scope outside function and class bodies
/// (assignment and loop targets, imports, `def`/`class` names).
fn module_bound_names(body: &[Stmt]) -> BTreeSet<String> {
    struct Collect(BTreeSet<String>);
    impl<'a> Visitor<'a> for Collect {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(def) => {
                    self.0.insert(def.name.to_string());
                    return;
                }
                Stmt::ClassDef(def) => {
                    self.0.insert(def.name.to_string());
                    return;
                }
                Stmt::Import(import) => {
                    for alias in &import.names {
                        let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                        self.0.insert(bound.split('.').next().unwrap_or("").to_string());
                    }
                }
                Stmt::ImportFrom(import) => {
                    for alias in &import.names {
                        self.0.insert(alias.asname.as_ref().unwrap_or(&alias.name).to_string());
                    }
                }
                _ => {}
            }
            visitor::walk_stmt(self, stmt);
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Name(name) if name.ctx == ExprContext::Store => {
                    self.0.insert(name.id.to_string());
                }
                // A comprehension's or lambda's own names are its own.
                Expr::Lambda(_)
                | Expr::ListComp(_)
                | Expr::SetComp(_)
                | Expr::DictComp(_)
                | Expr::Generator(_) => return,
                _ => {}
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut collect = Collect(BTreeSet::new());
    for stmt in body {
        collect.visit_stmt(stmt);
    }
    collect.0
}

/// Rewrites each top-level `from M import *` of a known module `M`. A name
/// the module also binds itself is left out: importing it would make one
/// global hold two types, and a read of it before the module's own binding
/// is then refused at compile time rather than run.
pub(crate) fn expand_wildcards(body: &mut Vec<Stmt>) {
    let mut read = read_names(body);
    for bound in module_bound_names(body) {
        read.remove(&bound);
    }
    body.retain_mut(|stmt| {
        let Stmt::ImportFrom(import) = stmt else {
            return true;
        };
        let wildcard = import.level == 0
            && import.names.len() == 1
            && import.names[0].name.as_str() == "*";
        let Some(names) = wildcard
            .then_some(import.module.as_ref())
            .flatten()
            .and_then(|module| exports(module.as_str()))
        else {
            return true;
        };
        let range = import.names[0].range;
        let used: Vec<Alias> = names
            .iter()
            .filter(|name| read.contains(**name))
            .map(|name| Alias {
                range,
                node_index: AtomicNodeIndex::default(),
                name: Identifier::new(*name, range),
                asname: None,
            })
            .collect();
        if used.is_empty() {
            return false;
        }
        import.names = used;
        true
    });
}
