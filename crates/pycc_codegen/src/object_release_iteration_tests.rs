//! The release of a `for` loop's and a comprehension's temporaries: the
//! produced iterable, filter and element (Part 1 of #1092) and the
//! iterator and comprehension result (Part 3 of #1092), on the normal exit
//! and on every failure edge.

use super::*;

/// The iterator `iter()` returns, as the IR names it (Part 3 of #1092).
const ITERATOR: &str = "foreign_iter_get";

/// A comprehension's result collection, as the IR names it.
const RESULT: &str = "objcomp_result";

/// The releases of exactly the value the IR names `%{name}`.
fn releases_of(text: &str, name: &str) -> usize {
    text.matches(&format!("{RELEASE}ptr %{name})")).count()
}

/// A comprehension over a CPython object holds a produced filter across its
/// truth test and releases a produced element once the packer has taken its
/// own reference (`[x.v for x in copy.a if x.ok]`). Since Part 3 of #1092
/// the iterator and the result are held for the comprehension's extent:
/// every failure edge after `iter()` releases the iterator, every one
/// after the result exists releases the result too, the normal exit
/// releases the iterator, and the discarded result is released by its
/// statement. Each per-trip item stays unreleased (#1499).
#[test]
fn a_comprehension_releases_its_produced_filter_and_element() {
    let var = || MirExpr::Name {
        name: "x".to_string(),
        ty: Ty::Object,
    };
    let comprehension = MirExpr::Comprehension(Box::new(pycc_mir::MirComprehension {
        var: "x".to_string(),
        var_ty: Ty::Object,
        source: pycc_mir::CompSource::Object(attr("a")),
        cond: Some(attr_of(var(), "ok")),
        elt: pycc_mir::MirCompElt::List(attr_of(var(), "v")),
    }));
    let ir = discard_ir("release_comprehension", comprehension);
    let get_iter_fail = blocks(&ir, "foreign_iter_get_fail");
    assert_eq!(get_iter_fail.len(), 1, "{ir}");
    assert_eq!(releases(get_iter_fail[0]), 1, "{ir}");
    assert_eq!(releases_of(get_iter_fail[0], ITERATOR), 0, "{ir}");
    let result_fail = blocks(&ir, "objcomp_result_fail");
    assert_eq!(result_fail.len(), 1, "{ir}");
    assert_eq!(releases(result_fail[0]), 1, "{ir}");
    assert_eq!(releases_of(result_fail[0], ITERATOR), 1, "{ir}");
    // The source's own attribute load precedes `iter()` and holds nothing.
    let mut later = blocks(&ir, "foreign_attr_fail");
    assert_eq!(later.len(), 3, "{ir}");
    assert_eq!(releases(later.remove(0)), 0, "{ir}");
    // The filter's and the element's attribute loads and their effect
    // checks, the filter's truth test, `next()` and the insertion: each
    // releases both.
    let unwinds = blocks(&ir, "effect_exc_unwind");
    assert_eq!(unwinds.len(), 2, "{ir}");
    later.extend(unwinds);
    for label in [
        "foreign_iter_next_fail",
        "foreign_truthy_fail",
        "objcomp_collect_fail",
    ] {
        let found = blocks(&ir, label);
        assert_eq!(found.len(), 1, "{label}\n{ir}");
        later.extend(found);
    }
    for fail in &later {
        assert_eq!(releases_of(fail, ITERATOR), 1, "{fail}\n{ir}");
        assert_eq!(releases_of(fail, RESULT), 1, "{fail}\n{ir}");
    }
    // The truth test's failure releases the filter as well.
    assert_eq!(releases(blocks(&ir, "foreign_truthy_fail")[0]), 3, "{ir}");
    let packed = ir
        .find("@pycc_ext_obj_pack_object(")
        .expect("the element is packed");
    let collected = ir
        .find("@pycc_ext_obj_collect(")
        .expect("the element is collected");
    assert_eq!(releases(&ir[packed..collected]), 1, "{ir}");
    let after = blocks(&ir, "foreign_iter_after");
    assert_eq!(after.len(), 1, "{ir}");
    assert_eq!(releases(after[0]), 1, "{ir}");
    assert_eq!(releases_of(after[0], ITERATOR), 1, "{ir}");
    // The comprehension hands its result to the statement, which releases
    // it once more on the normal path: nothing else is released.
    assert_eq!(releases_of(&ir, ITERATOR), 9, "{ir}");
    assert_eq!(releases_of(&ir, RESULT), 8, "{ir}");
}

/// A comprehension's result bound to a name is the binding's, not a
/// temporary: only the iterator is released at the exit (#1499 owns the
/// binding).
#[test]
fn a_bound_comprehension_result_is_not_released() {
    let comprehension = MirExpr::Comprehension(Box::new(pycc_mir::MirComprehension {
        var: "x".to_string(),
        var_ty: Ty::Object,
        source: pycc_mir::CompSource::Object(copy_name()),
        cond: None,
        elt: pycc_mir::MirCompElt::List(MirExpr::Name {
            name: "x".to_string(),
            ty: Ty::Object,
        }),
    }));
    let ir = f_ir(
        "release_comprehension_bound",
        vec![MirStmt::Assign {
            target: "c".to_string(),
            value: comprehension,
        }],
    );
    let after = blocks(&ir, "foreign_iter_after");
    assert_eq!(after.len(), 1, "{ir}");
    assert_eq!(releases(after[0]), 1, "{ir}");
    assert_eq!(releases_of(after[0], ITERATOR), 1, "{ir}");
}

/// `for x in copy.a:` releases its produced iterable once `iter()` has
/// returned -- on `iter()`'s failure edge and right after it, before the
/// loop header. Its iterator (Part 3 of #1092) is released by the loop's
/// cleanup target, on `next()`'s direct module-exec return, which looks
/// through that target, and at the loop's normal exit. Since Part 1 of
/// #1499 the only other releases are on each module global's rebind branch
/// (`object_slot.rs`): the fixture's `copy` import, and the loop target's
/// previous item on every trip.
#[test]
fn a_for_loop_releases_its_produced_iterable_and_its_iterator() {
    let ir = entry_ir(
        "release_for_iterable",
        vec![MirStmt::ForObject {
            var: "x".to_string(),
            iter: attr("a"),
            body: Vec::new(),
        }],
    );
    let rebinds = blocks(&ir, "global_release_old");
    assert_eq!(rebinds.len(), 2, "{ir}");
    assert_eq!(releases_of(rebinds[1], "global_old1"), 1, "{ir}");
    assert_eq!(releases(&ir), 5 + rebinds.len(), "{ir}");
    let get_iter_fail = blocks(&ir, "foreign_iter_get_fail");
    assert_eq!(get_iter_fail.len(), 1, "{ir}");
    assert_eq!(releases(get_iter_fail[0]), 1, "{ir}");
    let got = ir
        .find("@pycc_ext_obj_get_iter(")
        .expect("the iterable is iterated");
    let header = ir
        .find("@pycc_ext_obj_iter_next(")
        .expect("the loop advances");
    // `iter()`'s failure edge, its success edge, and the loop's cleanup
    // block, which is laid out ahead of the header.
    assert_eq!(releases(&ir[got..header]), 3, "{ir}");
    let cleanup = blocks(&ir, "foreign_iter_cleanup");
    assert_eq!(cleanup.len(), 1, "{ir}");
    assert_eq!(releases_of(cleanup[0], ITERATOR), 1, "{ir}");
    assert!(
        cleanup[0]
            .trim_end()
            .ends_with("br label %top_exception_exit"),
        "{ir}"
    );
    let next_fail = blocks(&ir, "foreign_iter_next_fail");
    assert_eq!(next_fail.len(), 1, "{ir}");
    assert_eq!(releases_of(next_fail[0], ITERATOR), 1, "{ir}");
    assert!(next_fail[0].trim_end().ends_with("ret i64 -1"), "{ir}");
    let after = blocks(&ir, "foreign_iter_after");
    assert_eq!(after.len(), 1, "{ir}");
    assert_eq!(releases_of(after[0], ITERATOR), 1, "{ir}");
}

/// Inside a module-level `try`, a body failure in a foreign `for` unwinds
/// through the loop's cleanup block -- which releases the iterator -- to
/// the handler dispatch, and a nested loop's cleanup chains to the outer
/// loop's. A failure inside the body's own `try` stays inside the loop and
/// releases no iterator.
#[test]
fn a_for_loop_body_failure_unwinds_through_its_cleanup_blocks() {
    let discard_attr = |name: &str| MirStmt::ExprStmt(attr(name));
    let inner_try = MirStmt::Try {
        body: vec![discard_attr("b")],
        handlers: vec![MirExceptHandler {
            exc_type_tag: None,
            binding_name: None,
            binding_ty: None,
            body: Vec::new(),
        }],
        orelse: Vec::new(),
        finalbody: Vec::new(),
    };
    let ir = entry_ir(
        "release_for_cleanup_chain",
        vec![MirStmt::Try {
            body: vec![MirStmt::ForObject {
                var: "x".to_string(),
                iter: copy_name(),
                body: vec![
                    MirStmt::ForObject {
                        var: "y".to_string(),
                        iter: copy_name(),
                        body: vec![discard_attr("c")],
                    },
                    inner_try,
                ],
            }],
            handlers: vec![MirExceptHandler {
                exc_type_tag: None,
                binding_name: None,
                binding_ty: None,
                body: Vec::new(),
            }],
            orelse: Vec::new(),
            finalbody: Vec::new(),
        }],
    );
    // The label a block's final unconditional branch targets.
    let target = |block: &str| -> String {
        let (_, label) = block
            .trim_end()
            .rsplit_once("br label %")
            .expect("the block ends in a branch");
        label.to_string()
    };
    let cleanups = blocks(&ir, "foreign_iter_cleanup");
    assert_eq!(cleanups.len(), 2, "{ir}");
    let (outer, inner) = (cleanups[0], cleanups[1]);
    assert_eq!(releases(outer), 1, "{ir}");
    assert_eq!(releases_of(outer, ITERATOR), 1, "{ir}");
    assert_eq!(releases(inner), 1, "{ir}");
    assert_eq!(releases_of(inner, ITERATOR), 0, "{ir}");
    assert_eq!(target(outer), "try_handler_dispatch", "{ir}");
    assert_eq!(target(inner), "foreign_iter_cleanup", "{ir}");
    let attr_fails = blocks(&ir, "foreign_attr_fail");
    assert_eq!(attr_fails.len(), 2, "{ir}");
    let inner_label = target(attr_fails[0]);
    assert!(
        inner_label.starts_with("foreign_iter_cleanup") && inner_label != "foreign_iter_cleanup",
        "the inner loop's body unwinds through the inner cleanup\n{ir}"
    );
    assert_eq!(releases(attr_fails[1]), 0, "{ir}");
    assert!(
        target(attr_fails[1]).starts_with("try_handler_dispatch"),
        "the body's own `try` catches before any cleanup\n{ir}"
    );
}

/// A comprehension evaluated while an outer operand is held -- here as the
/// argument of a method call, whose bound method is held across its
/// arguments -- releases that outer operand on every one of its own
/// failure edges, beside its own iterator and result:
/// `copy.m([x for x in copy.a])`.
#[test]
fn a_comprehension_failure_releases_an_outer_held_operand() {
    let comprehension = MirExpr::Comprehension(Box::new(pycc_mir::MirComprehension {
        var: "x".to_string(),
        var_ty: Ty::Object,
        source: pycc_mir::CompSource::Object(attr("a")),
        cond: None,
        elt: pycc_mir::MirCompElt::List(MirExpr::Name {
            name: "x".to_string(),
            ty: Ty::Object,
        }),
    }));
    let ir = discard_ir(
        "release_comprehension_outer",
        MirExpr::ObjMethodCall {
            base: boxed(copy_name()),
            method: "m".to_string(),
            args: vec![comprehension],
        },
    );
    for (label, held) in [
        ("objcomp_result_fail", 2),
        ("foreign_iter_next_fail", 3),
        ("objcomp_collect_fail", 3),
    ] {
        let found = blocks(&ir, label);
        assert_eq!(found.len(), 1, "{label}\n{ir}");
        assert_eq!(
            releases(found[0]),
            held,
            "{label} releases the bound method and what the comprehension holds\n{ir}"
        );
    }
    // `iter()`'s failure releases the produced source and the bound method.
    let get_iter_fail = blocks(&ir, "foreign_iter_get_fail");
    assert_eq!(get_iter_fail.len(), 1, "{ir}");
    assert_eq!(releases(get_iter_fail[0]), 2, "{ir}");
}

/// A native comprehension (`[i for i in range(2) if copy.a]`) holds a
/// produced CPython-object filter across its truth test and releases it on
/// that test's failure edge and on its fallthrough, once per iteration.
#[test]
fn a_native_comprehension_releases_its_produced_object_filter() {
    let int = |value: i64| MirExpr::IntLiteral(value);
    let comprehension = MirExpr::Comprehension(Box::new(pycc_mir::MirComprehension {
        var: "i".to_string(),
        var_ty: Ty::Int,
        source: pycc_mir::CompSource::Range {
            start: int(0),
            stop: int(2),
            step: int(1),
        },
        cond: Some(attr("a")),
        elt: pycc_mir::MirCompElt::List(MirExpr::Name {
            name: "i".to_string(),
            ty: Ty::Int,
        }),
    }));
    let ir = discard_ir("release_native_comprehension_filter", comprehension);
    assert_eq!(releases(&ir), 2, "{ir}");
    let truthy_fail = blocks(&ir, "foreign_truthy_fail");
    assert_eq!(truthy_fail.len(), 1, "{ir}");
    assert_eq!(releases(truthy_fail[0]), 1, "{ir}");
}
