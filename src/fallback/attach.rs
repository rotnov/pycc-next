//! Methods attached to a compiled class (v0.7 accept 3): a method whose
//! body `--non-strict` cannot compile runs in CPython while its class stays
//! a native class.
//!
//! In an `--ext` module (which publishes its classes) a method of a compiled
//! top-level class whose body fails with an unsupported construct
//! (`C0001`) does not send the class to CPython (`lift`'s split); instead:
//!
//! * its body in the compiled class becomes a *forwarder*:
//!   `return __pycc_fallback_ns['__pycc_pyN_m__'](self, a, b)` -- the method
//!   keeps its signature, so compiled callers and Python callers reach it as
//!   before, and `self` crosses as the instance's carrier (the published
//!   type's object, the one Python code holds);
//! * the method's source runs in CPython once, when the module starts, as a
//!   module-level function `__pycc_pyN_m__(self, a, b)` -- its parameters
//!   plain (no default or annotation, which the compiled forwarder applies
//!   and checks) -- and reads and stores `self.x` through the carrier's
//!   attribute descriptors.
//!
//! A forwarder of a method annotated `-> int`/`float`/`str`/`bool` converts
//! the CPython result (`int(...)`); one annotated `-> None` drops it.
//!
//! A method attaches only when CPython can run it on a carrier exactly as on
//! a Python instance, which the plan checks on its source and the frontend
//! on the typed class (`crate::frontend`): it is not a constructor, a
//! decorated method, a generator or one with `*args`, `**kwargs` or
//! keyword-only parameters; it uses no `super()`, `__class__` or private
//! (mangled) name; `self` appears only as `self.x`; every `self.x` it stores
//! is stored by another method too (a native field), and every `self.x` it
//! touches is a method of the class or a field the carrier exposes (an
//! `int`, `float`, `bool`, `str`, object or instance field). Anything else
//! sends the class to CPython with its other methods compiled (`lift`).
//! An embedded executable does not publish its classes, so there every
//! failing method splits its class.

use super::lift::Lift;
use pycc_ast::visitor::{self, Visitor};
use pycc_ast::{Expr, ExprContext, Stmt, StmtClassDef, StmtFunctionDef};
use std::collections::BTreeSet;

/// How an attached method's compiled forwarder returns the CPython result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Returns {
    /// As the object it is (unannotated, `object`, `Any`).
    Object,
    /// Dropped (`-> None`).
    Nothing,
    /// Converted by this builtin (`-> int` is `int(...)`).
    Convert(&'static str),
}

/// A lift that can attach to its compiled class.
#[derive(Debug, Clone)]
pub(super) struct Attach {
    /// The class's name.
    pub(super) class: String,
    /// The CPython function's name (`__pycc_pyN_m__`).
    pub(super) pyname: String,
    pub(super) returns: Returns,
    /// The attributes of `self` the body reads or stores.
    pub(super) self_attrs: BTreeSet<String>,
}

/// Decides which lifts of the top-level class `class` can attach to it.
pub(super) fn analyze(class: &StmtClassDef, lifts: &mut [(Lift, Stmt)]) {
    let methods: Vec<&StmtFunctionDef> = class
        .body
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::FunctionDef(def) => Some(def),
            _ => None,
        })
        .collect();
    // The `self.x` each method stores.
    let stores: Vec<BTreeSet<String>> = methods
        .iter()
        .map(|def| self_uses(def).map(|uses| uses.stored).unwrap_or_default())
        .collect();
    for (at, def) in methods.iter().enumerate() {
        let start = def.range.start().to_u32();
        let Some((lift, _)) = lifts.iter_mut().find(|(lift, _)| lift.range.start == start) else {
            continue;
        };
        let Some(uses) = eligible(def, lift).then(|| self_uses(def)).flatten() else {
            continue;
        };
        let fields: BTreeSet<&String> = stores
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != at)
            .flat_map(|(_, stored)| stored)
            .collect();
        if !uses.stored.iter().all(|name| fields.contains(name)) {
            continue;
        }
        let Some(returns) = returns(def) else {
            continue;
        };
        lift.attach = Some(Attach {
            class: class.name.as_str().to_string(),
            pyname: lift.name.replacen(super::lift::LIFT_PREFIX, "__pycc_py", 1),
            returns,
            self_attrs: uses.read.union(&uses.stored).cloned().collect(),
        });
    }
}

/// The checks on `def`'s shape (module doc).
fn eligible(def: &StmtFunctionDef, lift: &Lift) -> bool {
    let p = &def.parameters;
    let plain_annotation = |annotation: Option<&Expr>| match annotation {
        None => true,
        Some(Expr::Name(name)) => !matches!(
            name.id.as_str(),
            "list" | "dict" | "set" | "frozenset" | "tuple" | "bytearray" | "memoryview" | "type"
        ),
        Some(_) => false,
    };
    lift.plain_body
        && !lift.uses_class
        && def.decorator_list.is_empty()
        && !matches!(
            def.name.as_str(),
            "__init__" | "__new__" | "__init_subclass__" | "__class_getitem__" | "__set_name__"
        )
        && !pycc_ast::body_yields(&def.body)
        && p.vararg.is_none()
        && p.kwarg.is_none()
        && p.kwonlyargs.is_empty()
        && !(p.posonlyargs.is_empty() && p.args.is_empty())
        && p.posonlyargs
            .iter()
            .chain(&p.args)
            .all(|param| plain_annotation(param.parameter.annotation.as_deref()))
}

/// How the forwarder returns, by `def`'s return annotation; `None` for one
/// it cannot convert to.
fn returns(def: &StmtFunctionDef) -> Option<Returns> {
    Some(match def.returns.as_deref() {
        None => Returns::Object,
        Some(Expr::NoneLiteral(_)) => Returns::Nothing,
        Some(Expr::Name(name)) => match name.id.as_str() {
            "object" | "Any" => Returns::Object,
            "int" => Returns::Convert("int"),
            "float" => Returns::Convert("float"),
            "str" => Returns::Convert("str"),
            "bool" => Returns::Convert("bool"),
            _ => return None,
        },
        Some(_) => return None,
    })
}

/// What a method does with `self`.
#[derive(Default)]
struct SelfUses {
    read: BTreeSet<String>,
    stored: BTreeSet<String>,
}

/// The attributes `def`'s body reads and stores on its first parameter, or
/// `None` when the body uses that parameter any other way (passes it on,
/// rebinds it, returns it).
fn self_uses(def: &StmtFunctionDef) -> Option<SelfUses> {
    let receiver = def
        .parameters
        .posonlyargs
        .iter()
        .chain(&def.parameters.args)
        .next()?
        .parameter
        .name
        .as_str();
    struct Scan<'r> {
        receiver: &'r str,
        uses: SelfUses,
        escapes: bool,
    }
    impl<'a> Visitor<'a> for Scan<'_> {
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Attribute(attr)
                    if matches!(&*attr.value, Expr::Name(n) if n.id.as_str() == self.receiver) =>
                {
                    let name = attr.attr.as_str().to_string();
                    if matches!(attr.ctx, ExprContext::Load) {
                        self.uses.read.insert(name);
                    } else {
                        self.uses.stored.insert(name);
                    }
                }
                Expr::Name(name) if name.id.as_str() == self.receiver => self.escapes = true,
                _ => visitor::walk_expr(self, expr),
            }
        }
    }
    let mut scan = Scan {
        receiver,
        uses: SelfUses::default(),
        escapes: false,
    };
    for stmt in &def.body {
        scan.visit_stmt(stmt);
    }
    (!scan.escapes).then_some(scan.uses)
}

impl super::Plan {
    /// The compiled forwarder of attached unit `index`, replacing its body
    /// (padded with its line breaks).
    pub(super) fn attached_forwarder(&self, index: usize) -> Option<(std::ops::Range<u32>, String)> {
        let lift = self.lift(index)?;
        let attach = lift.attach.as_ref()?;
        let call = format!(
            "{}[{}]({})",
            super::NS,
            super::py_str(&attach.pyname),
            lift.params.join(", ")
        );
        let body = match attach.returns {
            Returns::Object => format!("return {call}"),
            Returns::Nothing => call,
            Returns::Convert(builtin) => format!("return {builtin}({call})"),
        };
        let lines = self.source[lift.body.start as usize..lift.body.end as usize]
            .matches('\n')
            .count();
        Some((lift.body.clone(), format!("{body}{}", "\n".repeat(lines))))
    }

    /// The CPython source defining attached unit `index`'s function, padded
    /// so its lines keep their numbers.
    pub(super) fn attached_source(&self, index: usize) -> Option<String> {
        let lift = self.lift(index)?;
        let attach = lift.attach.as_ref()?;
        let src = &self.source;
        let line = src[..lift.keyword as usize].matches('\n').count();
        let header = &src[lift.keyword as usize..lift.colon as usize];
        Some(format!(
            "{}def {}({}{}){}\n",
            "\n".repeat(line),
            attach.pyname,
            lift.params.join(", "),
            "\n".repeat(header.matches('\n').count()),
            &src[lift.colon as usize..lift.range.end as usize]
        ))
    }

    /// The attached units, in source order.
    pub(super) fn attached(&self) -> Vec<usize> {
        (self.top..self.stmts.len())
            .filter(|index| self.is_attached(*index))
            .collect()
    }
}
