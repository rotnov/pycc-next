//! Keyword arguments at the `--ext` host boundary (#1461, part of #884).
//!
//! A host call into a published function, method or constructor may name a
//! positional-or-keyword parameter, and it is bound exactly as CPython binds
//! the same call to the uncompiled source: the shim's
//! `pycc_ext_kw_bind_fastcall`/`pycc_ext_kw_bind_dict` place each keyword in
//! the slot it names and raise CPython's own `TypeError` wording for an
//! unknown name, a parameter given twice and a required one left out. The
//! bound slots then go through the **unchanged** unpack helpers, so a
//! keyword argument is admitted, converted and released exactly as the
//! same object passed positionally.
//!
//! Which exports qualify is decided here, from the entry module's source,
//! because HIR does not keep what keyword binding needs: lowering prepends
//! `posonlyargs` to `args` and drops the `/` (PEP 570), so a binder that
//! trusted HIR names would accept `f(a=1)` for `def f(a, /)`. An export is
//! keyword-enabled only when its source `def` is found -- a top-level
//! function or a method directly in a top-level class body, defined once --
//! and that `def`:
//!
//! * has no positional-only parameter, no `*args`, no keyword-only
//!   parameter and no `**kwargs`;
//! * declares exactly the export's carried parameters plus its receiver;
//! * declares exactly as many defaults as the export carries, so a
//!   module-level function -- whose defaults the boundary does not record
//!   yet (#1194) -- keeps the refusal rather than reporting a defaulted
//!   parameter as missing.
//!
//! Anything else keeps `METH_FASTCALL` without `METH_KEYWORDS`, where
//! CPython itself raises `takes no keyword arguments` (D-244 rule 7 as
//! amended by #1461). The names handed to the binder are the source's own,
//! receiver dropped.

use std::collections::HashMap;

use pycc_ast::{ModModule, Parameters, Stmt};
use pycc_hir::{HirExpr, HirModule};

use super::{ExtCtor, ExtExport, ExtReceiver, inherited, source_level_name};

/// What one source `def` says about keyword binding.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceSignature {
    /// Every positional parameter name, receiver included, in order.
    names: Vec<String>,
    /// `false` when the signature has a parameter kind the binder does not
    /// model: positional-only, `*args`, keyword-only or `**kwargs`.
    bindable: bool,
    /// How many of [`SourceSignature::names`] declare a default.
    defaults: usize,
}

impl SourceSignature {
    fn of(parameters: &Parameters) -> Self {
        let positional = || parameters.posonlyargs.iter().chain(&parameters.args);
        Self {
            names: positional()
                .map(|param| param.parameter.name.to_string())
                .collect(),
            bindable: parameters.posonlyargs.is_empty()
                && parameters.vararg.is_none()
                && parameters.kwonlyargs.is_empty()
                && parameters.kwarg.is_none(),
            defaults: positional().filter(|param| param.default.is_some()).count(),
        }
    }
}

/// The entry module's top-level function and method signatures, keyed by
/// source-level name (`f`, `Class.method`). A name defined more than once
/// maps to `None`: which definition a compiled item came from is unknown,
/// so it is refused.
#[derive(Debug, Default)]
pub(crate) struct SourceSignatures {
    by_name: HashMap<String, Option<SourceSignature>>,
}

impl SourceSignatures {
    /// The signatures of `source`, or an empty table -- every export
    /// refused -- when it does not parse. The frontend has already parsed
    /// the same text successfully, so the empty table is a fail-closed
    /// default rather than a reachable outcome.
    pub(crate) fn from_source(source: &str) -> Self {
        pycc_parser::parse(source).map_or_else(|_| Self::default(), |module| Self::collect(&module))
    }

    pub(crate) fn collect(module: &ModModule) -> Self {
        let mut table = Self::default();
        for stmt in &module.body {
            match stmt {
                Stmt::FunctionDef(def) => table.insert(def.name.to_string(), &def.parameters),
                Stmt::ClassDef(class) => {
                    for member in &class.body {
                        if let Stmt::FunctionDef(def) = member {
                            table.insert(format!("{}.{}", class.name, def.name), &def.parameters);
                        }
                    }
                }
                _ => {}
            }
        }
        table
    }

    fn insert(&mut self, key: String, parameters: &Parameters) {
        self.by_name
            .entry(key)
            .and_modify(|held| *held = None)
            .or_insert_with(|| Some(SourceSignature::of(parameters)));
    }

    /// The carried parameter names of the compiled item `compiled` when a
    /// host call to it may bind keywords, else `None`. See the module doc.
    fn keyword_names(
        &self,
        module: &HirModule,
        compiled: &str,
        has_receiver: bool,
        carried: usize,
        defaults: &[Option<HirExpr>],
    ) -> Option<Vec<String>> {
        // A receiver-exact inherited copy (#1337, D-254) is compiled from
        // its origin's source, so it binds against the origin's `def`.
        let source = inherited::source_item(module, compiled);
        let signature = self.by_name.get(source_level_name(&source))?.as_ref()?;
        let receiver = usize::from(has_receiver);
        let carried_defaults = defaults.iter().filter(|default| default.is_some()).count();
        (signature.bindable
            && signature.names.len() == carried + receiver
            && signature.defaults == carried_defaults)
            .then(|| signature.names[receiver..].to_vec())
    }
}

/// Sets [`ExtExport::keyword_names`] on every export.
pub(crate) fn bind_keyword_names(
    module: &HirModule,
    signatures: &SourceSignatures,
    exports: &mut [ExtExport],
) {
    for export in exports {
        export.keyword_names = signatures.keyword_names(
            module,
            &export.name,
            export.receiver != ExtReceiver::None,
            export.params.len(),
            &export.defaults,
        );
    }
}

/// Sets [`ExtCtor::keyword_names`] on every constructor, whose compiled
/// `__init__` always takes `self`.
pub(crate) fn bind_ctor_keyword_names(
    module: &HirModule,
    signatures: &SourceSignatures,
    ctors: &mut [ExtCtor],
) {
    for ctor in ctors {
        ctor.keyword_names =
            signatures.keyword_names(module, &ctor.name, true, ctor.params.len(), &ctor.defaults);
    }
}

/// The number of leading parameters a call must supply: all of them when
/// none is defaulted.
fn required(arity: usize, defaults: &[Option<HirExpr>]) -> usize {
    defaults.iter().position(Option::is_some).unwrap_or(arity)
}

/// A C string literal for a UTF-8 parameter name. Every byte outside
/// `[A-Za-z0-9_]` is a three-digit octal escape, so a non-ASCII identifier
/// reaches `PyUnicode_EqualToUTF8` byte-exact and no following character
/// can extend an escape.
pub(crate) fn c_name(name: &str) -> String {
    let escaped: String = name
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || byte == b'_' {
                char::from(byte).to_string()
            } else {
                format!("\\{byte:03o}")
            }
        })
        .collect();
    format!("\"{escaped}\"")
}

/// The file-static name table an entry point binds against. A
/// zero-parameter entry point still gets a one-element table, because C has
/// no zero-length array; with no parameter every keyword is unexpected, so
/// the element is never read.
fn names_table(prefix: &str, names: &[String]) -> String {
    let entries = if names.is_empty() {
        "NULL".to_string()
    } else {
        names
            .iter()
            .map(|name| c_name(name))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!("static const char *const pycc_ext_kwnames_{prefix}[] = {{{entries}}};\n")
}

/// A binding prologue's two halves.
pub(crate) struct Prologue {
    /// File-scope text: the name table.
    pub(crate) before: String,
    /// Function-body text.
    pub(crate) body: String,
}

/// The binding prologue of a keyword-enabled `METH_FASTCALL | METH_KEYWORDS`
/// wrapper, emitted before the wrapper's arity check.
///
/// A call with keywords is bound into `kw_slots`, its unfilled defaulted
/// slots get their default objects, and `args`/`nargs` are then pointed at
/// the full-arity array -- so the arity check, the unpack helpers and the
/// buffer releases that follow run exactly as for a positional call. A
/// keyword-free call skips the block and is unchanged.
pub(crate) fn fastcall_prologue(
    prefix: &str,
    source_name: &str,
    names: &[String],
    defaults: &[Option<HirExpr>],
) -> Prologue {
    let arity = names.len();
    let required = required(arity, defaults);
    let fills = super::defaults::keyword_default_fills(prefix, defaults, "        return NULL;\n");
    Prologue {
        before: names_table(prefix, names),
        body: format!(
            "    PyObject *kw_slots[{slots}];\n    \
             if (kwnames != NULL && PyTuple_Size(kwnames) != 0) {{\n        \
             if (pycc_ext_kw_bind_fastcall(\"{source_name}\", pycc_ext_kwnames_{prefix}, \
             {arity}, {required}, kw_slots, args, nargs, kwnames) < 0) {{\n            \
             return NULL;\n        }}\n{fills}        args = kw_slots;\n        \
             nargs = {arity};\n    }}\n",
            slots = arity.max(1),
        ),
    }
}

/// The binding prologue of a keyword-enabled `Py_tp_init`. A call with
/// keywords is bound into `kw_slots` and sets `kw_bound`, which the
/// constructor's count and item expressions ([`tp_init_count`],
/// [`tp_init_item`]) consult instead of the positional tuple.
pub(crate) fn tp_init_prologue(
    prefix: &str,
    source_name: &str,
    names: &[String],
    defaults: &[Option<HirExpr>],
) -> Prologue {
    let arity = names.len();
    let required = required(arity, defaults);
    let fills = super::defaults::keyword_default_fills(prefix, defaults, "        return -1;\n");
    Prologue {
        before: names_table(prefix, names),
        body: format!(
            "    PyObject *kw_slots[{slots}];\n    int kw_bound = 0;\n    \
             if (kwds != NULL && PyDict_Size(kwds) != 0) {{\n        \
             if (pycc_ext_kw_bind_dict(\"{source_name}\", pycc_ext_kwnames_{prefix}, \
             {arity}, {required}, kw_slots, args, kwds) < 0) {{\n            \
             return -1;\n        }}\n{fills}        kw_bound = 1;\n    }}\n",
            slots = arity.max(1),
        ),
    }
}

/// A keyword-enabled `Py_tp_init`'s argument count.
pub(crate) fn tp_init_count(arity: usize) -> String {
    format!("(kw_bound ? {arity} : PyTuple_Size(args))")
}

/// A keyword-enabled `Py_tp_init`'s argument `index`.
pub(crate) fn tp_init_item(index: usize) -> String {
    format!("(kw_bound ? kw_slots[{index}] : PyTuple_GetItem(args, {index}))")
}

/// The `PyMethodDef` flags for an export: `METH_KEYWORDS` is added exactly
/// when [`ExtExport::keyword_names`] is set, the same predicate that gives
/// [`super::wrapper_for`] its four-argument signature.
pub(crate) fn method_flags(base: &str, export: &ExtExport) -> String {
    if export.keyword_names.is_some() {
        format!("{base} | METH_KEYWORDS")
    } else {
        base.to_string()
    }
}

#[cfg(test)]
#[path = "keywords_tests.rs"]
mod tests;
