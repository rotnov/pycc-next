//! Unit tests for foreign-import emission, moved out of
//! `src/foreign_import.rs` when #1366 added one (AGENTS.md "Keep source
//! files decomposable"); the moved tests changed only for #1366's new
//! `level` argument.

use super::*;
use crate::{
    CompileOptions, EXT_MODULE_EXEC_SYMBOL, EXT_OBJ_ERROR_BRIDGE_SYMBOL,
    compile_to_object_with_observer,
};
use inkwell::values::AnyValue;
use pycc_mir::{MirExpr, MirItem, MirModule, MirStmt, Ty};

/// `print(<n>)` as a top-level statement, with `n` chosen distinct per
/// call site so the emitted literal identifies which statement's code a
/// given point in the IR belongs to.
fn print_int(n: i64) -> MirItem {
    MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
        callee: "print".to_string(),
        args: vec![MirExpr::IntLiteral(n)],
        ty: Ty::None,
    }))
}

/// The LLVM text of the module-exec entry point after compiling `items`
/// as an `ext` object.
///
/// Compiles all the way to an object file, exactly as
/// `ext_thunk_tests.rs` does, so LLVM's verifier runs over the blocks
/// `emit` appends before any assertion here is believed.
fn entry_ir(label: &str, items: Vec<MirItem>) -> String {
    let dir = pycc_scratch::ScratchDir::new(label).expect("failed to create scratch dir");
    let mut ir = String::new();
    let mut observer = |module: &inkwell::module::Module<'_>, _: Option<&'static str>| {
        if let Some(entry) = module.get_function(EXT_MODULE_EXEC_SYMBOL) {
            // D-029: an `LLVMString` reaches an owned `String` through
            // the crate's own wrapper, never through its `Drop`.
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

/// The #1080 ordering obligation, stated at the emission layer.
///
/// `crates/pycc_mir/src/tests/import.rs` pins that the *item* lands at
/// its source position; that is a statement about the IR the MIR
/// builder produces, not about the machine code codegen emits from it.
/// This is the other half: a module body statement written above an
/// `import` really is emitted above the import call, as CPython would
/// run it (D-244 rule 3), and a statement written below it below.
///
/// Position is read off the two `print` calls rather than off block
/// names: the import's own `foreign_import_fail`/`foreign_import_cont`
/// blocks are appended to the function in emission order regardless of
/// where the call sits, so a block-order assertion would hold just as
/// well for the hoisted prologue this replaced. It is read off the
/// emitted `pycc_rt_int_to_str` calls rather than off the literals
/// themselves because an `int` reaches the IR tag-encoded (`111` prints
/// as `i64 223`), which is a representation detail this test has no
/// business pinning.
#[test]
fn a_statement_above_a_foreign_import_is_emitted_above_the_import_call() {
    let ir = entry_ir(
        "foreign_import_order",
        vec![
            print_int(111),
            MirItem::ForeignImport {
                local_name: "numpy".to_string(),
                module_path: "numpy".to_string(),
                from: None,
            },
            print_int(222),
        ],
    );
    const PRINTED: &str = "pycc_rt_int_to_str";
    let before = ir.find(PRINTED).expect("the statement above the import");
    let after = ir.rfind(PRINTED).expect("the statement below the import");
    assert_ne!(before, after, "both statements are emitted: {ir}");
    let import = ir
        .find(EXT_OBJ_IMPORT_SYMBOL)
        .expect("the foreign import call");
    assert!(before < import, "{ir}");
    assert!(import < after, "{ir}");
}

/// Two foreign imports in one module share one extern declaration of
/// the shim helper.
///
/// `obj_import_fn` declares `pycc_ext_obj_import` lazily and returns
/// the existing `FunctionValue` on every later call; declaring it
/// twice would be an LLVM module-verifier error (a redefinition), so
/// the second import is what proves the early return is taken rather
/// than merely present. Both calls are still emitted, in source order.
#[test]
fn a_second_foreign_import_reuses_the_one_extern_declaration() {
    let mut declared = 0usize;
    let ir = entry_ir(
        "foreign_import_twice",
        vec![
            MirItem::ForeignImport {
                local_name: "numpy".to_string(),
                module_path: "numpy".to_string(),
                from: None,
            },
            MirItem::ForeignImport {
                local_name: "scipy".to_string(),
                module_path: "scipy".to_string(),
                from: None,
            },
        ],
    );
    let mut rest = ir.as_str();
    while let Some(at) = rest.find(EXT_OBJ_IMPORT_SYMBOL) {
        declared += 1;
        rest = &rest[at + EXT_OBJ_IMPORT_SYMBOL.len()..];
    }
    assert_eq!(declared, 2, "one call site per import: {ir}");
    let numpy = ir.find("numpy").expect("the first module name");
    let scipy = ir.find("scipy").expect("the second module name");
    assert!(numpy < scipy, "{ir}");
}

/// `if <test>:` wrapping a block foreign import (#1291) of `bindings`.
fn if_block_import(bindings: &[(&str, &str)]) -> MirItem {
    MirItem::TopLevelStmt(MirStmt::If {
        test: MirExpr::BoolLiteral(true),
        body: vec![MirStmt::ForeignImport {
            bindings: bindings
                .iter()
                .map(|(local, module)| ((*local).to_string(), (*module).to_string(), None))
                .collect(),
        }],
        orelse: vec![],
    })
}

/// #1291: a foreign import nested in a module-level `if` is emitted in
/// the branch that runs it, with the same failure edge as a top-level
/// one, and its name is stored to a module global that
/// `collect_module_bindings` declared.
#[test]
fn a_block_foreign_import_is_emitted_inside_its_branch() {
    let ir = entry_ir(
        "foreign_import_block",
        vec![
            print_int(111),
            if_block_import(&[("colorsys", "colorsys")]),
            print_int(222),
        ],
    );
    let import = ir.find(EXT_OBJ_IMPORT_SYMBOL).expect("the import call");
    let then_block = ir.find("if_then:").expect("the `if` branch block");
    assert!(then_block < import, "{ir}");
    assert!(ir.contains("foreign_import_fail"), "{ir}");
    assert_eq!(
        block_terminator(&ir, "foreign_import_unbridged"),
        "ret i64 -1",
        "{ir}"
    );
    assert!(
        ir.contains("store ptr %foreign_import, ptr @pyglobal_colorsys"),
        "the module global is stored: {ir}"
    );
}

/// The instruction lines of the block labelled `label` in `ir`, trimmed.
fn block_lines<'a>(ir: &'a str, label: &str) -> Vec<&'a str> {
    let header = format!("{label}:");
    let mut lines = ir.lines().skip_while(|line| !line.starts_with(&header));
    assert!(lines.next().is_some(), "no block `{label}`: {ir}");
    lines
        .take_while(|line| !line.is_empty() && !line.starts_with(|c: char| c.is_alphanumeric()))
        .map(str::trim)
        .collect()
}

/// The terminator of the block labelled `label` in `ir`: its last
/// non-empty instruction line, trimmed.
fn block_terminator<'a>(ir: &'a str, label: &str) -> &'a str {
    block_lines(ir, label)
        .last()
        .copied()
        .expect("a block has a terminator")
}

/// The two labels of the conditional branch on the bridge's answer:
/// `(raised, unbridged)`, with LLVM's uniquing digits stripped.
fn bridge_branch_labels(ir: &str) -> (String, String) {
    let call = ir
        .find(&format!("call i32 @{EXT_IMPORT_ERROR_BRIDGE_SYMBOL}()"))
        .expect("the bridge call");
    assert!(
        ir.find(EXT_OBJ_IMPORT_SYMBOL).expect("the import call") < call,
        "{ir}"
    );
    let branch = ir
        .lines()
        .find(|line| line.contains("br i1 %import_error_raised,"))
        .expect("the branch on the bridge's answer");
    let labels: Vec<String> = branch
        .split("label %")
        .skip(1)
        .map(|rest| {
            rest.trim_end_matches(|c: char| c == ',' || c.is_whitespace())
                .trim_end_matches(|c: char| c.is_ascii_digit())
                .to_string()
        })
        .collect();
    assert_eq!(labels.len(), 2, "{branch}");
    (labels[0].clone(), labels[1].clone())
}

/// A `try` whose body is `body` and whose one `except Exception:`
/// handler runs `handler`.
fn try_stmt(body: Vec<MirStmt>, handler: Vec<MirStmt>) -> MirItem {
    MirItem::TopLevelStmt(MirStmt::Try {
        body,
        handlers: vec![pycc_mir::MirExceptHandler {
            exc_type_tag: Some(vec![0]),
            binding_name: None,
            binding_ty: None,
            body: handler,
        }],
        orelse: vec![],
        finalbody: vec![],
    })
}

fn import_stmt(name: &str) -> MirStmt {
    MirStmt::ForeignImport {
        bindings: vec![(name.to_string(), name.to_string(), None)],
    }
}

/// #1293: an import in a `try` body bridges an `ImportError` to the
/// handler dispatch. Since Part 1 of #1096 the named unbridged edge --
/// any other exception the imported module raises -- is bridged by the
/// object bridge to the same dispatch, rather than returning directly.
#[test]
fn a_try_block_foreign_import_bridges_to_the_handler_dispatch() {
    let ir = entry_ir(
        "foreign_import_bridge_try",
        vec![try_stmt(vec![import_stmt("colorsys")], vec![MirStmt::NoOp])],
    );
    let (raised, unbridged) = bridge_branch_labels(&ir);
    assert_eq!(raised, "try_handler_dispatch", "{ir}");
    assert_eq!(unbridged, "foreign_import_unbridged", "{ir}");
    assert!(
        block_lines(&ir, "foreign_import_unbridged")
            .iter()
            .any(|line| line.contains(&format!("call i32 @{EXT_OBJ_ERROR_BRIDGE_SYMBOL}()"))),
        "{ir}"
    );
    assert_eq!(
        block_terminator(&ir, "foreign_import_unbridged"),
        "br label %try_handler_dispatch",
        "{ir}"
    );
}

/// With no enclosing `try`, the bridged exception goes where any other
/// module-level raise goes, and the unbridged edge keeps the direct
/// return: no handler could run, so the import fails with CPython's
/// exception untouched (Part 1 of #1096).
#[test]
fn an_if_block_foreign_import_bridges_to_the_top_exception_exit() {
    let ir = entry_ir(
        "foreign_import_bridge_if",
        vec![if_block_import(&[("colorsys", "colorsys")])],
    );
    let (raised, unbridged) = bridge_branch_labels(&ir);
    assert_eq!(raised, "top_exception_exit", "{ir}");
    assert_eq!(unbridged, "foreign_import_unbridged", "{ir}");
    assert_eq!(
        block_terminator(&ir, "foreign_import_unbridged"),
        "ret i64 -1",
        "{ir}"
    );
    assert!(
        !block_lines(&ir, "foreign_import_unbridged")
            .iter()
            .any(|line| line.contains(EXT_OBJ_ERROR_BRIDGE_SYMBOL)),
        "{ir}"
    );
}

/// An import in a handler body is past the dispatch: its failure goes to
/// the `try`'s finally target.
#[test]
fn a_handler_body_foreign_import_bridges_to_the_finally_target() {
    let ir = entry_ir(
        "foreign_import_bridge_handler",
        vec![try_stmt(vec![MirStmt::NoOp], vec![import_stmt("colorsys")])],
    );
    let (raised, _) = bridge_branch_labels(&ir);
    assert_eq!(raised, "try_finally", "{ir}");
}

/// Two bridged imports share one lazy declaration of the bridge helper
/// and call it once each.
#[test]
fn two_bridged_imports_in_one_module_share_one_bridge_declaration() {
    let dir = pycc_scratch::ScratchDir::new("foreign_import_bridge_two").expect("scratch");
    let mut ir = String::new();
    let mut observer = |module: &inkwell::module::Module<'_>, _: Option<&'static str>| {
        if module.get_function(EXT_MODULE_EXEC_SYMBOL).is_some() {
            ir = crate::llvm_string_to_owned(module.print_to_string());
        }
    };
    compile_to_object_with_observer(
        &MirModule {
            items: vec![if_block_import(&[("sys", "sys"), ("re", "re")])],
            ..Default::default()
        },
        &dir.join("two.o"),
        &CompileOptions {
            ext: true,
            ..CompileOptions::default()
        },
        Some(&mut observer),
    )
    .expect("ext codegen should succeed");
    let declared = format!("declare i32 @{EXT_IMPORT_ERROR_BRIDGE_SYMBOL}()");
    assert_eq!(ir.matches(&declared).count(), 1, "{ir}");
    assert!(
        !ir.contains(&format!("@{EXT_IMPORT_ERROR_BRIDGE_SYMBOL}.")),
        "{ir}"
    );
    let called = format!("call i32 @{EXT_IMPORT_ERROR_BRIDGE_SYMBOL}()");
    assert_eq!(ir.matches(&called).count(), 2, "{ir}");
}

/// A top-level import has nothing to enclose it, so it keeps the direct
/// return and never calls the bridge.
#[test]
fn a_top_level_foreign_import_item_never_calls_the_bridge() {
    let ir = entry_ir(
        "foreign_import_bridge_top_level",
        vec![MirItem::ForeignImport {
            local_name: "numpy".to_string(),
            module_path: "numpy".to_string(),
            from: None,
        }],
    );
    assert!(!ir.contains(EXT_IMPORT_ERROR_BRIDGE_SYMBOL), "{ir}");
    assert!(!ir.contains("foreign_import_unbridged"), "{ir}");
    assert_eq!(
        block_terminator(&ir, "foreign_import_fail"),
        "ret i64 -1",
        "{ir}"
    );
}

/// Two bindings of one node are emitted in source order, one call each.
#[test]
fn two_bindings_of_one_block_import_are_emitted_in_order() {
    let ir = entry_ir(
        "foreign_import_block_two",
        vec![if_block_import(&[("sys", "sys"), ("re", "re")])],
    );
    assert_eq!(ir.matches(EXT_OBJ_IMPORT_SYMBOL).count(), 2, "{ir}");
    let sys = ir.find("@pyglobal_sys").expect("the first global");
    let re = ir.find("@pyglobal_re").expect("the second global");
    assert!(sys < re, "{ir}");
}

/// The identical pair #1291 admits (`import numpy` in both arms of an
/// `if`/`else`) requests the module-path global
/// `pycc_foreign_module_numpy` twice. LLVM uniquifies the second name,
/// so each emission must still pass a global holding its own path
/// string: both calls are emitted, and every module-path global the
/// module declares holds `numpy`.
#[test]
fn an_identical_pair_in_both_arms_emits_two_imports_of_its_own_path() {
    let import = || MirStmt::ForeignImport {
        bindings: vec![("numpy".to_string(), "numpy".to_string(), None)],
    };
    let dir = pycc_scratch::ScratchDir::new("foreign_import_block_pair").expect("scratch");
    let mut ir = String::new();
    let mut observer = |module: &inkwell::module::Module<'_>, _: Option<&'static str>| {
        if module.get_function(EXT_MODULE_EXEC_SYMBOL).is_some() {
            ir = crate::llvm_string_to_owned(module.print_to_string());
        }
    };
    compile_to_object_with_observer(
        &MirModule {
            items: vec![MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::BoolLiteral(true),
                body: vec![import()],
                orelse: vec![import()],
            })],
            ..Default::default()
        },
        &dir.join("pair.o"),
        &CompileOptions {
            ext: true,
            ..CompileOptions::default()
        },
        Some(&mut observer),
    )
    .expect("ext codegen should succeed");
    let calls: Vec<&str> = ir
        .lines()
        .filter(|line| line.contains(&format!("call ptr @{EXT_OBJ_IMPORT_SYMBOL}(")))
        .collect();
    assert_eq!(calls.len(), 2, "{ir}");
    assert!(
        calls[0].ends_with("(ptr @pycc_foreign_module_numpy)"),
        "{ir}"
    );
    assert!(
        calls[1].ends_with("(ptr @pycc_foreign_module_numpy.1)"),
        "{ir}"
    );
    let globals: Vec<&str> = ir
        .lines()
        .filter(|line| line.starts_with("@pycc_foreign_module_numpy"))
        .collect();
    assert_eq!(globals.len(), 2, "{ir}");
    for global in globals {
        assert!(global.contains("c\"numpy\\00\""), "{global}");
    }
}

/// The gate `src/foreign_import.rs` exists to make coverable: a build
/// that neither passes `--ext` nor embeds CPython (D-248; an embedded
/// build compiles with `ext` set) has no interpreter to import into,
/// the driver has already refused with `I0403`, and this arm emits
/// nothing rather than asserting.
#[test]
fn a_native_build_emits_no_import_call_for_a_foreign_import_item() {
    let dir = pycc_scratch::ScratchDir::new("foreign_import_native").expect("scratch");
    let mut symbols = Vec::new();
    let mut observer = |module: &inkwell::module::Module<'_>, _: Option<&'static str>| {
        symbols = module
            .get_functions()
            .map(|f| f.get_name().to_string_lossy().into_owned())
            .collect();
    };
    compile_to_object_with_observer(
        &MirModule {
            items: vec![MirItem::ForeignImport {
                local_name: "numpy".to_string(),
                module_path: "numpy".to_string(),
                from: None,
            }],
            ..Default::default()
        },
        &dir.join("native.o"),
        &CompileOptions::default(),
        Some(&mut observer),
    )
    .expect("native codegen should succeed");
    assert!(
        !symbols.iter().any(|s| s == EXT_OBJ_IMPORT_SYMBOL),
        "{symbols:?}"
    );
}

/// The whole-module LLVM text after compiling `items` as an `ext`
/// object, verified by the same object emission `entry_ir` runs.
fn module_ir(label: &str, items: Vec<MirItem>) -> String {
    let dir = pycc_scratch::ScratchDir::new(label).expect("scratch");
    let mut ir = String::new();
    let mut observer = |module: &inkwell::module::Module<'_>, _: Option<&'static str>| {
        if module.get_function(EXT_MODULE_EXEC_SYMBOL).is_some() {
            ir = crate::llvm_string_to_owned(module.print_to_string());
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

/// The item `from itertools import product, chain` lowers to for
/// `fromlist[index]`.
fn from_item(index: usize) -> MirItem {
    let fromlist = vec!["product".to_string(), "chain".to_string()];
    MirItem::ForeignImport {
        local_name: fromlist[index].clone(),
        module_path: "itertools".to_string(),
        from: Some(pycc_mir::FromImport {
            name: fromlist[index].clone(),
            fromlist,
            index,
            level: 0,
        }),
    }
}

/// A top-level from-import item (#1278), like a top-level `import`, has
/// the direct return as its failure edge and never calls the #1293
/// bridge; a nested one (#1383) is a `MirStmt` and does.
#[test]
fn a_from_import_item_never_calls_the_bridge() {
    let ir = entry_ir("foreign_from_import_no_bridge", vec![from_item(0)]);
    assert!(ir.contains(EXT_OBJ_IMPORT_FROM_SYMBOL), "{ir}");
    assert!(!ir.contains(EXT_IMPORT_ERROR_BRIDGE_SYMBOL), "{ir}");
    assert_eq!(
        block_terminator(&ir, "foreign_import_fail"),
        "ret i64 -1",
        "{ir}"
    );
}

/// #1278: each name of a from-import calls `pycc_ext_obj_import_from`
/// with the module, the statement's whole fromlist, its length and the
/// name's own index; the helper is declared once, the plain import
/// helper not at all, and each result takes the usual failure edge and
/// is stored to its own module global.
#[test]
fn a_from_import_passes_the_whole_fromlist_and_its_own_index() {
    let ir = module_ir("foreign_import_from", vec![from_item(0), from_item(1)]);
    let calls: Vec<&str> = ir
        .lines()
        .filter(|line| line.contains(&format!("call ptr @{EXT_OBJ_IMPORT_FROM_SYMBOL}(")))
        .collect();
    assert_eq!(calls.len(), 2, "{ir}");
    assert!(
        calls[0].ends_with(
            "(ptr @pycc_foreign_module_product, ptr @pycc_foreign_fromlist_product, \
             i64 2, i64 0, i64 0)"
        ),
        "{ir}"
    );
    assert!(
        calls[1].ends_with(
            "(ptr @pycc_foreign_module_chain, ptr @pycc_foreign_fromlist_chain, i64 2, i64 1, \
             i64 0)"
        ),
        "{ir}"
    );
    assert_eq!(
        ir.matches(&format!("declare ptr @{EXT_OBJ_IMPORT_FROM_SYMBOL}("))
            .count(),
        1,
        "{ir}"
    );
    assert!(!ir.contains(&format!("@{EXT_OBJ_IMPORT_SYMBOL}(")), "{ir}");
    let fromlist = ir
        .lines()
        .find(|line| line.starts_with("@pycc_foreign_fromlist_product "))
        .expect("the fromlist global");
    assert!(
        fromlist.contains("private constant [2 x ptr]"),
        "{fromlist}"
    );
    for name in ["product", "chain"] {
        assert!(
            ir.lines()
                .any(|line| line.starts_with("@pycc_foreign_attr_")
                    && line.contains(&format!("c\"{name}\\00\""))),
            "the `{name}` string: {ir}"
        );
    }
    assert!(
        ir.lines()
            .any(|line| line.starts_with("@pycc_foreign_module_product ")
                && line.contains("c\"itertools\\00\"")),
        "the module string names the module, not the name: {ir}"
    );
    assert_eq!(
        ir.lines()
            .filter(|line| line.starts_with("foreign_import_fail"))
            .count(),
        2,
        "one failure edge per name: {ir}"
    );
    assert!(
        ir.contains("store ptr %foreign_import, ptr @pyglobal_product"),
        "{ir}"
    );
    assert!(ir.contains("@pyglobal_chain"), "{ir}");
}

/// #1366: a relative from-import passes its dot count as the fifth
/// argument, and `from . import sib` passes the empty module string.
#[test]
fn a_relative_from_import_passes_its_level() {
    let item = |module_path: &str, name: &str, level: u32| MirItem::ForeignImport {
        local_name: name.to_string(),
        module_path: module_path.to_string(),
        from: Some(pycc_mir::FromImport {
            name: name.to_string(),
            fromlist: vec![name.to_string()],
            index: 0,
            level,
        }),
    };
    let ir = module_ir(
        "foreign_import_relative",
        vec![item("", "sib", 1), item("lexer", "Token", 2)],
    );
    let calls: Vec<&str> = ir
        .lines()
        .filter(|line| line.contains(&format!("call ptr @{EXT_OBJ_IMPORT_FROM_SYMBOL}(")))
        .collect();
    assert_eq!(calls.len(), 2, "{ir}");
    assert!(calls[0].ends_with("i64 1, i64 0, i64 1)"), "{ir}");
    assert!(calls[1].ends_with("i64 1, i64 0, i64 2)"), "{ir}");
    assert!(
        ir.lines()
            .any(|line| line.starts_with("@pycc_foreign_module_sib ")
                && line.contains("[1 x i8] zeroinitializer")),
        "`from . import sib` imports the empty module name: {ir}"
    );
    assert!(
        ir.contains(&format!(
            "declare ptr @{EXT_OBJ_IMPORT_FROM_SYMBOL}(ptr, ptr, i64, i64, i64)"
        )),
        "{ir}"
    );
}

/// The block statement `from itertools import <names>` lowers to (#1383).
fn from_stmt(names: &[&str]) -> MirStmt {
    let fromlist: Vec<String> = names.iter().map(ToString::to_string).collect();
    MirStmt::ForeignImport {
        bindings: fromlist
            .iter()
            .enumerate()
            .map(|(index, name)| {
                (
                    name.clone(),
                    "itertools".to_string(),
                    Some(pycc_mir::FromImport {
                        name: name.clone(),
                        fromlist: fromlist.clone(),
                        index,
                        level: 0,
                    }),
                )
            })
            .collect(),
    }
}

/// #1383: a from-import nested in a `try` body calls
/// `pycc_ext_obj_import_from` and bridges its failure to the handler
/// dispatch exactly as a nested `import` does.
#[test]
fn a_try_block_from_import_bridges_to_the_handler_dispatch() {
    let ir = entry_ir(
        "foreign_from_import_bridge_try",
        vec![try_stmt(vec![from_stmt(&["product"])], vec![MirStmt::NoOp])],
    );
    let import = ir
        .find(&format!("call ptr @{EXT_OBJ_IMPORT_FROM_SYMBOL}("))
        .expect("the from-import call");
    let bridge = ir
        .find(&format!("call i32 @{EXT_IMPORT_ERROR_BRIDGE_SYMBOL}()"))
        .expect("the bridge call");
    assert!(import < bridge, "{ir}");
    assert!(
        !ir.contains(&format!("call ptr @{EXT_OBJ_IMPORT_SYMBOL}(")),
        "{ir}"
    );
    let (raised, unbridged) = bridge_branch_labels(&ir);
    assert_eq!(raised, "try_handler_dispatch", "{ir}");
    assert_eq!(unbridged, "foreign_import_unbridged", "{ir}");
    assert!(
        ir.contains("store ptr %foreign_import, ptr @pyglobal_product"),
        "{ir}"
    );
}

/// #1383: each name of a nested multi-name from-import is stored before
/// the next name is imported, so a failure on a later name leaves the
/// earlier ones bound, as CPython's `IMPORT_FROM`/`STORE_NAME` pairs do.
#[test]
fn a_nested_multi_name_from_import_stores_each_name_before_the_next_import() {
    let ir = module_ir(
        "foreign_from_import_block_order",
        vec![MirItem::TopLevelStmt(MirStmt::If {
            test: MirExpr::BoolLiteral(true),
            body: vec![from_stmt(&["product", "chain"])],
            orelse: vec![],
        })],
    );
    let lines: Vec<&str> = ir.lines().collect();
    let position = |needle: &dyn Fn(&str) -> bool| -> Vec<usize> {
        lines
            .iter()
            .enumerate()
            .filter(|(_, line)| needle(line))
            .map(|(at, _)| at)
            .collect()
    };
    let call = format!("call ptr @{EXT_OBJ_IMPORT_FROM_SYMBOL}(");
    let calls = position(&|line| line.contains(&call));
    assert_eq!(calls.len(), 2, "{ir}");
    let stores = position(&|line| {
        line.contains("store ptr") && line.ends_with("ptr @pyglobal_product, align 8")
            || line.contains("store ptr") && line.ends_with("ptr @pyglobal_product")
    });
    assert_eq!(stores.len(), 1, "{ir}");
    assert!(calls[0] < stores[0] && stores[0] < calls[1], "{ir}");
    assert!(lines[calls[1]].ends_with("i64 2, i64 1, i64 0)"), "{ir}");
    assert_eq!(
        ir.matches(&format!("call i32 @{EXT_IMPORT_ERROR_BRIDGE_SYMBOL}()"))
            .count(),
        2,
        "one bridged failure edge per name: {ir}"
    );
}

/// #1383 with #1291's identical-pair exemption: the same from-import at
/// top level, then in both arms of an `if`/`else`, emits three calls that
/// each pass a fromlist global of their own, and the module verifies.
#[test]
fn an_identical_from_import_at_top_level_and_in_both_arms_verifies() {
    let ir = module_ir(
        "foreign_from_import_block_pair",
        vec![
            from_item(0),
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::BoolLiteral(true),
                body: vec![from_stmt(&["product"])],
                orelse: vec![from_stmt(&["product"])],
            }),
        ],
    );
    let calls: Vec<&str> = ir
        .lines()
        .filter(|line| line.contains(&format!("call ptr @{EXT_OBJ_IMPORT_FROM_SYMBOL}(")))
        .collect();
    assert_eq!(calls.len(), 3, "{ir}");
    // LLVM uniquifies each repeated global name, so every call must pass a
    // fromlist global of its own rather than the first emission's.
    let fromlists: std::collections::BTreeSet<&str> = calls
        .iter()
        .map(|call| {
            call.split("ptr @")
                .find(|arg| arg.starts_with("pycc_foreign_fromlist_product"))
                .and_then(|arg| arg.split(',').next())
                .expect("a fromlist argument")
        })
        .collect();
    assert_eq!(fromlists.len(), 3, "{ir}");
}

/// #1381: a plain dotted import calls `pycc_ext_obj_import_dotted` with
/// `bind_root` 1 for `import xml.dom` (bound to the root `xml`) and 0 for
/// `import email.utils as eu` (bound to the leaf), at top level and nested
/// in a module-level block alike; an undotted import keeps
/// `pycc_ext_obj_import`, and the dotted helper is declared once.
#[test]
fn a_dotted_import_calls_the_dotted_helper_with_its_root_flag() {
    let plain = |local: &str, module: &str| MirItem::ForeignImport {
        local_name: local.to_string(),
        module_path: module.to_string(),
        from: None,
    };
    let ir = module_ir(
        "foreign_import_dotted",
        vec![
            plain("xml", "xml.dom"),
            plain("eu", "email.utils"),
            plain("json", "json"),
            if_block_import(&[("os", "os.path"), ("sp", "os.path")]),
        ],
    );
    let calls: Vec<&str> = ir
        .lines()
        .filter(|line| line.contains(&format!("call ptr @{EXT_OBJ_IMPORT_DOTTED_SYMBOL}(")))
        .collect();
    assert_eq!(calls.len(), 4, "{ir}");
    for (call, (local, root)) in calls
        .iter()
        .zip([("xml", 1), ("eu", 0), ("os", 1), ("sp", 0)])
    {
        assert!(
            call.ends_with(&format!("(ptr @pycc_foreign_module_{local}, i64 {root})")),
            "{call}"
        );
    }
    assert!(
        ir.lines()
            .any(|line| line.starts_with("@pycc_foreign_module_xml ")
                && line.contains("c\"xml.dom\\00\"")),
        "the helper is passed the whole dotted name: {ir}"
    );
    assert_eq!(
        ir.matches(&format!(
            "declare ptr @{EXT_OBJ_IMPORT_DOTTED_SYMBOL}(ptr, i64)"
        ))
        .count(),
        1,
        "{ir}"
    );
    assert_eq!(
        ir.lines()
            .filter(|line| line.contains(&format!("call ptr @{EXT_OBJ_IMPORT_SYMBOL}(")))
            .count(),
        1,
        "only `import json` uses the undotted helper: {ir}"
    );
    assert!(
        ir.contains("store ptr %foreign_import, ptr @pyglobal_xml"),
        "{ir}"
    );
}
