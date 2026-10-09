//! Multi-name `import a, b` statements (#1280): each alias is requested,
//! answered, and lowered on its own, while the statement keeps a single
//! diagnostic and a single span.

use super::*;

/// Lowers `source`, answering the plain-import request for each module in
/// `answers` with its paired driver answer and leaving every other request
/// unanswered.
fn lower_answering(
    source: &str,
    answers: &[(&str, ResolvedImport<'static>)],
) -> Result<LoweredModule, Vec<Diagnostic>> {
    let parsed = parse(source);
    let mut resolved = ResolvedImports::default();
    for request in project_import_requests(&parsed) {
        if let Some((_, answer)) = answers
            .iter()
            .find(|(name, _)| request.module.as_deref() == Some(*name))
        {
            resolved.insert(request.span, answer.clone());
        }
    }
    lower_module(&parsed, &resolved, None)
}

fn only_error(result: Result<LoweredModule, Vec<Diagnostic>>) -> Diagnostic {
    let diagnostics = result.expect_err("fixture must fail to lower");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    diagnostics.into_iter().next().expect("one diagnostic")
}

fn module_bindings(module: &LoweredModule) -> Vec<(&str, pycc_std::StdModule)> {
    module
        .hir
        .imports
        .iter()
        .filter_map(|binding| match binding {
            ImportBinding::Module { local_name, module } => Some((local_name.as_str(), *module)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_multi_name_stdlib_import_binds_every_module() {
    let lowered = lower_answering("import math, enum\n", &[]).expect("must lower");
    assert_eq!(
        module_bindings(&lowered),
        vec![
            ("math", pycc_std::resolve_module("math").expect("math")),
            ("enum", pycc_std::resolve_module("enum").expect("enum")),
        ]
    );
}

#[test]
fn an_alias_inside_a_multi_name_import_binds_the_alias() {
    let lowered = lower_answering("import math as m, enum\n", &[]).expect("must lower");
    let names: Vec<&str> = module_bindings(&lowered)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(names, vec!["m", "enum"]);
}

#[test]
fn two_foreign_modules_share_the_statement_position_and_span() {
    let source = "import sys, re\n";
    let lowered = lower_answering(
        source,
        &[
            ("sys", ResolvedImport::Foreign),
            ("re", ResolvedImport::Foreign),
        ],
    )
    .expect("must lower");
    let statement = Span::new(0, source.trim_end().len() as u32);
    let foreign: Vec<(&str, &str, usize, Span)> = lowered
        .hir
        .imports
        .iter()
        .filter_map(|binding| match binding {
            ImportBinding::Foreign {
                local_name,
                module_path,
                from: None,
                site: crate::ForeignImportSite::Item(item_index),
                span,
            } => Some((
                local_name.as_str(),
                module_path.as_str(),
                *item_index,
                *span,
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        foreign,
        vec![("sys", "sys", 0, statement), ("re", "re", 0, statement)]
    );
}

#[test]
fn an_unresolvable_later_alias_fails_the_whole_statement_at_its_span() {
    let source = "import sys, os.path\n";
    let diagnostic = only_error(lower_answering(source, &[("sys", ResolvedImport::Foreign)]));
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "import of module `os.path` is not supported yet"
    );
    assert_eq!(
        diagnostic.span,
        Some(Span::new(0, source.trim_end().len() as u32))
    );
}

#[test]
fn a_project_module_among_stdlib_modules_names_the_namespace_gap() {
    let diagnostic = only_error(lower_answering(
        "import math, pkg\n",
        &[("pkg", ResolvedImport::Found)],
    ));
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "module namespace bindings (`import pkg`) are not supported yet"
    );
}

#[test]
fn only_unaliased_unresolved_names_are_requested_each_at_its_own_span() {
    let source = "import sys, math as m, re\n";
    let requests = project_import_requests(&parse(source));
    let at = |needle: &str| {
        let start = source.find(needle).expect("fixture contains the name") as u32;
        Span::new(start, start + needle.len() as u32)
    };
    let shapes: Vec<(Option<&str>, Span)> = requests
        .iter()
        .map(|request| (request.module.as_deref(), request.span))
        .collect();
    assert_eq!(
        shapes,
        vec![(Some("sys"), at("sys")), (Some("re"), at("re"))]
    );
}
