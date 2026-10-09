//! Unit tests for `foreign_len.rs`, moved out of that file's inline
//! `mod tests` to keep it under AGENTS.md's ~1,000-line threshold.

use super::*;
use crate::{CompileOptions, EXT_MODULE_EXEC_SYMBOL, compile_to_object_with_observer};
use inkwell::values::AnyValue;
use pycc_mir::{MirExpr, MirItem, MirModule, MirStmt, Ty};

/// `import <module>` followed by the items `build` makes from a read of
/// that module's binding.
fn program(module: &str, build: impl Fn(MirExpr) -> Vec<MirStmt>) -> Vec<MirItem> {
    let mut items = vec![MirItem::ForeignImport {
        local_name: module.to_string(),
        module_path: module.to_string(),
        from: None,
    }];
    items.extend(
        build(MirExpr::Name {
            name: module.to_string(),
            ty: Ty::Object,
        })
        .into_iter()
        .map(MirItem::TopLevelStmt),
    );
    items
}

/// `x = <conversion>(<module>)` at module scope, for the Part 4 arms.
///
/// An assignment rather than a discarded expression statement so the
/// converted value is actually consumed, which is what forces the load
/// out of the out-slot to be emitted.
fn convert(module: &str, callee: &str, ty: Ty) -> Vec<MirItem> {
    program(module, |base| {
        vec![MirStmt::Assign {
            target: "converted".to_string(),
            value: MirExpr::Call {
                callee: callee.to_string(),
                args: vec![base],
                ty: ty.clone(),
            },
        }]
    })
}

/// One discarded `len(<module>)`.
fn len_of(module: &str) -> Vec<MirItem> {
    program(module, |base| {
        vec![MirStmt::ExprStmt(MirExpr::ObjLen {
            base: Box::new(base),
        })]
    })
}

/// The LLVM text of the module-exec entry point after compiling `items`
/// as an `ext` object -- `foreign_attr.rs`'s own `entry_ir`, which is
/// where the rationale for compiling all the way to an object file
/// (LLVM's verifier runs before any assertion is believed) lives.
fn entry_ir(label: &str, items: Vec<MirItem>) -> String {
    let dir = pycc_scratch::ScratchDir::new(label).expect("failed to create scratch dir");
    let mut ir = String::new();
    let mut observer = |module: &inkwell::module::Module<'_>, _: Option<&'static str>| {
        if let Some(entry) = module.get_function(EXT_MODULE_EXEC_SYMBOL) {
            ir = crate::llvm_string_to_owned(entry.print_to_string());
        }
    };
    compile_to_object_with_observer(
        &MirModule {
            items,
            ..Default::default()
        },
        &dir.join(format!("{label}.o")),
        &CompileOptions {
            ext: true,
            ..CompileOptions::default()
        },
        Some(&mut observer),
    )
    .expect("ext codegen should succeed");
    assert!(!ir.is_empty(), "no {EXT_MODULE_EXEC_SYMBOL} was emitted");
    ir
}

/// How many times `needle` occurs in `haystack`.
fn occurrences(haystack: &str, needle: &str) -> usize {
    let mut count = 0usize;
    let mut rest = haystack;
    while let Some(at) = rest.find(needle) {
        count += 1;
        rest = &rest[at + needle.len()..];
    }
    count
}

/// The call goes to the shared constant's symbol.
///
/// Asserted through [`EXT_OBJ_LEN_SYMBOL`] rather than against a literal
/// for `foreign_attr.rs`'s reason: the C definition and this declaration
/// resolve lazily at load time, so a literal spelled twice would be a
/// crash at first call rather than a link error.
#[test]
fn a_foreign_len_calls_the_shim_helper_by_its_shared_symbol() {
    let ir = entry_ir("foreign_len_call", len_of("numpy"));
    assert!(ir.contains(EXT_OBJ_LEN_SYMBOL), "{ir}");
}

/// A raising `PyObject_Size` stops the module body on the module-exec
/// failure edge rather than continuing with an unwritten out-slot.
#[test]
fn a_failed_foreign_len_returns_on_the_module_exec_failure_edge() {
    let ir = entry_ir("foreign_len_fail_edge", len_of("numpy"));
    assert!(ir.contains("foreign_len_failed"), "{ir}");
    assert!(ir.contains("foreign_len_fail:"), "{ir}");
    assert!(ir.contains("foreign_len_cont:"), "{ir}");
    assert!(
        ir.contains(&format!("ret i64 {EXT_MODULE_EXEC_FAILED}")),
        "{ir}"
    );
}

/// The out-slot `alloca` is hoisted into the entry block, so a
/// module-scope loop around a `len` does not grow the host's stack.
///
/// Asserted positionally: the entry block is everything up to the first
/// appended label, and the `alloca` must be inside it.
#[test]
fn the_out_slot_alloca_is_hoisted_into_the_entry_block() {
    let ir = entry_ir(
        "foreign_len_in_loop",
        program("numpy", |base| {
            vec![MirStmt::While {
                test: MirExpr::BoolLiteral(false),
                body: vec![MirStmt::ExprStmt(MirExpr::ObjLen {
                    base: Box::new(base),
                })],
            }]
        }),
    );
    let alloca_at = ir
        .find("foreign_len_out = alloca")
        .unwrap_or_else(|| panic!("no out-slot alloca: {ir}"));
    let first_label_at = ir
        .find("\n\n")
        .unwrap_or_else(|| panic!("no second basic block: {ir}"));
    assert!(alloca_at < first_label_at, "{ir}");
}

/// Two `len` calls in one module share one extern declaration and one
/// out-slot per call site.
///
/// `obj_len_fn` returns the existing `FunctionValue` on every call after
/// the first; a second `add_function` of one name is an LLVM
/// module-verifier error, so the second `len` is what proves the early
/// return is taken rather than merely present.
#[test]
fn a_second_foreign_len_reuses_the_one_extern_declaration() {
    let mut items = len_of("numpy");
    items.extend(len_of("scipy"));
    let ir = entry_ir("foreign_len_twice", items);
    assert_eq!(
        occurrences(&ir, EXT_OBJ_LEN_SYMBOL),
        2,
        "one call site per len: {ir}"
    );
}

/// An `if` on a CPython object calls the truth-testing helper and takes
/// the module-exec failure edge when it raises.
#[test]
fn a_foreign_condition_calls_the_shim_helper_and_can_fail() {
    let ir = entry_ir(
        "foreign_truthy_if",
        program("numpy", |base| {
            vec![MirStmt::If {
                test: base,
                body: vec![],
                orelse: vec![],
            }]
        }),
    );
    assert!(ir.contains(EXT_OBJ_TRUTHY_SYMBOL), "{ir}");
    assert!(ir.contains("foreign_truthy_failed"), "{ir}");
    assert!(ir.contains("foreign_truthy_fail:"), "{ir}");
    assert!(ir.contains("foreign_truthy_cont:"), "{ir}");
    assert!(
        ir.contains(&format!("ret i64 {EXT_MODULE_EXEC_FAILED}")),
        "{ir}"
    );
}

/// Every one of the five condition-position sites reaches the shim.
///
/// These are exactly the sites `pycc_types` used to refuse outright
/// (`reject_object_condition`'s ten call sites, five shapes checked in
/// both a module body and a function body). `lib.rs` reaches `truthy`
/// from each through a different `emit_stmt` arm, so one shape passing
/// says nothing about the other four; the comprehension guards in
/// particular sit behind their own loop scaffolding.
///
/// `tests/issue_1082_foreign_len_and_truth.rs` asserts the front-end
/// half of the same claim -- that each shape now type-checks at all.
/// Builds the statement list that places a condition expression at one
/// particular condition-position site, so the five shapes can be driven
/// from a single table.
type ConditionShape = fn(MirExpr) -> Vec<MirStmt>;

#[test]
fn every_condition_position_site_reaches_the_shim_helper() {
    let sites: [(&str, ConditionShape); 5] = [
        ("if", |test| {
            vec![MirStmt::If {
                test,
                body: vec![],
                orelse: vec![],
            }]
        }),
        ("while", |test| vec![MirStmt::While { test, body: vec![] }]),
        ("listcomp", |test| {
            vec![MirStmt::ListCompAssign {
                target: "xs".to_string(),
                var: "i".to_string(),
                var_ty: Ty::Int,
                source: pycc_mir::CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::IntLiteral(3),
                    step: MirExpr::IntLiteral(1),
                },
                cond: Some(Box::new(test)),
                elt: Box::new(MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }),
            }]
        }),
        ("setcomp", |test| {
            vec![MirStmt::SetCompAssign {
                ops: None,
                target: "ys".to_string(),
                var: "i".to_string(),
                var_ty: Ty::Int,
                source: pycc_mir::CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::IntLiteral(3),
                    step: MirExpr::IntLiteral(1),
                },
                cond: Some(Box::new(test)),
                elt: Box::new(MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }),
            }]
        }),
        ("dictcomp", |test| {
            vec![MirStmt::DictCompAssign {
                target: "zs".to_string(),
                var: "i".to_string(),
                var_ty: Ty::Int,
                source: pycc_mir::CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::IntLiteral(3),
                    step: MirExpr::IntLiteral(1),
                },
                cond: Some(Box::new(test)),
                key: Box::new(MirExpr::StringLiteral("k".to_string())),
                value: Box::new(MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }),
            }]
        }),
    ];
    for (label, build) in sites {
        let ir = entry_ir(
            &format!("foreign_truthy_site_{label}"),
            program("numpy", build),
        );
        assert!(ir.contains(EXT_OBJ_TRUTHY_SYMBOL), "{label}: {ir}");
        assert!(ir.contains("foreign_truthy_fail:"), "{label}: {ir}");
    }
}

/// Two foreign conditions in one module share one extern declaration --
/// `obj_truthy_fn`'s early return, proved the way `obj_len_fn`'s is.
#[test]
fn a_second_foreign_condition_reuses_the_one_extern_declaration() {
    let ir = entry_ir(
        "foreign_truthy_twice",
        program("numpy", |base| {
            vec![
                MirStmt::If {
                    test: base.clone(),
                    body: vec![],
                    orelse: vec![],
                },
                MirStmt::While {
                    test: base,
                    body: vec![],
                },
            ]
        }),
    );
    assert_eq!(
        occurrences(&ir, EXT_OBJ_TRUTHY_SYMBOL),
        2,
        "one call site per condition: {ir}"
    );
}

/// `float(o)` reaches the Part 4 shim helper rather than `lib.rs`'s
/// `to_float`, which panics on a `Scalar::Object`.
///
/// Asserted through [`EXT_OBJ_TO_FLOAT_SYMBOL`] rather than against a
/// literal for the lazy-link reason that constant records.
#[test]
fn a_foreign_float_conversion_calls_the_shim_helper_by_its_shared_symbol() {
    let ir = entry_ir(
        "foreign_to_float_call",
        convert("numpy", "float", Ty::Float),
    );
    assert!(ir.contains(EXT_OBJ_TO_FLOAT_SYMBOL), "{ir}");
}

/// A raising `PyNumber_Float` stops the module body on the module-exec
/// failure edge rather than continuing with an unwritten out-slot.
#[test]
fn a_failed_foreign_float_conversion_returns_on_the_module_exec_failure_edge() {
    let ir = entry_ir(
        "foreign_to_float_fail_edge",
        convert("numpy", "float", Ty::Float),
    );
    assert!(ir.contains("foreign_to_float_failed"), "{ir}");
    assert!(ir.contains("foreign_to_float_fail:"), "{ir}");
    assert!(ir.contains("foreign_to_float_cont:"), "{ir}");
    assert!(
        ir.contains(&format!("ret i64 {EXT_MODULE_EXEC_FAILED}")),
        "{ir}"
    );
}

/// The conversion's out-slot is a `double` hoisted into the entry block,
/// so a module-scope loop around a `float(o)` does not grow the host's
/// stack -- `the_out_slot_alloca_is_hoisted_into_the_entry_block`'s claim
/// for the second slot type `out_slot_in_entry_block` now serves.
#[test]
fn the_float_conversion_out_slot_is_a_double_in_the_entry_block() {
    let ir = entry_ir(
        "foreign_to_float_in_loop",
        program("numpy", |base| {
            vec![MirStmt::While {
                test: MirExpr::BoolLiteral(false),
                body: vec![MirStmt::Assign {
                    target: "converted".to_string(),
                    value: MirExpr::Call {
                        callee: "float".to_string(),
                        args: vec![base],
                        ty: Ty::Float,
                    },
                }],
            }]
        }),
    );
    let alloca_at = ir
        .find("foreign_to_float_out = alloca double")
        .unwrap_or_else(|| panic!("no double out-slot alloca: {ir}"));
    let first_label_at = ir
        .find("\n\n")
        .unwrap_or_else(|| panic!("no second basic block: {ir}"));
    assert!(alloca_at < first_label_at, "{ir}");
}

/// Two `float(o)` conversions in one module share one extern
/// declaration -- `obj_to_float_fn`'s early return.
///
/// The needle carries the call's own `(` because the bare symbol name
/// cannot tell the two outcomes apart: `LLVMAddFunction` does not reject
/// a duplicate name, it renames the second declaration to
/// `@pycc_ext_obj_to_float.1`, which still contains the bare symbol as a
/// substring. Counting `@pycc_ext_obj_to_float(` instead counts only the
/// call sites that reach the *first* declaration, so dropping the early
/// return leaves one of the two conversions calling the renamed
/// duplicate and the count falls to 1.
#[test]
fn a_second_foreign_float_conversion_reuses_the_one_extern_declaration() {
    let mut items = convert("numpy", "float", Ty::Float);
    items.extend(convert("scipy", "float", Ty::Float));
    let ir = entry_ir("foreign_to_float_twice", items);
    assert_eq!(
        occurrences(&ir, &format!("@{EXT_OBJ_TO_FLOAT_SYMBOL}(")),
        2,
        "one call site per conversion, both on the one declaration: {ir}"
    );
}

/// `bool(o)` adds no symbol of its own: it is PR 3a's truth test widened
/// to the `i8` a `Scalar::Bool` carries, and it inherits that helper's
/// module-exec failure edge unchanged.
/// `int(o)` and `str(o)` each reach their own Part 4 shim helper and
/// take the module-exec failure edge when it raises.
///
/// Asserted through the shared constants rather than against literals
/// for the lazy-link reason [`EXT_OBJ_TO_INT_SYMBOL`] records. The two
/// names share one `lib.rs` emission arm, so driving both through one
/// table is what proves the dispatch picks a different emitter per name
/// rather than the same one twice -- hence the negative assertion that
/// neither reaches the other's symbol.
#[test]
fn the_part_4b_conversions_call_their_shim_helpers_and_can_fail() {
    for (callee, ty, symbol, other) in [
        ("int", Ty::Int, EXT_OBJ_TO_INT_SYMBOL, EXT_OBJ_TO_STR_SYMBOL),
        ("str", Ty::Str, EXT_OBJ_TO_STR_SYMBOL, EXT_OBJ_TO_INT_SYMBOL),
    ] {
        let ir = entry_ir(
            &format!("foreign_to_{callee}_call"),
            convert("numpy", callee, ty),
        );
        assert!(ir.contains(symbol), "{callee}: {ir}");
        assert!(!ir.contains(other), "{callee}: {ir}");
        assert!(ir.contains(&format!("foreign_to_{callee}_failed")), "{ir}");
        assert!(ir.contains(&format!("foreign_to_{callee}_fail:")), "{ir}");
        assert!(ir.contains(&format!("foreign_to_{callee}_cont:")), "{ir}");
        assert!(
            ir.contains(&format!("ret i64 {EXT_MODULE_EXEC_FAILED}")),
            "{callee}: {ir}"
        );
    }
}

/// Each Part 4b conversion's out-slot carries the type its helper writes
/// and is hoisted into the entry block.
///
/// `the_out_slot_alloca_is_hoisted_into_the_entry_block`'s claim for the
/// third and fourth slot types `out_slot_in_entry_block` now serves: an
/// `i64` holding a D-141 encoded word, and a pointer holding the
/// `PyStrObj *` the shim copied out of CPython. The `while` wrapper is
/// what makes the hoist observable -- a conversion inside a module-scope
/// loop must not grow the host's stack.
#[test]
fn the_part_4b_conversion_out_slots_are_typed_and_in_the_entry_block() {
    for (callee, ty, slot) in [("int", Ty::Int, "i64"), ("str", Ty::Str, "ptr")] {
        let ir = entry_ir(
            &format!("foreign_to_{callee}_in_loop"),
            program("numpy", |base| {
                vec![MirStmt::While {
                    test: MirExpr::BoolLiteral(false),
                    body: vec![MirStmt::Assign {
                        target: "converted".to_string(),
                        value: MirExpr::Call {
                            callee: callee.to_string(),
                            args: vec![base],
                            ty: ty.clone(),
                        },
                    }],
                }]
            }),
        );
        let needle = format!("foreign_to_{callee}_out = alloca {slot}");
        let alloca_at = ir
            .find(&needle)
            .unwrap_or_else(|| panic!("no `{needle}`: {ir}"));
        let first_label_at = ir
            .find("\n\n")
            .unwrap_or_else(|| panic!("no second basic block: {ir}"));
        assert!(alloca_at < first_label_at, "{callee}: {ir}");
    }
}

/// Two conversions of one kind in one module share one extern
/// declaration -- `obj_to_int_fn`/`obj_to_str_fn`'s early return.
///
/// The needle carries the call's own `(` for the reason
/// `a_second_foreign_float_conversion_reuses_the_one_extern_declaration`
/// records: LLVM renames a duplicate declaration rather than rejecting
/// it, so the bare symbol would still match.
#[test]
fn a_second_part_4b_conversion_reuses_the_one_extern_declaration() {
    for (callee, ty, symbol) in [
        ("int", Ty::Int, EXT_OBJ_TO_INT_SYMBOL),
        ("str", Ty::Str, EXT_OBJ_TO_STR_SYMBOL),
    ] {
        let mut items = convert("numpy", callee, ty.clone());
        items.extend(convert("scipy", callee, ty));
        let ir = entry_ir(&format!("foreign_to_{callee}_twice"), items);
        assert_eq!(
            occurrences(&ir, &format!("@{symbol}(")),
            2,
            "{callee}: one call site per conversion, both on the one declaration: {ir}"
        );
    }
}

/// `x: tuple[float, float, float] = <object>` at module scope, for the
/// PR 4c arm: the MIR the front end produces for an annotated assignment
/// of a foreign object to a fixed-arity all-`float` tuple. The example is
/// spelled out rather than elided, because the PEP 585 variadic
/// `tuple[float, ...]` is the one spelling this arm never sees.
fn unpack(module: &str, arity: usize) -> Vec<MirItem> {
    program(module, |base| {
        vec![MirStmt::Assign {
            target: "unpacked".to_string(),
            value: MirExpr::ObjUnpackFloatTuple {
                base: Box::new(base),
                arity,
            },
        }]
    })
}

/// The unpack reaches the shim through the shared constant, passes the
/// arity as a call argument, and takes the module-exec failure edge.
///
/// Asserted through [`EXT_OBJ_UNPACK_FLOAT_TUPLE_SYMBOL`] rather than
/// against a literal for the lazy-link reason that constant records.
/// The arity is asserted as an *argument* because the whole point of
/// the helper's `long long arity` parameter is that nothing hard-codes
/// the three of `tuple[float, float, float]`; driving two different
/// arities through one table is what proves it.
#[test]
fn a_foreign_float_tuple_unpack_calls_the_shim_helper_with_its_arity() {
    for arity in [1usize, 3] {
        let ir = entry_ir(
            &format!("foreign_unpack_float_tuple_call_{arity}"),
            unpack("numpy", arity),
        );
        assert!(
            ir.contains(EXT_OBJ_UNPACK_FLOAT_TUPLE_SYMBOL),
            "{arity}: {ir}"
        );
        assert!(
            ir.contains(&format!(
                "i64 {arity}, ptr %foreign_unpack_float_tuple_out)"
            )),
            "{arity}: the arity travels as an argument: {ir}"
        );
        assert!(ir.contains("foreign_unpack_float_tuple_failed"), "{ir}");
        assert!(ir.contains("foreign_unpack_float_tuple_fail:"), "{ir}");
        assert!(ir.contains("foreign_unpack_float_tuple_cont:"), "{ir}");
        assert!(
            ir.contains(&format!("ret i64 {EXT_MODULE_EXEC_FAILED}")),
            "{arity}: {ir}"
        );
    }
}

/// The out-slot is one `[arity x double]` array hoisted into the entry
/// block, and the tuple is reassembled from it by value.
///
/// Two claims in one module, because they are the same design decision
/// seen from both ends. The array `alloca` must sit in the entry block
/// for `the_out_slot_alloca_is_hoisted_into_the_entry_block`'s reason --
/// this is the fifth slot type `out_slot_in_entry_block` serves, and the
/// first that is an aggregate -- and the result must leave the emitter
/// as an `insertvalue`-built struct rather than a pointer, because D-115
/// holds a tuple by value and no aggregate may cross the shim seam.
#[test]
fn the_unpack_out_slot_is_an_array_in_the_entry_block_rebuilt_by_value() {
    let ir = entry_ir(
        "foreign_unpack_float_tuple_in_loop",
        program("numpy", |base| {
            vec![MirStmt::While {
                test: MirExpr::BoolLiteral(false),
                body: vec![MirStmt::Assign {
                    target: "unpacked".to_string(),
                    value: MirExpr::ObjUnpackFloatTuple {
                        base: Box::new(base),
                        arity: 3,
                    },
                }],
            }]
        }),
    );
    let alloca_at = ir
        .find("foreign_unpack_float_tuple_out = alloca [3 x double]")
        .unwrap_or_else(|| panic!("no array out-slot alloca: {ir}"));
    let first_label_at = ir
        .find("\n\n")
        .unwrap_or_else(|| panic!("no second basic block: {ir}"));
    assert!(alloca_at < first_label_at, "{ir}");
    // One load and one `insertvalue` per element, and the aggregate the
    // last one produces is the emitter's whole result.
    assert_eq!(
        occurrences(&ir, "load double, ptr %foreign_unpack"),
        3,
        "{ir}"
    );
    assert_eq!(
        occurrences(&ir, "insertvalue { double, double, double }"),
        3,
        "{ir}"
    );
}

/// Two unpacks in one module share one extern declaration --
/// `obj_unpack_float_tuple_fn`'s early return, proved the way
/// `a_second_foreign_float_conversion_reuses_the_one_extern_declaration`
/// proves its own: LLVM renames a duplicate declaration rather than
/// rejecting it, so the needle carries the call's own `(`.
///
/// The two arities differ deliberately: one declaration has to serve
/// every arity, which is exactly why the arity is a parameter.
#[test]
fn a_second_foreign_unpack_reuses_the_one_extern_declaration() {
    let mut items = unpack("numpy", 3);
    items.extend(unpack("scipy", 2));
    let ir = entry_ir("foreign_unpack_float_tuple_twice", items);
    assert_eq!(
        occurrences(&ir, &format!("@{EXT_OBJ_UNPACK_FLOAT_TUPLE_SYMBOL}(")),
        2,
        "one call site per unpack, both on the one declaration: {ir}"
    );
}

#[test]
fn a_foreign_bool_conversion_reuses_the_truth_testing_helper() {
    let ir = entry_ir("foreign_bool_call", convert("numpy", "bool", Ty::Bool));
    assert!(ir.contains(EXT_OBJ_TRUTHY_SYMBOL), "{ir}");
    assert!(!ir.contains(EXT_OBJ_TO_FLOAT_SYMBOL), "{ir}");
    assert!(ir.contains("foreign_truthy_fail:"), "{ir}");
    assert!(
        ir.contains(&format!("ret i64 {EXT_MODULE_EXEC_FAILED}")),
        "{ir}"
    );
    assert!(ir.contains("bool_from_object"), "{ir}");
}
