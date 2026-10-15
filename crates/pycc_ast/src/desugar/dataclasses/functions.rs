//! `dataclasses.fields()`, `asdict()`, `astuple()` and `replace()`.
//!
//! A compiled dataclass carries no field metadata, so each call becomes a
//! call of a method the dataclass rewrite gives every dataclass (and every
//! non-dataclass subclass of one) the program asks for:
//!
//! * `fields(x)` -> `x._pycc_fields_()`, a static method returning a list
//!   (CPython: a tuple) of [`FIELD_CLASS`] instances (only `name` is
//!   compiled);
//! * `asdict(x)` -> `x._pycc_asdict_()` and `astuple(x)` ->
//!   `x._pycc_astuple_()`, built recursively from the field annotations as
//!   CPython's `_asdict_inner` does: a nested dataclass becomes its own
//!   dict (tuple), and a list, dict, set or tuple is copied with its
//!   elements converted;
//! * `replace(x, a=1)` -> `x._pycc_replace_<n>_(1)`, where `<n>` numbers
//!   the program's distinct keyword sequences, returning a new instance
//!   built by the generated `__init__` with the other fields copied.
//!
//! A method is generated only when some module desugared so far calls the
//! function ([`record_uses`]; the entry module is desugared first) and the
//! class's fields allow it: a field whose annotation is neither a scalar,
//! an enum, a dataclass written out earlier in the module nor a list, dict,
//! set, tuple or `Optional` of those keeps `asdict`/`astuple` from being
//! generated, and the call is then refused when it is type-checked (no such
//! method). `asdict()` of a dataclass whose converted values differ in type
//! is typed as a `dict[str, object]` in non-strict mode and refused
//! (`T0021`) in strict mode.

use super::{Expr, Field, Stmt};
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::{Arguments, ExprAttribute, ExprCall, ExprContext};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

/// The class `fields()` returns instances of.
pub(super) const FIELD_CLASS: &str = "_pycc_Field";

const FIELDS: &str = "_pycc_fields_";
const ASDICT: &str = "_pycc_asdict_";
const ASTUPLE: &str = "_pycc_astuple_";

/// The `dataclasses` functions a program calls, accumulated over the
/// modules desugared so far.
#[derive(Default, Clone)]
pub(super) struct Uses {
    pub(super) fields: bool,
    pub(super) asdict: bool,
    pub(super) astuple: bool,
    /// Each distinct keyword sequence of a `replace()` call, in the order
    /// first seen: its index names the method, which takes the keywords
    /// positionally in this order (so they are evaluated as written).
    pub(super) replace: Vec<Vec<String>>,
}

thread_local! {
    static USES: RefCell<Uses> = RefCell::new(Uses::default());
    static NON_STRICT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Starts a program: forgets the uses recorded for an earlier one and
/// records whether this one is compiled in non-strict mode, where an
/// `asdict()` of values of different types is a `dict[str, object]`.
pub fn start_program(non_strict: bool) {
    USES.with(|uses| *uses.borrow_mut() = Uses::default());
    NON_STRICT.with(|flag| flag.set(non_strict));
}

/// The local names `body` binds to the four functions by `from dataclasses
/// import ...`, and whether it imports `dataclasses` itself.
fn imported(body: &[Stmt]) -> (HashMap<String, &'static str>, bool) {
    let mut names = HashMap::new();
    let mut module = false;
    for stmt in body {
        match stmt {
            Stmt::ImportFrom(import)
                if import.module.as_deref() == Some("dataclasses") && import.level == 0 =>
            {
                for alias in &import.names {
                    if let Some(function) = function_name(alias.name.as_str()) {
                        let local = alias.asname.as_ref().unwrap_or(&alias.name);
                        names.insert(local.to_string(), function);
                    }
                }
            }
            Stmt::Import(import) => {
                module |= import
                    .names
                    .iter()
                    .any(|a| a.name.as_str() == "dataclasses" && a.asname.is_none());
            }
            _ => {}
        }
    }
    (names, module)
}

fn function_name(name: &str) -> Option<&'static str> {
    match name {
        "fields" => Some("fields"),
        "asdict" => Some("asdict"),
        "astuple" => Some("astuple"),
        "replace" => Some("replace"),
        _ => None,
    }
}

/// The function a call's callee names, if any.
fn callee(func: &Expr, names: &HashMap<String, &'static str>, module: bool) -> Option<&'static str> {
    match func {
        Expr::Name(n) => names.get(n.id.as_str()).copied(),
        Expr::Attribute(a) if module => match &*a.value {
            Expr::Name(n) if n.id.as_str() == "dataclasses" => function_name(a.attr.as_str()),
            _ => None,
        },
        _ => None,
    }
}

/// The keywords of a `replace(obj, k=v, ...)` call this module rewrites.
fn replace_keys(call: &ExprCall) -> Option<Vec<String>> {
    if call.arguments.args.len() != 1 {
        return None;
    }
    call.arguments
        .keywords
        .iter()
        .map(|k| k.arg.as_ref().map(|a| a.to_string()))
        .collect()
}

/// Whether `call` is a call this module rewrites, and to which function.
fn rewritable(call: &ExprCall, names: &HashMap<String, &'static str>, module: bool) -> Option<&'static str> {
    let function = callee(&call.func, names, module)?;
    match function {
        "replace" => replace_keys(call).map(|_| function),
        _ => (call.arguments.args.len() == 1 && call.arguments.keywords.is_empty())
            .then_some(function),
    }
}

/// Records the calls of `body` (module doc) and returns the program's uses
/// so far.
pub(super) fn record_uses(body: &[Stmt]) -> Uses {
    let (names, module) = imported(body);
    if names.is_empty() && !module {
        return USES.with(|uses| uses.borrow().clone());
    }
    struct Scan<'a> {
        names: &'a HashMap<String, &'static str>,
        module: bool,
        found: RefCell<Uses>,
    }
    impl Transformer for Scan<'_> {
        fn visit_expr(&self, expr: &mut Expr) {
            if let Expr::Call(call) = expr
                && let Some(function) = rewritable(call, self.names, self.module)
            {
                let mut found = self.found.borrow_mut();
                match function {
                    "fields" => found.fields = true,
                    "asdict" => found.asdict = true,
                    "astuple" => found.astuple = true,
                    _ => {
                        let keys = replace_keys(call).expect("rewritable");
                        if !found.replace.contains(&keys) {
                            found.replace.push(keys);
                        }
                    }
                }
            }
            transformer::walk_expr(self, expr);
        }
    }
    let scan = Scan {
        names: &names,
        module,
        found: RefCell::new(Uses::default()),
    };
    // The scan needs a mutable walk; it changes nothing.
    let mut copy = body.to_vec();
    for stmt in &mut copy {
        scan.visit_stmt(stmt);
    }
    let found = scan.found.into_inner();
    USES.with(|uses| {
        let mut uses = uses.borrow_mut();
        uses.fields |= found.fields;
        uses.asdict |= found.asdict;
        uses.astuple |= found.astuple;
        for keys in found.replace {
            if !uses.replace.contains(&keys) {
                uses.replace.push(keys);
            }
        }
        uses.clone()
    })
}

/// Rewrites the calls of `body` (module doc) and drops the four names from
/// its `from dataclasses import`.
pub(super) fn rewrite_calls(body: &mut Vec<Stmt>) {
    let (names, module) = imported(body);
    if names.is_empty() && !module {
        return;
    }
    let uses = USES.with(|uses| uses.borrow().clone());
    let rewrite = Rewrite {
        names: &names,
        module,
        replace: &uses.replace,
    };
    for stmt in body.iter_mut() {
        rewrite.visit_stmt(stmt);
    }
    body.retain_mut(|stmt| {
        let Stmt::ImportFrom(import) = stmt else {
            return true;
        };
        if import.module.as_deref() != Some("dataclasses") || import.level != 0 {
            return true;
        }
        import
            .names
            .retain(|alias| function_name(alias.name.as_str()).is_none());
        !import.names.is_empty()
    });
}

struct Rewrite<'a> {
    names: &'a HashMap<String, &'static str>,
    module: bool,
    replace: &'a [Vec<String>],
}

impl Transformer for Rewrite<'_> {
    fn visit_expr(&self, expr: &mut Expr) {
        transformer::walk_expr(self, expr);
        let Expr::Call(call) = expr else {
            return;
        };
        let Some(function) = rewritable(call, self.names, self.module) else {
            return;
        };
        let method = match function {
            "fields" => FIELDS.to_string(),
            "asdict" => ASDICT.to_string(),
            "astuple" => ASTUPLE.to_string(),
            _ => {
                let keys = replace_keys(call).expect("rewritable");
                let index = self
                    .replace
                    .iter()
                    .position(|k| *k == keys)
                    .expect("record_uses saw every replace() call");
                replace_method(index)
            }
        };
        let range = call.range;
        let receiver = call.arguments.args[0].clone();
        // `replace()`'s keywords become positional (method calls take no
        // keywords), in the order written.
        let args: Vec<Expr> = std::mem::take(&mut call.arguments.keywords)
            .into_iter()
            .map(|k| k.value)
            .collect();
        *expr = Expr::Call(ExprCall {
            node_index: Default::default(),
            range,
            func: Box::new(Expr::Attribute(ExprAttribute {
                node_index: Default::default(),
                range,
                value: Box::new(receiver),
                attr: ruff_python_ast::Identifier::new(method, range),
                ctx: ExprContext::Load,
            })),
            arguments: Arguments {
                node_index: Default::default(),
                range,
                args: args.into_boxed_slice(),
                keywords: Default::default(),
            },
        });
    }
}

fn replace_method(index: usize) -> String {
    format!("_pycc_replace_{index}_")
}

/// The module-level class `fields()` returns instances of.
pub(super) fn field_class_source() -> String {
    format!(
        "class {FIELD_CLASS}:\n    def __init__(self, name: str) -> None:\n        \
         self.name = name\n\n    def __repr__(self) -> str:\n        \
         return f\"Field(name={{self.name!r}})\"\n"
    )
}

/// How a field's value converts in `asdict`/`astuple` (module doc).
enum Shape {
    /// Kept as is: `int`, `float`, `str`, `bool`, an enum member.
    Scalar(String),
    /// A dataclass written out earlier in the module.
    Data(String),
    List(Box<Shape>),
    Dict(Box<Shape>, Box<Shape>),
    Set(Box<Shape>),
    Tuple(Vec<Shape>),
    /// `tuple[T, ...]` of a scalar.
    VarTuple(Box<Shape>),
    Optional(Box<Shape>),
}

/// What the rewrite knows when it shapes an annotation: the dataclasses
/// written out so far (with their converted types) and the enum classes.
pub(super) struct Context<'a> {
    /// Each written-out dataclass's `asdict` and `astuple` result types
    /// (`None` when not generated).
    pub(super) converted: &'a HashMap<String, (Option<String>, Option<String>)>,
    pub(super) enums: &'a HashSet<String>,
}

fn shape(annotation: &Expr, cx: &Context) -> Option<Shape> {
    match annotation {
        Expr::Name(n) => {
            let name = n.id.as_str();
            if matches!(name, "int" | "float" | "str" | "bool") || cx.enums.contains(name) {
                Some(Shape::Scalar(name.to_string()))
            } else if cx.converted.contains_key(name) {
                Some(Shape::Data(name.to_string()))
            } else {
                None
            }
        }
        Expr::StringLiteral(s) => {
            let text = s.value.to_str();
            let parsed = ruff_python_parser::parse_expression(text).ok()?;
            shape(parsed.expr(), cx)
        }
        Expr::Subscript(sub) => {
            let Expr::Name(base) = &*sub.value else {
                return None;
            };
            let args: Vec<&Expr> = match &*sub.slice {
                Expr::Tuple(t) => t.elts.iter().collect(),
                other => vec![other],
            };
            match (base.id.as_str(), args.as_slice()) {
                ("list", [elem]) => Some(Shape::List(Box::new(shape(elem, cx)?))),
                ("set", [elem]) => match shape(elem, cx)? {
                    scalar @ Shape::Scalar(_) => Some(Shape::Set(Box::new(scalar))),
                    _ => None,
                },
                ("dict", [k, v]) => Some(Shape::Dict(
                    Box::new(shape(k, cx)?),
                    Box::new(shape(v, cx)?),
                )),
                ("tuple", [elem, Expr::EllipsisLiteral(_)]) => match shape(elem, cx)? {
                    scalar @ Shape::Scalar(_) => Some(Shape::VarTuple(Box::new(scalar))),
                    _ => None,
                },
                ("tuple", elems) if !elems.is_empty() => Some(Shape::Tuple(
                    elems.iter().map(|e| shape(e, cx)).collect::<Option<_>>()?,
                )),
                ("Optional", [inner]) => Some(Shape::Optional(Box::new(shape(inner, cx)?))),
                _ => None,
            }
        }
        Expr::BinOp(op) if op.op == ruff_python_ast::Operator::BitOr => {
            let (inner, other) = match (&*op.left, &*op.right) {
                (Expr::NoneLiteral(_), inner) | (inner, Expr::NoneLiteral(_)) => (inner, ()),
                _ => return None,
            };
            let () = other;
            Some(Shape::Optional(Box::new(shape(inner, cx)?)))
        }
        _ => None,
    }
}

/// Whether a value of `shape` is kept as is (no conversion, no copy).
fn is_scalar(shape: &Shape) -> bool {
    match shape {
        Shape::Scalar(_) | Shape::VarTuple(_) => true,
        Shape::Optional(inner) => is_scalar(inner),
        Shape::Tuple(items) => items.iter().all(is_scalar),
        _ => false,
    }
}

/// `(expression, type)` converting `value` of `shape` for `asdict`
/// (`dict`) or `astuple`; `None` when a nested dataclass has no such
/// method. `depth` keeps comprehension variables apart.
fn convert(value: &str, shape: &Shape, dict: bool, cx: &Context, depth: usize) -> Option<(String, String)> {
    let var = |n: &str| format!("_pycc_{n}{depth}");
    Some(match shape {
        Shape::Scalar(ty) => (value.to_string(), ty.clone()),
        Shape::VarTuple(inner) => {
            let Shape::Scalar(ty) = &**inner else {
                return None;
            };
            (value.to_string(), format!("tuple[{ty}, ...]"))
        }
        Shape::Data(class) => {
            let (as_dict, as_tuple) = cx.converted.get(class)?;
            if dict {
                (format!("{value}.{ASDICT}()"), as_dict.clone()?)
            } else {
                (format!("{value}.{ASTUPLE}()"), as_tuple.clone()?)
            }
        }
        Shape::List(elem) => {
            let v = var("v");
            let (e, t) = convert(&v, elem, dict, cx, depth + 1)?;
            if is_scalar(elem) {
                (format!("list({value})"), format!("list[{t}]"))
            } else {
                (format!("[{e} for {v} in {value}]"), format!("list[{t}]"))
            }
        }
        Shape::Set(elem) => {
            let (_, t) = convert("", elem, dict, cx, depth + 1)?;
            (format!("set({value})"), format!("set[{t}]"))
        }
        Shape::Dict(k, v) => {
            let (kv, vv) = (var("k"), var("v"));
            let (ke, kt) = convert(&kv, k, dict, cx, depth + 1)?;
            let (ve, vt) = convert(&vv, v, dict, cx, depth + 1)?;
            if is_scalar(k) && is_scalar(v) {
                (format!("dict({value})"), format!("dict[{kt}, {vt}]"))
            } else {
                (
                    format!("{{{ke}: {ve} for {kv}, {vv} in {value}.items()}}"),
                    format!("dict[{kt}, {vt}]"),
                )
            }
        }
        Shape::Tuple(items) => {
            let mut exprs = Vec::new();
            let mut tys = Vec::new();
            for (i, item) in items.iter().enumerate() {
                let (e, t) = convert(&format!("{value}[{i}]"), item, dict, cx, depth + 1)?;
                exprs.push(e);
                tys.push(t);
            }
            let expr = if items.iter().all(is_scalar) {
                value.to_string()
            } else if exprs.len() == 1 {
                format!("({},)", exprs[0])
            } else {
                format!("({})", exprs.join(", "))
            };
            (expr, format!("tuple[{}]", tys.join(", ")))
        }
        Shape::Optional(inner) => {
            let (e, t) = convert(value, inner, dict, cx, depth)?;
            if is_scalar(inner) {
                (e, format!("{t} | None"))
            } else {
                (format!("(None if {value} is None else {e})"), format!("{t} | None"))
            }
        }
    })
}

/// What [`methods`] generates for one class.
pub(super) struct Generated {
    /// The methods, as class-body source with [`HOLE`]s.
    pub(super) source: String,
    /// The user annotations the [`HOLE`]s stand for.
    pub(super) holes: Vec<Expr>,
    /// The class's `asdict()` result type, when it has one to nest.
    pub(super) as_dict_ty: Option<String>,
    /// The class's `astuple()` result type.
    pub(super) as_tuple_ty: Option<String>,
}

/// The methods of a class with `fields` for the program's `uses`.
/// `init` tells whether the class's `__init__` is the generated one (taking
/// the `init` fields in order), which `replace()` calls.
pub(super) fn methods(
    class: &str,
    fields: &[Field],
    init: bool,
    uses: &Uses,
    cx: &Context,
) -> Generated {
    let mut source = String::new();
    let mut holes = Vec::new();
    if uses.fields {
        let items: Vec<String> = fields
            .iter()
            .map(|f| format!("{FIELD_CLASS}({:?})", f.name))
            .collect();
        source.push_str(&format!(
            "    @staticmethod\n    def {FIELDS}() -> list[{FIELD_CLASS}]:\n        return [{}]\n",
            items.join(", ")
        ));
    }
    let shapes: Option<Vec<Shape>> = fields.iter().map(|f| shape(&f.annotation, cx)).collect();
    let mut as_dict_ty = None;
    let mut as_tuple_ty = None;
    if let Some(shapes) = shapes.filter(|s| !s.is_empty()) {
        let converted = |dict: bool| -> Option<Vec<(String, String)>> {
            fields
                .iter()
                .zip(&shapes)
                .map(|(f, s)| convert(&format!("self.{}", f.name), s, dict, cx, 0))
                .collect()
        };
        if let Some(parts) = converted(true) {
            let first = &parts[0].1;
            // Values of different types: a CPython dict in non-strict mode;
            // strict mode has no type for it (the call is refused).
            let ty = if parts.iter().all(|(_, t)| t == first) {
                Some(format!("dict[str, {first}]"))
            } else if NON_STRICT.with(std::cell::Cell::get) {
                Some("dict[str, object]".to_string())
            } else {
                None
            };
            let entries: Vec<String> = fields
                .iter()
                .zip(&parts)
                .map(|(f, (e, _))| format!("{:?}: {e}", f.name))
                .collect();
            if uses.asdict
                && let Some(ty) = &ty
            {
                source.push_str(&format!(
                    "    def {ASDICT}(self) -> {ty}:\n        return {{{}}}\n",
                    entries.join(", ")
                ));
            }
            as_dict_ty = ty;
        }
        if let Some(parts) = converted(false) {
            let ty = format!(
                "tuple[{}]",
                parts.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(", ")
            );
            let value = if parts.len() == 1 {
                format!("({},)", parts[0].0)
            } else {
                format!(
                    "({})",
                    parts.iter().map(|(e, _)| e.as_str()).collect::<Vec<_>>().join(", ")
                )
            };
            if uses.astuple {
                source.push_str(&format!(
                    "    def {ASTUPLE}(self) -> {ty}:\n        return {value}\n"
                ));
            }
            as_tuple_ty = Some(ty);
        }
    }
    if init {
        let init_fields: Vec<&Field> = fields.iter().filter(|f| f.init).collect();
        for (index, keys) in uses.replace.iter().enumerate() {
            if !keys.iter().all(|k| init_fields.iter().any(|f| f.name == *k)) {
                continue;
            }
            let params: Vec<String> = keys
                .iter()
                .map(|key| {
                    let field = init_fields
                        .iter()
                        .find(|f| f.name == *key)
                        .expect("every key names an init field");
                    holes.push(field.annotation.clone());
                    format!("{key}: {HOLE}{}", holes.len() - 1)
                })
                .collect();
            let args: Vec<String> = init_fields
                .iter()
                .map(|f| {
                    if keys.contains(&f.name) {
                        f.name.clone()
                    } else {
                        format!("self.{}", f.name)
                    }
                })
                .collect();
            let mut all = vec!["self".to_string()];
            all.extend(params);
            source.push_str(&format!(
                "    def {}({}) -> \"{class}\":\n        return {class}({})\n",
                replace_method(index),
                all.join(", "),
                args.join(", ")
            ));
        }
    }
    Generated {
        source,
        holes,
        as_dict_ty,
        as_tuple_ty,
    }
}

/// The placeholder a `replace()` parameter annotation is written as.
const HOLE: &str = "__pycc_rhole_";

/// Fills the [`HOLE`]s of `stmts` with `holes`.
pub(super) fn fill_holes(stmts: &mut [Stmt], holes: &[Expr]) {
    struct Fill<'a>(&'a [Expr]);
    impl Transformer for Fill<'_> {
        fn visit_expr(&self, expr: &mut Expr) {
            if let Expr::Name(n) = expr
                && let Some(index) = n.id.as_str().strip_prefix(HOLE)
                && let Ok(index) = index.parse::<usize>()
            {
                *expr = self.0[index].clone();
                return;
            }
            transformer::walk_expr(self, expr);
        }
    }
    let fill = Fill(holes);
    for stmt in stmts {
        fill.visit_stmt(stmt);
    }
}
