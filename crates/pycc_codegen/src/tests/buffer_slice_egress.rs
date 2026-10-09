//! Part 2 of #1175 (#1179): returning a sub-range of a caller-owned buffer
//! parameter.
//!
//! Its own cohesion-driven submodule rather than more lines in the already
//! far-over-threshold `tests.rs`, under AGENTS.md's decomposability rule and
//! the layout `d029_guard` beside it established.
//!
//! What this part adds to codegen is an *arity* change: a compiled function
//! whose body can return `b[start:stop]` takes three trailing `long long *`
//! out-pointers -- `has_slice`, `start`, `stop` -- because the bounds are
//! produced inside the callee's frame and the compiled body still returns the
//! whole view's `PyccExtBufferView *` so the wrapper's pointer-identity test
//! stays exact. Two independent walks decide whether those out-slots exist,
//! one over HIR in the driver and one over MIR here, and a wrapper that
//! disagrees with the compiled signature is an undiagnosable ABI mismatch
//! rather than a diagnostic. Everything below pins one half of that.

use super::*;

/// `return b[1:]` as a MIR statement.
fn return_slice_of_b() -> MirStmt {
    MirStmt::ReturnBufferSlice {
        name: "b".to_string(),
        start: Some(MirExpr::IntLiteral(1)),
        stop: None,
    }
}

/// An `except:` handler whose body is `body`.
fn handler(body: Vec<MirStmt>) -> MirExceptHandler {
    MirExceptHandler {
        exc_type_tag: None,
        binding_name: None,
        binding_ty: None,
        body,
    }
}

/// The out-slot list is a property of the *body*, not of the declared return
/// type: all three buffer-return provenances declare `-> memoryview`, and
/// only this one carries bounds.
#[test]
fn only_a_buffer_slice_body_adds_the_three_out_slots() {
    assert!(ext_thunk_out_tys(&Ty::MemoryView, false).is_empty());
    assert_eq!(
        ext_thunk_out_tys(&Ty::MemoryView, true),
        vec![Ty::Int, Ty::Int, Ty::Int]
    );
    assert!(ext_thunk_out_tys(&Ty::Int, false).is_empty());
    assert_eq!(
        ext_thunk_out_tys(&Ty::Int, true),
        vec![Ty::Int, Ty::Int, Ty::Int]
    );
}

/// A tuple return keeps its own out-slots and is never widened by the buffer
/// fact. The two cannot co-occur -- a `-> tuple` signature cannot also return
/// a buffer -- and the tuple arm deliberately wins so an impossible
/// combination cannot silently produce a third arity.
#[test]
fn a_tuple_return_keeps_exactly_its_own_out_slots() {
    let tuple = Ty::Tuple(Box::new(vec![Ty::Int, Ty::Float]));
    assert_eq!(
        ext_thunk_out_tys(&tuple, false),
        vec![Ty::Int, Ty::Float],
        "the tuple's own elements"
    );
    assert_eq!(ext_thunk_out_tys(&tuple, true), vec![Ty::Int, Ty::Float]);
}

/// The buffer fact is a third reason a thunk exists, beside a tuple
/// parameter and a tuple return -- and it does not make a thunk appear for a
/// name that is not exported at all.
#[test]
fn the_buffer_slice_fact_is_a_third_reason_a_thunk_exists() {
    assert!(!ext_thunk_required(
        "f",
        &[Ty::MemoryView],
        &Ty::MemoryView,
        false
    ));
    assert!(ext_thunk_required(
        "f",
        &[Ty::MemoryView],
        &Ty::MemoryView,
        true
    ));
    assert!(
        !ext_thunk_required("_f", &[Ty::MemoryView], &Ty::MemoryView, true),
        "a private name is not exported, so it has no thunk to widen"
    );
}

/// The MIR walk's positive arms, one nested position per block-carrying
/// variant. A variant this walk fails to recurse into declares a narrower
/// signature than the `ReturnBufferSlice` site then writes through, so the
/// three stores would land past the end of the parameter list.
#[test]
fn a_buffer_slice_return_is_found_inside_every_nested_block() {
    let nested: Vec<Vec<MirStmt>> = vec![
        vec![return_slice_of_b()],
        vec![MirStmt::If {
            test: MirExpr::BoolLiteral(true),
            body: vec![return_slice_of_b()],
            orelse: vec![],
        }],
        vec![MirStmt::If {
            test: MirExpr::BoolLiteral(true),
            body: vec![],
            orelse: vec![return_slice_of_b()],
        }],
        vec![MirStmt::While {
            test: MirExpr::BoolLiteral(true),
            body: vec![return_slice_of_b()],
        }],
        vec![MirStmt::ForRange {
            var: "i".to_string(),
            start: MirExpr::IntLiteral(0),
            stop: MirExpr::IntLiteral(1),
            step: MirExpr::IntLiteral(1),
            body: vec![return_slice_of_b()],
        }],
        vec![MirStmt::ForList {
            var: "i".to_string(),
            list: "xs".to_string(),
            body: vec![return_slice_of_b()],
        }],
        vec![MirStmt::ForObject {
            var: "i".to_string(),
            iter: MirExpr::IntLiteral(0),
            body: vec![return_slice_of_b()],
        }],
        vec![MirStmt::ForDict {
            var: "k".to_string(),
            dict: "d".to_string(),
            body: vec![return_slice_of_b()],
        }],
        vec![MirStmt::ForSet {
            var_ty: pycc_mir::Ty::Int,
            var: "k".to_string(),
            set: "s".to_string(),
            body: vec![return_slice_of_b()],
        }],
        vec![MirStmt::Seq(vec![return_slice_of_b()])],
        vec![MirStmt::Try {
            body: vec![return_slice_of_b()],
            handlers: vec![],
            orelse: vec![],
            finalbody: vec![],
        }],
        vec![MirStmt::Try {
            body: vec![],
            handlers: vec![handler(vec![return_slice_of_b()])],
            orelse: vec![],
            finalbody: vec![],
        }],
        vec![MirStmt::Try {
            body: vec![],
            handlers: vec![],
            orelse: vec![return_slice_of_b()],
            finalbody: vec![],
        }],
        vec![MirStmt::Try {
            body: vec![],
            handlers: vec![],
            orelse: vec![],
            finalbody: vec![return_slice_of_b()],
        }],
        vec![MirStmt::TryStar {
            body: vec![return_slice_of_b()],
            handlers: vec![],
            orelse: vec![],
            finalbody: vec![],
        }],
        vec![MirStmt::TryStar {
            body: vec![],
            handlers: vec![handler(vec![return_slice_of_b()])],
            orelse: vec![],
            finalbody: vec![],
        }],
        vec![MirStmt::TryStar {
            body: vec![],
            handlers: vec![],
            orelse: vec![return_slice_of_b()],
            finalbody: vec![],
        }],
        vec![MirStmt::TryStar {
            body: vec![],
            handlers: vec![],
            orelse: vec![],
            finalbody: vec![return_slice_of_b()],
        }],
        // Two levels deep, so the recursion is not merely one-deep.
        vec![MirStmt::While {
            test: MirExpr::BoolLiteral(true),
            body: vec![MirStmt::If {
                test: MirExpr::BoolLiteral(true),
                body: vec![return_slice_of_b()],
                orelse: vec![],
            }],
        }],
    ];
    for (index, body) in nested.iter().enumerate() {
        assert!(body_returns_buffer_slice(body), "shape {index}: {body:?}");
    }
}

/// The negative arms. A whole-view `return b` in particular must answer
/// `false`: Part 1's export rides the plain `fnptr_<name>` cast path with no
/// thunk and no out-slots, and widening its signature here would change
/// generated C that this part does not touch.
#[test]
fn a_body_without_a_buffer_slice_return_is_not_reported() {
    let no: Vec<MirStmt> = vec![
        MirStmt::NoOp,
        MirStmt::Unreachable,
        MirStmt::Return(None),
        MirStmt::Return(Some(MirExpr::Name {
            name: "b".to_string(),
            ty: Ty::MemoryView,
        })),
        MirStmt::ExprStmt(MirExpr::IntLiteral(0)),
        MirStmt::If {
            test: MirExpr::BoolLiteral(true),
            body: vec![MirStmt::NoOp],
            orelse: vec![MirStmt::Return(None)],
        },
        MirStmt::Try {
            body: vec![MirStmt::NoOp],
            handlers: vec![handler(vec![MirStmt::NoOp])],
            orelse: vec![MirStmt::NoOp],
            finalbody: vec![MirStmt::NoOp],
        },
    ];
    assert!(!body_returns_buffer_slice(&[]));
    for stmt in no {
        assert!(
            !body_returns_buffer_slice(std::slice::from_ref(&stmt)),
            "{stmt:?}"
        );
    }
}

/// The emitted signature and the emitted stores, end to end in LLVM IR and
/// without CPython headers.
///
/// The declaration pass widens on the per-name body fact *alone* -- not on
/// `ext`, not on exportability -- so the declaration and the
/// `ReturnBufferSlice` site can never disagree about the parameter list. A private function that
/// returns a slice therefore also carries the three trailing pointers, unused
/// by any caller; this asserts that rather than leaving it to be discovered
/// as a crash.
#[test]
fn the_declaration_pass_widens_a_buffer_slice_body_by_three_out_pointers() {
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "sliced".to_string(),
                params: vec![("b".to_string(), Ty::MemoryView)],
                return_ty: Ty::MemoryView,
                body: vec![return_slice_of_b()],
            },
            MirItem::Function {
                name: "whole".to_string(),
                params: vec![("b".to_string(), Ty::MemoryView)],
                return_ty: Ty::MemoryView,
                body: vec![MirStmt::Return(Some(MirExpr::Name {
                    name: "b".to_string(),
                    ty: Ty::MemoryView,
                }))],
            },
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("buffer_slice_signature")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("buffer_slice_signature.o");
    let mut observed = Vec::new();
    let mut observer = |module: &inkwell::module::Module<'_>, _applied| {
        for name in ["pyfn_sliced", "pyfn_whole"] {
            let function = module
                .get_function(name)
                .unwrap_or_else(|| panic!("`{name}` should be declared"));
            observed.push((name, function.count_params()));
        }
        // The thunk exists for the slice-returning export only, and
        // forwards the same three out-pointers rather than consuming them
        // the way a tuple return's are consumed.
        observed.push((
            "pycc_ext_thunk_sliced",
            module
                .get_function("pycc_ext_thunk_sliced")
                .expect("the slice-returning export needs a thunk")
                .count_params(),
        ));
        assert!(
            module.get_function("pycc_ext_thunk_whole").is_none(),
            "Part 1's whole-view export still rides the plain cast path"
        );
        // Each of the three out-pointers is stored through exactly once.
        // Counting *all* stores would also count the entry block's
        // parameter alloca, so this keys on the pointer operand being the
        // out-parameter itself.
        let sliced = module
            .get_function("pyfn_sliced")
            .expect("`pyfn_sliced` should be declared");
        for offset in 1..4 {
            let out_param = sliced
                .get_nth_param(offset)
                .expect("the declaration pass widened this signature");
            observed.push((
                "out-slot stores",
                sliced
                    .get_basic_blocks()
                    .iter()
                    .flat_map(|block| block.get_instructions())
                    .filter(|instruction| {
                        instruction.get_opcode() == inkwell::values::InstructionOpcode::Store
                            && matches!(
                                instruction.get_operand(1),
                                Some(inkwell::values::Operand::Value(value))
                                    if value == out_param
                            )
                    })
                    .count() as u32,
            ));
        }
    };
    compile_to_object_with_observer(
        &mir,
        &obj_path,
        &CompileOptions {
            ext: true,
            ..CompileOptions::default()
        },
        Some(&mut observer),
    )
    .expect("codegen should succeed");

    assert_eq!(
        observed,
        vec![
            // One declared buffer parameter plus `has_slice`, `start` and
            // `stop`.
            ("pyfn_sliced", 4),
            // Part 1's whole-view egress is untouched.
            ("pyfn_whole", 1),
            ("pycc_ext_thunk_sliced", 4),
            // Exactly one store per out-slot.
            ("out-slot stores", 1),
            ("out-slot stores", 1),
            ("out-slot stores", 1),
        ]
    );
}

/// The out-slot fact is resolved per *name*, unioned over every definition
/// of it, because every definition shares one `fnptr_<name>` slot and one
/// `UserFunction::fn_type`.
#[test]
fn the_out_slot_fact_unions_over_every_definition_of_a_name() {
    let bare = |name: &str| MirItem::Function {
        name: name.to_string(),
        params: vec![("b".to_string(), Ty::MemoryView)],
        return_ty: Ty::MemoryView,
        body: vec![MirStmt::Return(Some(MirExpr::Name {
            name: "b".to_string(),
            ty: Ty::MemoryView,
        }))],
    };
    let sliced = |name: &str| MirItem::Function {
        name: name.to_string(),
        params: vec![("b".to_string(), Ty::MemoryView)],
        return_ty: Ty::MemoryView,
        body: vec![return_slice_of_b()],
    };

    // A non-`Function` item is skipped rather than answered, and a name with
    // no slicing definition anywhere stays absent.
    let items = vec![
        MirItem::TopLevelStmt(MirStmt::NoOp),
        bare("never"),
        bare("never"),
        bare("bare_then_sliced"),
        sliced("bare_then_sliced"),
        sliced("sliced_then_bare"),
        bare("sliced_then_bare"),
    ];
    let names = crate::ext::buffer_slice_out_names(&items);
    assert_eq!(
        names.into_iter().collect::<Vec<_>>(),
        vec![
            "bare_then_sliced".to_string(),
            "sliced_then_bare".to_string()
        ]
    );
}

/// ...and the widening that fact drives reaches every definition's LLVM
/// signature, in both orderings.
///
/// The regression: widening only the slicing definition left the shared
/// `fn_type` describing the other one, so the export thunk forwarded four
/// arguments through a one-argument function type and `verify()` aborted the
/// compiler on input the front end had accepted.
#[test]
fn every_definition_of_a_redefined_sliced_name_is_widened() {
    for (label, first, second) in [
        (
            "bare_then_sliced",
            MirStmt::Return(Some(MirExpr::Name {
                name: "b".to_string(),
                ty: Ty::MemoryView,
            })),
            return_slice_of_b(),
        ),
        (
            "sliced_then_bare",
            return_slice_of_b(),
            MirStmt::Return(Some(MirExpr::Name {
                name: "b".to_string(),
                ty: Ty::MemoryView,
            })),
        ),
    ] {
        let mir = MirModule {
            items: vec![
                MirItem::Function {
                    name: "redefined".to_string(),
                    params: vec![("b".to_string(), Ty::MemoryView)],
                    return_ty: Ty::MemoryView,
                    body: vec![first],
                },
                MirItem::Function {
                    name: "redefined".to_string(),
                    params: vec![("b".to_string(), Ty::MemoryView)],
                    return_ty: Ty::MemoryView,
                    body: vec![second],
                },
            ],
            class_defs: Vec::new(),
        };
        let dir = pycc_scratch::ScratchDir::new("codegen_1179_redef").expect("scratch");
        let obj_path = dir.join("redef.o");
        let mut observed: Vec<(&str, u32)> = Vec::new();
        let mut observer = |module: &inkwell::module::Module<'_>, _applied| {
            for symbol in [
                "pyfn_redefined",
                "pyfn_redefined__redef_1",
                "pycc_ext_thunk_redefined",
            ] {
                observed.push((
                    symbol,
                    module
                        .get_function(symbol)
                        .unwrap_or_else(|| panic!("`{symbol}` should be declared"))
                        .count_params(),
                ));
            }
        };
        compile_to_object_with_observer(
            &mir,
            &obj_path,
            &CompileOptions {
                ext: true,
                ..CompileOptions::default()
            },
            Some(&mut observer),
        )
        .expect("codegen should succeed");

        // One declared buffer parameter plus `has_slice`, `start` and `stop`,
        // for both definitions and for the thunk that forwards them.
        assert_eq!(
            observed,
            vec![
                ("pyfn_redefined", 4),
                ("pyfn_redefined__redef_1", 4),
                ("pycc_ext_thunk_redefined", 4),
            ],
            "{label}"
        );
    }
}
