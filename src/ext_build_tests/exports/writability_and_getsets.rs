//! Buffer-parameter writability flags and generated getsets.

use super::*;

/// Part 1 of #1142: `collect_exports` records which buffer parameters the
/// body stores into, and only those.
///
/// The three-parameter shape is the point. The flag is per parameter, so a
/// buffer the body only reads stays read-only -- acquiring it writable
/// would refuse the read-only exporters D-244's boundary has always
/// accepted -- and the `int` beside them is inert, since `PyBUF_WRITABLE`
/// has no meaning for a slot that acquires no buffer.
#[test]
fn an_exports_buffer_parameter_is_writable_exactly_when_its_body_stores_into_it() {
    let hir = module(vec![func_with_body(
        "mix",
        &[
            ("read", Ty::MemoryView),
            ("written", Ty::MemoryView),
            ("n", Ty::Int),
        ],
        Ty::None,
        vec![pycc_hir::HirStmt::While {
            test: pycc_hir::HirExpr::BoolLiteral(true),
            body: vec![element_store("written")],
        }],
    )]);
    let exports = collect_exports(&hir).expect("a carriable signature");
    assert_eq!(
        exports
            .iter()
            .map(|e| &e.param_writable)
            .collect::<Vec<_>>(),
        vec![&vec![false, true, false]]
    );
}

/// The flags are indexed against the *post-split* parameter list, so a
/// method whose receiver was dropped does not shift them by one.
///
/// An off-by-one here would acquire the wrong slot writable: `self` never
/// crosses the boundary, so a flag list still counting it would mark the
/// buffer after the written one, or run past the end.
#[test]
fn an_instance_methods_writability_flags_skip_the_dropped_receiver() {
    let class = "Grid";
    let mut hir = module_with_classes(
        vec![
            init_func(class, &[("w", Ty::Int)], Ty::None),
            func_with_body(
                &format!("{class}.fill"),
                &[
                    ("self", Ty::Instance(Box::new(class.to_string()))),
                    ("b", Ty::MemoryView),
                ],
                Ty::None,
                vec![element_store("b")],
            ),
        ],
        vec![(class.to_string(), constructible_class_def(class))],
    );
    declare_member(&mut hir, class, Member::Method("fill"));
    let exports = collect_exports(&hir).expect("a carriable signature");
    let fill = exports
        .iter()
        .find(|e| e.name == "Grid.fill")
        .expect("the method is exported");
    assert_eq!(fill.params, vec![Ty::MemoryView]);
    assert_eq!(fill.param_writable, vec![true]);
}

/// The same walk on a constructor, which `collect_exports` never sees:
/// `__init__` is refused as an export outright, so `ctor_descriptor` has to
/// look its body up for itself. Left unplumbed, `Py_tp_init` would acquire
/// read-only storage this body writes through.
#[test]
fn a_constructors_buffer_parameter_carries_the_same_writability_flag() {
    let class = "Grid";
    let mut hir = module_with_classes(
        vec![
            func_with_body(
                &format!("{class}.__init__"),
                &[
                    ("self", Ty::Instance(Box::new(class.to_string()))),
                    ("b", Ty::MemoryView),
                    ("read", Ty::MemoryView),
                ],
                Ty::None,
                vec![element_store("b")],
            ),
            func(
                &format!("{class}.area"),
                &[("self", Ty::Instance(Box::new(class.to_string())))],
                Ty::Int,
            ),
        ],
        vec![(class.to_string(), constructible_class_def(class))],
    );
    declare_member(&mut hir, class, Member::Method("area"));
    let exports = collect_exports(&hir).expect("a carriable signature");
    let ctors = collect_constructors(&hir, &collect_class_publications(&hir, &exports));
    assert_eq!(
        ctors.iter().map(|c| &c.param_writable).collect::<Vec<_>>(),
        vec![&vec![true, false]]
    );
}
