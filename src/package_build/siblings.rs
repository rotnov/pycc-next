//! The calling side of the cross-module native calls of a package built
//! whole (v0.8): a member build that was given the interfaces of other
//! modules of its package (`--sibling-interface`, [`Producer`]) calls their
//! compiled functions natively.
//!
//! A module-level `from <sibling> import name` of a member runs in CPython
//! (an `--ext` module's imported names are module variables Python code
//! reads, so the fallback loop keeps them in the module dict), and the
//! compiled code calls the name as `__pycc_fallback_ns['name'](a, b)`. For
//! a name such an import binds -- once, and not bound otherwise at module
//! scope -- to a function the sibling's interface lists, every such call
//! that passes exactly the function's parameters positionally is rewritten,
//! before the module is lowered, to `_pycc_sib_name(a, b)`, a function added
//! at the top of the module:
//!
//! ```python
//! def _pycc_sib_name(_pycc_sib_a0: str, _pycc_sib_a1: int) -> str:
//!     return __pycc_fallback_ns['name'](_pycc_sib_a0, _pycc_sib_a1)
//! ```
//!
//! Its signature is the sibling's compiled one and its body the call
//! CPython makes; codegen calls the sibling's compiled function instead of
//! that body while the module dict still binds the name to the object the
//! sibling published for the function (`pycc_codegen::SiblingTarget`).
//! Only a non-strict module is rewritten: the body's `return` of an object
//! converts it to the declared type, as a non-strict boundary does.
//!
//! A call's native result type may make a statement fail to compile that
//! compiled with the object (a `str` element in a display of objects): the
//! fallback loop then compiles that statement without the rewrite before
//! letting it fall back ([`deny_failures`]).
//!
//! The added functions' source ranges lie past the end of the module's
//! source (a comment line as long as the source is parsed before them), so
//! no fallback statement or traceback line can be taken from the source
//! for them.

use super::interface::{Interface, InterfaceFn, content_hash, interface_ty};
use pycc_ast::visitor::transformer::{Transformer, walk_expr};
use pycc_ast::{Expr, ExprContext, ModModule, Ranged, Stmt};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

/// The prefix of an added function's parameters (reserved, as every
/// `_pycc_` name is).
const SIBLING_PARAM_PREFIX: &str = "_pycc_sib_a";

/// One module this member may call natively.
#[derive(Debug)]
pub(crate) struct Producer {
    pub(crate) interface: Interface,
    /// [`content_hash`] of the interface text read.
    pub(crate) hash: u64,
}

impl Producer {
    /// Reads `DOTTED=PATH`.
    pub(crate) fn read(entry: &str) -> Result<Self, String> {
        let (dotted, path) = entry
            .split_once('=')
            .ok_or_else(|| format!("`--sibling-interface {entry}` is not `DOTTED=PATH`"))?;
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read the interface of `{dotted}` (`{path}`): {e}"))?;
        let interface = Interface::parse(&text)
            .map_err(|e| format!("the interface of `{dotted}` (`{path}`): {e}"))?;
        if interface.module != dotted {
            return Err(format!(
                "`{path}` is the interface of `{}`, not of `{dotted}`",
                interface.module
            ));
        }
        Ok(Self {
            interface,
            hash: content_hash(&text),
        })
    }
}

/// The codegen targets of the last module [`apply_siblings`] rewrote.
static TARGETS: Mutex<Vec<pycc_codegen::SiblingTarget>> = Mutex::new(Vec::new());

/// The top-level statements (by [`statement_key`]) whose calls are not
/// rewritten ([`deny_failures`]).
static DENIED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// The source ranges and keys of the top-level statements the last
/// [`apply_siblings`] rewrote calls in.
static REWRITTEN: Mutex<Vec<(std::ops::Range<u32>, String)>> = Mutex::new(Vec::new());

/// What names a top-level statement for [`DENIED`]: a `def` or `class` by
/// its name, every other statement as the module's code.
fn statement_key(stmt: &Stmt) -> String {
    match stmt {
        Stmt::FunctionDef(def) => def.name.to_string(),
        Stmt::ClassDef(class) => class.name.to_string(),
        _ => "<module>".to_string(),
    }
}

/// The fallback loop's hook: when one of the frontend's failures (at the
/// source offsets `offsets` of the module the last attempt compiled) lies
/// in a top-level statement whose calls were rewritten, the next attempt
/// leaves that statement's calls as written instead of the statement
/// falling back. Whether any statement was newly left alone.
pub(crate) fn deny_failures(offsets: impl IntoIterator<Item = u32>) -> bool {
    let rewritten = REWRITTEN.lock().expect("the rewritten lock");
    let mut denied = DENIED.lock().expect("the denied lock");
    let mut new = false;
    for at in offsets {
        for (range, key) in rewritten.iter() {
            if range.contains(&at) && denied.insert(key.clone()) {
                new = true;
                if std::env::var_os("PYCC_FALLBACK_DUMP").is_some() {
                    eprintln!("--- calls to siblings left as written in `{key}` ---");
                }
            }
        }
    }
    new
}

/// What codegen needs for the functions [`apply_siblings`] last added.
pub(crate) fn codegen_targets() -> Vec<pycc_codegen::SiblingTarget> {
    TARGETS.lock().expect("the targets lock").clone()
}

/// The module an import names, relative to the member `dotted`
/// (`is_package` when it is a package's `__init__`), or `None` when it
/// climbs above the top-level package.
pub(super) fn imported_module(
    dotted: &str,
    is_package: bool,
    level: u32,
    module: Option<&str>,
) -> Option<String> {
    if level == 0 {
        return module.map(str::to_string);
    }
    let mut parts: Vec<&str> = dotted.split('.').collect();
    if !is_package {
        parts.pop();
    }
    for _ in 1..level {
        parts.pop()?;
    }
    if parts.is_empty() {
        return None;
    }
    let mut out = parts.join(".");
    if let Some(module) = module {
        out.push('.');
        out.push_str(module);
    }
    Some(out)
}

/// The module-scope `from ... import` statements of `body`, at top level or
/// in a module-level `if`/`try` block, but in no block `keep` refuses.
pub(super) fn module_imports_where<'a>(
    body: &'a [Stmt],
    out: &mut Vec<&'a pycc_ast::StmtImportFrom>,
    keep: &dyn Fn(&Stmt) -> bool,
) {
    for stmt in body {
        if !keep(stmt) {
            continue;
        }
        match stmt {
            Stmt::ImportFrom(import) => out.push(import),
            Stmt::If(node) => {
                module_imports_where(&node.body, out, keep);
                for clause in &node.elif_else_clauses {
                    module_imports_where(&clause.body, out, keep);
                }
            }
            Stmt::Try(node) => {
                module_imports_where(&node.body, out, keep);
                for handler in &node.handlers {
                    let pycc_ast::ExceptHandler::ExceptHandler(handler) = handler;
                    module_imports_where(&handler.body, out, keep);
                }
                module_imports_where(&node.orelse, out, keep);
                module_imports_where(&node.finalbody, out, keep);
            }
            _ => {}
        }
    }
}

/// The interface function an imported name binds: the producer's index in
/// the member's list, and the function's in the producer's interface.
#[derive(Clone, Copy)]
struct Candidate {
    producer: usize,
    function: usize,
}

/// Rewrites the calls `__pycc_fallback_ns['name'](...)` of candidates:
/// `__pycc_` names are never the program's, so every such read is the
/// module's own namespace.
struct Calls<'a> {
    producers: &'a [Producer],
    candidates: BTreeMap<String, Candidate>,
    /// The candidates some call was rewritten for.
    used: RefCell<BTreeSet<String>>,
    /// How many calls were rewritten.
    count: Cell<usize>,
}

impl Calls<'_> {
    fn function(&self, candidate: Candidate) -> &InterfaceFn {
        &self.producers[candidate.producer].interface.functions[candidate.function]
    }
}

impl Transformer for Calls<'_> {
    fn visit_expr(&self, expr: &mut Expr) {
        walk_expr(self, expr);
        let Expr::Call(call) = expr else {
            return;
        };
        let Expr::Subscript(subscript) = call.func.as_ref() else {
            return;
        };
        let (Expr::Name(ns), Expr::StringLiteral(key)) =
            (subscript.value.as_ref(), subscript.slice.as_ref())
        else {
            return;
        };
        if ns.id.as_str() != crate::fallback::NS {
            return;
        }
        let name = key.value.to_str().to_string();
        let Some(&candidate) = self.candidates.get(&name) else {
            return;
        };
        let arguments = &call.arguments;
        if !arguments.keywords.is_empty()
            || arguments.args.len() != self.function(candidate).params.len()
            || arguments
                .args
                .iter()
                .any(|arg| matches!(arg, Expr::Starred(_)))
        {
            return;
        }
        let range = call.func.range();
        *call.func = Expr::Name(pycc_ast::ExprName {
            node_index: Default::default(),
            range,
            id: format!("{}{name}", pycc_hir::SIBLING_CALL_PREFIX).into(),
            ctx: ExprContext::Load,
        });
        self.used.borrow_mut().insert(name);
        self.count.set(self.count.get() + 1);
    }
}

/// The source of the function a call of the imported `local` goes through.
fn added_function(local: &str, function: &InterfaceFn) -> String {
    let ty = |ty: &pycc_hir::Ty| interface_ty(ty).expect("listed types have a spelling");
    let params: Vec<String> = function
        .params
        .iter()
        .enumerate()
        .map(|(i, param)| format!("{SIBLING_PARAM_PREFIX}{i}: {}", ty(param)))
        .collect();
    let args: Vec<String> = (0..function.params.len())
        .map(|i| format!("{SIBLING_PARAM_PREFIX}{i}"))
        .collect();
    let call = format!("{}['{local}']({})", crate::fallback::NS, args.join(", "));
    let body = if function.ret == pycc_hir::Ty::None {
        call
    } else {
        format!("return {call}")
    };
    format!(
        "def {}{local}({}) -> {}:\n    {body}\n",
        pycc_hir::SIBLING_CALL_PREFIX,
        params.join(", "),
        ty(&function.ret)
    )
}

/// Where the added functions go: after the module's docstring and its
/// `from __future__` imports.
fn insertion_point(body: &[Stmt]) -> usize {
    let mut index = 0;
    if let Some(Stmt::Expr(expr)) = body.first()
        && matches!(expr.value.as_ref(), Expr::StringLiteral(_))
    {
        index = 1;
    }
    while let Some(Stmt::ImportFrom(import)) = body.get(index)
        && import.level == 0
        && import
            .module
            .as_ref()
            .is_some_and(|m| m.as_str() == "__future__")
    {
        index += 1;
    }
    index
}

/// The imported names of the member's module, whose source as written is
/// `original`, that a call may reach natively: bound once at module scope,
/// by a module-level `from <producer> import`, to a function the producer's
/// interface lists.
fn candidates(member: &super::Member, original: &str) -> BTreeMap<String, Candidate> {
    let Ok(parsed) = pycc_parser::parse_all(original) else {
        return BTreeMap::new();
    };
    let mut imports = Vec::new();
    module_imports_where(&parsed.body, &mut imports, &|_| true);
    // How many module-scope imports bind each name.
    let mut bindings: BTreeMap<String, usize> = BTreeMap::new();
    let mut found: BTreeMap<String, Candidate> = BTreeMap::new();
    for import in &imports {
        let module = imported_module(
            &member.dotted,
            member.is_package,
            import.level,
            import.module.as_ref().map(|m| m.as_str()),
        );
        let producer = module.and_then(|module| {
            member
                .producers
                .iter()
                .position(|producer| producer.interface.module == module)
        });
        for alias in &import.names {
            let local = alias.asname.as_ref().unwrap_or(&alias.name).to_string();
            *bindings.entry(local.clone()).or_insert(0) += 1;
            let Some(producer) = producer else {
                continue;
            };
            if let Some(function) = member.producers[producer]
                .interface
                .functions
                .iter()
                .position(|function| function.name == alias.name.as_str())
            {
                found.insert(local, Candidate { producer, function });
            }
        }
    }
    let defined = pycc_ast::namespace::module_bindings(&parsed).defined;
    found.retain(|local, _| bindings[local] == 1 && !defined.contains(local));
    found
}

/// Rewrites the calls of the member's module `parsed` -- the module as the
/// fallback loop compiles it, `source_len` bytes long; `original` is its
/// source as written -- that may reach a sibling's compiled function, as
/// the module docs say, and records the codegen targets. Does nothing
/// outside a member build, without producers, or in strict mode.
pub(crate) fn apply_siblings(
    parsed: &mut ModModule,
    original: &str,
    source_len: usize,
    non_strict: bool,
) {
    let mut targets = TARGETS.lock().expect("the targets lock");
    targets.clear();
    let Some(member) = super::member() else {
        return;
    };
    if member.producers.is_empty() || !non_strict {
        return;
    }
    let found = candidates(member, original);
    if found.is_empty() {
        return;
    }
    let calls = Calls {
        producers: &member.producers,
        candidates: found,
        used: RefCell::new(BTreeSet::new()),
        count: Cell::new(0),
    };
    let denied = DENIED.lock().expect("the denied lock").clone();
    let mut rewritten = REWRITTEN.lock().expect("the rewritten lock");
    rewritten.clear();
    for stmt in &mut parsed.body {
        let key = statement_key(stmt);
        if denied.contains(&key) {
            continue;
        }
        let before = calls.count.get();
        calls.visit_stmt(stmt);
        if calls.count.get() != before {
            let range = stmt.range();
            rewritten.push((range.start().into()..range.end().into(), key));
        }
    }
    drop(rewritten);
    let used = calls.used.borrow();
    if used.is_empty() {
        return;
    }
    let mut source = format!("#{}\n", " ".repeat(source_len.saturating_sub(1)));
    for local in used.iter() {
        let candidate = calls.candidates[local];
        let producer = &member.producers[candidate.producer];
        let function = calls.function(candidate);
        source.push_str(&added_function(local, function));
        targets.push(pycc_codegen::SiblingTarget {
            function: format!("{}{local}", pycc_hir::SIBLING_CALL_PREFIX),
            global: local.clone(),
            producer_symbol: format!("{}pyfn_{}", producer.interface.prefix, function.name),
            table_symbol: format!("{}pycc_ext_interface_table", producer.interface.prefix),
            index: candidate.function as u32,
        });
    }
    let added = pycc_parser::parse_all(&source)
        .expect("the added functions are well-formed Python")
        .body;
    let at = insertion_point(&parsed.body);
    parsed.body.splice(at..at, added);
}

/// Under `--report-fallbacks`: the calls of the member's module that reach
/// a sibling's compiled function directly, one line per imported name.
pub(crate) fn report(typed: &pycc_hir::HirModule) {
    let Some(member) = super::member() else {
        return;
    };
    let compiled: BTreeSet<&str> = typed
        .items
        .iter()
        .filter_map(|item| match item {
            pycc_hir::HirItem::Function { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    for target in codegen_targets() {
        if !compiled.contains(target.function.as_str()) {
            continue;
        }
        let producer = member
            .producers
            .iter()
            .find(|producer| target.table_symbol.starts_with(&producer.interface.prefix))
            .map_or("", |producer| producer.interface.module.as_str());
        eprintln!(
            "native call: `{}` calls `{producer}`'s compiled function while the name binds it",
            target.global
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_imports_resolve_against_the_member() {
        let resolve = |dotted, is_package, level, module| {
            imported_module(dotted, is_package, level, module).unwrap_or_default()
        };
        assert_eq!(resolve("p.a", false, 1, Some("b")), "p.b");
        assert_eq!(resolve("p.s", true, 1, Some("b")), "p.s.b");
        assert_eq!(resolve("p.s.a", false, 2, Some("b")), "p.b");
        assert_eq!(resolve("p.a", false, 1, None), "p");
        assert_eq!(resolve("p.a", false, 0, Some("q.r")), "q.r");
        assert_eq!(imported_module("p", true, 2, Some("b")), None);
        assert_eq!(imported_module("p", false, 1, Some("b")), None);
    }

    #[test]
    fn the_added_function_calls_the_imported_object() {
        let function = InterfaceFn {
            name: "f".to_string(),
            params: vec![pycc_hir::Ty::Str, pycc_hir::Ty::Int],
            ret: pycc_hir::Ty::Str,
        };
        assert_eq!(
            added_function("g", &function),
            "def _pycc_sib_g(_pycc_sib_a0: str, _pycc_sib_a1: int) -> str:\n    return \
             __pycc_fallback_ns['g'](_pycc_sib_a0, _pycc_sib_a1)\n"
        );
        let none = InterfaceFn {
            params: vec![],
            ret: pycc_hir::Ty::None,
            ..function
        };
        assert_eq!(
            added_function("g", &none),
            "def _pycc_sib_g() -> None:\n    __pycc_fallback_ns['g']()\n"
        );
    }

    #[test]
    fn added_functions_go_after_the_docstring_and_future_imports() {
        let parsed = pycc_parser::parse_all(
            "\"\"\"Doc.\"\"\"\nfrom __future__ import annotations\nimport x\n",
        )
        .unwrap();
        assert_eq!(insertion_point(&parsed.body), 2);
        let parsed = pycc_parser::parse_all("import x\n").unwrap();
        assert_eq!(insertion_point(&parsed.body), 0);
    }

    #[test]
    fn module_imports_include_if_and_try_blocks_only() {
        let parsed = pycc_parser::parse_all(
            "from a import b\nif c:\n    from d import e\nelse:\n    from f import g\ntry:\n    \
             from h import i\nexcept E:\n    from j import k\nelse:\n    from l import m\n\
             finally:\n    from n import o\ndef p():\n    from q import r\n",
        )
        .unwrap();
        let mut imports = Vec::new();
        module_imports_where(&parsed.body, &mut imports, &|_| true);
        let modules: Vec<&str> = imports
            .iter()
            .map(|import| import.module.as_ref().unwrap().as_str())
            .collect();
        assert_eq!(modules, ["a", "d", "f", "h", "j", "l", "n"]);
    }

    #[test]
    fn only_namespace_calls_with_the_parameters_are_rewritten() {
        let producers = [Producer {
            interface: Interface {
                module: "p.u".to_string(),
                prefix: "pycc_P1p1u_".to_string(),
                functions: vec![InterfaceFn {
                    name: "f".to_string(),
                    params: vec![pycc_hir::Ty::Int],
                    ret: pycc_hir::Ty::Int,
                }],
            },
            hash: 0,
        }];
        let calls = Calls {
            producers: &producers,
            candidates: [(
                "f".to_string(),
                Candidate {
                    producer: 0,
                    function: 0,
                },
            )]
            .into(),
            used: RefCell::new(BTreeSet::new()),
            count: Cell::new(0),
        };
        let mut parsed = pycc_parser::parse_all(
            "x = __pycc_fallback_ns['f'](__pycc_fallback_ns['f'](1))\n\
             y = __pycc_fallback_ns['f'](1, 2)\nz = __pycc_fallback_ns['f'](n=1)\n\
             w = __pycc_fallback_ns['g'](1)\nv = other['f'](1)\nu = f(1)\n",
        )
        .unwrap();
        calls.visit_body(&mut parsed.body);
        assert_eq!(calls.count.get(), 2);
        assert_eq!(calls.used.borrow().len(), 1);
    }
}
