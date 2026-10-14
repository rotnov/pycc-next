//! The full Python signature of an `--ext` module's public functions and
//! methods, published to Python callers.
//!
//! A public `def` with a default, a positional-only or keyword-only
//! parameter, a `*args` or a `**kwargs` cannot cross the CPython boundary as
//! written: the boundary binds plain positional arguments, and inside the
//! module the function takes its flattened parameters
//! ([`super::signatures`]). For each such `def` (and each such method of a
//! public class) this module plans two halves:
//!
//! * a compiled **entry**, `_pycc_sig_<qualname>`, added to the module next
//!   to the `def` (after the class for a method) by
//!   [`split_ext_signatures`] before desugaring. It takes every parameter
//!   positionally -- `*args` and `**kwargs` as CPython objects (a tuple, and
//!   the keys and values of a dict as two tuples), converted element by
//!   element to the declared type -- and calls the function with its own
//!   signature, so the call is bound exactly as one inside the module;
//! * a **Python wrapper** with the declared signature, defaults included,
//!   whose source the artifact runs when the entry is published: CPython
//!   itself binds the host's call and raises its own `TypeError` for a
//!   missing, unexpected or duplicate argument, and the wrapper calls the
//!   entry. The wrapper is bound to the function's name in the module, or
//!   reached through the method's row of the class's type object.
//!
//! A `def` is planned only when every parameter (the receiver aside) and its
//! return are annotated, its `*args`/`**kwargs` elements are `int`, `float`,
//! `str` or `bool`, it is not a generator and carries no decorator, and --
//! for a method -- no other top-level class defines a method of that name.
//! Anything else keeps the boundary's refusal.

use super::closures::qualname;
use ruff_python_ast::{Expr, ModModule, Parameters, Stmt, StmtFunctionDef};
use ruff_text_size::{Ranged, TextRange};
use std::collections::HashMap;
use std::fmt::Write as _;

/// The prefix of an entry's name.
const ENTRY_PREFIX: &str = "_pycc_sig_";

/// One published signature: the function or method, its compiled entry and
/// the source of its Python wrapper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtSignature {
    /// `f`, or `Class.method`.
    pub qualname: String,
    /// The compiled entry the wrapper calls (a module-level function).
    pub entry: String,
    /// Python source defining `_pycc_make(_pycc_entry)`, which returns the
    /// wrapper: a function of the declared signature (its defaults
    /// evaluated in the module's namespace when the source runs) and of
    /// `__qualname__` [`ExtSignature::qualname`].
    pub wrapper: String,
}

impl ExtSignature {
    /// `Some((class, method))` for a method's signature.
    #[must_use]
    pub fn method(&self) -> Option<(&str, &str)> {
        self.qualname.split_once('.')
    }
}

/// The qualname whose entry `name` is (`_pycc_sig_...`), or `None` for any
/// other name.
#[must_use]
pub fn ext_entry_target(name: &str) -> Option<String> {
    qualname::decode(name.strip_prefix(ENTRY_PREFIX)?)
}

fn entry_name(qualname: &str) -> String {
    format!("{ENTRY_PREFIX}{}", qualname::encode(qualname))
}

/// The element type of a `*args`/`**kwargs`, by its annotation.
#[derive(Clone, Copy)]
enum Element {
    Int,
    Float,
    Str,
    Bool,
}

impl Element {
    fn of(annotation: Option<&Expr>) -> Option<Self> {
        let Some(Expr::Name(name)) = annotation else {
            return None;
        };
        Some(match name.id.as_str() {
            "int" => Element::Int,
            "float" => Element::Float,
            "str" => Element::Str,
            "bool" => Element::Bool,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Element::Int => "int",
            Element::Float => "float",
            Element::Str => "str",
            Element::Bool => "bool",
        }
    }

    /// The wrapper's test that `x` is a value of this type (the names are
    /// the wrapper's own bindings of the builtins).
    fn check(self, x: &str) -> String {
        match self {
            Element::Int => format!("_pycc_isinstance({x}, _pycc_int)"),
            Element::Float => format!("_pycc_isinstance({x}, (_pycc_int, _pycc_float))"),
            Element::Str => format!("_pycc_isinstance({x}, _pycc_str)"),
            Element::Bool => format!("_pycc_isinstance({x}, _pycc_bool)"),
        }
    }

    /// The message's description of the expected type, as the boundary's
    /// own scalar messages put it.
    fn wanted(self) -> &'static str {
        match self {
            Element::Int => "an integer",
            Element::Float => "a float",
            Element::Str => "a str",
            Element::Bool => "a bool",
        }
    }
}

/// The type an unannotated parameter with literal default `default` has.
fn literal_type(default: &Expr) -> Option<&'static str> {
    Some(match default {
        Expr::NumberLiteral(n) => match n.value {
            ruff_python_ast::Number::Int(_) => "int",
            ruff_python_ast::Number::Float(_) => "float",
            ruff_python_ast::Number::Complex { .. } => return None,
        },
        Expr::BooleanLiteral(_) => "bool",
        Expr::StringLiteral(_) => "str",
        Expr::NoneLiteral(_) => "object",
        Expr::UnaryOp(u) => return literal_type(&u.operand),
        _ => return None,
    })
}

/// The source text of `range`.
fn text(source: &str, range: TextRange) -> Option<&str> {
    source.get(range.start().to_usize()..range.end().to_usize())
}

/// Whether `def` publishes a signature the boundary cannot bind alone.
fn needs_wrapper(parameters: &Parameters) -> bool {
    !parameters.posonlyargs.is_empty()
        || !parameters.kwonlyargs.is_empty()
        || parameters.vararg.is_some()
        || parameters.kwarg.is_some()
        || parameters.args.iter().any(|p| p.default.is_some())
}

/// A planned `def`: its signature and the source of its entry.
struct Planned {
    signature: ExtSignature,
    entry_source: String,
}

/// A declared parameter's default, as the wrapper and the entry see it.
enum Default {
    None,
    /// A literal, written into the wrapper as it is.
    Literal(String),
    /// Any other expression: evaluated once by the compiled module where the
    /// `def` is (`super::signatures` hoists it into a global), and filled in
    /// by the entry when the host omits the argument. Only for a parameter
    /// of a scalar type, which the entry receives as an object.
    Computed(Element, String),
}

/// One named parameter (the receiver aside).
struct Param {
    name: String,
    annotation: String,
    default: Default,
}

/// Declares `p` in the entry's `params`; the value the entry's call passes
/// for it (a computed default's is filled in by `body`).
fn entry_param(p: &Param, params: &mut Vec<String>, body: &mut String) -> String {
    match &p.default {
        Default::Computed(element, global) => {
            params.push(format!("{}: object", p.name));
            params.push(format!("_pycc_has_{}: bool", p.name));
            let ty = element.name();
            let _ = writeln!(
                body,
                "    _pycc_v_{0}: {ty} = {ty}({0}) if _pycc_has_{0} else {global}",
                p.name
            );
            format!("_pycc_v_{}", p.name)
        }
        _ => {
            params.push(format!("{}: {}", p.name, p.annotation));
            p.name.clone()
        }
    }
}

/// `p` in the wrapper's declared signature; pushes what the wrapper passes
/// the entry for it to `passed`, and a computed default's type check to
/// `checks` (`position` is its positional number, or `None` for a
/// keyword-only parameter).
fn wrapper_param(
    p: &Param,
    position: Option<usize>,
    qualname: &str,
    passed: &mut Vec<String>,
    checks: &mut String,
) -> String {
    match &p.default {
        Default::None => {
            passed.push(p.name.clone());
            p.name.clone()
        }
        Default::Literal(text) => {
            passed.push(p.name.clone());
            format!("{}=({text})", p.name)
        }
        Default::Computed(element, _) => {
            passed.push(p.name.clone());
            passed.push(format!("{} is not _pycc_missing", p.name));
            let at = match position {
                Some(n) => n.to_string(),
                None => format!("'{}'", p.name),
            };
            let _ = writeln!(
                checks,
                "        if {0} is not _pycc_missing and not {check}:\n            \
                 raise _pycc_TypeError(f\"{qualname}() argument {at}: \
                 '{{_pycc_type({0}).__name__}}' object cannot be interpreted as {wanted}\")",
                p.name,
                check = element.check(&p.name),
                wanted = element.wanted(),
            );
            format!("{}=_pycc_missing", p.name)
        }
    }
}

/// Plans `def` (the method of `class` when given), or `None`.
fn plan_def(def: &StmtFunctionDef, class: Option<&str>, source: &str) -> Option<Planned> {
    let name = def.name.as_str();
    if def.is_async
        || !def.decorator_list.is_empty()
        || name.starts_with('_')
        || super::ext_generators::yields(&def.body)
        || !needs_wrapper(&def.parameters)
    {
        return None;
    }
    let parameters = &def.parameters;
    let qualname = match class {
        Some(class) => format!("{class}.{name}"),
        None => name.to_string(),
    };
    let param = |p: &ruff_python_ast::ParameterWithDefault| -> Option<Param> {
        let name = p.parameter.name.to_string();
        if name.starts_with("_pycc") {
            return None;
        }
        let annotation = p.parameter.annotation.as_deref();
        let default = match p.default.as_deref() {
            None => Default::None,
            Some(d) if super::signatures::is_literal(d) => {
                Default::Literal(text(source, d.range())?.to_string())
            }
            Some(_) => Default::Computed(
                Element::of(annotation)?,
                super::signatures::default_global(&qualname, &name),
            ),
        };
        let annotation = match annotation {
            Some(annotation) => text(source, annotation.range())?.to_string(),
            // An unannotated parameter with a literal default has the
            // default's type (#1409), `None` an object's.
            None => literal_type(p.default.as_deref()?)?.to_string(),
        };
        Some(Param {
            name,
            annotation,
            default,
        })
    };
    // A method's receiver: its first parameter, positional-or-keyword and
    // without a default.
    let (receiver, regular) = match class {
        Some(_) => {
            let (first, rest) = parameters.args.split_first()?;
            if !parameters.posonlyargs.is_empty() || first.default.is_some() {
                return None;
            }
            (Some(first.parameter.name.to_string()), rest)
        }
        None => (None, &parameters.args[..]),
    };
    let posonly = parameters
        .posonlyargs
        .iter()
        .map(&param)
        .collect::<Option<Vec<_>>>()?;
    let regular = regular.iter().map(&param).collect::<Option<Vec<_>>>()?;
    let kwonly = parameters
        .kwonlyargs
        .iter()
        .map(&param)
        .collect::<Option<Vec<_>>>()?;
    let returns = text(source, def.returns.as_deref()?.range())?.to_string();
    let vararg = match &parameters.vararg {
        Some(v) => Some((v.name.to_string(), Element::of(v.annotation.as_deref())?)),
        None => None,
    };
    let kwarg = match &parameters.kwarg {
        Some(k) => Some((k.name.to_string(), Element::of(k.annotation.as_deref())?)),
        None => None,
    };
    let positional: Vec<&Param> = posonly.iter().chain(&regular).collect();
    let entry = entry_name(&qualname);

    // The entry: every parameter positional, `*args`/`**kwargs` and a
    // computed default's parameter as objects.
    let mut params: Vec<String> = Vec::new();
    let mut body = String::new();
    if let (Some(receiver), Some(class)) = (&receiver, class) {
        params.push(format!("{receiver}: {class}"));
    }
    let mut call: Vec<String> = positional
        .iter()
        .map(|p| entry_param(p, &mut params, &mut body))
        .collect();
    if let Some((v, element)) = &vararg {
        params.push(format!("{v}: object"));
        let ty = element.name();
        let _ = writeln!(body, "    _pycc_r: list[{ty}] = []");
        let _ = writeln!(body, "    _pycc_i = 0");
        let _ = writeln!(body, "    while _pycc_i < len({v}):");
        let _ = writeln!(body, "        _pycc_r.append({ty}({v}[_pycc_i]))");
        let _ = writeln!(body, "        _pycc_i += 1");
        call.push("*_pycc_r".to_string());
    }
    for p in &kwonly {
        let value = entry_param(p, &mut params, &mut body);
        call.push(format!("{}={value}", p.name));
    }
    if let Some((_, element)) = &kwarg {
        params.push("_pycc_kk: object".to_string());
        params.push("_pycc_kv: object".to_string());
        let ty = element.name();
        let _ = writeln!(body, "    _pycc_k: dict[str, {ty}] = {{}}");
        let _ = writeln!(body, "    _pycc_j = 0");
        let _ = writeln!(body, "    while _pycc_j < len(_pycc_kk):");
        let _ = writeln!(
            body,
            "        _pycc_k[str(_pycc_kk[_pycc_j])] = {ty}(_pycc_kv[_pycc_j])"
        );
        let _ = writeln!(body, "        _pycc_j += 1");
        call.push("**_pycc_k".to_string());
    }
    let callee = match &receiver {
        Some(receiver) => format!("{receiver}.{name}"),
        None => name.to_string(),
    };
    let call = format!("{callee}({})", call.join(", "));
    if returns == "None" {
        let _ = writeln!(body, "    {call}");
    } else {
        let _ = writeln!(body, "    return {call}");
    }
    let entry_source = format!("def {entry}({}) -> {returns}:\n{body}", params.join(", "));

    // The wrapper: the declared signature, without annotations.
    let mut declared: Vec<String> = receiver.iter().cloned().collect();
    let mut passed: Vec<String> = receiver.iter().cloned().collect();
    let mut checks = String::new();
    // Where CPython numbers the next positional argument, for a message.
    let mut position = usize::from(receiver.is_some());
    for (index, p) in posonly.iter().enumerate() {
        position += 1;
        declared.push(wrapper_param(
            p,
            Some(position),
            &qualname,
            &mut passed,
            &mut checks,
        ));
        if index + 1 == posonly.len() {
            declared.push("/".to_string());
        }
    }
    for p in &regular {
        position += 1;
        declared.push(wrapper_param(
            p,
            Some(position),
            &qualname,
            &mut passed,
            &mut checks,
        ));
    }
    match &vararg {
        Some((v, element)) => {
            declared.push(format!("*{v}"));
            passed.push(v.clone());
            let _ = writeln!(
                checks,
                "        for _pycc_n, _pycc_x in enumerate({v}, {first}):\n            \
                 if not {check}:\n                \
                 raise _pycc_TypeError(f\"{qualname}() argument {{_pycc_n}}: \
                 '{{_pycc_type(_pycc_x).__name__}}' object cannot be interpreted as {wanted}\")",
                first = position + 1,
                check = element.check("_pycc_x"),
                wanted = element.wanted(),
            );
        }
        None if !kwonly.is_empty() => declared.push("*".to_string()),
        None => {}
    }
    for p in &kwonly {
        declared.push(wrapper_param(p, None, &qualname, &mut passed, &mut checks));
    }
    if let Some((k, element)) = &kwarg {
        declared.push(format!("**{k}"));
        passed.push(format!("_pycc_tuple({k})"));
        passed.push(format!("_pycc_tuple({k}.values())"));
        let _ = writeln!(
            checks,
            "        for _pycc_n, _pycc_x in {k}.items():\n            \
             if not {check}:\n                \
             raise _pycc_TypeError(f\"{qualname}() argument '{{_pycc_n}}': \
             '{{_pycc_type(_pycc_x).__name__}}' object cannot be interpreted as {wanted}\")",
            check = element.check("_pycc_x"),
            wanted = element.wanted(),
        );
    }
    let wrapper = format!(
        "def _pycc_make(_pycc_entry, _pycc_missing=object(), _pycc_tuple=tuple, \
         _pycc_isinstance=isinstance, _pycc_type=type, _pycc_TypeError=TypeError, \
         _pycc_int=int, _pycc_float=float, _pycc_str=str, _pycc_bool=bool):\n    \
         def {name}({declared}):\n{checks}        \
         return _pycc_entry({passed})\n    \
         {name}.__qualname__ = \"{qualname}\"\n    \
         {name}.__code__ = {name}.__code__.replace(co_qualname=\"{qualname}\")\n    \
         return {name}\n",
        declared = declared.join(", "),
        passed = passed.join(", "),
    );
    Some(Planned {
        signature: ExtSignature {
            qualname,
            entry,
            wrapper,
        },
        entry_source,
    })
}

/// Every method name defined by more than one top-level class.
fn shared_method_names(body: &[Stmt]) -> Vec<String> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for stmt in body {
        if let Stmt::ClassDef(class) = stmt {
            for item in &class.body {
                if let Stmt::FunctionDef(def) = item {
                    *counts.entry(def.name.as_str()).or_default() += 1;
                }
            }
        }
    }
    counts
        .into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|(name, _)| name.to_string())
        .collect()
}

/// Writes out `module`'s `@dataclass` classes (`super::dataclasses`), so
/// their generated `__init__` is published with its declared signature.
/// The later [`super::desugar`] finds them already written out.
pub fn expand_dataclasses_in(module: &mut ModModule) {
    let mut body: Vec<Stmt> = std::mem::take(&mut module.body).into_iter().collect();
    super::dataclasses::expand_dataclasses(&mut body);
    module.body = body.into();
}

/// Each planned top-level statement's index with what it plans.
fn plan(module: &ModModule, source: &str) -> Vec<(usize, Planned)> {
    let shared = shared_method_names(&module.body);
    let mut out = Vec::new();
    for (index, stmt) in module.body.iter().enumerate() {
        match stmt {
            Stmt::FunctionDef(def) => {
                if let Some(planned) = plan_def(def, None, source) {
                    out.push((index, planned));
                }
            }
            Stmt::ClassDef(class)
                if !class.name.starts_with('_') && class.decorator_list.is_empty() =>
            {
                for item in &class.body {
                    if let Stmt::FunctionDef(def) = item
                        && !shared.iter().any(|name| name == def.name.as_str())
                        && let Some(planned) = plan_def(def, Some(class.name.as_str()), source)
                    {
                        out.push((index, planned));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The signatures `module` (parsed from `source`) publishes through a
/// wrapper; [`split_ext_signatures`] adds their entries.
#[must_use]
pub fn ext_signatures(module: &ModModule, source: &str) -> Vec<ExtSignature> {
    // The `@dataclass` classes' generated methods are planned as written
    // out, as [`split_ext_signatures`] plans them.
    let mut module = module.clone();
    expand_dataclasses_in(&mut module);
    plan(&module, source)
        .into_iter()
        .map(|(_, planned)| planned.signature)
        .collect()
}

/// Adds the entry of every signature [`ext_signatures`] plans to `module`,
/// right after the `def` or class statement it belongs to. Runs on the
/// parsed `--ext` entry module, before [`super::desugar`].
pub fn split_ext_signatures(module: &mut ModModule, source: &str) {
    expand_dataclasses_in(module);
    let mut planned = plan(module, source);
    if planned.is_empty() {
        return;
    }
    // From the last statement back, so the indices stay valid.
    planned.sort_by_key(|(index, _)| std::cmp::Reverse(*index));
    for (index, planned) in planned {
        let range = module.body[index].range();
        let entry = super::signatures::parse_helper(&planned.entry_source, range);
        for (offset, stmt) in entry.into_iter().enumerate() {
            module.body.insert(index + 1 + offset, stmt);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> ModModule {
        ruff_python_parser::parse_module(source)
            .expect("valid Python")
            .into_syntax()
    }

    #[test]
    fn an_entry_name_round_trips_its_qualname() {
        for qualname in ["f", "Box.area", "a_b.c_d"] {
            assert_eq!(
                ext_entry_target(&entry_name(qualname)).as_deref(),
                Some(qualname)
            );
        }
        assert_eq!(ext_entry_target("f"), None);
    }

    #[test]
    fn plain_and_unannotated_defs_are_not_planned() {
        let source = "def a(x: int) -> int:\n    return x\n\n\
                      def b(x, y: int = 1) -> int:\n    return y\n\n\
                      def _c(x: int = 1) -> int:\n    return x\n\n\
                      def d(*xs: list[int]) -> int:\n    return 0\n\n\
                      def e(x: int = 1):\n    return x\n";
        assert!(ext_signatures(&parse(source), source).is_empty());
    }

    #[test]
    fn a_function_publishes_its_declared_signature() {
        let source = "def f(a: int, /, b: int = 2 * 5, *rest: int, c: str = \"x\", **kw: float) -> int:\n    return a\n";
        let module = parse(source);
        let signatures = ext_signatures(&module, source);
        let [signature] = signatures.as_slice() else {
            panic!("{signatures:?}")
        };
        assert_eq!(signature.qualname, "f");
        assert_eq!(signature.entry, "_pycc_sig_f");
        assert!(
            signature
                .wrapper
                .contains("def f(a, /, b=_pycc_missing, *rest, c=(\"x\"), **kw):"),
            "{}",
            signature.wrapper
        );
        assert!(
            signature.wrapper.contains(
                "return _pycc_entry(a, b, b is not _pycc_missing, rest, c, _pycc_tuple(kw), \
                 _pycc_tuple(kw.values()))"
            ),
            "{}",
            signature.wrapper
        );
        let mut split = module.clone();
        split_ext_signatures(&mut split, source);
        assert_eq!(split.body.len(), 2);
    }

    #[test]
    fn a_method_shared_by_two_classes_is_not_planned() {
        let source = "class A:\n    def m(self, x: int = 1) -> int:\n        return x\n\n\
                      class B:\n    def m(self, x: int = 2) -> int:\n        return x\n\n\
                      class C:\n    def k(self, *, x: int = 3) -> int:\n        return x\n";
        let signatures = ext_signatures(&parse(source), source);
        let names: Vec<&str> = signatures.iter().map(|s| s.qualname.as_str()).collect();
        assert_eq!(names, ["C.k"]);
        assert_eq!(signatures[0].method(), Some(("C", "k")));
        assert!(signatures[0].wrapper.contains("def k(self, *, x=(3)):"));
    }
}
