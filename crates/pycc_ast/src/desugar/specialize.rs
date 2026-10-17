//! Guarded native specializations of an `--ext --non-strict` module's
//! unannotated functions and methods.
//!
//! Python may call an exported function with anything, so an unannotated
//! parameter of one is a CPython object, and every operation on it goes
//! through the C API. Most untyped code is called with one type per
//! parameter, though, and its body says which: a parameter that is indexed,
//! sliced, measured with `len()` or given a `str` method is text; one used
//! as an index, a slice bound or a `range()` argument, or in arithmetic with
//! an integer literal, is an `int`. For such a function this pass adds a
//! *native specialization*, a copy of the `def` with those parameters
//! annotated, and puts a guard at the top of the original:
//!
//! ```python
//! def diff_bisect(self, text1, text2, deadline):
//!     if type(text1) is builtins.str and type(text2) is builtins.str:
//!         return self._pycc_native_diff_bisect(text1, text2, deadline)
//!     ...                                     # the object path, unchanged
//! def _pycc_native_diff_bisect(self, text1: str, text2: str, deadline: object) -> object:
//!     ...                                     # the same body, native
//! ```
//!
//! The guard tests the *exact* type, so a subclass of `str` (which may
//! override any operation) keeps the object path, and the specialization
//! runs only on the values its annotations describe: a guess that is wrong
//! costs a failed test per call, never a different result. Immutable values
//! are guessed (`str`, `int`), and `bytearray`, the one mutable type whose
//! native form is the CPython object itself: converting a CPython `list`
//! into a native one would copy it, and the caller would no longer see the
//! function's stores into it, but a `bytearray` specialization reads and
//! writes the object's own buffer (`pycc_hir::ndarray`'s byte buffers).
//!
//! A parameter is guessed a `bytearray` when an item of it is used as a
//! number: `s[i] - 1`, `(x + y) & 0xff` where `x = s[i]`, `abs(s[i])`. A
//! `str` item is never one (`str - int`, `str & int` raise), so that use
//! outweighs the sequence evidence that would otherwise guess `str`.
//!
//! The copy keeps the source ranges of the original, so a traceback through
//! it names the original's lines. The fallback planner may have routed a
//! builtin call of the original through CPython (`builtins.abs(x)`, since
//! `abs` of an object is not compiled); the copy calls the builtin itself
//! again (`abs(x)`), which its typed arguments may well compile, unless the
//! driver says otherwise (`keep_builtins`, after the copy failed with it). A specialization the checker refuses is
//! dropped by the driver (`skip`), which then re-runs this pass without it;
//! [`Specialized`] records what each one covers for that attribution.
//!
//! Only a function that loops (a `for`, a `while` or a comprehension) is
//! specialized: converting a `str` argument copies it, which a body that
//! does constant work would not win back.
//!
//! A module function with a parameter annotated `Iterable[T]` (`T` a
//! scalar type) gets a second kind of specialization: Python may pass it
//! any iterable, which only the object holds, but the module's own calls
//! usually pass a `frozenset[T]` constant. Its copy takes `frozenset[T]`
//! there and has no guard: `pycc_mir` calls it in place of the original
//! from a compiled call whose arguments have exactly its parameter types,
//! so the frozenset is never converted to a CPython object.

use super::build::name;
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{Expr, ExprContext, Identifier, ModModule, Number, Stmt, StmtFunctionDef};
use ruff_text_size::{Ranged, TextRange};

/// The prefix of a specialization's name.
pub const NATIVE_PREFIX: &str = "_pycc_native_";

/// The module-level name the guards read CPython's `builtins` module
/// through: `type(x) is str` on a CPython object compares with the `str`
/// type object, which compiled code reaches as an attribute of the module.
const BUILTINS: &str = "__pycc_specialize_builtins";

/// The types a guard tests for, and the prefix of the module globals that
/// hold them (`__pycc_specialize_int = builtins.int`).
const GUARD_TYPES: [&str; 3] = ["int", "str", "bytearray"];
const GUARD_TYPE_PREFIX: &str = "__pycc_specialize_";

/// The annotation a `bytearray` guess writes into the copy: the byte
/// buffer type, which only the specializer spells (a program's own
/// `bytearray` annotation keeps meaning what it did).
pub const BYTEARRAY_ANNOTATION: &str = "__pycc_bytearray";

/// The name the fallback planner's preamble binds CPython's `builtins`
/// module to, through which it routes a builtin call pycc does not compile.
pub const FALLBACK_BUILTINS: &str = "__pycc_fallback_builtins";

/// One planned specialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Specialized {
    /// `f`, or `Class.method`.
    pub qualname: String,
    /// The original `def`'s range, which its copy shares.
    pub range: TextRange,
    /// The guessed type of each specialized parameter.
    pub params: Vec<(String, &'static str)>,
    /// Whether the copy calls a builtin its original calls through
    /// [`FALLBACK_BUILTINS`] (module doc).
    pub restored_builtins: bool,
}

/// Adds the guarded specializations of `module`'s unannotated top-level
/// functions and methods, except those `skip` names, and returns them. The
/// copies of those `keep_builtins` names keep calling builtins as their
/// originals do (module doc).
pub fn specialize_untyped(
    module: &mut ModModule,
    skip: &dyn Fn(&str) -> bool,
    keep_builtins: &dyn Fn(&str) -> bool,
) -> Vec<Specialized> {
    // The guard spells `type` (its types are attributes of the `builtins`
    // module it imports under its own name) and a copy's annotations spell
    // the guessed types: a module that binds one of them at module level is
    // left alone, and so is a function that binds `type` itself (`plan`).
    // A function binding `type` locally (`type, data = chunk`) shadows it
    // only there.
    if ["type", "str", "int", "bytearray"]
        .iter()
        .any(|builtin| binds_at_module_level(&module.body, builtin))
    {
        return Vec::new();
    }
    // The builtins the attribute snapshots' analysis takes as native
    // (`attrs`) the module does not rebind.
    let module_bound: std::collections::HashSet<&str> = attrs::BUILTINS_READ
        .iter()
        .copied()
        .filter(|builtin| binds_at_module_level(&module.body, builtin))
        .collect();
    let module_binds = |name: &str| module_bound.contains(name);
    let call_sites = call_sites::CallSites::of(module);
    let mut out = Vec::new();
    let old = std::mem::take(&mut module.body);
    let mut body = Vec::with_capacity(old.len());
    for stmt in old {
        match stmt {
            Stmt::FunctionDef(mut def) => {
                let planned = plan(&def, None, skip, keep_builtins, &module_binds, &call_sites);
                if planned.is_none()
                    && let Some((clone, spec)) = plan_iterable(&def, skip)
                {
                    body.push(Stmt::FunctionDef(def));
                    body.push(Stmt::FunctionDef(clone));
                    out.push(spec);
                    continue;
                }
                match planned {
                    Some((clone, spec)) => {
                        add_guard(&mut def, None, &spec);
                        body.push(Stmt::FunctionDef(def));
                        body.push(Stmt::FunctionDef(clone));
                        out.push(spec);
                    }
                    None => body.push(Stmt::FunctionDef(def)),
                }
            }
            Stmt::ClassDef(mut class) => {
                let class_name = class.name.to_string();
                let facts = attrs::ClassFacts::of(&class);
                let old = std::mem::take(&mut class.body);
                let mut members = Vec::with_capacity(old.len());
                for item in old {
                    match item {
                        Stmt::FunctionDef(mut def) => {
                            match plan(
                                &def,
                                Some((&class_name, &facts)),
                                skip,
                                keep_builtins,
                                &module_binds,
                                &call_sites,
                            ) {
                                Some((clone, spec)) => {
                                    add_guard(&mut def, Some(&class_name), &spec);
                                    members.push(Stmt::FunctionDef(def));
                                    members.push(Stmt::FunctionDef(clone));
                                    out.push(spec);
                                }
                                None => members.push(Stmt::FunctionDef(def)),
                            }
                        }
                        other => members.push(other),
                    }
                }
                class.body = members.into();
                body.push(Stmt::ClassDef(class));
            }
            other => body.push(other),
        }
    }
    if !out.is_empty() {
        // After the docstring and the `__future__` imports.
        let mut at = usize::from(matches!(body.first(), Some(Stmt::Expr(e))
            if e.value.is_string_literal_expr()));
        while matches!(body.get(at), Some(Stmt::ImportFrom(import))
            if import.module.as_ref().map(|m| m.as_str()) == Some("__future__"))
        {
            at += 1;
        }
        // The guards' types are read once, into globals of their own: an
        // attribute of the `builtins` module read at every call cost more
        // than the rest of the guard.
        let mut source = format!("import builtins as {BUILTINS}\n");
        for ty in GUARD_TYPES {
            source.push_str(&format!("{GUARD_TYPE_PREFIX}{ty} = {BUILTINS}.{ty}\n"));
        }
        let import = super::signatures::parse_helper(&source, TextRange::default());
        for (offset, stmt) in import.into_iter().enumerate() {
            body.insert(at + offset, stmt);
        }
    }
    module.body = body.into();
    out
}

/// Whether `body`, a module's statements, binds `name` in the module's own
/// scope: a top-level `def` or `class` of that name, or any binding by
/// another statement (counted conservatively, nested scopes included).
fn binds_at_module_level(body: &[Stmt], name: &str) -> bool {
    body.iter().any(|stmt| match stmt {
        Stmt::FunctionDef(def) => def.name.as_str() == name,
        Stmt::ClassDef(class) => class.name.as_str() == name,
        other => super::ext_generators::bindings(std::slice::from_ref(other), name) > 0,
    })
}

/// The type an unannotated parameter with literal default `default` has
/// (as the original's lowering gives it, #1409).
fn literal_type(default: &Expr) -> Option<&'static str> {
    Some(match default {
        Expr::NumberLiteral(n) => match n.value {
            Number::Int(_) => "int",
            Number::Float(_) => "float",
            Number::Complex { .. } => return None,
        },
        Expr::BooleanLiteral(_) => "bool",
        Expr::StringLiteral(_) => "str",
        Expr::NoneLiteral(_) => "object",
        Expr::UnaryOp(u) => return literal_type(&u.operand),
        _ => return None,
    })
}

/// The specialization of `def` (a method of `class` when given) and what it
/// covers, or `None` when it is not specialized.
fn plan(
    def: &StmtFunctionDef,
    class: Option<(&str, &attrs::ClassFacts)>,
    skip: &dyn Fn(&str) -> bool,
    keep_builtins: &dyn Fn(&str) -> bool,
    module_binds: &dyn Fn(&str) -> bool,
    call_sites: &call_sites::CallSites,
) -> Option<(StmtFunctionDef, Specialized)> {
    let class_facts = class.map(|(_, facts)| facts);
    let class = class.map(|(name, _)| name);
    let fn_name = def.name.as_str();
    let p = &def.parameters;
    if fn_name.starts_with("__") || fn_name.starts_with("_pycc")
        || def.is_async
        || !def.decorator_list.is_empty()
        || p.vararg.is_some()
        || p.kwarg.is_some()
        || !p.posonlyargs.is_empty()
        || !p.kwonlyargs.is_empty()
        || super::ext_generators::yields(&def.body)
        || !loops(&def.body)
        || p.iter().any(|param| param.name().as_str() == "type")
        || super::ext_generators::bindings(&def.body, "type") > 0
    {
        return None;
    }
    let qualname = match class {
        Some(class) => format!("{class}.{fn_name}"),
        None => fn_name.to_string(),
    };
    if skip(&qualname) {
        return None;
    }
    let (receiver, params) = match class {
        Some(_) => {
            let (first, rest) = p.args.split_first()?;
            if first.default.is_some() {
                return None;
            }
            (Some(first), rest)
        }
        None => (None, &p.args[..]),
    };
    let names: Vec<String> = params.iter().map(|a| a.parameter.name.to_string()).collect();
    if names.iter().any(|n| n.starts_with("_pycc")) {
        return None;
    }
    let evidence = Evidence::of(&def.body, &names);
    let mut guessed = Vec::new();
    let mut annotations = Vec::with_capacity(params.len());
    for (index, param) in params.iter().enumerate() {
        let param_name = param.parameter.name.as_str();
        let annotation: Expr = match (&param.parameter.annotation, &param.default) {
            (Some(annotation), _) => annotation.as_ref().clone(),
            (None, Some(default)) => {
                if !super::signatures::is_literal(default) {
                    return None;
                }
                name(literal_type(default)?, ExprContext::Load, param.range())
            }
            (None, None) => {
                let passed = call_sites.passes_bytearray(class.is_some(), fn_name, index);
                let ty = evidence.guess(param_name, passed).unwrap_or("object");
                if ty != "object" {
                    guessed.push((param_name.to_string(), ty));
                }
                let spelling = if ty == "bytearray" {
                    BYTEARRAY_ANNOTATION
                } else {
                    ty
                };
                name(spelling, ExprContext::Load, param.range())
            }
        };
        annotations.push(annotation);
    }
    // A method's integer attributes it may take as snapshots (`attrs`).
    let snapshots = match (receiver, class_facts) {
        (Some(receiver), Some(facts)) => {
            let known: std::collections::HashMap<String, &'static str> =
                guessed.iter().cloned().collect();
            attrs::snapshot_attrs(def, receiver.parameter.name.as_str(), facts, &known, module_binds)
        }
        _ => Vec::new(),
    };
    if let Some(receiver) = receiver {
        for attr in &snapshots {
            guessed.push((format!("{}.{attr}", receiver.parameter.name), "int"));
        }
    }
    if guessed.is_empty() {
        return None;
    }
    let mut clone = def.clone();
    clone.name = Identifier::new(format!("{NATIVE_PREFIX}{fn_name}"), def.name.range());
    let offset = usize::from(receiver.is_some());
    for (index, annotation) in annotations.into_iter().enumerate() {
        let arg = &mut clone.parameters.args[index + offset];
        arg.default = None;
        arg.parameter.annotation = Some(Box::new(annotation));
    }
    if let Some(first) = clone.parameters.args.first_mut()
        && receiver.is_some()
    {
        first.default = None;
    }
    if clone.returns.is_none() {
        clone.returns = Some(Box::new(name("object", ExprContext::Load, def.name.range())));
    }
    if let Some(receiver) = receiver
        && !snapshots.is_empty()
    {
        attrs::take_snapshots(&mut clone, receiver.parameter.name.as_str(), &snapshots);
    }
    let restored_builtins = !keep_builtins(&qualname) && restore_builtins(&mut clone.body);
    Some((
        clone,
        Specialized {
            qualname,
            range: def.range(),
            params: guessed,
            restored_builtins,
        },
    ))
}

/// Turns each `FALLBACK_BUILTINS.f(...)` call of `body` back into `f(...)`
/// (module doc); returns whether there was one.
fn restore_builtins(body: &mut [Stmt]) -> bool {
    use ruff_python_ast::visitor::transformer::{self, Transformer};
    struct Restore(std::cell::Cell<bool>);
    impl Transformer for Restore {
        fn visit_stmt(&self, stmt: &mut Stmt) {
            if !matches!(stmt, Stmt::FunctionDef(_) | Stmt::ClassDef(_)) {
                transformer::walk_stmt(self, stmt);
            }
        }
        fn visit_expr(&self, expr: &mut Expr) {
            if let Expr::Call(call) = expr
                && let Expr::Attribute(attr) = call.func.as_ref()
                && matches!(attr.value.as_ref(), Expr::Name(n) if n.id.as_str() == FALLBACK_BUILTINS)
            {
                let range = attr.range();
                let bare = name(attr.attr.as_str(), ExprContext::Load, range);
                *call.func = bare;
                self.0.set(true);
            }
            transformer::walk_expr(self, expr);
        }
    }
    let restore = Restore(std::cell::Cell::new(false));
    for stmt in body {
        restore.visit_stmt(stmt);
    }
    restore.0.get()
}

/// The scalar element type `Iterable[T]` annotates, as the specialization's
/// `frozenset[T]` spelling.
fn iterable_element(annotation: &Expr) -> Option<&'static str> {
    let Expr::Subscript(sub) = annotation else {
        return None;
    };
    if !matches!(sub.value.as_ref(), Expr::Name(base) if base.id.as_str() == "Iterable") {
        return None;
    }
    let Expr::Name(element) = sub.slice.as_ref() else {
        return None;
    };
    Some(match element.id.as_str() {
        "str" => "frozenset[str]",
        "int" => "frozenset[int]",
        "float" => "frozenset[float]",
        "bool" => "frozenset[bool]",
        _ => return None,
    })
}

/// The `frozenset[T]` copy of the module function `def` for its
/// `Iterable[T]` parameters (module doc), or `None`.
fn plan_iterable(
    def: &StmtFunctionDef,
    skip: &dyn Fn(&str) -> bool,
) -> Option<(StmtFunctionDef, Specialized)> {
    let fn_name = def.name.as_str();
    let p = &def.parameters;
    if fn_name.starts_with("__") || fn_name.starts_with("_pycc")
        || def.is_async
        || !def.decorator_list.is_empty()
        || p.vararg.is_some()
        || p.kwarg.is_some()
        || !p.posonlyargs.is_empty()
        || !p.kwonlyargs.is_empty()
        || super::ext_generators::yields(&def.body)
        || skip(fn_name)
        || p.args.iter().any(|a| {
            a.parameter.annotation.is_none() || a.parameter.name.as_str().starts_with("_pycc")
        })
    {
        return None;
    }
    let mut clone = def.clone();
    let mut specialized = Vec::new();
    for arg in &mut clone.parameters.args {
        arg.default = None;
        let annotation = arg.parameter.annotation.as_deref()?;
        if let Some(spelling) = iterable_element(annotation) {
            let range = annotation.range();
            let replacement = super::signatures::parse_helper(&format!("x: {spelling}\n"), range);
            let Some(Stmt::AnnAssign(assign)) = replacement.into_iter().next() else {
                return None;
            };
            arg.parameter.annotation = Some(assign.annotation);
            specialized.push((arg.parameter.name.to_string(), spelling));
        }
    }
    if specialized.is_empty() {
        return None;
    }
    clone.name = Identifier::new(format!("{NATIVE_PREFIX}{fn_name}"), def.name.range());
    if clone.returns.is_none() {
        clone.returns = Some(Box::new(name("object", ExprContext::Load, def.name.range())));
    }
    Some((
        clone,
        Specialized {
            qualname: fn_name.to_string(),
            range: def.range(),
            params: specialized,
            restored_builtins: false,
        },
    ))
}

/// Puts the guard calling `spec`'s specialization at the top of `def`
/// (after its docstring).
fn add_guard(def: &mut StmtFunctionDef, class: Option<&str>, spec: &Specialized) {
    let names: Vec<String> = def
        .parameters
        .args
        .iter()
        .map(|a| a.parameter.name.to_string())
        .collect();
    // An attribute snapshot (`attrs`, spelled `self.name`) is passed last.
    let snapshots: Vec<&str> = spec
        .params
        .iter()
        .map(|(param, _)| param.as_str())
        .filter(|param| param.contains('.'))
        .collect();
    let (callee, args) = match class {
        Some(_) => (
            format!("{}.{NATIVE_PREFIX}{}", names[0], def.name),
            names[1..].iter().map(String::as_str).chain(snapshots.iter().copied()).collect::<Vec<_>>().join(", "),
        ),
        None => (format!("{NATIVE_PREFIX}{}", def.name), names.join(", ")),
    };
    let test = spec
        .params
        .iter()
        .map(|(param, ty)| format!("type({param}) is {GUARD_TYPE_PREFIX}{ty}"))
        .collect::<Vec<_>>()
        .join(" and ");
    let source = format!("if {test}:\n    return {callee}({args})\n");
    let guard = super::signatures::parse_helper(&source, def.name.range());
    let at = usize::from(matches!(def.body.first(), Some(Stmt::Expr(e))
        if matches!(e.value.as_ref(), Expr::StringLiteral(_))));
    for (offset, stmt) in guard.into_iter().enumerate() {
        def.body.insert(at + offset, stmt);
    }
}

/// Whether `body` loops (not counting nested functions and classes).
fn loops(body: &[Stmt]) -> bool {
    struct Find(bool);
    impl<'a> Visitor<'a> for Find {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
                Stmt::For(_) | Stmt::While(_) => self.0 = true,
                _ => visitor::walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Lambda(_) => {}
                Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_) | Expr::Generator(_) => {
                    self.0 = true;
                }
                _ => visitor::walk_expr(self, expr),
            }
        }
    }
    let mut find = Find(false);
    for stmt in body {
        find.visit_stmt(stmt);
    }
    find.0
}

mod attrs;
mod call_sites;
mod evidence;
use evidence::Evidence;

#[cfg(test)]
mod tests;
