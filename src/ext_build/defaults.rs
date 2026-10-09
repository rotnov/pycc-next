//! Default parameter values at the `--ext` host boundary (the method part
//! of #1140).
//!
//! A method's default is recorded by `pycc_hir` in
//! `HirClassDef::method_defaults` and has two consumers. Since Part 1 of
//! #1191 (#1438) an in-module regular instance-method call may omit a
//! defaulted argument: `pycc_types` and `pycc_mir` fill it through
//! `HirClassDef::omitted_method_defaults` by appending the default node, so
//! that call never reaches this module. A short constructor, static-method,
//! class-method or `super()` call is still the arity `T0021`. This module is
//! the other consumer, the host boundary, where a host call may omit any
//! defaulted argument. The generated `METH_FASTCALL` wrapper and `Py_tp_init` then
//! accept any argument count from the first defaulted parameter's index up
//! to the full arity, and hand each omitted parameter a `PyObject *` that
//! the parameter's **unchanged** unpack helper consumes -- so an omitted
//! argument is admitted, converted and released exactly as the same literal
//! passed explicitly by the host would be. No conversion code is added.
//!
//! The objects are `Py_None`, `Py_True` and `Py_False` for those three
//! literals, and otherwise one file-static object per defaulted parameter,
//! created on first use and kept for the life of the process: a default is
//! a literal, so CPython's evaluate-once-at-`def`-time rule and this
//! create-once rule produce indistinguishable values, and an `int`,
//! `float` or `str` object is immutable.
//!
//! A host call that names a defaulted parameter by keyword (#1461) reaches
//! the same cached objects through [`keyword_default_fills`].
//!
//! A module-level export's defaults are not recorded anywhere this module
//! can read, so it keeps the exact arity check -- and, since its source
//! `def` then declares defaults the export does not carry, keyword calls
//! stay refused (`super::keywords`); #1194 tracks that half.

use pycc_hir::{HirExpr, HirModule};

/// The defaults of the compiled item `name`, parallel to its full parameter
/// list (receiver included), or empty when it declares none.
///
/// A receiver-exact inherited copy (#1337, D-254) is compiled from its
/// origin's source and so carries its origin's defaults.
pub(crate) fn defaults_of(module: &HirModule, name: &str) -> Vec<Option<HirExpr>> {
    let source = super::inherited::source_item(module, name);
    module
        .class_defs
        .iter()
        .flat_map(|(_, def)| def.method_defaults.iter())
        .find(|(held, _)| *held == source)
        .map(|(_, defaults)| defaults.clone())
        .unwrap_or_default()
}

/// [`defaults_of`] restricted to the carried parameters: the leading
/// receiver entry is dropped when `has_receiver`, exactly as the caller
/// drops the receiver from the parameter list.
pub(crate) fn carried_defaults(
    module: &HirModule,
    name: &str,
    has_receiver: bool,
) -> Vec<Option<HirExpr>> {
    let mut defaults = defaults_of(module, name);
    if has_receiver && !defaults.is_empty() {
        defaults.remove(0);
    }
    defaults
}

/// How a generated entry point reads its arguments: the C expression for
/// the argument count, and for argument `index`.
pub(crate) struct ArgSource<'a> {
    /// `nargs` for a `METH_FASTCALL` wrapper, `PyTuple_Size(args)` for a
    /// `Py_tp_init`.
    pub(crate) count: &'a str,
    /// The `PyObject *` the host passed at `index`.
    pub(crate) item: &'a dyn Fn(usize) -> String,
    /// The bail statement, already indented for a nested block.
    pub(crate) fail: &'a str,
}

/// The range arity check and one `PyObject *v{index}` local per defaulted
/// parameter, holding either the host's argument or the default object.
///
/// `prefix` makes the file-static default objects' names unique across the
/// generated file (one wrapper or one `Py_tp_init` each). The caller must
/// emit [`default_object_helpers`] with the same `prefix` before the entry
/// point, and read a defaulted argument through [`arg_expr`].
pub(crate) fn range_arity_check(
    source_name: &str,
    prefix: &str,
    defaults: &[Option<HirExpr>],
    source: &ArgSource<'_>,
) -> String {
    let arity = defaults.len();
    let required = required_count(defaults);
    let count = source.count;
    let fail = source.fail;
    let mut out = format!(
        "    if ({count} < {required} || {count} > {arity}) {{\n        \
         PyErr_Format(PyExc_TypeError, \"{source_name}() takes from {required} to {arity} \
         arguments (%zd given)\", {count});\n{fail}    }}\n"
    );
    for (index, default) in defaults.iter().enumerate() {
        let Some(default) = default else { continue };
        let item = (source.item)(index);
        let object = default_object(prefix, index, default);
        out.push_str(&format!(
            "    PyObject *v{index} = {count} > {index} ? {item} : {};\n",
            object.expr
        ));
        if object.definition.is_some() {
            out.push_str(&format!("    if (v{index} == NULL) {{\n{fail}    }}\n"));
        }
    }
    out
}

/// The `PyObject *` argument `index` is read from: `v{index}` for a
/// defaulted parameter (see [`range_arity_check`]), the host's own argument
/// otherwise.
pub(crate) fn arg_expr(defaults: &[Option<HirExpr>], index: usize, item: String) -> String {
    match defaults.get(index) {
        Some(Some(_)) => format!("v{index}"),
        _ => item,
    }
}

/// The keyword binder's default fills (#1461): one statement per defaulted
/// parameter that stores the parameter's default object into a `kw_slots`
/// entry the host left unfilled.
///
/// The same objects [`range_arity_check`] substitutes for an omitted
/// trailing argument, under the same `prefix`, so a default reached through
/// a keyword call and one reached through a short positional call are the
/// one cached object. After these fills every slot is set, which is what
/// lets the caller treat the bound array as a full-arity positional call.
/// The text is indented for the binding block it sits in; `fail` is the
/// entry point's bail statement at the usual one-block indentation.
pub(crate) fn keyword_default_fills(
    prefix: &str,
    defaults: &[Option<HirExpr>],
    fail: &str,
) -> String {
    let mut out = String::new();
    for (index, default) in defaults.iter().enumerate() {
        let Some(default) = default else { continue };
        let object = default_object(prefix, index, default);
        out.push_str(&format!(
            "        if (kw_slots[{index}] == NULL) {{\n            kw_slots[{index}] = {};\n",
            object.expr
        ));
        if object.definition.is_some() {
            out.push_str(&format!(
                "            if (kw_slots[{index}] == NULL) {{\n        {fail}            }}\n"
            ));
        }
        out.push_str("        }\n");
    }
    out
}

/// The file-static helper functions that create the cached default objects
/// for one entry point, or the empty string when none is needed.
pub(crate) fn default_object_helpers(prefix: &str, defaults: &[Option<HirExpr>]) -> String {
    defaults
        .iter()
        .enumerate()
        .filter_map(|(index, default)| {
            default
                .as_ref()
                .and_then(|default| default_object(prefix, index, default).definition)
        })
        .collect()
}

/// The number of leading parameters without a default.
///
/// Python requires every parameter after a defaulted one to be defaulted
/// too, so this is also the smallest argument count the host may pass.
fn required_count(defaults: &[Option<HirExpr>]) -> usize {
    defaults
        .iter()
        .position(Option::is_some)
        .unwrap_or(defaults.len())
}

/// One default's `PyObject *` expression and, for a cached object, the
/// helper that creates it.
struct DefaultObject {
    expr: String,
    definition: Option<String>,
}

fn default_object(prefix: &str, index: usize, default: &HirExpr) -> DefaultObject {
    let constructor = match default {
        HirExpr::NoneLiteral => return singleton("Py_None"),
        HirExpr::BoolLiteral(true) => return singleton("Py_True"),
        HirExpr::BoolLiteral(false) => return singleton("Py_False"),
        HirExpr::IntLiteral(value) => format!("PyLong_FromLongLong({})", c_long_long(*value)),
        HirExpr::FloatLiteral(value) => format!("PyFloat_FromDouble({})", c_double(*value)),
        HirExpr::StringLiteral(text) => c_unicode(text),
        // `func::params::check_default` admits only the five literal kinds.
        other => unreachable!("a non-literal default reached the boundary: {other:?}"),
    };
    let helper = format!("pycc_ext_default_{prefix}_{index}");
    DefaultObject {
        expr: format!("{helper}()"),
        definition: Some(format!(
            "static PyObject *{helper}(void)\n{{\n    static PyObject *value = NULL;\n    \
             if (value == NULL) {{\n        value = {constructor};\n    }}\n    \
             return value;\n}}\n"
        )),
    }
}

fn singleton(name: &str) -> DefaultObject {
    DefaultObject {
        expr: name.to_string(),
        definition: None,
    }
}

/// A C `long long` constant expression for `value`. `i64::MIN` has no
/// literal spelling in C -- `9223372036854775808LL` overflows before the
/// negation applies -- so it is spelled as an expression.
fn c_long_long(value: i64) -> String {
    if value == i64::MIN {
        "(-9223372036854775807LL - 1)".to_string()
    } else {
        format!("{value}LL")
    }
}

/// A C `double` expression for `value` that reads back bit-exactly.
///
/// Rust's `{:e}` renders the shortest decimal that round-trips, which a C
/// compiler parses back to the same double. An infinity (`1e999` in the
/// source) has no decimal spelling, so it is `HUGE_VAL`. A NaN cannot be
/// written as a literal at all.
fn c_double(value: f64) -> String {
    if value.is_infinite() {
        let sign = if value < 0.0 { "-" } else { "" };
        format!("{sign}HUGE_VAL")
    } else {
        format!("{value:e}")
    }
}

/// `PyUnicode_FromStringAndSize` over a `str` default's UTF-8 bytes.
///
/// Every byte outside `[A-Za-z0-9 ]` is a three-digit octal escape: a
/// fixed width means no following character can extend an escape, and an
/// explicit length means an embedded NUL is carried rather than ending the
/// string.
fn c_unicode(text: &str) -> String {
    let escaped: String = text
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || byte == b' ' {
                char::from(byte).to_string()
            } else {
                format!("\\{byte:03o}")
            }
        })
        .collect();
    format!("PyUnicode_FromStringAndSize(\"{escaped}\", {})", text.len())
}

#[cfg(test)]
#[path = "defaults_tests.rs"]
mod tests;
