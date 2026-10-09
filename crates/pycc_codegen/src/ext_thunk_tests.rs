//! Unit tests for #1050's `ext` export thunks.
//!
//! These are this crate's first `CompileOptions { ext: true, .. }` tests.
//! They assert the *exact* LLVM signature of each emitted thunk rather than
//! merely that one exists, because the signature is the whole contract: the
//! generated C in `src/ext_build.rs` declares the same symbol with widths it
//! derives independently from `crate::ext`'s convention, and nothing in
//! either toolchain can diagnose a disagreement. The `--ext` link defers
//! undefined symbols (`-undefined dynamic_lookup` on Mach-O, `-Bsymbolic` on
//! ELF), so a wrapper calling a thunk that was never emitted links cleanly
//! and dies on the first call -- which is why the negative assertions here
//! (no thunk for a private, dotted or monomorphized name) are as load-bearing
//! as the positive ones.
//!
//! Deliberately in a `*_tests.rs` file rather than appended to `tests.rs`:
//! that file is already 15k lines, and AGENTS.md's decomposability rule puts
//! a new unit beside the module it tests.

use super::*;
use pycc_mir::{MirExpr, MirItem, MirModule, MirStmt, Ty};

/// One emitted thunk as the tests observe it: symbol, LLVM function type,
/// and the names of its basic blocks in order.
#[derive(Debug, PartialEq, Eq)]
struct Thunk {
    symbol: String,
    signature: String,
    blocks: Vec<String>,
}

/// Compiles `mir` as an `ext` object and reports every `pycc_ext_thunk_*`
/// function the module holds, in module order.
///
/// Goes all the way to an object file rather than stopping at the observer's
/// module, so LLVM's own verifier runs over the emitted thunks: a
/// mistyped `insertvalue` or a block left without a terminator fails here
/// rather than in a CPython process much later.
fn thunks_of(label: &str, mir: &MirModule) -> Vec<Thunk> {
    thunks_of_with(
        label,
        mir,
        &CompileOptions {
            ext: true,
            ..CompileOptions::default()
        },
    )
}

/// [`thunks_of`] under explicit `options`.
fn thunks_of_with(label: &str, mir: &MirModule, options: &CompileOptions) -> Vec<Thunk> {
    let dir = pycc_scratch::ScratchDir::new(label).expect("failed to create scratch dir");
    let obj_path = dir.join(format!("{label}.o"));
    let mut observed = Vec::new();
    let mut observer = |module: &inkwell::module::Module<'_>, _pipeline: Option<&'static str>| {
        for function in module.get_functions() {
            let symbol = function.get_name().to_string_lossy().into_owned();
            if !symbol.starts_with(EXT_THUNK_PREFIX) {
                continue;
            }
            observed.push(Thunk {
                symbol,
                // D-029: an `LLVMString` must reach an owned `String`
                // through the crate's own wrapper, never through its
                // `Drop`, which faults on Windows against this LLVM
                // release.
                signature: llvm_string_to_owned(function.get_type().print_to_string()),
                blocks: function
                    .get_basic_blocks()
                    .iter()
                    .map(|block| {
                        // LLVM disambiguates a requested name against every
                        // other value name already taken in the function, so
                        // the `fnptr_is_null` block comes back as
                        // `fnptr_is_null1` -- the `icmp` of the same name got
                        // there first. The suffix is an artifact of naming,
                        // not of structure, so it is trimmed here rather than
                        // written into each expectation.
                        let name = block.get_name().to_string_lossy().into_owned();
                        name.trim_end_matches(|c: char| c.is_ascii_digit())
                            .to_string()
                    })
                    .collect(),
            });
        }
    };
    compile_to_object_with_observer(mir, &obj_path, options, Some(&mut observer))
        .expect("ext codegen should succeed");
    observed
}

/// `def <name>(<params>) -> <return_ty>` with a body that just produces a
/// value of the declared return type. The body is irrelevant to thunk
/// emission -- which runs off the declaration pass -- but has to be
/// emittable for the compile to reach the verifier.
fn func(name: &str, params: &[(&str, Ty)], return_ty: Ty) -> MirItem {
    let body = match &return_ty {
        Ty::None => Vec::new(),
        ty => vec![MirStmt::Return(Some(value_of(ty)))],
    };
    MirItem::Function {
        name: name.to_string(),
        params: params
            .iter()
            .map(|(n, ty)| ((*n).to_string(), ty.clone()))
            .collect(),
        return_ty,
        body,
    }
}

fn value_of(ty: &Ty) -> MirExpr {
    match ty {
        Ty::Int => MirExpr::IntLiteral(7),
        Ty::Float => MirExpr::FloatLiteral(2.5),
        Ty::Bool => MirExpr::BoolLiteral(true),
        Ty::Tuple(elems) => MirExpr::TupleLiteral(elems.iter().map(value_of).collect()),
        other => panic!("no literal fixture for {other:?}"),
    }
}

fn module(items: Vec<MirItem>) -> MirModule {
    MirModule {
        items,
        class_defs: Vec::new(),
    }
}

fn tuple(elems: Vec<Ty>) -> Ty {
    Ty::Tuple(Box::new(elems))
}

#[test]
fn suppress_export_thunks_emits_no_thunk_for_a_signature_that_otherwise_gets_one() {
    // The embedded-executable mode (Part 1 of #1028) compiles with `ext` for
    // its module-body entry point but exports nothing, and never runs the
    // driver's `collect_exports` admissibility check, so a thunk emitted for
    // a public signature there would be unchecked dead code. The default
    // (`false`) keeps `--ext`'s behaviour, so the same module still gets
    // its thunk without the flag.
    let mir = module(vec![func(
        "f",
        &[("t", tuple(vec![Ty::Int, Ty::Float]))],
        Ty::Int,
    )]);
    assert_eq!(thunks_of("ext_thunk_default_emits", &mir).len(), 1);
    let suppressed = thunks_of_with(
        "ext_thunk_suppressed",
        &mir,
        &CompileOptions {
            ext: true,
            suppress_export_thunks: true,
            ..CompileOptions::default()
        },
    );
    assert_eq!(suppressed, Vec::new());
}

#[test]
fn a_tuple_parameter_flattens_in_place_into_the_thunks_parameter_list() {
    let observed = thunks_of(
        "ext_thunk_tuple_param",
        &module(vec![func(
            "f",
            &[("t", tuple(vec![Ty::Int, Ty::Float]))],
            Ty::Int,
        )]),
    );

    assert_eq!(
        observed,
        vec![Thunk {
            symbol: "pycc_ext_thunk_f".to_string(),
            // `int` is `i64` and `float` is `f64`, matching
            // `ty_to_basic_type`; the aggregate itself never appears.
            signature: "i64 (i64, double)".to_string(),
            blocks: vec![
                "entry".to_string(),
                "fnptr_not_null".to_string(),
                "fnptr_is_null".to_string(),
            ],
        }]
    );
}

#[test]
fn a_tuple_return_leaves_through_one_trailing_out_pointer_per_element() {
    let observed = thunks_of(
        "ext_thunk_tuple_return",
        &module(vec![func("g", &[], tuple(vec![Ty::Int, Ty::Bool]))]),
    );

    // `void`, not the struct: returning the aggregate by value would put
    // pycc's own aggregate convention on the C side of the seam, which is
    // exactly what the thunk exists to prevent.
    assert_eq!(observed[0].symbol, "pycc_ext_thunk_g");
    assert_eq!(observed[0].signature, "void (ptr, ptr)");
    assert_eq!(observed.len(), 1);
}

#[test]
fn out_pointers_are_appended_after_every_flattened_parameter() {
    // The one fixture that pins the *order* of the combined list, and the
    // only one mixing `str` with a tuple: every parameter slot first, in
    // declaration order with tuples spread in place, then one out-pointer
    // per returned element.
    let observed = thunks_of(
        "ext_thunk_both_directions",
        &module(vec![func(
            "h",
            &[
                ("a", Ty::Int),
                ("t", tuple(vec![Ty::Int, Ty::Bool])),
                ("s", Ty::Str),
            ],
            tuple(vec![Ty::Float, Ty::Bool]),
        )]),
    );

    assert_eq!(
        observed[0].signature,
        "void (i64, i64, i8, ptr, ptr, ptr)".to_string()
    );
}

#[test]
fn a_tuple_carrying_export_returning_none_keeps_the_void_return() {
    // `-> None` is already LLVM `void`, so the thunk has no out-pointers
    // *and* no return value -- the arm that would silently read an
    // uninitialized call result if it shared the scalar path.
    let observed = thunks_of(
        "ext_thunk_none_return",
        &module(vec![func(
            "n",
            &[(
                "t",
                tuple(vec![Ty::Int, Ty::Int, Ty::Int, Ty::Int, Ty::Int]),
            )],
            Ty::None,
        )]),
    );

    assert_eq!(
        observed[0].signature,
        "void (i64, i64, i64, i64, i64)".to_string()
    );
}

#[test]
fn every_thunk_guards_the_function_pointer_slot_before_dispatching() {
    // The thunk reaches the compiled function through `fnptr_<name>`, not
    // through its mangled symbol, so a call that arrives before the `def`
    // executed has to raise pycc's `NameError` exactly as a Python-level
    // call would -- never jump through a null slot. `--ext` makes that
    // reachable in a way a native build never is: the host can import a
    // module whose body raised partway through and call an export anyway.
    let observed = thunks_of(
        "ext_thunk_null_guard",
        &module(vec![func("f", &[("t", tuple(vec![Ty::Int]))], Ty::Int)]),
    );

    assert_eq!(
        observed[0].blocks,
        vec![
            "entry".to_string(),
            "fnptr_not_null".to_string(),
            "fnptr_is_null".to_string(),
        ]
    );
}

#[test]
fn a_top_level_statement_is_stepped_over_rather_than_treated_as_an_export() {
    // `mir.items` interleaves the module body with the `def`s, so the
    // thunk loop walks statements as well as functions. A module whose
    // body runs before (and between) its definitions is the ordinary
    // shape, not an edge case -- and the statement carries no name,
    // parameters or return type to ask `ext_thunk_required` about.
    let observed = thunks_of(
        "ext_thunk_top_level_stmt",
        &module(vec![
            MirItem::TopLevelStmt(MirStmt::NoOp),
            func("f", &[("t", tuple(vec![Ty::Int, Ty::Float]))], Ty::Int),
            MirItem::TopLevelStmt(MirStmt::NoOp),
        ]),
    );

    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0].symbol, "pycc_ext_thunk_f");
}

#[test]
fn a_rebound_public_name_gets_exactly_one_thunk() {
    // Two `def`s of one name share one `fnptr_` slot and one signature
    // (`T0021` refuses a redefinition that changes it), so a second thunk
    // would be a duplicate symbol -- the C compiler would reject the
    // artifact outright.
    let observed = thunks_of(
        "ext_thunk_rebound",
        &module(vec![
            func("f", &[("t", tuple(vec![Ty::Int, Ty::Int]))], Ty::Int),
            func("f", &[("t", tuple(vec![Ty::Int, Ty::Int]))], Ty::Int),
        ]),
    );

    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0].symbol, "pycc_ext_thunk_f");
}

#[test]
fn no_thunk_is_emitted_for_a_name_the_export_set_excludes() {
    // Each of these carries a tuple and would otherwise qualify. A thunk
    // for one of them is not merely dead weight: `0gen_` names have no
    // `fnptr_` global at all, so emitting one would panic, and a `_private`
    // or private-method thunk would advertise a symbol `collect_exports`
    // never generates a wrapper for.
    //
    // #1145 moved the bare `Shape.area` spelling *into* the export set --
    // it is an instance method now, not an unrecognized third segment -- so
    // the corpus carries a property setter in its place. That is still a
    // name no `PyMethodDef` row can ever name, and it keeps this test about
    // the name predicate rather than about a kind the predicate admits.
    let observed = thunks_of(
        "ext_thunk_excluded_names",
        &module(vec![
            func("_private", &[("t", tuple(vec![Ty::Int]))], Ty::Int),
            func("Shape._area", &[("t", tuple(vec![Ty::Int]))], Ty::Int),
            func(
                "Shape.width.setter",
                &[("t", tuple(vec![Ty::Int]))],
                Ty::Int,
            ),
            func("0gen_pair", &[("t", tuple(vec![Ty::Int]))], Ty::Int),
        ]),
    );

    assert_eq!(observed, Vec::new());
}

#[test]
fn a_scalar_only_signature_gets_no_thunk() {
    // The wrapper for a scalar-only export still calls through
    // `fnptr_<name>` directly, so a thunk here would be emitted into every
    // artifact and called by nothing.
    let observed = thunks_of(
        "ext_thunk_scalar_only",
        &module(vec![
            func("f", &[("a", Ty::Int), ("s", Ty::Str)], Ty::Bool),
            func("g", &[], Ty::None),
        ]),
    );

    assert_eq!(observed, Vec::new());
}

#[test]
fn a_native_build_emits_no_thunks_at_all() {
    let mir = module(vec![func(
        "f",
        &[("t", tuple(vec![Ty::Int, Ty::Int]))],
        tuple(vec![Ty::Int, Ty::Int]),
    )]);
    let dir = pycc_scratch::ScratchDir::new("ext_thunk_native_build")
        .expect("failed to create scratch dir");
    let mut observed = Vec::new();
    let mut observer = |module: &inkwell::module::Module<'_>, _pipeline: Option<&'static str>| {
        observed.push(module.get_function("pycc_ext_thunk_f").is_some());
    };
    compile_to_object_with_observer(
        &mir,
        &dir.join("ext_thunk_native_build.o"),
        &CompileOptions::default(),
        Some(&mut observer),
    )
    .expect("native codegen should succeed");

    assert_eq!(observed, vec![false]);
}

// --- #1143: the method mangling and the widened export predicate ---------

#[test]
fn mangling_is_the_identity_on_a_dot_free_name() {
    // Every `ext` symbol emitted before #1143 was dot-free, and the mangling
    // must not move one of them: an existing artifact's exported C symbols
    // are its ABI.
    for name in ["f", "compute", "a_b_c", "x0"] {
        assert_eq!(crate::mangle_ext_name(name), name);
    }
}

#[test]
fn mangling_a_dotted_name_is_a_length_prefixed_c_identifier() {
    assert_eq!(
        crate::mangle_ext_name("Grid.scale.static"),
        "0m4_Grid5_scale6_static"
    );
    assert_eq!(
        crate::mangle_ext_name("Grid.make.classmethod"),
        "0m4_Grid4_make11_classmethod"
    );
}

#[test]
fn mangling_is_injective_across_shapes_that_would_collide_if_flattened() {
    // Dot-flattening (`Grid_scale_static`) is not injective: a class named
    // `Grid_scale` with a method `static` flattens to the same symbol. The
    // length prefixes are what make the segmentation recoverable, so the
    // colliding pairs below must stay distinct.
    //
    // The identity arm cannot collide with the prefixed arm: a dot-free name
    // maps to itself, and the only dot-free string that could equal some
    // dotted name's image starts with `0m`, which no Python identifier can
    // (an identifier cannot start with a digit) and which no compiler-
    // generated name uses either -- those all start with `0gen_`.
    let names = [
        "Grid.scale.static",
        "Grid_scale.static",
        "Grid.scale_static",
        "Grid_scale_static",
        "Grid.make.classmethod",
        "Grid.makeclassmethod",
    ];
    let mut mangled: Vec<String> = names.iter().map(|n| crate::mangle_ext_name(n)).collect();
    mangled.sort();
    let before = mangled.len();
    mangled.dedup();
    assert_eq!(mangled.len(), before, "mangling collided: {mangled:?}");
}

#[test]
fn the_export_predicate_admits_every_method_kind_a_mangled_name_can_carry_a_receiver_for() {
    // `.static`, `.classmethod` and the bare spelling are admitted; `.setter`
    // is refused, as is any other third segment. The bare spelling is a
    // regular method, a property getter or an abstract method, which this
    // name cannot tell apart -- #1145 admits all three here and lets the
    // driver, which can see `HirModule::class_defs`, remove the last two.
    // This predicate is deliberately a *superset* of the driver's admitted
    // set: at worst a thunk is emitted for a name no wrapper calls, while
    // the opposite error makes `ext_thunk_required` answer `false` for a
    // `tuple`-carrying instance method and emit the wrong call form.
    assert!(crate::is_ext_exportable_name("Grid.scale.static"));
    assert!(crate::is_ext_exportable_name("Grid.make.classmethod"));
    assert!(crate::is_ext_exportable_name("Grid.scale"));
    assert!(!crate::is_ext_exportable_name("Grid.width.setter"));
    assert!(!crate::is_ext_exportable_name("Grid.scale.other"));
    assert!(!crate::is_ext_exportable_name("Grid.scale.static.extra"));
}

#[test]
fn the_export_predicate_applies_the_public_name_rule_to_both_segments() {
    assert!(!crate::is_ext_exportable_name("_Grid.scale.static"));
    assert!(!crate::is_ext_exportable_name("Grid._scale.static"));
    assert!(!crate::is_ext_exportable_name("_f"));
    assert!(crate::is_ext_exportable_name("f"));
}

#[test]
fn the_export_predicate_admits_exactly_the_two_pep_562_module_hooks() {
    // #1467: the driver publishes an entry module's `__getattr__` and
    // `__dir__`, so a tuple-carrying hook needs its thunk. No other dunder,
    // and no near-miss spelling, is admitted.
    assert!(crate::is_ext_exportable_name("__getattr__"));
    assert!(crate::is_ext_exportable_name("__dir__"));
    assert!(!crate::is_ext_exportable_name("__getattr__x"));
    assert!(!crate::is_ext_exportable_name("__getattribute__"));
    assert!(!crate::is_ext_exportable_name("_getattr"));
    assert!(!crate::is_ext_exportable_name("Grid.__getattr__"));
}

#[test]
fn the_export_predicate_admits_a_classes_comparison_and_hash_slot_dunders() {
    // #1427: the driver wraps these for `tp_richcompare`/`tp_hash`, on a
    // published class or on the carrier type of a private one, so a
    // tuple-carrying comparison needs its thunk whatever the class is named.
    for dunder in [
        "__lt__", "__le__", "__eq__", "__ne__", "__gt__", "__ge__", "__hash__",
    ] {
        assert!(crate::is_ext_exportable_name(&format!("Grid.{dunder}")));
        assert!(crate::is_ext_exportable_name(&format!("_Grid.{dunder}")));
    }
    assert!(!crate::is_ext_exportable_name("__eq__"));
    assert!(!crate::is_ext_exportable_name(".__eq__"));
    assert!(!crate::is_ext_exportable_name("Grid.__eq__.static"));
    assert!(!crate::is_ext_exportable_name("Grid.__bool__"));
    assert!(!crate::is_ext_exportable_name("0gen_Grid.__eq__"));
}

#[test]
fn the_export_predicate_refuses_an_empty_segment_and_a_specialization() {
    // `is_public_name`'s underlying rule (`!starts_with('_')`) is `true` for
    // the empty string, so each empty segment is refused explicitly rather
    // than left to it.
    assert!(!crate::is_ext_exportable_name(""));
    assert!(!crate::is_ext_exportable_name(".static"));
    assert!(!crate::is_ext_exportable_name("Grid..static"));
    assert!(!crate::is_ext_exportable_name("0gen_identity_int"));
    assert!(!crate::is_ext_exportable_name("0gen_f.scale.static"));
}

/// Every global the emitted module holds, in module order.
fn globals_of(label: &str, mir: &MirModule) -> Vec<String> {
    let dir = pycc_scratch::ScratchDir::new(label).expect("failed to create scratch dir");
    let obj_path = dir.join(format!("{label}.o"));
    let mut observed = Vec::new();
    let mut observer = |module: &inkwell::module::Module<'_>, _pipeline: Option<&'static str>| {
        for global in module.get_globals() {
            observed.push(global.get_name().to_string_lossy().into_owned());
        }
    };
    compile_to_object_with_observer(
        mir,
        &obj_path,
        &CompileOptions {
            ext: true,
            ..CompileOptions::default()
        },
        Some(&mut observer),
    )
    .expect("ext codegen should succeed");
    observed
}

#[test]
fn a_methods_function_pointer_global_carries_the_mangled_spelling() {
    // `fnptr_Grid.scale.static` is not a C identifier, so the global the
    // generated wrapper declares `extern` must be the mangled spelling --
    // and the two spellings are produced by one function, so this test is
    // what keeps the emitted global and the generated `extern` equal.
    let observed = globals_of(
        "ext_method_fnptr_global",
        &module(vec![
            func("Grid.scale.static", &[("n", Ty::Int)], Ty::Int),
            func("plain", &[("n", Ty::Int)], Ty::Int),
        ]),
    );
    assert!(
        observed
            .iter()
            .any(|g| g == "fnptr_0m4_Grid5_scale6_static"),
        "{observed:?}"
    );
    // A dot-free name is untouched: an existing artifact's symbols are ABI.
    assert!(observed.iter().any(|g| g == "fnptr_plain"), "{observed:?}");
    assert!(
        !observed.iter().any(|g| g.contains("fnptr_Grid.scale")),
        "{observed:?}"
    );
}

#[test]
fn a_method_that_needs_a_thunk_gets_the_mangled_thunk_symbol() {
    let observed = thunks_of(
        "ext_method_thunk_symbol",
        &module(vec![func(
            "Grid.pair.static",
            &[("n", Ty::Int)],
            tuple(vec![Ty::Int, Ty::Int]),
        )]),
    );
    assert_eq!(
        observed
            .iter()
            .map(|t| t.symbol.as_str())
            .collect::<Vec<_>>(),
        vec!["pycc_ext_thunk_0m4_Grid4_pair6_static"]
    );
}
