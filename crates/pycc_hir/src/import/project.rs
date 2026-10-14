//! The project arm of an import (#898, D-222): binding the names of a
//! `from m import a, b` against the project module `m` the driver loaded,
//! with the class and alias copies the importer needs. Split out of
//! `import.rs` when per-module namespaces (v0.3 accept 1) grew it past
//! ~1000 lines.

use super::{
    LoweredImport, RegisteredModule, ResolvedImports, ResolvedModule, check_alias_shape,
    check_from_import_shape, import_local_name, statement_span,
};
use crate::{
    HirItem, ImportBinding, ProjectBindingKind, is_builtin_exception_class, top_level_bound_names,
    unsupported,
};
use pycc_ast::StmtImportFrom;
use pycc_diag::{Diagnostic, Span};

/// The project arm of [`lower_import_stmt`]: binds every name of a
/// `from m import a, b` against the loaded module `m`.
pub(super) fn lower_project_from_import(
    import: &StmtImportFrom,
    module: &ResolvedModule<'_>,
    resolved: &ResolvedImports<'_>,
) -> Result<LoweredImport, Diagnostic> {
    check_from_import_shape(import)?;
    let mut lowered = LoweredImport::default();
    for alias in &import.names {
        check_alias_shape(import, alias)?;
        let name = alias.name.as_str();
        if module
            .submodule_names
            .iter()
            .any(|submodule| submodule == name)
        {
            return Err(unsupported(
                "module namespace bindings (`from pkg import submodule`) are not supported yet",
                import.range,
            ));
        }
        if !bind_project_name(
            name,
            module,
            resolved,
            statement_span(import.range),
            &mut lowered,
        )? {
            let module_name = match &import.module {
                Some(module_name) => format!("module `{module_name}` (`{}`)", module.display_path),
                None => format!("package `{}`", module.display_path),
            };
            return Err(Diagnostic::error(
                "T0021",
                format!("{module_name} has no top-level name `{name}`"),
                statement_span(import.range),
            ));
        }
    }
    Ok(lowered)
}

/// Looks `name` up in `module`'s top level in the documented order and,
/// when found, records the binding (and any class/alias copies it needs)
/// into `lowered`. Returns `Ok(false)` when the module has no such name,
/// and `Err` when the name resolves to a shape this part cannot re-export
/// (see the re-export branch). `span` is the importing statement's source
/// span, which is where such a diagnostic is reported.
fn bind_project_name(
    name: &str,
    module: &ResolvedModule<'_>,
    resolved: &ResolvedImports<'_>,
    span: Span,
    lowered: &mut LoweredImport,
) -> Result<bool, Diagnostic> {
    let origin = module.hir;
    let is_synthetic = |class_name: &str| {
        origin.seeded_builtin_exception_classes && is_builtin_exception_class(class_name)
    };
    let project = |kind| ImportBinding::Project {
        local_name: name.to_string(),
        module_path: module.display_path.clone(),
        kind,
    };
    if origin
        .class_defs
        .iter()
        .any(|(class_name, _)| class_name == name && !is_synthetic(class_name))
    {
        // The module being imported from is registered like every loaded
        // module (the driver registers each one as it lowers), so its
        // `__slots__` rows are found under its display path.
        let registered = resolved.origin(&module.display_path);
        copy_class_with_ancestors(registered, resolved, name, lowered);
        lowered.bindings.push(project(ProjectBindingKind::Class));
        return Ok(true);
    }
    if origin
        .items
        .iter()
        .any(|item| matches!(item, HirItem::Function { name: function_name, .. } if function_name == name))
    {
        lowered.bindings.push(project(ProjectBindingKind::Function));
        return Ok(true);
    }
    if let Some(alias) = origin
        .type_aliases
        .iter()
        .find(|(alias_name, _)| alias_name == name)
    {
        lowered.aliases.push(alias.clone());
        lowered
            .bindings
            .push(project(ProjectBindingKind::TypeAlias));
        return Ok(true);
    }
    if let Some(binding) = origin
        .imports
        .iter()
        .find(|binding| import_local_name(binding) == name)
    {
        // A re-export: `pkg/__init__.py` did `from .geometry import Point`
        // and the importer asks `pkg` for `Point`. The recorded binding
        // already names the defining module, so copy the class/alias from
        // there and keep pointing at it (one hop always suffices). A
        // re-exported stdlib binding is cloned as-is.
        if let ImportBinding::Project {
            module_path, kind, ..
        } = binding
        {
            let defining = resolved.origin(module_path);
            match kind {
                ProjectBindingKind::Class => {
                    copy_class_with_ancestors(defining, resolved, name, lowered);
                }
                ProjectBindingKind::TypeAlias => {
                    let alias = defining
                        .hir
                        .type_aliases
                        .iter()
                        .find(|(alias_name, _)| alias_name == name)
                        .expect("a TypeAlias binding names an alias its origin module defines");
                    lowered.aliases.push(alias.clone());
                }
                ProjectBindingKind::Function | ProjectBindingKind::Variable => {}
            }
        }
        if let ImportBinding::Foreign {
            module_path, from, ..
        } = binding
        {
            // Part 1 of #1026 binds a foreign import at its own source
            // position in its own module: its item index counts the items
            // *that* module's preceding statements produced, and
            // `program::link` rebases it onto the linked program as though
            // it belonged to the module that recorded it. Cloning the
            // binding into the importer would hand the importer's offset
            // to a dependency-local index (an out-of-range splice in
            // `pycc_mir`) and would run a second CPython import for one
            // source statement. The importer produces no item of its own
            // here, so there is no position in its item list that could
            // carry the binding honestly; representing this shape is a
            // later part's work.
            return Err(unsupported(
                format!(
                    "`{}` binds `{name}` to {}; re-exporting a foreign import across \
                     project modules is not supported yet",
                    module.display_path,
                    match from {
                        None => format!(
                            "the CPython module object `{}`",
                            crate::foreign_bound_module(name, module_path, None)
                        ),
                        Some(_) => crate::foreign_bound_object(module_path, from.as_ref()),
                    }
                ),
                span.start..span.end,
            ));
        }
        lowered.bindings.push(binding.clone());
        return Ok(true);
    }
    if top_level_bound_names(&origin.items).contains(name) {
        // #1244: CPython answers `from m import x` with `ImportError` when
        // `m`'s top level has deleted `x` by the time the import runs.
        // Refused whenever `m`'s top level deletes `x` anywhere, even if it
        // binds `x` again afterwards -- conservative, and sound.
        if crate::stmt::del::top_level_deleted_names(&origin.items).contains(name) {
            return Err(unsupported(
                format!(
                    "`{}` deletes its top-level `{name}` with `del`, so importing `{name}` \
                     from it is not supported",
                    module.display_path
                ),
                span.start..span.end,
            ));
        }
        lowered.bindings.push(project(ProjectBindingKind::Variable));
        return Ok(true);
    }
    Ok(false)
}

/// Copies `name`'s class definition and every class in its MRO (which
/// starts with the class itself) from `origin` into `classes`, skipping
/// any already copied. `class::lower_class` looks every MRO entry of a base
/// up in the importer's class table (`.expect("every class in the MRO must
/// be in defined_classes")`), so a class cannot be imported without its
/// ancestors; the seeded builtin exception ancestors come along too and
/// `module::lower_module` reconciles them with the importer's own seeding.
/// Each copied user class brings its `__slots__` row (#1368); a seeded
/// builtin exception class has none.
fn copy_class_with_ancestors(
    origin: RegisteredModule<'_>,
    resolved: &ResolvedImports<'_>,
    name: &str,
    lowered: &mut LoweredImport,
) {
    let classes = &mut lowered.classes;
    let (_, def) = origin
        .hir
        .class_defs
        .iter()
        .find(|(class_name, _)| class_name == name)
        .expect("a class binding names a class its origin module defines");
    for ancestor in &def.mro {
        if classes.iter().any(|(copied, _)| copied == ancestor) {
            continue;
        }
        // An ancestor the origin module itself imported (`class Dog(base.
        // Animal)` in `zoo`, imported from `zoo`) is defined, and so
        // registered, by another module of the program.
        let defining = std::iter::once(origin)
            .chain(resolved.modules.values().copied())
            .find(|module| {
                module
                    .hir
                    .class_defs
                    .iter()
                    .any(|(class_name, _)| class_name == ancestor)
            })
            .expect("every class in an MRO is in some loaded module's class table");
        let entry = defining
            .hir
            .class_defs
            .iter()
            .find(|(class_name, _)| class_name == ancestor)
            .expect("found above");
        classes.push(entry.clone());
        if let Some(row) = defining
            .class_slots
            .iter()
            .find(|(class_name, _)| class_name == ancestor)
        {
            lowered.class_slots.push(row.clone());
        }
    }
}
