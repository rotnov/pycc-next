//! The failures of a non-strict build the fallback loop answers rather
//! than the build stopping (`crate::fallback`): an exported signature the
//! CPython boundary cannot carry, a method that cannot attach, a signature
//! that cannot bind, and a package member's sibling class that does not
//! type as the class it stands for. Extracted from `frontend.rs` per
//! AGENTS.md's file-decomposition rule.

use super::{FileDiagnostics, FrontendFailure};
use pycc_diag::{Diagnostic, Span};
use pycc_hir::HirModule;
use std::path::Path;

/// In non-strict mode, a compiled function Python code must see (an
/// `--ext` module's exports, or an embedded build's once it has fallback
/// code) whose signature the CPython boundary cannot carry is a failure
/// of the statement defining it, so the fallback loop runs that function in
/// CPython instead of the build stopping at the `C0003` the artifact
/// planner would report.
pub(super) fn export_gaps_as_failures(
    path: &Path,
    typed: &HirModule,
    ext_module: bool,
) -> Result<(), FrontendFailure> {
    if !crate::mode::non_strict_for(path) || !(ext_module || crate::fallback::active()) {
        return Ok(());
    }
    let source = crate::fallback::entry_source(path);
    let hooks = if ext_module {
        match crate::ext_build::EntryHooks::from_source(&source) {
            Ok(hooks) => hooks.defined().clone(),
            // A module hook bound other than by a top-level `def`
            // (`__getattr__ = getattr_migration(__name__)`) is a failure of
            // the statement binding it: run by CPython, it binds the hook in
            // the module's dict, where CPython looks it up (PEP 562).
            Err(refusals) => {
                let origins = refusals.iter().map(|d| d.span.map(|s| s.start)).collect();
                return Err(FrontendFailure::Compile {
                    files: vec![FileDiagnostics {
                        path: path.to_string_lossy().into_owned(),
                        source,
                        diagnostics: refusals,
                        origins,
                    }],
                });
            }
        }
    } else {
        std::collections::BTreeSet::new()
    };
    // An embedded build publishes no inferred function: fallback code that
    // calls one makes it fall back instead (`crate::fallback`).
    let inferred = if ext_module {
        std::collections::BTreeSet::new()
    } else {
        crate::fallback::inferred_functions(&source)
    };
    let mut excluded = inferred;
    excluded.extend(crate::fallback::twinned_exports());
    if ext_module && let Some(refusals) = unattachable_methods(typed) {
        let (diagnostics, origins) = refusals.into_iter().unzip();
        return Err(FrontendFailure::Compile {
            files: vec![FileDiagnostics {
                path: path.to_string_lossy().into_owned(),
                source,
                diagnostics,
                origins,
            }],
        });
    }
    if ext_module && let Some(unbound) = unbindable_signatures(&source) {
        let (diagnostics, origins) = unbound.into_iter().unzip();
        return Err(FrontendFailure::Compile {
            files: vec![FileDiagnostics {
                path: path.to_string_lossy().into_owned(),
                source,
                diagnostics,
                origins,
            }],
        });
    }
    let Err(gaps) = crate::ext_build::collect_exports_excluding(typed, &hooks, &excluded) else {
        return Ok(());
    };
    let Ok(module) = pycc_parser::parse_all(&source) else {
        return Ok(());
    };
    let origin_of = |name: &str| {
        // A signature entry's gap is its function's (`_pycc_sig_f`), and a
        // method's mangled name is `Class.method`: its class statement.
        let target = pycc_ast::ext_entry_target(name);
        let name = target.as_deref().unwrap_or(name);
        let owner = name.split('.').next().unwrap_or(name);
        module.body.iter().find_map(|stmt| {
            let defines = match stmt {
                pycc_ast::Stmt::FunctionDef(def) => def.name.as_str() == owner,
                pycc_ast::Stmt::ClassDef(class) => class.name.as_str() == owner,
                pycc_ast::Stmt::Assign(assign) => assign.targets.iter().any(
                    |target| matches!(target, pycc_ast::Expr::Name(n) if n.id.as_str() == owner),
                ),
                _ => false,
            };
            defines.then(|| pycc_ast::stmt_range(stmt).start)
        })
    };
    if std::env::var_os("PYCC_FALLBACK_DUMP").is_some() {
        for (name, _gap) in &gaps {
            eprintln!(
                "export gap: {name} -> {:?}: {}",
                origin_of(name),
                _gap.message
            );
        }
    }
    let (diagnostics, origins): (Vec<_>, Vec<_>) = gaps
        .into_iter()
        .map(|(name, gap)| (gap, origin_of(&name)))
        .unzip();
    Err(FrontendFailure::Compile {
        files: vec![FileDiagnostics {
            path: path.to_string_lossy().into_owned(),
            source,
            diagnostics,
            origins,
        }],
    })
}

/// The classes a method of which runs in CPython attached to them
/// (`crate::fallback`'s `attach`) where that method uses an attribute of
/// `self` a compiled instance does not show Python code: one that is not a
/// method, property or field of the class, or a field the carrier exposes
/// no descriptor for (a `list[int]` field). Each is a failure of the class
/// statement, which then runs in CPython with its methods compiled.
fn unattachable_methods(typed: &HirModule) -> Option<Vec<(pycc_diag::Diagnostic, Option<u32>)>> {
    let class_of = |name: &str| {
        typed
            .class_defs
            .iter()
            .find(|(held, _)| held == name)
            .map(|(_, def)| def)
    };
    let exposed = |class: &str, attr: &str| {
        let Some(def) = class_of(class) else {
            return false;
        };
        def.mro.iter().filter_map(|name| class_of(name)).any(|def| {
            def.methods.iter().any(|(name, _)| name == attr)
                || def.properties.iter().any(|p| p.name == attr)
                || def.static_methods.iter().any(|(name, _)| name == attr)
                || def.class_methods.iter().any(|(name, _)| name == attr)
                || def.attrs.iter().any(|(name, ty)| {
                    name == attr
                        && matches!(
                            ty,
                            pycc_hir::Ty::Int
                                | pycc_hir::Ty::Float
                                | pycc_hir::Ty::Bool
                                | pycc_hir::Ty::Str
                                | pycc_hir::Ty::Object
                                | pycc_hir::Ty::Instance(_)
                        )
                })
        })
    };
    let mut out = Vec::new();
    for (class, origin, attrs) in crate::fallback::attached_methods() {
        if let Some(attr) = attrs.iter().find(|attr| !exposed(&class, attr)) {
            out.push((
                pycc_diag::Diagnostic::error(
                    "C0001",
                    format!(
                        "a method of `{class}` running in CPython uses `self.{attr}`, which a \
                         compiled instance does not show Python code"
                    ),
                    pycc_diag::Span::new(origin, origin + 1),
                ),
                Some(origin),
            ));
        }
    }
    (!out.is_empty()).then_some(out)
}

/// In a non-strict `--ext` module, the compiled functions and methods
/// Python code would call with a signature the boundary cannot bind: a
/// default, a keyword-only or positional-only parameter, `*args` or
/// `**kwargs`, with no published wrapper applying it
/// (`pycc_ast::ext_signatures`). Each is a failure of its top-level
/// statement, which the fallback loop then runs in CPython with its bodies
/// compiled (`crate::fallback`'s `lift`): CPython binds the call.
fn unbindable_signatures(source: &str) -> Option<Vec<(pycc_diag::Diagnostic, Option<u32>)>> {
    let module = pycc_parser::parse_all(source).ok()?;
    let planned: std::collections::BTreeSet<String> =
        pycc_ast::ext_signatures(&module, source, true)
            .into_iter()
            .map(|signature| signature.qualname)
            .collect();
    let refusal = |qualname: &str, at: pycc_ast::TextRange| {
        let range: std::ops::Range<u32> = at.into();
        (
            pycc_diag::Diagnostic::error(
                "C0001",
                format!(
                    "the --ext boundary cannot bind a call of `{qualname}` with its declared \
                     signature (a default, keyword-only or positional-only parameter, `*args` or \
                     `**kwargs`)"
                ),
                pycc_diag::Span::new(range.start, range.end),
            ),
            Some(range.start),
        )
    };
    // Python calls a function it can see: a public one, a private one (in
    // non-strict mode), never a compiler name.
    // A private one stays as compiled before: Python code rarely calls it,
    // and never omits its defaults.
    let published = |name: &str| !name.starts_with('_');
    let mut out = Vec::new();
    for stmt in &module.body {
        match stmt {
            pycc_ast::Stmt::FunctionDef(def)
                if def.decorator_list.is_empty()
                    && published(def.name.as_str())
                    && pycc_ast::ext_needs_wrapper(&def.parameters)
                    && !planned.contains(def.name.as_str()) =>
            {
                out.push(refusal(def.name.as_str(), def.range));
            }
            pycc_ast::Stmt::ClassDef(class) => {
                for member in &class.body {
                    if let pycc_ast::Stmt::FunctionDef(def) = member {
                        let qualname = format!("{}.{}", class.name.as_str(), def.name.as_str());
                        let name = def.name.as_str();
                        // A constructor binds defaults and keywords itself
                        // (`ext_build::constructors`), but no other shape.
                        let p = &def.parameters;
                        let init_unbound = name == "__init__"
                            && (!p.posonlyargs.is_empty()
                                || !p.kwonlyargs.is_empty()
                                || p.vararg.is_some()
                                || p.kwarg.is_some());
                        if (init_unbound
                            || published(name)
                                && pycc_ast::ext_needs_wrapper(&def.parameters)
                                && !planned.contains(&qualname))
                            && def.decorator_list.is_empty()
                        {
                            out.push(refusal(&qualname, def.range));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    (!out.is_empty()).then_some(out)
}

/// A package member's sibling classes must type as the classes they stand
/// for; one that does not is a failure the fallback loop answers by reading
/// that class through CPython (`crate::package_build`).
pub(super) fn sibling_mismatches_as_failures(
    path: &Path,
    typed: &HirModule,
) -> Result<(), FrontendFailure> {
    let mismatches: Vec<Diagnostic> = crate::package_build::verify_sibling_classes(typed)
        .into_iter()
        .map(|(at, message)| Diagnostic::error("C0001", message, Span::new(at, at)))
        .collect();
    if mismatches.is_empty() {
        return Ok(());
    }
    Err(FrontendFailure::compile(
        &path.to_string_lossy(),
        &crate::fallback::entry_source(path),
        mismatches,
    ))
}
