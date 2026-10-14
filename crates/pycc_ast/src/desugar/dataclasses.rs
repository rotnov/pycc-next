//! `@dataclass` classes (PEP 557) become plain classes with the methods
//! `dataclasses` would generate, written out as Python source:
//!
//! * `__init__` taking the fields in order (a base dataclass's fields
//!   first), with their defaults. A `field(default_factory=f)` default is
//!   written [`FACTORY_MARKER`]`(f)`, which `super::signatures` turns into a
//!   call of `f` at every call site that omits the argument -- a fresh
//!   value per call, as the generated `__init__` makes one. `__post_init__`
//!   is called when the class defines it.
//! * `__repr__` (`P(x=1, name='a')`), `__eq__` (the fields compared in
//!   order), the four ordering methods for `order=True` (the fields as a
//!   tuple), and `__hash__` of the fields' tuple for `frozen=True`.
//! * `frozen=True`: each field is stored under [`frozen_slot`] and read
//!   through a property whose setter raises `FrozenInstanceError`
//!   (`dataclasses.FrozenInstanceError: cannot assign to field 'x'`), an
//!   `AttributeError` subclass declared once as [`FROZEN_ERROR`].
//!
//! A method the class body defines itself is kept and not generated, as
//! in CPython. A non-dataclass subclass of a dataclass (in the same module)
//! that defines no `__repr__` gets one naming its own class, as CPython's
//! repr does (`type(self).__qualname__`).
//!
//! A class this module cannot write out -- an unknown `@dataclass(...)`
//! option or `field(...)` argument, an `InitVar`, a field without a default
//! after one with a default -- is left decorated, and HIR lowering reports
//! it (`C0001`).

use super::signatures::parse_helper;
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::{Expr, ExprContext, Stmt, StmtClassDef};
use ruff_text_size::{Ranged, TextRange};
use std::collections::HashMap;

/// The call wrapping a `default_factory` default (see the module doc).
pub const FACTORY_MARKER: &str = "__pycc_default_factory__";

/// The class `dataclasses.FrozenInstanceError` compiles to; the runtime
/// prints it by its CPython name (`pycc_codegen`'s exception rendering).
pub const FROZEN_ERROR: &str = "__pycc_FrozenInstanceError";

/// The attribute a frozen dataclass stores field `name` in.
fn frozen_slot(name: &str) -> String {
    format!("_pycc_frozen_{name}")
}

/// One dataclass field.
#[derive(Clone)]
struct Field {
    name: String,
    annotation: Expr,
    default: Option<Default>,
    init: bool,
    repr: bool,
    compare: bool,
}

#[derive(Clone)]
enum Default {
    Value(Expr),
    Factory(Expr),
}

/// The `@dataclass(...)` options this module writes out.
#[derive(Clone)]
struct Options {
    init: bool,
    repr: bool,
    eq: bool,
    order: bool,
    frozen: bool,
    unsafe_hash: bool,
}

/// What the module knows of each class written out so far: its fields
/// and options (a non-dataclass subclass of a dataclass inherits both).
#[derive(Clone)]
struct Known {
    fields: Vec<Field>,
    options: Options,
}

/// Rewrites every top-level `@dataclass` class of `body` (module doc).
pub(super) fn expand_dataclasses(body: &mut Vec<Stmt>) {
    let mut known: HashMap<String, Known> = HashMap::new();
    let mut any_frozen = false;
    let mut factories: Vec<(usize, Vec<Stmt>)> = Vec::new();
    for (index, stmt) in body.iter_mut().enumerate() {
        let Stmt::ClassDef(class) = stmt else {
            continue;
        };
        let bases = base_names(class);
        let inherited = bases.iter().rev().filter_map(|b| known.get(b)).fold(
            Vec::<Field>::new(),
            |mut acc, k| {
                merge_fields(&mut acc, &k.fields);
                acc
            },
        );
        let base_options = bases.iter().find_map(|b| known.get(b)).map(|k| k.options.clone());
        let base_frozen = base_options.as_ref().is_some_and(|o| o.frozen);
        match dataclass_options(class) {
            Some(Ok(options)) => {
                let Some(mut own) = own_fields(class) else {
                    continue;
                };
                let helpers = factory_helpers(&class.name, &mut own, class.range);
                let mut fields = inherited;
                merge_fields(&mut fields, &own);
                // CPython refuses a frozen dataclass inheriting from a non-frozen one,
                // and the reverse (`TypeError`): left to HIR.
                let has_dataclass_base = bases.iter().any(|b| known.contains_key(b));
                if !defaults_in_order(&fields) || (has_dataclass_base && options.frozen != base_frozen) {
                    continue;
                }
                write_out(class, &fields, &options);
                factories.push((index, helpers));
                any_frozen |= options.frozen;
                known.insert(class.name.to_string(), Known { fields, options });
            }
            Some(Err(())) => {}
            None => {
                if let Some(options) = base_options {
                    let generated = dunders(class, &inherited, &options);
                    class.body.extend(generated);
                    known.insert(
                        class.name.to_string(),
                        Known {
                            fields: inherited,
                            options,
                        },
                    );
                }
            }
        }
    }
    for (index, helpers) in factories.into_iter().rev() {
        body.splice(index..index, helpers);
    }
    let imports_error = drop_dataclasses_imports(body);
    if any_frozen || imports_error {
        let at = body
            .iter()
            .position(|s| !matches!(s, Stmt::Import(_) | Stmt::ImportFrom(_)) && !is_docstring(s))
            .unwrap_or(body.len());
        let range = body.get(at).map_or_else(TextRange::default, Ranged::range);
        let class = parse_helper(
            &format!("class {FROZEN_ERROR}(AttributeError):\n    pass\n"),
            range,
        );
        body.splice(at..at, class);
        if imports_error {
            let rename = Rename;
            for stmt in body.iter_mut() {
                rename.visit_stmt(stmt);
            }
        }
    }
}

fn is_docstring(stmt: &Stmt) -> bool {
    matches!(stmt, Stmt::Expr(e) if matches!(*e.value, Expr::StringLiteral(_)))
}

/// The bare-name bases of `class`.
fn base_names(class: &StmtClassDef) -> Vec<String> {
    class
        .arguments
        .as_ref()
        .map(|args| {
            args.args
                .iter()
                .filter_map(|b| match b {
                    Expr::Name(n) => Some(n.id.to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Appends `fields` to `acc`, a field already present keeping its position
/// and taking the later definition (CPython's field-collection order).
fn merge_fields(acc: &mut Vec<Field>, fields: &[Field]) {
    for field in fields {
        match acc.iter_mut().find(|f| f.name == field.name) {
            Some(existing) => *existing = field.clone(),
            None => acc.push(field.clone()),
        }
    }
}

/// CPython's `non-default argument 'y' follows default argument` rule.
fn defaults_in_order(fields: &[Field]) -> bool {
    let mut seen_default = false;
    for field in fields.iter().filter(|f| f.init) {
        if field.default.is_some() {
            seen_default = true;
        } else if seen_default {
            return false;
        }
    }
    true
}

/// Whether `expr` names the `dataclass` decorator (`dataclass`,
/// `dataclasses.dataclass`).
fn is_dataclass_name(expr: &Expr) -> bool {
    match expr {
        Expr::Name(n) => n.id.as_str() == "dataclass",
        Expr::Attribute(a) => {
            a.attr.as_str() == "dataclass"
                && matches!(&*a.value, Expr::Name(n) if n.id.as_str() == "dataclasses")
        }
        _ => false,
    }
}

/// The class's `@dataclass` options: `None` when it is not a dataclass,
/// `Some(Err)` for one this module does not write out.
fn dataclass_options(class: &StmtClassDef) -> Option<Result<Options, ()>> {
    let [decorator] = class.decorator_list.as_slice() else {
        return class
            .decorator_list
            .iter()
            .any(|d| {
                is_dataclass_name(&d.expression)
                    || matches!(&d.expression, Expr::Call(c) if is_dataclass_name(&c.func))
            })
            .then_some(Err(()));
    };
    let mut options = Options {
        init: true,
        repr: true,
        eq: true,
        order: false,
        frozen: false,
        unsafe_hash: false,
    };
    match &decorator.expression {
        e if is_dataclass_name(e) => {}
        Expr::Call(call) if is_dataclass_name(&call.func) => {
            if !call.arguments.args.is_empty() {
                return Some(Err(()));
            }
            for keyword in call.arguments.keywords.iter() {
                let (Some(arg), Expr::BooleanLiteral(value)) = (&keyword.arg, &keyword.value)
                else {
                    return Some(Err(()));
                };
                let slot = match arg.as_str() {
                    "init" => &mut options.init,
                    "repr" => &mut options.repr,
                    "eq" => &mut options.eq,
                    "order" => &mut options.order,
                    "frozen" => &mut options.frozen,
                    "unsafe_hash" => &mut options.unsafe_hash,
                    // `match_args` changes nothing pycc reads.
                    "match_args" => continue,
                    _ => return Some(Err(())),
                };
                *slot = value.value;
            }
        }
        _ => return None,
    }
    if class.type_params.is_some() || (options.order && !options.eq) {
        return Some(Err(()));
    }
    if options.order
        && ["__lt__", "__le__", "__gt__", "__ge__"]
            .iter()
            .any(|m| defines(class, m))
    {
        return Some(Err(()));
    }
    Some(Ok(options))
}

/// Whether the class body defines method `name`.
fn defines(class: &StmtClassDef, name: &str) -> bool {
    class
        .body
        .iter()
        .any(|s| matches!(s, Stmt::FunctionDef(def) if def.name.as_str() == name))
}

/// Whether `annotation` is `ClassVar[...]` / `typing.ClassVar[...]` (not a
/// field), or `InitVar[...]` (unsupported: `None` from [`own_fields`]).
fn annotation_marker(annotation: &Expr) -> Option<&str> {
    let base = match annotation {
        Expr::Subscript(s) => &*s.value,
        other => other,
    };
    match base {
        Expr::Name(n) if matches!(n.id.as_str(), "ClassVar" | "InitVar") => Some(n.id.as_str()),
        Expr::Attribute(a) if matches!(a.attr.as_str(), "ClassVar" | "InitVar") => {
            Some(a.attr.as_str())
        }
        _ => None,
    }
}

/// The fields `class`'s own body declares, or `None` when one cannot be
/// written out.
fn own_fields(class: &StmtClassDef) -> Option<Vec<Field>> {
    let mut fields = Vec::new();
    for stmt in &class.body {
        let Stmt::AnnAssign(ann) = stmt else {
            continue;
        };
        match annotation_marker(&ann.annotation) {
            Some("ClassVar") => continue,
            Some(_) => return None,
            None => {}
        }
        let Expr::Name(target) = &*ann.target else {
            return None;
        };
        let mut field = Field {
            name: target.id.to_string(),
            annotation: (*ann.annotation).clone(),
            default: None,
            init: true,
            repr: true,
            compare: true,
        };
        match ann.value.as_deref() {
            None => {}
            Some(Expr::Call(call))
                if matches!(&*call.func, Expr::Name(n) if n.id.as_str() == "field")
                    || matches!(&*call.func, Expr::Attribute(a) if a.attr.as_str() == "field") =>
            {
                if !call.arguments.args.is_empty() {
                    return None;
                }
                for keyword in call.arguments.keywords.iter() {
                    let arg = keyword.arg.as_ref()?;
                    match (arg.as_str(), &keyword.value) {
                        ("default", value) if field.default.is_none() => {
                            field.default = Some(Default::Value(value.clone()));
                        }
                        ("default_factory", value) if field.default.is_none() => {
                            field.default = Some(Default::Factory(value.clone()));
                        }
                        ("init", Expr::BooleanLiteral(b)) => field.init = b.value,
                        ("repr", Expr::BooleanLiteral(b)) => field.repr = b.value,
                        ("compare", Expr::BooleanLiteral(b)) => field.compare = b.value,
                        _ => return None,
                    }
                }
            }
            Some(value) => {
                // CPython refuses a `list`/`dict`/`set` default (`ValueError:
                // mutable default`): leave it to HIR's refusal.
                if matches!(
                    value,
                    Expr::List(_) | Expr::Dict(_) | Expr::Set(_) | Expr::ListComp(_)
                ) {
                    return None;
                }
                field.default = Some(Default::Value(value.clone()));
            }
        }
        if !field.init && field.default.is_none() {
            return None;
        }
        fields.push(field);
    }
    Some(fields)
}

/// A module-level `def __pycc_factory_<C>_<f>() -> T` for each
/// `default_factory` field of class `class_name` (its return annotation
/// types the fresh value, an empty `[]` or `{}` included), each field's
/// factory replaced by the helper's name.
fn factory_helpers(class_name: &str, fields: &mut [Field], range: TextRange) -> Vec<Stmt> {
    let mut helpers = Vec::new();
    for field in fields.iter_mut() {
        let Some(Default::Factory(factory)) = &field.default else {
            continue;
        };
        let name = format!("__pycc_factory_{class_name}_{}", field.name);
        let mut holes = Holes { exprs: Vec::new() };
        let annotation = holes.hole(&field.annotation);
        let value = match factory {
            Expr::Name(n) if n.id.as_str() == "list" => "[]".to_string(),
            Expr::Name(n) if n.id.as_str() == "dict" => "{}".to_string(),
            other => format!("{}()", holes.hole(other)),
        };
        let mut stmts = parse_helper(
            &format!("def {name}() -> {annotation}:\n    return {value}\n"),
            range,
        );
        for stmt in &mut stmts {
            holes.visit_stmt(stmt);
        }
        helpers.extend(stmts);
        field.default = Some(Default::Factory(super::build::load(&name, range)));
    }
    helpers
}

/// Placeholder names standing for user expressions in generated source.
struct Holes {
    exprs: Vec<Expr>,
}

impl Holes {
    fn hole(&mut self, expr: &Expr) -> String {
        self.exprs.push(expr.clone());
        format!("__pycc_hole_{}", self.exprs.len() - 1)
    }
}

impl Transformer for Holes {
    fn visit_expr(&self, expr: &mut Expr) {
        if let Expr::Name(n) = expr
            && let Some(index) = n.id.as_str().strip_prefix("__pycc_hole_")
            && let Ok(index) = index.parse::<usize>()
        {
            *expr = self.exprs[index].clone();
            return;
        }
        transformer::walk_expr(self, expr);
    }
}

/// Parses the generated methods `source` (indented as a class body) into
/// statements at `range`, with every hole filled.
fn methods(source: &str, holes: &Holes, range: TextRange) -> Vec<Stmt> {
    let parsed = parse_helper(&format!("class __pycc_dataclass:\n{source}"), range);
    let Some(Stmt::ClassDef(class)) = parsed.into_iter().next() else {
        unreachable!("a generated class parses as a class");
    };
    let mut body: Vec<Stmt> = class.body.into_iter().collect();
    for stmt in &mut body {
        holes.visit_stmt(stmt);
    }
    body
}

/// The expression reading field `name` of `receiver`.
fn read(receiver: &str, name: &str) -> String {
    format!("{receiver}.{name}")
}

/// `__repr__` of class `name` over `fields`.
fn repr_method(name: &str, fields: &[Field], range: TextRange) -> Vec<Stmt> {
    let parts: Vec<String> = fields
        .iter()
        .filter(|f| f.repr)
        .map(|f| format!("{}={{{}!r}}", f.name, read("self", &f.name)))
        .collect();
    let source = format!(
        "    def __repr__(self) -> str:\n        return f\"{name}({})\"\n",
        parts.join(", ")
    );
    methods(&source, &Holes { exprs: Vec::new() }, range)
}

/// Rewrites `class` into a plain class (module doc).
fn write_out(class: &mut StmtClassDef, fields: &[Field], options: &Options) {
    let range = class.range;
    let mut holes = Holes { exprs: Vec::new() };
    let mut source = String::new();
    // Field declarations are the generated `__init__`'s business; class
    // variables, methods and docstrings stay.
    let body: Vec<Stmt> = std::mem::take(&mut class.body)
        .into_iter()
        .filter(|s| match s {
            Stmt::AnnAssign(ann) => annotation_marker(&ann.annotation) == Some("ClassVar"),
            _ => true,
        })
        .collect();
    class.body = body.into();
    class.decorator_list.clear();
    let store = |field: &str| {
        if options.frozen {
            format!("self.{}", frozen_slot(field))
        } else {
            format!("self.{field}")
        }
    };
    if options.init && !defines(class, "__init__") {
        let mut params = vec!["self".to_string()];
        let mut lines = Vec::new();
        for field in fields {
            let annotation = holes.hole(&field.annotation);
            if field.init {
                let param = match &field.default {
                    None => format!("{}: {annotation}", field.name),
                    Some(Default::Value(value)) => {
                        format!("{}: {annotation} = {}", field.name, holes.hole(value))
                    }
                    Some(Default::Factory(factory)) => format!(
                        "{}: {annotation} = {FACTORY_MARKER}({})",
                        field.name,
                        holes.hole(factory)
                    ),
                };
                params.push(param);
                lines.push(format!("        {} = {}\n", store(&field.name), field.name));
            } else {
                let value = match &field.default {
                    Some(Default::Value(value)) => holes.hole(value),
                    Some(Default::Factory(factory)) => format!("{}()", holes.hole(factory)),
                    None => unreachable!("`own_fields` refuses an `init=False` field without a default"),
                };
                lines.push(format!("        {} = {value}\n", store(&field.name)));
            }
        }
        if defines(class, "__post_init__") {
            lines.push("        self.__post_init__()\n".to_string());
        }
        if lines.is_empty() {
            lines.push("        pass\n".to_string());
        }
        source.push_str(&format!(
            "    def __init__({}) -> None:\n{}",
            params.join(", "),
            lines.concat()
        ));
    }
    if options.frozen {
        for field in fields {
            let annotation = holes.hole(&field.annotation);
            source.push_str(&format!(
                "    @property\n    def {f}(self) -> {annotation}:\n        return self.{slot}\n\
                 \x20   @{f}.setter\n    def {f}(self, value: {annotation}) -> None:\n        \
                 raise {FROZEN_ERROR}(\"cannot assign to field '{f}'\")\n",
                f = field.name,
                slot = frozen_slot(&field.name),
            ));
        }
    }
    let mut generated = if source.is_empty() {
        Vec::new()
    } else {
        methods(&source, &holes, range)
    };
    generated.extend(dunders(class, fields, options));
    class.body.extend(generated);
    if class.body.is_empty() {
        class.body.push(pass(range));
    }
}

/// `__eq__`, the ordering methods, `__hash__` and `__repr__` of `class`
/// over `fields`, as `options` asks for them; a method the class body
/// defines itself is not generated. A subclass gets its own (its name in
/// the repr, its own instances as `other`).
fn dunders(class: &StmtClassDef, fields: &[Field], options: &Options) -> Vec<Stmt> {
    let name = class.name.to_string();
    let compared: Vec<&Field> = fields.iter().filter(|f| f.compare).collect();
    let tuple = |receiver: &str| -> String {
        match compared.as_slice() {
            [] => "()".to_string(),
            [one] => format!("({},)", read(receiver, &one.name)),
            many => format!(
                "({})",
                many.iter()
                    .map(|f| read(receiver, &f.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    };
    let mut source = String::new();
    if options.eq && !defines(class, "__eq__") {
        let test = if compared.is_empty() {
            "True".to_string()
        } else {
            compared
                .iter()
                .map(|f| format!("{} == {}", read("self", &f.name), read("other", &f.name)))
                .collect::<Vec<_>>()
                .join(" and ")
        };
        source.push_str(&format!(
            "    def __eq__(self, other: \"{name}\") -> bool:\n        return {test}\n"
        ));
    }
    if options.order {
        for (method, op) in [("__lt__", "<"), ("__le__", "<="), ("__gt__", ">"), ("__ge__", ">=")] {
            if defines(class, method) {
                continue;
            }
            let body = match (compared.is_empty(), op) {
                (true, "<=" | ">=") => "True".to_string(),
                (true, _) => "False".to_string(),
                (false, _) => format!("{} {op} {}", tuple("self"), tuple("other")),
            };
            source.push_str(&format!(
                "    def {method}(self, other: \"{name}\") -> bool:\n        return {body}\n"
            ));
        }
    }
    if ((options.eq && options.frozen) || options.unsafe_hash)
        && !compared.is_empty()
        && !defines(class, "__hash__")
    {
        source.push_str(&format!(
            "    def __hash__(self) -> int:\n        return hash({})\n",
            tuple("self")
        ));
    }
    let mut generated = if source.is_empty() {
        Vec::new()
    } else {
        methods(&source, &Holes { exprs: Vec::new() }, class.range)
    };
    if options.repr && !defines(class, "__repr__") {
        generated.extend(repr_method(&name, fields, class.range));
    }
    generated
}

fn pass(range: TextRange) -> Stmt {
    Stmt::Pass(ruff_python_ast::StmtPass {
        node_index: ruff_python_ast::AtomicNodeIndex::default(),
        range,
    })
}

/// Drops the names this module replaces from `from dataclasses import
/// ...` (`field`, `FrozenInstanceError`); `true` when `FrozenInstanceError`
/// was imported (its uses are renamed to [`FROZEN_ERROR`]).
fn drop_dataclasses_imports(body: &mut Vec<Stmt>) -> bool {
    let mut frozen_error = false;
    body.retain_mut(|stmt| {
        let Stmt::ImportFrom(import) = stmt else {
            return true;
        };
        if import.module.as_deref() != Some("dataclasses") || import.level != 0 {
            return true;
        }
        import.names.retain(|alias| {
            let dropped = alias.asname.is_none()
                && matches!(alias.name.as_str(), "field" | "FrozenInstanceError");
            frozen_error |= dropped && alias.name.as_str() == "FrozenInstanceError";
            !dropped
        });
        !import.names.is_empty()
    });
    frozen_error
}

/// Renames reads of `FrozenInstanceError` to [`FROZEN_ERROR`].
struct Rename;

impl Transformer for Rename {
    fn visit_expr(&self, expr: &mut Expr) {
        if let Expr::Name(n) = expr
            && n.id.as_str() == "FrozenInstanceError"
            && n.ctx == ExprContext::Load
        {
            n.id = FROZEN_ERROR.into();
            return;
        }
        transformer::walk_expr(self, expr);
    }
}

#[cfg(test)]
mod tests;
