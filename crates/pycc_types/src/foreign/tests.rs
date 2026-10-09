//! Unit coverage for the foreign-binding helpers themselves; the
//! end-to-end refusals live in `tests/issue_1080_foreign_object.rs`.

use super::*;
use pycc_hir::ProjectBindingKind;

#[test]
fn lists_only_the_foreign_bindings() {
    let imports = vec![
        ImportBinding::Module {
            local_name: "math".to_string(),
            module: pycc_std::StdModule::Math,
        },
        ImportBinding::Symbol {
            local_name: "sqrt".to_string(),
            module: pycc_std::StdModule::Math,
            symbol: pycc_std::resolve_symbol(pycc_std::StdModule::Math, "sqrt")
                .expect("`math.sqrt` is a registered stdlib symbol"),
        },
        ImportBinding::Project {
            local_name: "helper".to_string(),
            module_path: "pkg.helper".to_string(),
            kind: ProjectBindingKind::Function,
        },
        ImportBinding::Foreign {
            local_name: "numpy".to_string(),
            module_path: "numpy".to_string(),
            from: None,
            site: pycc_hir::ForeignImportSite::Item(0),
            span: Span::new(0, 0),
        },
    ];
    assert_eq!(foreign_object_names(&imports), vec!["numpy"]);
}

#[test]
fn the_guard_admits_every_other_type_and_refuses_the_object() {
    assert!(reject_object_read("x", &Ty::Int).is_ok());
    let diagnostic = reject_object_read("numpy", &Ty::Object).expect_err("object is refused");
    assert_eq!(diagnostic.code, "I0404");
    assert!(diagnostic.message.contains("numpy"), "{diagnostic:?}");
}

/// Lowers a source snippet exactly as `crate::module`'s own tests do. No
/// resolver participates, so the snippet must not contain an `import`:
/// a foreign binding is classified by the driver (`src/modules.rs`) and is
/// attached to the lowered module by hand below.
fn lower(source: &str) -> pycc_hir::HirModule {
    let module = pycc_parser::parse(source).expect("test source must parse");
    pycc_hir::lower_checked(&module).expect("test source must lower")
}

fn with_foreign_import(mut hir: pycc_hir::HirModule) -> pycc_hir::HirModule {
    hir.imports.push(ImportBinding::Foreign {
        local_name: "numpy".to_string(),
        module_path: "numpy".to_string(),
        from: None,
        site: pycc_hir::ForeignImportSite::Item(0),
        span: Span::new(0, 0),
    });
    hir
}

/// Part 1 of #1026 gave `HirModule::imports` a reader downstream of the
/// type checker -- `pycc_mir::build` splices a `MirItem::ForeignImport` per
/// foreign binding, and `src/main.rs`'s `I0403` gate reads the same list --
/// where before it was fully discharged during HIR lowering.
/// `monomorphize` used to drop the field on the strength of having no such
/// reader, which silently produced an artifact that imported nothing.
///
/// Both of its exits are asserted: the early return taken by a module with
/// no generic function, and the rewriting path taken by one with.
#[test]
fn the_resolved_module_still_carries_its_foreign_imports() {
    for source in [
        "def f(x: int) -> int:\n    return x\n",
        "def f[T](x: T) -> T:\n    return x\n\n\ndef g() -> int:\n    return f(1)\n",
    ] {
        let resolved = crate::check_and_resolve_all_keyed(&with_foreign_import(lower(source)))
            .expect("the fixture type-checks");
        assert_eq!(
            foreign_object_names(&resolved.imports),
            vec!["numpy"],
            "{source}"
        );
    }
}

/// The same lowering helper, with the foreign binding recorded at an
/// arbitrary position rather than always at 0 -- which is what the
/// positional claims below are about.
fn with_foreign_import_at(mut hir: pycc_hir::HirModule, item_index: usize) -> pycc_hir::HirModule {
    hir.imports.push(ImportBinding::Foreign {
        local_name: "numpy".to_string(),
        module_path: "numpy".to_string(),
        from: None,
        site: pycc_hir::ForeignImportSite::Item(item_index),
        span: Span::new(0, 0),
    });
    hir
}

/// The recorded position of the single foreign binding in `imports`.
fn foreign_position(imports: &[ImportBinding]) -> usize {
    imports
        .iter()
        .find_map(|binding| match binding {
            ImportBinding::Foreign {
                site: pycc_hir::ForeignImportSite::Item(item_index),
                ..
            } => Some(*item_index),
            _ => None,
        })
        .expect("the fixture records exactly one foreign binding")
}

/// PR 1c of #1080 review finding 1, the panic arm. `monomorphize` drops
/// every original generic function, so an import recorded after two of
/// them used to survive as index 2 against an item list of length 0 --
/// `pycc_mir::splice_foreign_imports` then panicked in `Vec::insert`
/// ("insertion index (is 2) should be <= len (is 0)"). The position is
/// recomputed to 0, which is both in range and where the import belongs:
/// nothing that preceded it still exists.
#[test]
fn a_foreign_import_after_dropped_generics_is_repositioned_to_the_surviving_prefix() {
    let source = "def _a[T](x: T) -> T:\n    return x\n\n\ndef _b[T](x: T) -> T:\n    return x\n";
    let resolved = crate::check_and_resolve_all_keyed(&with_foreign_import_at(lower(source), 2))
        .expect("the fixture type-checks");
    assert!(resolved.items.is_empty(), "{:?}", resolved.items);
    assert_eq!(foreign_position(&resolved.imports), 0);
}

/// The same finding's silent arm, which is the one that reached an
/// artifact: one dropped generic followed by a surviving `def` left the
/// import at index 1 against a one-item list, so `insert(1, ..)` *appended*
/// it -- emitting the import after the function instead of before it, in
/// violation of D-244 rule 3. The recomputed position is 0, so the import
/// still precedes the item that followed it in the source.
#[test]
fn a_foreign_import_before_a_surviving_item_keeps_preceding_it() {
    let source = "def _a[T](x: T) -> T:\n    return x\n\n\ndef f() -> int:\n    return 1\n";
    let resolved = crate::check_and_resolve_all_keyed(&with_foreign_import_at(lower(source), 1))
        .expect("the fixture type-checks");
    assert_eq!(foreign_position(&resolved.imports), 0);
    let names: Vec<&str> = resolved
        .items
        .iter()
        .filter_map(|item| match item {
            pycc_hir::HirItem::Function { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(names, vec!["f"], "{:?}", resolved.items);
}

/// `monomorphize`'s other exit -- the early return a module with no
/// generic, no generic class and no protocol parameter takes -- returns
/// `hir.items` unchanged, so every recorded position must survive
/// untouched. Asserted rather than argued, because "this exit does not
/// rewrite the list" is exactly the kind of claim a later change breaks.
#[test]
fn the_non_generic_exit_leaves_a_recorded_position_alone() {
    let source = "x = 1\n\n\ndef f() -> int:\n    return 1\n";
    let resolved = crate::check_and_resolve_all_keyed(&with_foreign_import_at(lower(source), 1))
        .expect("the fixture type-checks");
    assert_eq!(foreign_position(&resolved.imports), 1);
    assert_eq!(resolved.items.len(), 2);
}

/// PR 1c of #1080 review finding 2. The pre-seed above cannot supersede a
/// binding the source-order pass makes *later*, so `def json()` followed by
/// `import json` left `json` marked def-rebound, the call gate skipped the
/// `I0404` refusal, and `json()` compiled into a call to the shadowed
/// function -- where CPython raises `TypeError: 'module' object is not
/// callable`. Applying the binding at its recorded position is what refuses
/// it. Since #1313 admits a module-body direct call of a CPython object, and
/// #1325 binding its result to a module-level name, and #1340 printing it,
/// the probe declares the call's result `int`: that is refused only if
/// `json` is the foreign object (an `object` is assignable to nothing),
/// while the `int` the shadowed `def` returns would bind cleanly. What the
/// test pins is that the call is no longer a native call of the shadowed
/// `def`.
#[test]
fn a_foreign_import_supersedes_an_earlier_def_of_the_same_name() {
    let source = "def json() -> int:\n    return 1\n\n\nx: int = json()\n";
    let hir = with_foreign_import_named(lower(source), "json", 1);
    let diagnostics = crate::check_all(&hir).expect_err("the binding must be refused");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == "T0025" && d.message.contains("`object`")),
        "{diagnostics:?}"
    );
}

/// The opposite order is a program CPython *runs*: the `def` executes after
/// the import and rebinds the name to a function, so the call is an
/// ordinary call and must keep compiling. Refusing this one would be an
/// over-rejection introduced by the fix above, which is why both orders are
/// pinned.
#[test]
fn a_def_after_a_foreign_import_rebinds_the_name_and_still_compiles() {
    let source = "def json() -> int:\n    return 1\n\n\nx = json()\n";
    let hir = with_foreign_import_named(lower(source), "json", 0);
    crate::check_all(&hir).expect("the `def` below the import wins");
}

/// As [`with_foreign_import_at`], for a fixture that needs the binding to
/// collide with a name the source itself defines.
fn with_foreign_import_named(
    mut hir: pycc_hir::HirModule,
    local_name: &str,
    item_index: usize,
) -> pycc_hir::HirModule {
    hir.imports.push(ImportBinding::Foreign {
        local_name: local_name.to_string(),
        module_path: local_name.to_string(),
        from: None,
        site: pycc_hir::ForeignImportSite::Item(item_index),
        span: Span::new(0, 0),
    });
    hir
}

/// `unroll_enum_loops` runs *after* `monomorphize` and is the one remaining
/// pass that changes how many items the list holds: a top-level
/// `for c in Color:` becomes one loop-variable assignment plus one body copy
/// per enum member. A position recorded after such a loop therefore has to
/// be recomputed as well, or `pycc_mir::splice_foreign_imports` emits the
/// import *inside* the unrolled sequence -- running it before top-level
/// statements that precede it in source. The non-foreign binding alongside
/// it pins that the remap carries the rest of the table through untouched.
#[test]
fn a_foreign_import_after_an_unrolled_enum_loop_is_repositioned_past_it() {
    let source =
        "class Color(Enum):\n    RED = 1\n    GREEN = 2\nfor c in Color:\n    print(c.value)\n";
    let mut hir = with_foreign_import_at(lower(source), 1);
    hir.imports.push(ImportBinding::Module {
        local_name: "math".to_string(),
        module: pycc_std::StdModule::Math,
    });
    let resolved = crate::check_and_resolve_all_keyed(&hir).expect("the fixture type-checks");
    assert_eq!(resolved.items.len(), 4, "{:?}", resolved.items);
    assert_eq!(foreign_position(&resolved.imports), 4);
    assert!(
        matches!(resolved.imports[1], ImportBinding::Module { .. }),
        "{:?}",
        resolved.imports
    );
}

// ---------------------------------------------------------------------------
// Part 2 of #1026 (#1081): the consumer-side refusals.
//
// Part 1 refused the *read* of a foreign binding, so one test per binding
// covered every derived operation. Part 2 admits the read and refuses each
// consuming site on its own (see this module's `super` doc), so the coverage
// obligation changes shape: every refused consumer is owed a test, and each
// is owed it in **both** producer shapes -- `numpy.pi` itself, and a call to
// an unannotated private helper whose solver-inferred return is `object`
// (`constraints.rs`'s `AttrGet` term). The second shape is what proves a
// refusal keys on the *type* rather than on the producing expression.
// ---------------------------------------------------------------------------

/// The label [`both_producer_shapes`] gives the helper-call shape.
const HELPER_SHAPE: &str = "a private helper returning `object`";

/// `snippet` in each of the two producer shapes: literally, and with every
/// `numpy.pi` replaced by a call to a private helper that returns one.
///
/// The helper carries no return annotation on purpose: in a native build
/// `object` is unspellable in one (`docs/TYPE_SYSTEM.md`'s `object` row;
/// D-258 admits it only in an `--ext` module), and `numpy.pi`
/// is no class a foreign import binds that could name it (Part 1 of #1367),
/// so a solver-inferred return is how this helper's call gets the type.
///
/// #1316 admits the helper's foreign read and Part 1 of #1333 its `return`
/// of a CPython object, so the helper really produces one and a caller sees
/// the consumer's own refusal.
fn both_producer_shapes(snippet: &str) -> [(&'static str, String); 2] {
    [
        ("`numpy.pi`", snippet.to_string()),
        (
            HELPER_SHAPE,
            format!(
                "def _h():\n    return numpy.pi\n\n\n{}",
                snippet.replace("numpy.pi", "_h()")
            ),
        ),
    ]
}

/// The diagnostics `check_all` reports for `source` with `numpy` bound as a
/// foreign object, or `None` when it type-checks.
fn check_foreign(source: &str) -> Option<Vec<pycc_diag::Diagnostic>> {
    crate::check_all(&with_foreign_import(lower(source))).err()
}

/// Asserts that `source` is refused with exactly one diagnostic carrying
/// `code` and mentioning `phrase`. `shape` names the producer shape under
/// test, so a failure says which of the two broke.
fn assert_refused(shape: &str, source: &str, code: &str, phrase: &str) {
    let diagnostics =
        check_foreign(source).unwrap_or_else(|| panic!("must be refused ({shape}): {source}"));
    assert_eq!(
        diagnostics.len(),
        1,
        "{shape}: {diagnostics:?} for {source}"
    );
    assert_eq!(
        diagnostics[0].code, code,
        "{shape}: {diagnostics:?} for {source}"
    );
    assert!(
        diagnostics[0].message.contains(phrase),
        "{shape}: {diagnostics:?} for {source}"
    );
}

/// A discarded attribute load on a CPython object is the one operation
/// Part 2 adds, so at **module scope** it must not be refused.
///
/// This is the positive half of the migration: Part 1's
/// `reject_object_read` refused it unconditionally, and a regression that
/// reinstated it there would be invisible to every refusal test below.
#[test]
fn a_module_scope_attribute_load_on_a_cpython_object_is_admitted() {
    let source = "numpy.pi\n";
    assert!(check_foreign(source).is_none(), "{source}");
}

/// #1316: a function body may read a module-level foreign name, and since
/// Part 1 of #1333 a CPython object may leave the function through
/// `return`. The read itself is admitted (`numpy.pi` as a discarded
/// statement), and so is rendering it (#1340).
#[test]
fn a_function_body_reads_renders_and_returns_a_foreign_object() {
    for source in [
        "def f() -> None:\n    numpy.pi\n",
        "def _h():\n    return numpy.pi\n\n\n_h()\n",
        "def _h():\n    return numpy.pi\n\n\nx = 1\n",
        "def _h():\n    return numpy\n\n\nx = 1\n",
        "def f() -> None:\n    print(numpy)\n",
        "def f() -> None:\n    print(numpy.pi, 1, \"x\", None)\n",
        "def f() -> None:\n    print(f\"<{numpy.pi}>\")\n",
    ] {
        assert!(check_foreign(source).is_none(), "{source}");
    }
}

/// Since Part 1 of #1333 the function-body read is keyed on the `object`
/// type, not on the foreign import: an `object` global the module did not
/// bind by a foreign import is read and called like one, and both have
/// type `object`.
///
/// The first half is pinned on a hand-built function environment, so it
/// does not depend on how the global came to be bound. A module-level `for`
/// loop variable over a CPython object may be unbound after the loop and is
/// `T0041` first, which the second half pins.
#[test]
fn a_non_import_object_global_is_read_in_a_function_body() {
    let mut module = crate::env::Environment::new();
    module.bind("x".to_string(), Ty::Object);
    module.foreign_globals.insert("numpy".to_string());
    let body = module.child_for_function(&[]);
    for expr in [
        pycc_hir::HirExpr::Name("x".to_string()),
        pycc_hir::HirExpr::Call {
            callee: "x".to_string(),
            args: vec![],
        },
    ] {
        let ty = crate::expr::infer_expr_in(&body, &[], &expr)
            .expect("a non-import `object` global is admitted");
        assert_eq!(ty, Ty::Object, "{expr:?}");
    }
    assert_refused(
        "a module-level `for` loop variable",
        "for x in numpy.pi:\n    pass\ndef f() -> None:\n    len(x)\n",
        "T0041",
        "may not be bound",
    );
}

/// A module-body read placed *above* its own `import` is an unbound name.
///
/// PR 2a of #1081 removed Part 1's pre-seed in `crate::module` for exactly
/// this: with the read admitted, the seed made such a program type-check
/// and trap at run time. `T0021` is also the closer answer -- CPython
/// raises `NameError`.
#[test]
fn a_module_body_read_above_the_import_is_unbound() {
    let mut hir = lower("numpy.pi\n");
    hir.imports.push(ImportBinding::Foreign {
        local_name: "numpy".to_string(),
        module_path: "numpy".to_string(),
        from: None,
        site: pycc_hir::ForeignImportSite::Item(1),
        span: Span::new(0, 0),
    });
    let diagnostics = crate::check_all(&hir).expect_err("a read above the import is refused");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, "T0021", "{diagnostics:?}");
    assert!(
        diagnostics[0].message.contains("`numpy` is not defined"),
        "{diagnostics:?}"
    );
}

/// Every consuming site the refusal migration moved the `I0404` to, in
/// both producer shapes.
///
/// Every snippet here sits at **module scope**, which is the whole set of
/// positions a `Ty::Object` can still occupy after PR 2a of #1081 narrowed
/// the read inside a function body.
///
/// PR 3a of #1082 removed this table's five condition rows -- `if`,
/// `while` and the three comprehension guards -- along with the ten
/// `reject_object_condition` call sites they pinned. A truth test on a
/// CPython object is a supported operation now
/// (`crates/pycc_codegen/src/foreign_len.rs`), so there is no refusal left
/// to assert; `tests/issue_1082_foreign_len_and_truth.rs` asserts the
/// acceptance in its place. What remains here is the rendering and `match`
/// group, which PR 3a does not touch. #1316 admits the in-function shapes of
/// all five condition sites, which
/// [`the_in_function_condition_sites_are_admitted`] pins.
///
/// #1325 removed the `x = numpy.pi` row: a module-level binding is admitted
/// now (`foreign/binding_tests.rs`). The walrus no longer reaches that
/// guard's refusal either, so it draws the `T0050` its operand always would
/// have, pinned separately below the table.
#[test]
fn every_module_scope_consuming_site_refuses_a_cpython_object_in_both_producer_shapes() {
    for (phrase, snippet) in [(
        "matching on a CPython object",
        "match numpy.pi:\n    case 1:\n        print(1)\n",
    )] {
        for (shape, source) in both_producer_shapes(snippet) {
            assert_refused(shape, &source, "I0404", phrase);
        }
    }
    for (shape, source) in both_producer_shapes("if (y := numpy.pi):\n    print(1)\n") {
        assert_refused(
            shape,
            &source,
            "T0050",
            "walrus assignment (`:=`) value of type `object`",
        );
    }
}

/// #1340 removed the table's rendering row: `print` and f-string
/// interpolation of a CPython object are admitted at module scope, with any
/// number of other `print` arguments, because `pycc_codegen` renders the
/// object through the shim (`crates/pycc_codegen/src/string_render.rs`).
///
/// The helper shape is admitted too, since Part 1 of #1333 admits the
/// helper's own `return`.
#[test]
fn rendering_a_cpython_object_is_admitted_in_both_producer_shapes() {
    for snippet in [
        "print(numpy.pi)\n",
        "print(numpy.pi, 1, \"x\", None, True)\n",
        "print(f\"pi={numpy.pi}!\")\n",
    ] {
        for (shape, source) in both_producer_shapes(snippet) {
            assert!(check_foreign(&source).is_none(), "{shape}: {source}");
        }
    }
}

/// The five condition sites inside a function body, which is a separate
/// pass in `crate::lib`, are all admitted (#1316): a function body now has
/// the failure edge a raising `PyObject_IsTrue` takes
/// (`pycc_codegen::foreign_fail`).
#[test]
fn the_in_function_condition_sites_are_admitted() {
    for snippet in [
        "def f() -> None:\n    if numpy.pi:\n        print(1)\n",
        "def f() -> None:\n    while numpy.pi:\n        print(1)\n",
        "def f() -> None:\n    xs = [i for i in range(3) if numpy.pi]\n",
        "def f() -> None:\n    ys = {i for i in range(3) if numpy.pi}\n",
        "def f() -> None:\n    zs = {\"k\": i for i in range(3) if numpy.pi}\n",
    ] {
        assert!(check_foreign(snippet).is_none(), "{snippet}");
    }
}

/// `isinstance` is the one site whose two producer shapes give different
/// answers, so it is pinned separately rather than bent into the table.
///
/// The direct shape is a run-time `PyObject_IsInstance` since Part 1 of
/// #1371 (`foreign::compare::check_object_isinstance`), so a builtin class
/// is admitted and a tuple of classes is the refused shape. The helper
/// shape never gets there: `isinstance` already refuses *any* call
/// expression as its first argument with `C0001`, because evaluating one
/// would lose its side effects.
#[test]
fn isinstance_admits_a_cpython_object_and_refuses_a_call_expression() {
    assert!(check_foreign("if isinstance(numpy.pi, int):\n    print(1)\n").is_none());
    assert_refused(
        "`numpy.pi`",
        "if isinstance(numpy.pi, (int, str)):\n    print(1)\n",
        "I0404",
        "against a tuple of classes",
    );
    assert_refused(
        "a private helper returning `object`",
        &both_producer_shapes("if isinstance(numpy.pi, int):\n    print(1)\n")[1].1,
        "C0001",
        "side effects would be lost",
    );
}

/// The consumers Part 2 deliberately leaves to their pre-existing
/// diagnostics, pinned so a later refusal migration cannot silently
/// re-label them.
///
/// `docs/TYPE_SYSTEM.md`'s `object` row enumerates exactly this split: a
/// site that would otherwise *accept* the value needs the new `I0404`,
/// while a site whose existing rule already rejects `object` by name keeps
/// that rule -- a second guard there would be unreachable and untestable.
#[test]
fn the_operations_with_a_pre_existing_refusal_keep_it() {
    for (code, phrase, snippet) in [
        ("T0021", "operator Add is not defined", "x = numpy.pi + 1\n"),
        (
            "T0021",
            "unary operator USub is not defined",
            "x = -numpy.pi\n",
        ),
        (
            "T0039",
            "tuple element type `object`",
            "t = (numpy.pi, 1)\n",
        ),
        // A slice load (`numpy.pi[0:2]`) left this list in Part 2b of
        // #1371; `foreign/slice/tests.rs` pins its admission.
        // `not numpy.pi` left this list in Part 10 of #1371;
        // `not_on_a_cpython_object_is_admitted` below pins its admission.
    ] {
        assert_refused("`numpy.pi`", snippet, code, phrase);
    }
}

/// Part 10 of #1371: `not o` is the object's own truth test, negated into
/// a native `bool`, in a condition, a value position and a `-> bool`
/// return alike.
#[test]
fn not_on_a_cpython_object_is_admitted() {
    for source in [
        "if not numpy.pi:\n    print(1)\n",
        "b = not numpy.pi\nprint(b)\n",
        "def f() -> bool:\n    return not numpy.pi\n\nprint(f())\n",
    ] {
        assert!(check_foreign(source).is_none(), "{source}");
    }
}

/// PR 3b of #1082: a discarded subscript *load* on a CPython object at
/// module scope is admitted, and so is a consumer that needs a real value
/// out of it.
///
/// This is the positive half of the migration, and the second snippet is
/// the one that matters: a bare `numpy.pi[0]` would still pass if the arm
/// merely stopped reporting `T0033`, while `len(numpy.pi[0])` only
/// type-checks if the arm actually answers `Ty::Object`.
#[test]
fn a_module_scope_subscript_of_a_cpython_object_is_admitted() {
    for source in [
        "numpy.pi[0]\n",
        "print(len(numpy.pi[0]))\n",
        "numpy.pi[1.5]\n",
        "numpy.pi[True]\n",
        "numpy.pi[\"k\"]\n",
    ] {
        assert!(check_foreign(source).is_none(), "{source}");
    }
}

/// Only an operand with a `pycc_ext_obj_pack_*` helper may be a key: the
/// four scalars and, since Part 2a of #1371, a second CPython object
/// (`foreign::is_packable_operand`). Everything else is refused here rather
/// than reaching codegen with no packer.
#[test]
fn a_subscript_key_outside_the_packable_operands_is_refused() {
    for snippet in ["numpy.pi[None]\n", "numpy.pi[[1]]\n", "numpy.pi[(1, 2)]\n"] {
        assert_refused(
            "`numpy.pi`",
            snippet,
            "I0404",
            "indexing a CPython object with a",
        );
    }
}

/// Part 2a of #1371: a second CPython object is a packable key, in both
/// producer shapes of the key.
#[test]
fn an_object_subscript_key_is_admitted() {
    for (shape, source) in both_producer_shapes("numpy.e[numpy.pi]\n") {
        assert!(check_foreign(&source).is_none(), "{shape}: {source}");
    }
}

/// C6/K1 of the #1082 plan: binding the *result* of a subscript load to a
/// name was refused by PR 3b with the `check_assignment` entry guard's own
/// `I0404`. #1325 is the name-binding work that decision deferred: at
/// module scope the binding now checks clean, like any other producer's.
#[test]
fn binding_a_subscript_load_to_a_module_level_name_is_admitted() {
    assert!(check_foreign("x = numpy.pi[0]\n").is_none());
}

/// K7 of the #1082 plan: a subscript *store* is explicitly out of scope.
///
/// `o[k] = v` is a different HIR shape, refused by `pycc_hir` with `C0001`
/// before this crate ever sees it, so admitting the load cannot admit the
/// store by accident. Pinned so a later part has to change this assertion
/// deliberately.
#[test]
fn a_subscript_store_on_a_cpython_object_is_still_refused() {
    let source = "numpy.pi[0] = 1\n";
    let module = pycc_parser::parse(source).expect("test source must parse");
    let diagnostic = pycc_hir::lower_checked(&module).expect_err("a store target is refused");
    assert_eq!(diagnostic.code, "C0001", "{diagnostic:?}");
    assert!(
        diagnostic
            .message
            .contains("only assigning to a bare-name subscript target"),
        "{diagnostic:?}"
    );
}

/// The solver-side mirror (C4 of the #1082 plan): `constraints.rs`'s own
/// `Subscript` arm lifts a `Ty::Object` base to a `Ty::Object` term, so an
/// unannotated private helper returning one materializes its signature.
///
/// Without the lift the helper's return variable stays unresolved and
/// signature materialization reports a `T0021` asking for an annotation
/// `object` cannot be spelled in (`docs/TYPE_SYSTEM.md`'s `object` row) --
/// the exact dead end the
/// `AttrGet` arm's own comment describes. Since Part 1 of #1333 admits the
/// `return`, the program type-checks.
#[test]
fn a_private_helper_returning_a_subscript_load_is_admitted() {
    let source = "def _h():\n    return numpy.pi[0]\n\n\nx = 1\n";
    assert!(check_foreign(source).is_none(), "{source}");
}

// -- PR 3c of #1082: `for` over an `object` value --------------------------

/// Both admitted iterable shapes type-check at module scope, with the loop
/// variable readable inside the body.
///
/// The body operation is `len(x)` because binding `x` to another name is
/// still refused (`check_assignment`'s K1 guard), so a `len` is the shape
/// that proves the loop variable really is bound to `Ty::Object` rather
/// than merely accepted and dropped.
#[test]
fn both_admitted_for_iterable_shapes_over_a_cpython_object_are_admitted() {
    for source in [
        "for x in numpy.pi:\n    print(len(x))\n",
        "for x in numpy.array(1):\n    print(len(x))\n",
        "for x in numpy.pi:\n    pass\n",
    ] {
        assert!(check_foreign(source).is_none(), "{source}");
    }
}

/// F8 of the #1082 plan: the iterable's *type*, not its syntactic shape,
/// is what decides. A class instance's `int` attribute and its `int`-
/// returning method call are an `Expr::Attribute` and an `Expr::Call`
/// iterable, so both lower to `HirStmt::ForObject` and are refused here
/// rather than by `pycc_hir`.
///
/// This is also where the admitted shapes cost something, and the cost is
/// wider than one spelling: **every** attribute or attribute-call iterable
/// now lowers to `ForObject` and reaches the type checker, where all of
/// them used to be `pycc_hir`'s own `C0001`. Which diagnostic each one
/// draws depends on the receiver, not on the loop -- `for x in C.value:`
/// over an `int` attribute reaches this arm and is the `I0404` asserted
/// below, `for x in xs.copy():` over a `list[int]` and `for k in
/// d.keys():` over a `dict[str, int]` are `T0043` ("cannot call a method
/// on `...`: it is not a class instance") from inferring the iterable, and
/// a receiver carrying its own pre-existing refusal reports that first, so
/// an `int`-keyed `d.keys()` is `T0036`. The `C0001` arm survives only for
/// a callee that is neither a name nor an attribute
/// (`tests/diagnostics/c0001_for_call_not_bare_name.py`).
#[test]
fn a_non_object_iterable_in_either_shape_is_refused_by_the_checker() {
    for (shape, source) in [
        (
            "a non-object attribute iterable",
            "class C:\n    v: int = 1\n\n\nc = C()\nfor x in c.v:\n    pass\n",
        ),
        (
            "a non-object method-call iterable",
            "class C:\n    def m(self) -> int:\n        return 1\n\n\nc = C()\nfor x in c.m():\n    pass\n",
        ),
    ] {
        assert_refused(
            shape,
            source,
            "I0404",
            "is only supported when the iterable is a CPython object",
        );
    }
}

/// The loop inherits PR 2a's positional bound: a function body has no
/// module-exec failure edge, so the statement is refused there whatever
/// the iterable turns out to be.
#[test]
fn a_for_loop_over_an_attribute_iterable_is_refused_inside_a_function() {
    assert_refused(
        "a function-body `for` over an attribute iterable",
        "def _h() -> int:\n    for x in numpy.pi:\n        print(len(x))\n    return 1\n\n\nprint(_h())\n",
        "I0404",
        "is not supported inside a function body",
    );
}

/// The same refusal covers an iterable that is not a foreign object at all.
///
/// `pycc_hir` routes every attribute and attribute-callee-call iterable to
/// `ForObject` on shape alone, so `d.keys()` and `xs.copy()` reach this arm
/// too, and it refuses them without ever inferring the iterable. A module
/// body refuses both as well, with their own diagnostics -- which is why
/// the message states the function-body bound and promises nothing about
/// moving the loop to module scope.
#[test]
fn a_function_body_for_over_a_non_object_iterable_gets_the_same_refusal() {
    for (shape, source) in [
        (
            "a method call on a local `dict`",
            "def _h() -> int:\n    d = {\"a\": 2}\n    for k in d.keys():\n        print(k)\n    return 1\n\n\nprint(_h())\n",
        ),
        (
            "a method call on a local `list`",
            "def _h() -> int:\n    xs = [1, 2]\n    for x in xs.copy():\n        print(x)\n    return 1\n\n\nprint(_h())\n",
        ),
    ] {
        assert_refused(
            shape,
            source,
            "I0404",
            "is not supported inside a function body",
        );
        let diagnostics = check_foreign(source).expect(shape);
        assert!(
            !diagnostics[0].message.contains("module body"),
            "{shape}: the refusal must not point at a scope that refuses it too: \
             {diagnostics:?}",
        );
    }
}

/// A target that was *declared* but never assigned is refused too.
///
/// `x: int` records the name in `declared`, not `bindings`, so the
/// `lookup_any` guard above does not see it -- yet the representation
/// conflict is identical, and `pycc_codegen` still allocates one slot per
/// name. `check_assignment` answers a declared target with `T0026`, so a
/// `for` target does the same. A value-less `Final[int]` declaration is the
/// same shape and takes the same path: `T0045` only fires once the name has
/// a runtime value, which a declaration alone does not give it.
#[test]
fn a_declared_but_unassigned_loop_target_is_refused() {
    for (shape, source) in [
        (
            "a plain declaration",
            "x: int\n\nfor x in numpy.pi:\n    pass\n",
        ),
        (
            "a value-less `Final` declaration",
            "x: Final[int]\n\nfor x in numpy.pi:\n    pass\n",
        ),
    ] {
        assert_refused(shape, source, "T0026", "previously declared as `x: int`");
    }
}

/// The loop variable is only *maybe* bound after the loop, because the
/// loop may run zero times -- and a module-body read of a `Ty::Object`
/// name is otherwise admitted, so without the downgrade the read would
/// compile against a slot the loop never wrote.
#[test]
fn the_loop_variable_is_maybe_bound_after_the_loop() {
    assert_refused(
        "a read of the loop variable after the loop",
        "for x in numpy.pi:\n    pass\n\nprint(len(x))\n",
        "T0041",
        "may not be bound on every path",
    );
}

/// The other side of that branch -- a loop variable already bound before the
/// loop -- is not a definite-assignment question at all but a
/// representation one, and it is refused.
///
/// This test previously asserted the opposite. `ForObject` binds its target
/// with `env.bind`, which overwrites, so `x = 1` followed by
/// `for x in numpy.pi:` left `x` as `int` for the reads above the loop and
/// `object` for those below it -- and `pycc_codegen` allocates exactly one
/// storage slot per name per function, so one of those two access sets is
/// always wrong. D-040's sticky-representation rule is what the refusal
/// restores, and reusing `T0023` keeps this ordering and its mirror
/// (`for x in numpy.pi:` first, then `x = 1`, which `check_assignment`
/// already refused) reporting the same thing.
#[test]
fn a_pre_bound_loop_variable_of_another_type_is_refused() {
    for (shape, source) in [
        (
            "a definite `int` binding",
            "x = 1\n\nfor x in numpy.pi:\n    pass\n",
        ),
        (
            "a definite `float` binding",
            "x = 1.5\n\nfor x in numpy.pi:\n    pass\n",
        ),
        (
            "a binding made on only one branch",
            "b = True\nif b:\n    x = 1\n\nfor x in numpy.pi:\n    pass\n",
        ),
    ] {
        assert_refused(shape, source, "T0023", "cannot assign `object` to `x`");
    }
}

/// K6/K10/K11 of the #1082 plan, pinned: the iterable shapes PR 3c
/// deliberately leaves out keep the exact refusals they already had, so
/// admitting the attribute and method-call shapes widened none of them.
///
/// The two `C0001`s are `pycc_hir`'s, word for word -- the new routes were
/// added as guards *ahead* of those `let ... else` bindings rather than by
/// restructuring them, so a regression that moved a diagnostic into the
/// type checker fails here.
#[test]
fn the_deferred_for_iterables_keep_their_refusals_and_a_bare_module_is_admitted() {
    for (source, phrase) in [
        (
            "for x in numpy.pi[0]:\n    pass\n",
            "got a subscript expression (`obj[key]`) as the iterable",
        ),
        (
            "for x in (1, 2):\n    pass\n",
            "got a tuple as the iterable",
        ),
    ] {
        let module = pycc_parser::parse(source).expect("test source must parse");
        let diagnostic = pycc_hir::lower_checked(&module).expect_err("the shape is refused");
        assert_eq!(diagnostic.code, "C0001", "{source}: {diagnostic:?}");
        assert!(
            diagnostic.message.contains(phrase),
            "{source}: {diagnostic:?}"
        );
    }
    // A bare foreign name is an `Expr::Name` iterable, so it lowers to
    // `HirStmt::ForList`. Since #1325 that arm routes a definitely-bound
    // `object` name to the object loop, keyed on the type alone: a bare
    // module is admitted, and CPython's own `TypeError` answers it at run
    // time.
    assert!(check_foreign("for x in numpy:\n    pass\n").is_none());
}

/// The in-function constraint solver walks a `ForObject` before the
/// positional refusal above is reported (both diagnostics are collected
/// and merged), so both of its own walks have to propagate a failure.
///
/// An unbound local is the smallest expression the solver refuses, and
/// placing it once in the iterable and once in the body separates the two
/// walks: a regression that stopped walking either position would report
/// this loop's `I0404` instead of the `T0021`, because the solver would
/// no longer see the unbound read at all.
#[test]
fn the_in_function_solver_propagates_a_failure_from_either_position() {
    for (position, source) in [
        (
            "the iterable",
            "def _h() -> int:\n    for x in numpy.wrap(later):\n        return 1\n    later = 2\n    return 0\n\n\nprint(_h())\n",
        ),
        (
            "the body",
            "def _h() -> int:\n    for x in numpy.pi:\n        y = later\n    later = 2\n    return 0\n\n\nprint(_h())\n",
        ),
    ] {
        assert_refused(position, source, "T0021", "is not bound before this use");
    }
}

/// The other side of `the_monomorphization_pass_walks_a_for_loop_iterable`:
/// when the iterable's own names *do* resolve in `monomorphize`'s
/// environment, the rewrite succeeds and the pass keeps walking.
///
/// `check_all` refuses this module (`c.v` is an `int`, not a CPython
/// object, so the loop is an `I0404`), which is why the pass is driven
/// directly rather than through `check_and_resolve_all_keyed`. That is
/// the point: it pins the arm's success path against the day the foreign
/// seeding gap closes, and the non-empty body pins the pre-scan's own
/// descent into the body statements.
#[test]
fn the_monomorphization_pass_rewrites_a_for_iterable_whose_names_resolve() {
    let source = "class Box[T]:\n    def __init__(self, v: T) -> None:\n        self.v = v\n\n\nclass C:\n    v: int = 1\n\n\nb = Box[int](1)\nc = C()\nfor x in c.v:\n    print(1)\n";
    let hir = lower(source);
    crate::monomorphize::monomorphize(&hir).expect("the rewrite sweep walks the loop");
}

/// The generic-call *rejection* walk over a generic function's own body
/// has to descend into the iterable, not just the body: unlike `ForList`'s
/// bare name, a `ForObject`'s iterable is a real expression and can hold a
/// call. Without that descent the `T0042` below is never reported and the
/// recursive generic instantiation reaches monomorphization.
#[test]
fn a_generic_call_hidden_in_the_iterable_is_still_rejected() {
    assert_refused(
        "a generic call inside a `for` iterable",
        "def _gen[T](x: T) -> T:\n    for y in numpy.wrap(_gen(1)):\n        pass\n    return x\n\n\nprint(_gen(1))\n",
        "T0042",
        "calls itself",
    );
}

/// `monomorphize` walks every top-level statement, and a `ForObject`'s
/// iterable -- unlike a `ForList`'s bare name -- is a real expression the
/// pass has to descend into. This fixture reaches both of its walks over
/// that position: `instantiate_generic_class_methods`'s pre-scan for
/// `(class, type argument)` pairs runs first, then the rewrite sweep.
///
/// The rewrite sweep always fails on this statement, and that failure is
/// the assertion. `monomorphize` seeds no foreign name into its own
/// environment -- a pre-existing gap that refuses `print(len(numpy.pi))`
/// (PR 3a) and `print(len(numpy.pi[0]))` (PR 3b) in a module that also
/// has a generic function or class, exactly as it refuses this loop. PR
/// 3c neither introduced nor widens it. The `T0021` can only be reported
/// by the walk of the iterable itself, so it is what proves the descent
/// happens at all; the loop body is unreachable behind it, which is why
/// the rewrite arm is the walk and nothing else.
#[test]
fn the_monomorphization_pass_walks_a_for_loop_iterable() {
    let source = "class Box[T]:\n    def __init__(self, v: T) -> None:\n        self.v = v\n\n\nb = Box[int](1)\nfor x in numpy.pi:\n    pass\n";
    let hir = with_foreign_import(lower(source));
    // The check phase admits the loop -- `numpy.pi` is a `Ty::Object`
    // there -- so the failure below really is monomorphization's.
    assert!(crate::check_all(&hir).is_ok(), "the check phase admits it");
    let Err(diagnostics) = crate::check_and_resolve_all_keyed(&hir) else {
        panic!("monomorphization refuses the iterable");
    };
    let [(_, diagnostic)] = diagnostics.as_slice() else {
        panic!("exactly one diagnostic: {diagnostics:?}");
    };
    assert_eq!(diagnostic.code, "T0021", "{diagnostic:?}");
    assert!(
        diagnostic.message.contains("`numpy` is not defined"),
        "{diagnostic:?}"
    );
}

/// An enum `for` loop nested inside a `for x in <object>:` body is unrolled
/// like one nested inside any other loop.
///
/// `unroll_enum_loops_in_stmts` recurses into every body-carrying statement
/// kind by hand, and `HirStmt::ForObject` is new in PR 3c of #1082, so
/// before this arm existed the catch-all cloned the loop whole and left the
/// inner `for c in Color:` unexpanded. `pycc_types` accepted that module and
/// MIR lowering then panicked on the enum class name having no recorded
/// type -- the `pycc check` / `pycc build` divergence D-245 exists to stop.
#[test]
fn an_enum_loop_nested_inside_a_for_object_body_is_unrolled() {
    let source = "class Color(Enum):\n    RED = 1\n    GREEN = 2\nfor x in numpy.pi:\n    for c in Color:\n        print(c.value)\n";
    let resolved = crate::check_and_resolve_all_keyed(&with_foreign_import(lower(source)))
        .expect("the fixture type-checks");
    let body = resolved
        .items
        .iter()
        .find_map(|item| match item {
            pycc_hir::HirItem::TopLevelStmt(pycc_hir::HirStmt::ForObject { body, .. }) => {
                Some(body)
            }
            _ => None,
        })
        .expect("the module has exactly one `for` over an object");
    assert!(
        !body.iter().any(|stmt| matches!(
            stmt,
            pycc_hir::HirStmt::ForList { list, .. } if list == "Color"
        )),
        "the nested enum loop survived unrolling: {body:?}"
    );
    // Two members, each contributing one `c = Color.<M>` assignment plus the
    // one-statement body, so the unrolled sequence is exactly four
    // statements. Pinning the count keeps the test from passing on an arm
    // that merely dropped the loop.
    assert_eq!(body.len(), 4, "{body:?}");
}

/// The converse of the test above: a target the loop introduces itself, and
/// one already bound to `Ty::Object` by an earlier loop, are both admitted.
/// Without this the refusal could be satisfied by rejecting every
/// `ForObject`.
#[test]
fn a_for_object_target_may_rebind_a_name_already_bound_to_an_object() {
    assert!(
        check_foreign("for x in numpy.pi:\n    pass\n\nfor x in numpy.pi:\n    pass\n").is_none()
    );
}

// ---------------------------------------------------------------------------
// A foreign import nested in a module-level `if`/`try` block (#1291).
// ---------------------------------------------------------------------------

/// Lowers `source` with every plain `import` request answered `Foreign`,
/// the way the driver answers an undotted non-`pycc_std` root.
fn lower_all_foreign(source: &str) -> pycc_hir::HirModule {
    let module = pycc_parser::parse(source).expect("test source must parse");
    let mut resolved = pycc_hir::ResolvedImports::default();
    for request in pycc_hir::project_import_requests(&module) {
        resolved.insert(request.span, pycc_hir::ResolvedImport::Foreign);
    }
    pycc_hir::lower_module(&module, &resolved, None)
        .unwrap_or_else(|diagnostics| panic!("{source:?} must lower: {diagnostics:#?}"))
        .hir
}

fn check_block(source: &str) -> Result<(), Vec<pycc_diag::Diagnostic>> {
    crate::check_all(&lower_all_foreign(source)).map(|_| ())
}

#[test]
fn a_read_after_an_if_else_that_imports_in_both_arms_is_admitted() {
    let source = "if c:\n    import colorsys\nelse:\n    import colorsys\ncolorsys.ONE_THIRD\n";
    let source = format!("c = True\n{source}");
    check_block(&source).unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
}

#[test]
fn a_read_inside_the_importing_arm_is_admitted() {
    for source in [
        "c = True\nif c:\n    import colorsys\n    colorsys.ONE_THIRD\n",
        "try:\n    import colorsys\n    colorsys.ONE_THIRD\nexcept Exception:\n    pass\n",
    ] {
        check_block(source).unwrap_or_else(|diagnostics| panic!("{source:?}: {diagnostics:#?}"));
    }
}

/// A read after a block that may not have run the import is a
/// possibly-unbound read. The `try` case is #1289's pre-existing
/// behaviour (a `try` join does not yet treat the handler as the only
/// other path), pinned so #1289 flips it deliberately.
#[test]
fn a_read_after_a_block_that_may_skip_the_import_is_t0041() {
    for source in [
        "c = True\nif c:\n    import colorsys\ncolorsys.ONE_THIRD\n",
        "try:\n    import colorsys\nexcept Exception:\n    pass\ncolorsys.ONE_THIRD\n",
    ] {
        let diagnostics = check_block(source).expect_err("the read may be unbound");
        assert_eq!(diagnostics.len(), 1, "{source:?}: {diagnostics:#?}");
        assert_eq!(diagnostics[0].code, "T0041", "{source:?}: {diagnostics:#?}");
    }
}

/// A read before the block is an unbound name: the nested import binds
/// only where it runs.
#[test]
fn a_read_before_the_block_is_unbound() {
    let diagnostics = check_block("colorsys.ONE_THIRD\nc = True\nif c:\n    import colorsys\n")
        .expect_err("a read above the import is refused");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].code, "T0021", "{diagnostics:#?}");
}

#[test]
fn a_block_binding_is_not_bound_by_item_position() {
    let mut env = crate::Environment::new();
    let imports = vec![ImportBinding::Foreign {
        local_name: "colorsys".to_string(),
        module_path: "colorsys".to_string(),
        from: None,
        site: pycc_hir::ForeignImportSite::Block { optional: false },
        span: Span::new(0, 0),
    }];
    bind_foreign_objects_at(&mut env, &imports, 0);
    assert!(env.lookup("colorsys").is_none());
}

/// `pycc_hir` never lowers a `ForeignImport` inside a function body, but
/// the function-body statement checker and its local-name prescan handle
/// the node the same way the module-level checker does, so a hand-built
/// one binds its name as a function local.
#[test]
fn a_hand_built_function_body_foreign_import_binds_a_local() {
    let mut hir = lower("def f() -> None:\n    pass\n");
    let Some(pycc_hir::HirItem::Function { body, .. }) = hir.items.first_mut() else {
        panic!("the fixture is one function");
    };
    *body = vec![
        pycc_hir::HirStmt::ForeignImport {
            bindings: vec![("colorsys".to_string(), "colorsys".to_string(), None)],
            span: Span::new(0, 0),
        },
        pycc_hir::HirStmt::ForeignImport {
            bindings: vec![("colorsys".to_string(), "colorsys".to_string(), None)],
            span: Span::new(0, 0),
        },
    ];
    crate::check_all(&hir).unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
}

/// Both item-count-changing passes -- `monomorphize` dropping generic
/// originals and `unroll_enum_loops` expanding a top-level `for c in
/// Color:` -- remap every `ForeignImportSite::Item` position, and must
/// carry a block site through unchanged: a nested import runs where its
/// `HirStmt::ForeignImport` stands, so it has no item position to remap.
#[test]
fn a_block_site_survives_both_item_remapping_passes_unchanged() {
    for (source, items) in [
        (
            "def _a[T](x: T) -> T:\n    return x\n\n\ndef f() -> int:\n    return 1\n",
            1,
        ),
        (
            "class Color(Enum):\n    RED = 1\n    GREEN = 2\nfor c in Color:\n    print(c.value)\n",
            4,
        ),
    ] {
        let mut hir = lower(source);
        hir.imports.push(ImportBinding::Foreign {
            local_name: "colorsys".to_string(),
            module_path: "colorsys".to_string(),
            from: None,
            site: pycc_hir::ForeignImportSite::Block { optional: true },
            span: Span::new(0, 0),
        });
        let resolved = crate::check_and_resolve_all_keyed(&hir)
            .unwrap_or_else(|diagnostics| panic!("{source:?}: {diagnostics:#?}"));
        assert_eq!(resolved.items.len(), items, "{:?}", resolved.items);
        assert!(
            matches!(
                resolved.imports.as_slice(),
                [ImportBinding::Foreign {
                    site: pycc_hir::ForeignImportSite::Block { optional: true },
                    ..
                }]
            ),
            "{source:?}: {:?}",
            resolved.imports
        );
    }
}
