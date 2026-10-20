//! Native instances across the modules of a package built whole (v0.8): a
//! member reads the instances of a class another module compiled
//! (`from .core import BlockState`) through their slots and calls their
//! compiled methods directly, instead of through CPython's attribute
//! protocol.
//!
//! The other module's interface lists the class's shape
//! ([`super::interface::InterfaceClass`]). The member's module gets a
//! *sibling class* of that shape, added past the end of its source as the
//! sibling functions are (`super::siblings`):
//!
//! ```python
//! class _pycc_sibc_BlockState:
//!     def __init__(self, _pycc_sib_a0: str, _pycc_sib_a1: int) -> None:
//!         self.src = _pycc_sib_a0
//!         self.cursor = _pycc_sib_a1
//!     def get_text(self, _pycc_sib_a0: int) -> str:
//!         raise TypeError("_pycc_sibc")
//! ```
//!
//! Its `__init__` gives it the other class's slots, in the same order and
//! of the same types, so its instances have the other class's layout; its
//! other methods' bodies are never compiled: codegen calls the other
//! module's compiled method instead (`pycc_codegen::SiblingClass`), and
//! routes every conversion between an instance and a CPython object to the
//! other module (its carrier type, its `isinstance`). An annotation naming
//! the imported class (`state: BlockState`) resolves to the sibling class
//! (`pycc_hir::set_sibling_class_names`), and so does a parameter so
//! annotated of a body CPython runs the `def` of (`crate::fallback`'s
//! lifts), so the parameter is converted once, on entry, and every read of
//! it is a slot read.
//!
//! The sibling class must type exactly as the class it stands for: after
//! the type check its slots and method signatures are compared with the
//! interface ([`verify`]). A difference, or any failure the class takes
//! part in (an attribute it does not have, a call its signatures refuse),
//! makes the next attempt read that class through CPython again
//! ([`deny_failures`]): the annotations naming it resolve to `object`.

use super::interface::InterfaceClass;
use super::siblings::{imported_module, module_imports_where};
use super::{Member, Producer};
use pycc_ast::{Ranged, Stmt};
use pycc_hir::Ty;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

/// The prefix of a sibling class's parameters (reserved, as every `_pycc_`
/// name is).
const PARAM_PREFIX: &str = "_pycc_sib_a";

/// A producer's listed class: the producer's index in the member's list
/// and the class's in its interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClassRef {
    pub(crate) producer: usize,
    pub(crate) class: usize,
}

/// One sibling class the last [`stubs`] added.
#[derive(Debug, Clone)]
struct Added {
    /// `_pycc_sibc_<class>`.
    stub: String,
    class: ClassRef,
    /// Its source range in the module compiled.
    range: std::ops::Range<u32>,
}

/// The sibling classes (by name) read through CPython ([`deny_failures`]).
static DENIED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// The sibling classes the last [`stubs`] added.
static ADDED: Mutex<Vec<Added>> = Mutex::new(Vec::new());

/// The functions (by [`function_key`]) whose annotations read no class
/// natively ([`deny_failures`]).
static DENIED_FUNCTIONS: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// The source ranges and keys of the functions of the module compiled
/// whose annotations name a class read natively ([`functions`]).
static FUNCTIONS: Mutex<Vec<(std::ops::Range<u32>, String)>> = Mutex::new(Vec::new());

/// The sibling class standing for `class`.
fn stub_name(class: &str) -> String {
    format!("{}{class}", pycc_hir::SIBLING_CLASS_PREFIX)
}

fn producer_class(producers: &[Producer], class: ClassRef) -> &InterfaceClass {
    &producers[class.producer].interface.classes[class.class]
}

/// The names of `source` (the member's module as written) bound, once at
/// module scope, by a module-level `from <producer> import` of a class the
/// producer's interface lists, with that class; none when a class of the
/// module itself has the class's name, or two producers list classes of the
/// same name (each instance's run-time class is found by name).
pub(crate) fn candidates(
    dotted: &str,
    is_package: bool,
    producers: &[Producer],
    source: &str,
) -> BTreeMap<String, ClassRef> {
    // For measurement: `PYCC_NO_SIBLING_CLASSES=1` reads every class of
    // another module through CPython.
    if std::env::var_os("PYCC_NO_SIBLING_CLASSES").is_some() {
        return BTreeMap::new();
    }
    let Ok(parsed) = pycc_parser::parse_all(source) else {
        return BTreeMap::new();
    };
    let mut imports = Vec::new();
    module_imports_where(&parsed.body, &mut imports, &|_| true);
    let mut bindings: BTreeMap<String, usize> = BTreeMap::new();
    let mut found: BTreeMap<String, ClassRef> = BTreeMap::new();
    for import in &imports {
        let module = imported_module(
            dotted,
            is_package,
            import.level,
            import.module.as_ref().map(|m| m.as_str()),
        );
        let producer = module.and_then(|module| {
            producers
                .iter()
                .position(|producer| producer.interface.module == module)
        });
        for alias in &import.names {
            let local = alias.asname.as_ref().unwrap_or(&alias.name).to_string();
            *bindings.entry(local.clone()).or_insert(0) += 1;
            let Some(producer) = producer else {
                continue;
            };
            if let Some(class) = producers[producer]
                .interface
                .classes
                .iter()
                .position(|class| class.name == alias.name.as_str())
            {
                found.insert(local, ClassRef { producer, class });
            }
        }
    }
    let mut defined = pycc_ast::namespace::module_bindings(&parsed).defined;
    defined.extend(module_stores(&parsed.body));
    let own = class_names(&parsed.body);
    let mut by_name: BTreeMap<&str, BTreeSet<usize>> = BTreeMap::new();
    for producer in producers.iter().enumerate() {
        for class in &producer.1.interface.classes {
            by_name
                .entry(class.name.as_str())
                .or_default()
                .insert(producer.0);
        }
    }
    found.retain(|local, class| {
        let name = producer_class(producers, *class).name.as_str();
        bindings[local] == 1
            && !defined.contains(local)
            && !own.contains(name)
            && by_name[name].len() == 1
            && closure(producers, *class).is_some()
    });
    found
}

/// The names module-level code of `body` stores to (an assignment target,
/// a `def` or `class` name), outside function and class bodies: what
/// rebinds an imported name there.
fn module_stores(body: &[Stmt]) -> BTreeSet<String> {
    use pycc_ast::visitor::{Visitor, walk_expr, walk_stmt};
    struct Stores(BTreeSet<String>);
    impl<'a> Visitor<'a> for Stores {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(def) => {
                    self.0.insert(def.name.to_string());
                }
                Stmt::ClassDef(class) => {
                    self.0.insert(class.name.to_string());
                }
                _ => walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, expr: &'a pycc_ast::Expr) {
            if let pycc_ast::Expr::Name(name) = expr
                && name.ctx == pycc_ast::ExprContext::Store
            {
                self.0.insert(name.id.to_string());
            }
            walk_expr(self, expr);
        }
    }
    let mut stores = Stores(BTreeSet::new());
    for stmt in body {
        stores.visit_stmt(stmt);
    }
    stores.0
}

/// The class names `body` defines, at any depth.
fn class_names(body: &[Stmt]) -> BTreeSet<String> {
    use pycc_ast::visitor::{Visitor, walk_stmt};
    struct Classes(BTreeSet<String>);
    impl<'a> Visitor<'a> for Classes {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            if let Stmt::ClassDef(class) = stmt {
                self.0.insert(class.name.to_string());
            }
            walk_stmt(self, stmt);
        }
    }
    let mut classes = Classes(BTreeSet::new());
    for stmt in body {
        classes.visit_stmt(stmt);
    }
    classes.0
}

/// `class` and the classes of its producer its slots and methods name, or
/// `None` when one of them is not listed.
fn closure(producers: &[Producer], class: ClassRef) -> Option<Vec<ClassRef>> {
    let interface = &producers[class.producer].interface;
    let mut out = vec![class];
    let mut at = 0;
    while let Some(&next) = out.get(at) {
        at += 1;
        let listed = &interface.classes[next.class];
        let mut named = Vec::new();
        for (_, ty) in &listed.slots {
            instances(ty, &mut named);
        }
        for method in &listed.methods {
            instances(&method.ret, &mut named);
            for param in &method.params {
                instances(param, &mut named);
            }
        }
        for name in named {
            let index = interface.classes.iter().position(|c| c.name == name)?;
            let found = ClassRef {
                producer: class.producer,
                class: index,
            };
            if !out.contains(&found) {
                out.push(found);
            }
        }
    }
    Some(out)
}

/// The classes `ty` holds instances of.
fn instances(ty: &Ty, out: &mut Vec<String>) {
    match ty {
        Ty::Instance(class) => out.push(class.as_ref().clone()),
        Ty::List(inner) | Ty::Set(inner) | Ty::Optional(inner) => instances(inner, out),
        Ty::Dict(pair) => {
            instances(&pair.0, out);
            instances(&pair.1, out);
        }
        _ => {}
    }
}

/// The Python spelling of `ty` in the member, an instance of a listed class
/// as its sibling class.
fn python_ty(ty: &Ty) -> String {
    match ty {
        Ty::Instance(class) => stub_name(class),
        Ty::List(inner) => format!("list[{}]", python_ty(inner)),
        Ty::Set(inner) => format!("set[{}]", python_ty(inner)),
        Ty::Dict(pair) => format!("dict[{}, {}]", python_ty(&pair.0), python_ty(&pair.1)),
        Ty::Optional(inner) => format!("{} | None", python_ty(inner)),
        other => super::interface::interface_ty(other)
            .expect("listed types have a spelling")
            .to_string(),
    }
}

/// The sibling class standing for `class` (module docs).
fn stub_source(class: &InterfaceClass) -> String {
    let stub = stub_name(&class.name);
    let params: Vec<String> = class
        .slots
        .iter()
        .enumerate()
        .map(|(i, (_, ty))| format!(", {PARAM_PREFIX}{i}: {}", python_ty(ty)))
        .collect();
    let mut out = format!(
        "class {stub}:\n    def __init__(self{}) -> None:\n",
        params.concat()
    );
    if class.slots.is_empty() {
        out.push_str("        pass\n");
    }
    for (i, (slot, _)) in class.slots.iter().enumerate() {
        out.push_str(&format!("        self.{slot} = {PARAM_PREFIX}{i}\n"));
    }
    for method in &class.methods {
        let params: Vec<String> = method
            .params
            .iter()
            .enumerate()
            .map(|(i, ty)| format!(", {PARAM_PREFIX}{i}: {}", python_ty(ty)))
            .collect();
        out.push_str(&format!(
            "    def {}(self{}) -> {}:\n        raise TypeError(\"{}\")\n",
            method.name,
            params.concat(),
            python_ty(&method.ret),
            pycc_hir::SIBLING_CLASS_PREFIX
        ));
    }
    out
}

/// The sibling classes to add to the member's module and the classes they
/// stand for, every candidate's closure but the denied ones; sets the
/// names annotations resolve (`pycc_hir::set_sibling_class_names`).
pub(super) fn stubs(member: &Member) -> Vec<(String, ClassRef, String)> {
    let denied = DENIED.lock().expect("the denied lock").clone();
    let mut added: Vec<(String, ClassRef, String)> = Vec::new();
    let mut names = Vec::new();
    for (local, class) in &member.classes {
        let closure = closure(&member.producers, *class).expect("candidates have closures");
        let stubs: Vec<String> = closure
            .iter()
            .map(|c| stub_name(&producer_class(&member.producers, *c).name))
            .collect();
        if stubs.iter().any(|stub| denied.contains(stub)) {
            names.push((local.clone(), None));
            continue;
        }
        names.push((local.clone(), Some(stubs[0].clone())));
        for (c, stub) in closure.into_iter().zip(stubs) {
            if !added.iter().any(|(s, _, _)| *s == stub) {
                let source = stub_source(producer_class(&member.producers, c));
                added.push((stub, c, source));
            }
        }
    }
    pycc_hir::set_sibling_class_names(names);
    // A class's annotations name the classes defined before it: each class
    // after those its types hold (a cycle keeps its order, and fails).
    let mut ordered: Vec<(String, ClassRef, String)> = Vec::new();
    while !added.is_empty() {
        let ready = added
            .iter()
            .position(|(stub, class, _)| {
                let mut named = Vec::new();
                let listed = producer_class(&member.producers, *class);
                for (_, ty) in &listed.slots {
                    instances(ty, &mut named);
                }
                for method in &listed.methods {
                    instances(&method.ret, &mut named);
                    method
                        .params
                        .iter()
                        .for_each(|ty| instances(ty, &mut named));
                }
                named.iter().all(|name| {
                    let other = stub_name(name);
                    other == *stub || ordered.iter().any(|(done, _, _)| *done == other)
                })
            })
            .unwrap_or(0);
        ordered.push(added.remove(ready));
    }
    ordered
}

/// Records where the sibling classes `added` landed in the module
/// compiled, `body`.
pub(super) fn record(body: &[Stmt], added: &[(String, ClassRef, String)]) {
    let mut out = ADDED.lock().expect("the added lock");
    out.clear();
    for stmt in body {
        let Stmt::ClassDef(def) = stmt else {
            continue;
        };
        if let Some((stub, class, _)) = added.iter().find(|(stub, _, _)| def.name.as_str() == stub)
        {
            let range = stmt.range();
            out.push(Added {
                stub: stub.clone(),
                class: *class,
                range: range.start().into()..range.end().into(),
            });
        }
    }
}

/// Forgets the sibling classes of an earlier module (nothing is added).
pub(super) fn clear() {
    ADDED.lock().expect("the added lock").clear();
    FUNCTIONS.lock().expect("the functions lock").clear();
    pycc_hir::set_sibling_class_names(Vec::new());
}

/// What names a function of the module compiled for [`DENIED_FUNCTIONS`]:
/// a top-level `def` (a lifted body's too) by its name, a method of a
/// class compiled as `Class.method`.
fn function_key(class: Option<&str>, name: &str) -> String {
    match class {
        Some(class) => format!("{class}.{name}"),
        None => name.to_string(),
    }
}

/// Whether an annotation names one of `locals` (a bare name, or one as a
/// string).
fn annotation_names(annotation: &pycc_ast::Expr, locals: &BTreeSet<&str>) -> bool {
    match annotation {
        pycc_ast::Expr::Name(name) => locals.contains(name.id.as_str()),
        pycc_ast::Expr::StringLiteral(text) => locals.contains(text.value.to_str().trim()),
        _ => false,
    }
}

/// Rewrites every annotation naming one of the names, in a function and
/// the functions nested in it, to `object`; counts what it saw.
struct Annotations<'a> {
    locals: &'a BTreeSet<&'a str>,
    rewrite: bool,
    seen: std::cell::Cell<bool>,
}

impl pycc_ast::visitor::transformer::Transformer for Annotations<'_> {
    fn visit_annotation(&self, expr: &mut pycc_ast::Expr) {
        if annotation_names(expr, self.locals) {
            self.seen.set(true);
            if self.rewrite {
                let range = expr.range();
                *expr = pycc_ast::Expr::Name(pycc_ast::ExprName {
                    node_index: Default::default(),
                    range,
                    id: "object".into(),
                    ctx: pycc_ast::ExprContext::Load,
                });
            }
        }
    }

    // No annotation is inside an expression a function compiles.
    fn visit_expr(&self, _expr: &mut pycc_ast::Expr) {}
}

/// Finds the functions of `body` (the member's module as compiled: its
/// top-level `def`s, lifted bodies included, and the methods of its
/// classes) whose annotations name a class the module reads natively:
/// records each, or, for one [`DENIED_FUNCTIONS`] holds, rewrites those
/// annotations to `object`.
pub(super) fn functions(body: &mut [Stmt], member: &Member) {
    let denied_classes = DENIED.lock().expect("the denied lock").clone();
    let locals: BTreeSet<&str> = member
        .classes
        .iter()
        .filter(|(_, class)| {
            !denied_classes.contains(&stub_name(&producer_class(&member.producers, **class).name))
        })
        .map(|(local, _)| local.as_str())
        .collect();
    let mut out = FUNCTIONS.lock().expect("the functions lock");
    out.clear();
    if locals.is_empty() {
        return;
    }
    let denied = DENIED_FUNCTIONS.lock().expect("the denied functions lock");
    let mut visit = |stmt: &mut Stmt, class: Option<&str>| {
        let Stmt::FunctionDef(def) = stmt else {
            return;
        };
        let key = function_key(class, def.name.as_str());
        let annotations = Annotations {
            locals: &locals,
            rewrite: denied.contains(&key),
            seen: std::cell::Cell::new(false),
        };
        use pycc_ast::visitor::transformer::Transformer;
        annotations.visit_stmt(stmt);
        if annotations.seen.get() && !annotations.rewrite {
            let range = stmt.range();
            out.push((range.start().into()..range.end().into(), key));
        }
    };
    for stmt in body.iter_mut() {
        if let Stmt::ClassDef(class) = stmt {
            let name = class.name.to_string();
            for inner in class.body.iter_mut() {
                visit(inner, Some(&name));
            }
        } else {
            visit(stmt, None);
        }
    }
}

/// The fallback loop's hook (module docs): a failure inside a function
/// whose annotations name a class read natively makes that function's
/// annotations read it through CPython; any other failure inside a sibling
/// class, or one whose message names it, denies the class. Whether anything
/// was newly denied.
pub(crate) fn deny_failures<'a>(
    failures: impl IntoIterator<Item = (Option<u32>, &'a str)>,
) -> bool {
    let added = ADDED.lock().expect("the added lock");
    let functions = FUNCTIONS.lock().expect("the functions lock");
    let mut denied = DENIED.lock().expect("the denied lock");
    let mut denied_functions = DENIED_FUNCTIONS.lock().expect("the denied functions lock");
    let dump = std::env::var_os("PYCC_FALLBACK_DUMP").is_some();
    let mut new = false;
    for (at, message) in failures {
        if let Some(at) = at
            && let Some((_, key)) = functions.iter().find(|(range, _)| range.contains(&at))
        {
            if denied_functions.insert(key.clone()) {
                new = true;
                if dump {
                    eprintln!("--- `{key}` reads its classes through CPython: {message} ---");
                }
            }
            continue;
        }
        for entry in added.iter() {
            let inside = at.is_some_and(|at| entry.range.contains(&at));
            if (inside || names(message, &entry.stub)) && denied.insert(entry.stub.clone()) {
                new = true;
                if std::env::var_os("PYCC_FALLBACK_DUMP").is_some() {
                    eprintln!("--- `{}` read through CPython: {message} ---", entry.stub);
                }
            }
        }
    }
    new
}

/// Whether `message` names the class `stub` (not a longer name it
/// prefixes).
fn names(message: &str, stub: &str) -> bool {
    message.match_indices(stub).any(|(at, _)| {
        !message[at + stub.len()..]
            .chars()
            .next()
            .is_some_and(|c| c == '_' || c.is_alphanumeric())
    })
}

/// The mismatches between the sibling classes the type check produced in
/// `typed` and the classes they stand for, each with the offset of its
/// class (module docs): a sibling class that did not compile, or whose
/// slots or method signatures differ.
pub(crate) fn verify(typed: &pycc_hir::HirModule) -> Vec<(u32, String)> {
    let Some(member) = super::member() else {
        return Vec::new();
    };
    let added = ADDED.lock().expect("the added lock");
    let mut out = Vec::new();
    for entry in added.iter() {
        let class = producer_class(&member.producers, entry.class);
        let mismatch = match typed
            .class_defs
            .iter()
            .find(|(name, _)| *name == entry.stub)
        {
            None => Some("it did not compile".to_string()),
            Some((_, def)) => differs(class, def, typed),
        };
        if let Some(reason) = mismatch {
            out.push((
                entry.range.start,
                format!("sibling class `{}` does not match: {reason}", entry.stub),
            ));
        }
    }
    out
}

/// How the sibling class `def` (typed in `typed`) differs from `class`.
fn differs(
    class: &InterfaceClass,
    def: &pycc_hir::HirClassDef,
    typed: &pycc_hir::HirModule,
) -> Option<String> {
    let real = |ty: &Ty| unstub(ty);
    let slots: Vec<(String, Ty)> = def
        .attrs
        .iter()
        .map(|(name, ty)| (name.clone(), real(ty)))
        .collect();
    if slots != class.slots {
        return Some(format!("slots {slots:?}, expected {:?}", class.slots));
    }
    for method in &class.methods {
        let Some((_, mangled)) = def.methods.iter().find(|(name, _)| *name == method.name) else {
            return Some(format!("no method `{}`", method.name));
        };
        let signature = typed.items.iter().find_map(|item| match item {
            pycc_hir::HirItem::Function {
                name,
                params,
                return_ty,
                ..
            } if name == mangled => Some((params, return_ty)),
            _ => None,
        });
        let Some((params, ret)) = signature else {
            return Some(format!("method `{}` did not compile", method.name));
        };
        let params: Vec<Ty> = params.iter().skip(1).map(|(_, ty)| real(ty)).collect();
        if params != method.params || real(ret) != method.ret {
            return Some(format!("method `{}` has another signature", method.name));
        }
    }
    None
}

/// `ty` with each sibling class replaced by the class it stands for.
fn unstub(ty: &Ty) -> Ty {
    match ty {
        Ty::Instance(class) => Ty::Instance(Box::new(
            class
                .strip_prefix(pycc_hir::SIBLING_CLASS_PREFIX)
                .unwrap_or(class)
                .to_string(),
        )),
        Ty::List(inner) => Ty::List(Box::new(unstub(inner))),
        Ty::Set(inner) => Ty::Set(Box::new(unstub(inner))),
        Ty::Optional(inner) => Ty::Optional(Box::new(unstub(inner))),
        Ty::Dict(pair) => Ty::Dict(Box::new((unstub(&pair.0), unstub(&pair.1)))),
        other => other.clone(),
    }
}

/// What codegen needs for the sibling classes the last [`stubs`] added.
pub(crate) fn codegen_classes() -> Vec<pycc_codegen::SiblingClass> {
    let Some(member) = super::member() else {
        return Vec::new();
    };
    let denied = DENIED.lock().expect("the denied lock");
    ADDED
        .lock()
        .expect("the added lock")
        .iter()
        .filter(|entry| !denied.contains(&entry.stub))
        .map(|entry| {
            let producer = &member.producers[entry.class.producer].interface;
            let class = producer_class(&member.producers, entry.class);
            pycc_codegen::SiblingClass {
                class: entry.stub.clone(),
                real: class.name.clone(),
                prefix: producer.prefix.clone(),
                methods: class
                    .methods
                    .iter()
                    .map(|method| {
                        (
                            format!("{}.{}", entry.stub, method.name),
                            format!("{}pyfn_{}", producer.prefix, method.function),
                        )
                    })
                    .collect(),
            }
        })
        .collect()
}

/// The C call reading the CPython object `arg` into `out` (a `void *`
/// lvalue) as an instance of the sibling class `class`, answering non-zero
/// on a refusal (a `TypeError` naming `what`, `f() argument 1`, set): the
/// other module's own conversion, under the class's own name. `None` for a
/// class that is no sibling class.
pub(crate) fn c_unpack_instance(arg: &str, what: &str, class: &str, out: &str) -> Option<String> {
    let sibling = codegen_classes()
        .into_iter()
        .find(|sibling| sibling.class == class)?;
    Some(format!(
        "{}{}({arg}, \"{}\", \"{what}\", (void **)&{out})",
        sibling.prefix,
        pycc_codegen::EXT_OBJ_COERCE_INSTANCE_SYMBOL,
        sibling.real
    ))
}

/// The declarations of the other modules' conversions
/// [`c_unpack_instance`] calls, for the generated C.
pub(crate) fn c_declarations() -> String {
    let prefixes: BTreeSet<String> = codegen_classes()
        .into_iter()
        .map(|sibling| sibling.prefix)
        .collect();
    prefixes
        .iter()
        .map(|prefix| {
            format!(
                "int {prefix}{}(PyObject *o, const char *class_name, const char *what, void \
                 **out);\n",
                pycc_codegen::EXT_OBJ_COERCE_INSTANCE_SYMBOL
            )
        })
        .collect()
}

/// `typed` without the methods of its sibling classes, which the module
/// does not publish (the other module does), or `None` when it has none.
/// The classes stay, for the layouts of its own classes holding their
/// instances; `crate::ext_build` gives them no CPython type.
pub(crate) fn without_stubs(typed: &pycc_hir::HirModule) -> Option<pycc_hir::HirModule> {
    let stub = |name: &str| name.starts_with(pycc_hir::SIBLING_CLASS_PREFIX);
    if !typed.class_defs.iter().any(|(name, _)| stub(name)) {
        return None;
    }
    let mut out = typed.clone();
    out.items
        .retain(|item| !matches!(item, pycc_hir::HirItem::Function { name, .. } if stub(name)));
    Some(out)
}

/// Whether a lifted body's parameter annotated `annotation` (the name a
/// module-level import binds, or that name as a string) is read natively:
/// it is spelled so in the compiled body's signature, where it resolves to
/// the sibling class.
pub(crate) fn lifted_annotation(annotation: &pycc_ast::Expr) -> Option<String> {
    let member = super::member()?;
    let name = match annotation {
        pycc_ast::Expr::Name(name) => name.id.to_string(),
        pycc_ast::Expr::StringLiteral(text) => text.value.to_str().trim().to_string(),
        _ => return None,
    };
    let class = member.classes.get(&name)?;
    let denied = DENIED.lock().expect("the denied lock");
    let stub = stub_name(&producer_class(&member.producers, *class).name);
    (!denied.contains(&stub)).then_some(name)
}

/// Under `--report-fallbacks`: the classes of other modules whose
/// instances the member's module reads natively, one line each.
pub(crate) fn report(typed: &pycc_hir::HirModule) {
    let Some(member) = super::member() else {
        return;
    };
    for class in codegen_classes() {
        if typed
            .class_defs
            .iter()
            .any(|(name, _)| *name == class.class)
        {
            let module = member
                .producers
                .iter()
                .find(|producer| producer.interface.prefix == class.prefix)
                .map_or("", |producer| producer.interface.module.as_str());
            eprintln!(
                "native class: instances of `{module}.{}` are read natively",
                class.real
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package_build::interface::{Interface, InterfaceMethod};

    fn producer() -> Producer {
        Producer {
            interface: Interface {
                module: "p.core".to_string(),
                prefix: "pycc_P1p4core_".to_string(),
                functions: vec![],
                classes: vec![
                    InterfaceClass {
                        name: "State".to_string(),
                        slots: vec![
                            ("src".to_string(), Ty::Str),
                            (
                                "parent".to_string(),
                                Ty::Optional(Box::new(Ty::Instance(Box::new("Env".to_string())))),
                            ),
                        ],
                        methods: vec![InterfaceMethod {
                            name: "get".to_string(),
                            function: "State.get".to_string(),
                            params: vec![Ty::Int],
                            ret: Ty::Str,
                        }],
                    },
                    InterfaceClass {
                        name: "Env".to_string(),
                        slots: vec![],
                        methods: vec![],
                    },
                ],
            },
            hash: 0,
        }
    }

    #[test]
    fn a_sibling_class_has_the_slots_and_the_methods() {
        let producers = [producer()];
        assert_eq!(
            stub_source(&producers[0].interface.classes[0]),
            "class _pycc_sibc_State:\n    def __init__(self, _pycc_sib_a0: str, _pycc_sib_a1: \
             _pycc_sibc_Env | None) -> None:\n        self.src = _pycc_sib_a0\n        \
             self.parent = _pycc_sib_a1\n    def get(self, _pycc_sib_a0: int) -> str:\n        \
             raise TypeError(\"_pycc_sibc_\")\n"
        );
        assert_eq!(
            stub_source(&producers[0].interface.classes[1]),
            "class _pycc_sibc_Env:\n    def __init__(self) -> None:\n        pass\n"
        );
    }

    #[test]
    fn candidates_are_imported_classes_with_their_closure() {
        let producers = [producer()];
        let found = candidates(
            "p.parse",
            false,
            &producers,
            "from .core import State\nfrom .core import Env as E\n",
        );
        assert_eq!(found.len(), 2);
        assert_eq!(
            found["State"],
            ClassRef {
                producer: 0,
                class: 0
            }
        );
        assert_eq!(closure(&producers, found["State"]).unwrap().len(), 2);
        // Rebound, or a class of the module's own of the same name.
        let found = candidates(
            "p.parse",
            false,
            &producers,
            "from .core import State\nState = 1\nclass Env:\n    pass\nfrom .core import Env\n",
        );
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn messages_name_a_class_exactly() {
        assert!(names("class `_pycc_sibc_S` has no", "_pycc_sibc_S"));
        assert!(!names("class `_pycc_sibc_State` has no", "_pycc_sibc_S"));
        assert!(names("_pycc_sibc_S", "_pycc_sibc_S"));
    }

    #[test]
    fn sibling_types_read_back_as_the_classes() {
        let ty = Ty::List(Box::new(Ty::Optional(Box::new(Ty::Instance(Box::new(
            "_pycc_sibc_S".to_string(),
        ))))));
        assert_eq!(
            unstub(&ty),
            Ty::List(Box::new(Ty::Optional(Box::new(Ty::Instance(Box::new(
                "S".to_string()
            ))))))
        );
        assert_eq!(
            python_ty(&Ty::Dict(Box::new((Ty::Str, Ty::Object)))),
            "dict[str, object]"
        );
    }
}
