//! #1340: rendering a CPython object with `print(o)` and `f"{o}"`.
//!
//! Every test compiles real MIR as an `ext` object -- LLVM's verifier runs
//! before any assertion here is believed -- and reads the LLVM text of the
//! module-exec entry point or of the user function `f` (mangled `pyfn_f`).
//! `tests/issue_1340_print_object.rs` runs the same shapes against CPython.

use crate::{
    CompileOptions, EXT_MODULE_EXEC_SYMBOL, EXT_OBJ_ERROR_BRIDGE_SYMBOL, EXT_OBJ_FORMAT_SYMBOL,
    EXT_OBJ_TO_STR_SYMBOL, compile_to_object_with_observer,
};
use inkwell::values::AnyValue;
use pycc_mir::{MirExpr, MirFStringPart, MirItem, MirModule, MirStmt, Ty};

/// `import numpy` -- the foreign module global every test reads.
fn import_numpy() -> MirItem {
    MirItem::ForeignImport {
        local_name: "numpy".to_string(),
        module_path: "numpy".to_string(),
        from: None,
    }
}

fn numpy() -> MirExpr {
    MirExpr::Name {
        name: "numpy".to_string(),
        ty: Ty::Object,
    }
}

/// `print(<args>)` as a statement.
fn print(args: Vec<MirExpr>) -> MirStmt {
    MirStmt::ExprStmt(MirExpr::Call {
        callee: "print".to_string(),
        args,
        ty: Ty::None,
    })
}

/// `f"<{numpy}>"`.
fn interpolate_numpy() -> MirExpr {
    MirExpr::FString(vec![
        MirFStringPart::Literal("<".to_string()),
        MirFStringPart::Interpolation(Box::new(numpy())),
        MirFStringPart::Literal(">".to_string()),
    ])
}

/// The LLVM text of the function `name` after compiling `import numpy`
/// plus `items` as an `ext` object.
fn function_ir(label: &str, items: Vec<MirItem>, name: &str) -> String {
    let dir = pycc_scratch::ScratchDir::new(label).expect("failed to create scratch dir");
    let mut all = vec![import_numpy()];
    all.extend(items);
    let mut ir = String::new();
    let mut observer = |module: &inkwell::module::Module<'_>, _: Option<&'static str>| {
        if let Some(function) = module.get_function(name) {
            ir = crate::llvm_string_to_owned(function.print_to_string());
        }
    };
    compile_to_object_with_observer(
        &MirModule {
            items: all,
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
    assert!(!ir.is_empty(), "no {name} was emitted");
    ir
}

/// The module-exec entry point after compiling `stmts` at module scope.
fn module_ir(label: &str, stmts: Vec<MirStmt>) -> String {
    function_ir(
        label,
        stmts.into_iter().map(MirItem::TopLevelStmt).collect(),
        EXT_MODULE_EXEC_SYMBOL,
    )
}

/// The byte offset of the first call to `symbol` in `ir`.
fn call_at(ir: &str, symbol: &str) -> usize {
    ir.find(&format!("@{symbol}("))
        .unwrap_or_else(|| panic!("no call to `{symbol}`:\n{ir}"))
}

/// `print(o)` renders through `str()` -- the shim's `PyObject_Str` helper
/// -- and never through `format()`, and a raising `__str__` takes the
/// module-exec failure edge.
#[test]
fn printing_an_object_calls_the_str_helper_and_can_fail() {
    let ir = module_ir("print_object", vec![print(vec![numpy()])]);
    assert!(ir.contains(&format!("@{EXT_OBJ_TO_STR_SYMBOL}(")), "{ir}");
    assert!(!ir.contains(EXT_OBJ_FORMAT_SYMBOL), "{ir}");
    assert!(ir.contains("foreign_to_str_fail:"), "{ir}");
    assert!(ir.contains("foreign_to_str_cont:"), "{ir}");
    // The converted text is written and then released, like any phase-1
    // conversion's temporary.
    assert!(call_at(ir.as_str(), "pycc_rt_print_write_str") > call_at(&ir, EXT_OBJ_TO_STR_SYMBOL));
    assert!(ir.contains("@pycc_rt_str_decref("), "{ir}");
}

/// `f"{o}"` renders through `format(o, '')` -- the shim's
/// `PyObject_Format` helper, which reaches `__format__` -- and never
/// through `str()`. The incref check sees the object operand itself, so no
/// `str` incref is emitted for it.
#[test]
fn interpolating_an_object_calls_the_format_helper_and_can_fail() {
    let ir = module_ir("fstring_object", vec![print(vec![interpolate_numpy()])]);
    assert!(ir.contains(&format!("@{EXT_OBJ_FORMAT_SYMBOL}(")), "{ir}");
    assert!(!ir.contains(EXT_OBJ_TO_STR_SYMBOL), "{ir}");
    assert!(!ir.contains("@pycc_rt_str_incref("), "{ir}");
    assert!(ir.contains("foreign_format_fail:"), "{ir}");
    assert!(ir.contains("foreign_format_cont:"), "{ir}");
    assert!(ir.contains("foreign_format_out = alloca ptr"), "{ir}");
}

/// Two interpolations in one module share one extern declaration of the
/// format helper (`obj_format_fn`'s early return); LLVM would rename a
/// duplicate declaration rather than reject it, so the suffixed spelling
/// must not appear.
#[test]
fn a_second_interpolation_reuses_the_one_format_declaration() {
    let ir = module_ir(
        "fstring_object_twice",
        vec![
            print(vec![interpolate_numpy()]),
            print(vec![interpolate_numpy()]),
        ],
    );
    assert_eq!(
        ir.matches(&format!("@{EXT_OBJ_FORMAT_SYMBOL}(")).count(),
        2,
        "{ir}"
    );
    assert!(!ir.contains(&format!("@{EXT_OBJ_FORMAT_SYMBOL}.")), "{ir}");
}

/// CPython's `print` evaluates every argument before converting any, so
/// the object's `str()` call comes *after* the later argument's call to a
/// user function -- and after the separator it follows, so the conversion
/// sits immediately before its own write.
#[test]
fn a_printed_object_is_converted_after_every_argument_is_evaluated() {
    let g = MirItem::Function {
        name: "g".to_string(),
        params: Vec::new(),
        return_ty: Ty::Int,
        body: vec![MirStmt::Return(Some(MirExpr::IntLiteral(1)))],
    };
    let call_g = MirExpr::Call {
        callee: "g".to_string(),
        args: Vec::new(),
        ty: Ty::Int,
    };
    let ir = function_ir(
        "print_object_order",
        vec![
            g,
            MirItem::TopLevelStmt(print(vec![
                MirExpr::StringLiteral("a".to_string()),
                numpy(),
                call_g,
                MirExpr::NoneLiteral,
            ])),
        ],
        EXT_MODULE_EXEC_SYMBOL,
    );
    // A module body reaches a user function through its function-pointer
    // slot, so the call is the one named `call_user_fn`.
    let g_at = ir
        .find("%call_user_fn = call")
        .unwrap_or_else(|| panic!("no call to `g`:\n{ir}"));
    let to_str_at = call_at(&ir, EXT_OBJ_TO_STR_SYMBOL);
    assert!(g_at < to_str_at, "{ir}");
    let first_write_at = call_at(&ir, "pycc_rt_print_write_str");
    assert!(
        first_write_at < to_str_at,
        "the literal is written first:\n{ir}"
    );
    let separator_at = ir[first_write_at..]
        .find("@pycc_rt_print_space(")
        .map(|at| at + first_write_at)
        .unwrap_or_else(|| panic!("no separator after the first write:\n{ir}"));
    assert!(separator_at < to_str_at, "{ir}");
    // The written line so far is flushed after the separator and before
    // CPython runs `__str__` (which may raise or write to its own stdout).
    let flush_at = call_at(&ir, "pycc_rt_print_flush");
    assert!(separator_at < flush_at && flush_at < to_str_at, "{ir}");
    assert_eq!(ir.matches("@pycc_rt_print_flush(").count(), 1, "{ir}");
    assert!(ir.contains("@pycc_rt_print_none("), "{ir}");
}

/// Only an object argument flushes: a native-only `print` and an
/// interpolated object -- which writes nothing until the whole f-string is
/// built -- emit no flush.
#[test]
fn only_a_printed_object_flushes_stdout() {
    let native = module_ir(
        "print_native_no_flush",
        vec![print(vec![
            MirExpr::StringLiteral("a".to_string()),
            MirExpr::IntLiteral(1),
        ])],
    );
    assert!(!native.contains("@pycc_rt_print_flush("), "{native}");
    let fstring = module_ir("fstring_no_flush", vec![print(vec![interpolate_numpy()])]);
    assert!(!fstring.contains("@pycc_rt_print_flush("), "{fstring}");
}

/// Inside a function body both renderings take the #1316 bridge instead of
/// the module-exec return, so a raising `__str__` or `__format__` is a
/// catchable pycc exception.
#[test]
fn rendering_an_object_in_a_function_body_bridges_its_failure() {
    let f = MirItem::Function {
        name: "f".to_string(),
        params: Vec::new(),
        return_ty: Ty::None,
        body: vec![
            print(vec![numpy()]),
            print(vec![interpolate_numpy()]),
            MirStmt::Return(None),
        ],
    };
    let ir = function_ir("render_object_in_function", vec![f], "pyfn_f");
    assert!(ir.contains(&format!("@{EXT_OBJ_TO_STR_SYMBOL}(")), "{ir}");
    assert!(ir.contains(&format!("@{EXT_OBJ_FORMAT_SYMBOL}(")), "{ir}");
    assert!(
        ir.contains(&format!("@{EXT_OBJ_ERROR_BRIDGE_SYMBOL}(")),
        "{ir}"
    );
    assert!(ir.contains("foreign_to_str_fail:"), "{ir}");
    assert!(ir.contains("foreign_format_fail:"), "{ir}");
    assert!(!ir.contains("ret i64"), "{ir}");
}
