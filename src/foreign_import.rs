//! The native-build gate on a foreign CPython import (Part 1 of #1026,
//! narrowed by Part 1 of #1028).
//!
//! `import numpy` binds an opaque CPython module object, which only exists
//! while a CPython interpreter is running the artifact. `pycc build --ext`
//! produces exactly that: an extension module whose `Py_mod_exec` slot runs
//! inside the interpreter that loaded it. A build without `--ext` gets an
//! interpreter only by *embedding* one (D-128's `auto` default, realized for
//! the standard library by Part 1 of #1028): the executable starts a bundled
//! CPython and runs the compiled module as its `__main__`. Embedding is
//! possible on a macOS, Linux or Windows host with no `--target`, for
//! every root except a standard-library one the bundle excludes (Tcl/Tk);
//! a root outside the standard library is bundled from the program's
//! `pycc.lock` closure, which the build checks and refuses there, naming
//! `pycc lock` (#1242, on Windows #1296). Every other foreign import is
//! refused here with `I0403` rather than compiled into a call that could
//! only fail at run time.
//!
//! The interop policy (D-128, #1224) is evaluated first, per import: an
//! import the effective policy rejects is `I0402` on every host and
//! `--target`, and `I0403` applies only to a root the policy admits.
//!
//! This is the reason `crates/pycc_codegen/src/foreign_import.rs` may
//! silently ignore a `MirItem::ForeignImport` under `!options.ext` instead
//! of asserting: an embedded build compiles with `options.ext` set, and this
//! gate has refused every other program that could reach it.

use crate::embed::stdlib_roots::is_excluded_stdlib_root;
use crate::interop_policy::{self, EffectivePolicy};
use pycc_diag::Diagnostic;
use pycc_hir::{
    FromImport, HirModule, ImportBinding, foreign_import_statement, opens_foreign_statement,
};

/// Whether this build can embed a CPython interpreter at all, before any
/// individual import is looked at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EmbedHost {
    /// A macOS, Linux or Windows host building for itself.
    Available,
    /// `--target` was given: the bundled interpreter is the build host's,
    /// so it cannot serve another target.
    CrossTarget,
}

impl EmbedHost {
    /// Resolves the host from the build's `--target`, which reports the
    /// same reason on every Tier-1 leg. Every Tier-1 host embeds for
    /// itself: Windows since #1296 no longer narrows the roots it embeds.
    pub(crate) fn resolve(target: Option<&str>) -> Self {
        if target.is_some() {
            EmbedHost::CrossTarget
        } else {
            EmbedHost::Available
        }
    }
}

/// Whether a native build has to embed a CPython interpreter: `true` when
/// the program has at least one foreign import and every one of them is
/// embeddable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NeedsInterpreter(pub(crate) bool);

/// Why one foreign import cannot be embedded, in precedence order:
/// `CrossTarget` is host-level, applying to every foreign import in the
/// program and winning over the per-root `ExcludedStdlibRoot`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum I0403Reason {
    CrossTarget,
    ExcludedStdlibRoot,
}

/// The `I0403` message for `import {module_path}` (`as {local_name}` when
/// aliased, #1381) -- or, when `from` is `Some`, for `from {module_path}
/// import a, b` (#1278) -- refused for `reason`.
///
/// Every reason names `pycc build --ext` as the working alternative: the
/// code keeps its meaning ("this build cannot give the import a CPython
/// interpreter; `--ext` can"), only the set of cases reaching it narrowed.
pub(crate) fn i0403_message(
    local_name: &str,
    module_path: &str,
    from: Option<&FromImport>,
    reason: I0403Reason,
) -> String {
    let statement = foreign_import_statement(local_name, module_path, from);
    match reason {
        I0403Reason::CrossTarget => format!(
            "`{statement}` imports a CPython module, which requires \
             `pycc build --ext` in a `--target` build: an embedded executable \
             bundles the build host's own interpreter, which cannot serve another target"
        ),
        I0403Reason::ExcludedStdlibRoot => format!(
            "`{statement}` imports a standard-library module that needs \
             Tcl/Tk libraries from outside the interpreter, which requires \
             `pycc build --ext`: an embedded executable does not bundle it"
        ),
    }
}

/// The reason `module_path` cannot be embedded on `host`, or `None` when
/// it can.
fn refusal_reason(module_path: &str, host: EmbedHost) -> Option<I0403Reason> {
    if host == EmbedHost::CrossTarget {
        return Some(I0403Reason::CrossTarget);
    }
    let root = module_path.split('.').next().unwrap_or(module_path);
    // A root outside the standard library is embeddable too: the build
    // bundles it from the program's `pycc.lock` closure, and refuses there,
    // naming `pycc lock`, when the lock is missing or stale (#1242, #1296).
    is_excluded_stdlib_root(root).then_some(I0403Reason::ExcludedStdlibRoot)
}

/// Classifies a program for a build without `--ext`: `Ok(NeedsInterpreter(
/// false))` when it has no foreign import (a plain native executable),
/// `Ok(NeedsInterpreter(true))` when every foreign import is embeddable,
/// and otherwise one gap per refused import, in source order: `I0402` when
/// `policy` rejects it (#1224), else `I0403` when it cannot be embedded. An
/// import is classified once, so it is never reported under both codes.
///
/// Shaped like `src/ext_build.rs`'s `collect_exports`: a driver-side
/// refusal against typed HIR that returns every gap at once rather than
/// only the first, so one build reports the whole list.
///
/// Each gap is paired with the import's own position in `hir.imports`. A
/// multi-file program's foreign import usually lives in a dependency, not
/// in the entry file, and that position is what says which --
/// `pycc_hir::link` concatenates each module's import table in the
/// program's file order, so `src/frontend.rs`'s `ProgramSources` turns the
/// position back into the owning file and the `I0403` renders against that
/// file's path and source instead of being attributed wholesale to the
/// entry (PR 1c of #1080 review finding 2).
///
/// The position, not the item index: an import's recorded item index is the
/// item count at the moment it lowered, so a *trailing* import in one file
/// and a *leading* import in the next record the same linked index and no
/// arithmetic on the per-file item bounds can tell them apart. The import
/// table has no such boundary ambiguity.
pub(crate) fn classify_for_native_build(
    hir: &HirModule,
    host: EmbedHost,
    policy: &EffectivePolicy,
) -> Result<NeedsInterpreter, Vec<(usize, Diagnostic)>> {
    let mut any_foreign = false;
    let gaps: Vec<(usize, Diagnostic)> = hir
        .imports
        .iter()
        .enumerate()
        .filter_map(|(position, binding)| match binding {
            ImportBinding::Foreign {
                local_name,
                module_path,
                from,
                span,
                ..
            } => {
                any_foreign = true;
                // #1278: `from tkinter import Tk, Label` is one statement
                // and one diagnostic, reported on its first name only.
                if !opens_foreign_statement(from.as_ref()) {
                    return None;
                }
                if let Some(rejected) =
                    interop_policy::rejection(policy, local_name, module_path, from.as_ref(), *span)
                {
                    return Some((position, rejected));
                }
                refusal_reason(module_path, host).map(|reason| {
                    (
                        position,
                        Diagnostic::error(
                            "I0403",
                            i0403_message(local_name, module_path, from.as_ref(), reason),
                            // The import statement's own range, carried on
                            // the binding. Before it was, every `I0403` was
                            // built with `Span::new(0, 0)`, so a foreign
                            // import that was not the first statement
                            // reported at `<file>:1:1` and highlighted an
                            // unrelated line (PR 1c of #1080 review round 4).
                            *span,
                        ),
                    )
                })
            }
            ImportBinding::Module { .. }
            | ImportBinding::Symbol { .. }
            | ImportBinding::Project { .. } => None,
        })
        .collect();
    if gaps.is_empty() {
        return Ok(NeedsInterpreter(any_foreign));
    }
    Err(gaps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pycc_diag::Span;
    use pycc_hir::ProjectBindingKind;

    fn hir(imports: Vec<ImportBinding>) -> HirModule {
        HirModule {
            seeded_builtin_exception_classes: false,
            items: Vec::new(),
            type_aliases: Vec::new(),
            imports,
            class_defs: Vec::new(),
        }
    }

    /// The binding of an unaliased `import {name}`, which binds the root
    /// of a dotted name (#1381).
    fn foreign(name: &str) -> ImportBinding {
        ImportBinding::Foreign {
            local_name: name.split('.').next().unwrap_or(name).to_string(),
            module_path: name.to_string(),
            from: None,
            site: pycc_hir::ForeignImportSite::Item(0),
            span: Span::new(0, 0),
        }
    }

    fn project() -> ImportBinding {
        // A project import is the admitted binding spelled here because
        // the driver crate deliberately does not depend on `pycc_std`,
        // which `ImportBinding::Module`/`Symbol` need to construct; the
        // stdlib arms are covered by `crates/pycc_types/src/foreign/tests.rs`,
        // whose filter is the same shape.
        ImportBinding::Project {
            local_name: "helper".to_string(),
            module_path: "pkg.helper".to_string(),
            kind: ProjectBindingKind::Function,
        }
    }

    const AUTO: EffectivePolicy = EffectivePolicy::Auto;

    const ALL_HOSTS: [EmbedHost; 2] = [EmbedHost::Available, EmbedHost::CrossTarget];

    #[test]
    fn the_host_resolves_from_the_target_alone() {
        assert_eq!(EmbedHost::resolve(None), EmbedHost::Available);
        assert_eq!(
            EmbedHost::resolve(Some("x86_64-apple-darwin")),
            EmbedHost::CrossTarget
        );
    }

    #[test]
    fn a_program_without_a_foreign_import_needs_no_interpreter_on_any_host() {
        for host in ALL_HOSTS {
            assert_eq!(
                classify_for_native_build(&hir(vec![project()]), host, &AUTO),
                Ok(NeedsInterpreter(false)),
                "{host:?}"
            );
            assert_eq!(
                classify_for_native_build(&hir(Vec::new()), host, &AUTO),
                Ok(NeedsInterpreter(false)),
                "{host:?}"
            );
        }
    }

    #[test]
    fn a_standard_library_program_embeds_on_an_available_host() {
        let program = hir(vec![foreign("json"), project(), foreign("gc")]);
        assert_eq!(
            classify_for_native_build(&program, EmbedHost::Available, &AUTO),
            Ok(NeedsInterpreter(true))
        );
    }

    fn messages(host: EmbedHost, imports: Vec<ImportBinding>) -> Vec<(usize, String)> {
        classify_for_native_build(&hir(imports), host, &AUTO)
            .expect_err("refused")
            .into_iter()
            .map(|(position, gap)| {
                assert_eq!(gap.code, "I0403");
                (position, gap.message)
            })
            .collect()
    }

    #[test]
    fn a_host_level_reason_refuses_every_foreign_import_even_a_standard_library_one() {
        let reason = I0403Reason::CrossTarget;
        let gaps = messages(
            EmbedHost::CrossTarget,
            vec![foreign("json"), project(), foreign("numpy")],
        );
        assert_eq!(
            gaps,
            vec![
                (0, i0403_message("json", "json", None, reason)),
                (2, i0403_message("numpy", "numpy", None, reason)),
            ]
        );
    }

    /// #1291: a nested import is refused like a top-level one, at its own
    /// span.
    #[test]
    fn a_block_foreign_import_is_refused_at_its_own_span() {
        let nested = ImportBinding::Foreign {
            local_name: "json".to_string(),
            module_path: "json".to_string(),
            from: None,
            site: pycc_hir::ForeignImportSite::Block { optional: false },
            span: Span::new(10, 21),
        };
        let gaps = classify_for_native_build(&hir(vec![nested]), EmbedHost::CrossTarget, &AUTO)
            .expect_err("refused");
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].1.code, "I0403");
        assert_eq!(gaps[0].1.span, Some(Span::new(10, 21)));
    }

    #[test]
    fn only_an_excluded_root_is_refused_on_an_available_host() {
        // A mixed program: the embeddable `json` and the third-party roots
        // `numpy` and `scipy` (bundled from the lock, #1242) are not
        // reported, and the refused import carries its position in the
        // whole import table -- not its position among the foreign ones --
        // because that is the index the driver joins against the program's
        // per-file import bounds to name the file that owns the import.
        let gaps = messages(
            EmbedHost::Available,
            vec![
                foreign("numpy"),
                project(),
                foreign("json"),
                foreign("tkinter"),
                foreign("scipy.linalg"),
            ],
        );
        assert_eq!(
            gaps,
            vec![(
                3,
                i0403_message("tkinter", "tkinter", None, I0403Reason::ExcludedStdlibRoot)
            ),]
        );
        assert_eq!(
            classify_for_native_build(&hir(vec![foreign("numpy")]), EmbedHost::Available, &AUTO),
            Ok(NeedsInterpreter(true))
        );
    }

    #[test]
    fn a_dotted_path_is_admitted_by_its_root() {
        assert_eq!(
            classify_for_native_build(
                &hir(vec![foreign("xml.etree")]),
                EmbedHost::Available,
                &AUTO
            ),
            Ok(NeedsInterpreter(true))
        );
        assert_eq!(
            messages(EmbedHost::Available, vec![foreign("tkinter.ttk")]),
            vec![(
                0,
                i0403_message(
                    "tkinter",
                    "tkinter.ttk",
                    None,
                    I0403Reason::ExcludedStdlibRoot
                )
            )]
        );
    }

    #[test]
    fn every_reason_names_the_import_and_the_ext_alternative() {
        for (reason, detail) in [
            (I0403Reason::CrossTarget, "`--target` build"),
            (I0403Reason::ExcludedStdlibRoot, "Tcl/Tk"),
        ] {
            let message = i0403_message("numpy", "numpy", None, reason);
            assert!(message.starts_with("`import numpy` imports a"), "{message}");
            assert!(message.contains("requires `pycc build --ext`"), "{message}");
            assert!(message.contains(detail), "{message}");
        }
    }

    /// #1224: the policy is evaluated first and per import. A rejected root
    /// is `I0402` on every host -- never `I0403`, even where the host alone
    /// would refuse it -- and an admitted root still meets the embedding
    /// gate, so one program can carry one of each.
    #[test]
    fn a_policy_rejection_wins_over_the_embedding_refusal_per_import() {
        use crate::interop_policy::PolicySource;
        let policy = EffectivePolicy::Allowlist {
            allow: vec!["json".to_string(), "tkinter".to_string()],
            source: PolicySource::CliFlag,
        };
        let classified = classify_for_native_build(
            &hir(vec![foreign("pprint"), foreign("tkinter"), foreign("json")]),
            EmbedHost::Available,
            &policy,
        )
        .expect_err("refused");
        let codes: Vec<(usize, &str)> = classified
            .iter()
            .map(|(position, gap)| (*position, gap.code))
            .collect();
        assert_eq!(codes, vec![(0, "I0402"), (1, "I0403")]);
        for host in ALL_HOSTS {
            let deny = EffectivePolicy::Deny {
                source: PolicySource::Pure,
            };
            let gaps = classify_for_native_build(&hir(vec![foreign("json")]), host, &deny)
                .expect_err("refused");
            assert_eq!(gaps.len(), 1);
            assert_eq!(gaps[0].1.code, "I0402", "{host:?}");
        }
        assert_eq!(
            classify_for_native_build(&hir(vec![foreign("json")]), EmbedHost::Available, &policy),
            Ok(NeedsInterpreter(true))
        );
    }

    /// The binding `from {module} import {names}` makes for `names[index]`,
    /// at the fixed statement span `span`.
    fn from_foreign(module: &str, names: &[&str], index: usize, span: Span) -> ImportBinding {
        ImportBinding::Foreign {
            local_name: names[index].to_string(),
            module_path: module.to_string(),
            from: Some(FromImport {
                name: names[index].to_string(),
                fromlist: names.iter().map(ToString::to_string).collect(),
                index,
                level: 0,
            }),
            site: pycc_hir::ForeignImportSite::Item(0),
            span,
        }
    }

    /// #1278: every name of `from tkinter import Tk, Label` shares one
    /// statement, so it is refused once, on its first name, quoting the
    /// statement as written.
    #[test]
    fn a_multi_name_from_import_is_refused_once_quoting_the_statement() {
        let span = Span::new(0, 29);
        let statement = |index| from_foreign("tkinter", &["Tk", "Label"], index, span);
        let gaps = messages(EmbedHost::Available, vec![statement(0), statement(1)]);
        assert_eq!(
            gaps,
            vec![(
                0,
                i0403_message(
                    "Tk",
                    "tkinter",
                    Some(&FromImport {
                        name: "Tk".to_string(),
                        fromlist: vec!["Tk".to_string(), "Label".to_string()],
                        index: 0,
                        level: 0,
                    }),
                    I0403Reason::ExcludedStdlibRoot
                )
            )]
        );
        let message = &gaps[0].1;
        let prefix = "`from tkinter import Tk, Label` imports a";
        assert!(message.starts_with(prefix), "{message}");
    }

    /// The dedup keys on the statement, not on the span: two linked files
    /// can hold byte-identical spans, and each statement keeps its own
    /// diagnostic, as each alias of a plain `import a, b` does.
    #[test]
    fn identical_spans_from_two_statements_each_keep_their_diagnostic() {
        let span = Span::new(0, 22);
        let gaps = messages(
            EmbedHost::Available,
            vec![
                from_foreign("tkinter", &["Tk"], 0, span),
                from_foreign("tkinter", &["Tk"], 0, span),
            ],
        );
        assert_eq!(gaps.len(), 2, "{gaps:?}");
        let plain = messages(EmbedHost::CrossTarget, vec![foreign("json"), foreign("gc")]);
        assert_eq!(plain.len(), 2, "{plain:?}");
    }

    /// A later name of a from-import is still a foreign import, so the
    /// program still needs an interpreter even though that name is never
    /// reported on its own.
    #[test]
    fn a_later_from_import_name_still_needs_an_interpreter() {
        assert_eq!(
            classify_for_native_build(
                &hir(vec![from_foreign(
                    "json",
                    &["dumps", "loads"],
                    1,
                    Span::new(0, 0)
                )]),
                EmbedHost::Available,
                &AUTO
            ),
            Ok(NeedsInterpreter(true))
        );
    }
}
