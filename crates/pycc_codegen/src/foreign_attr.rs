//! Emission for `MirExpr::ObjAttrGet` (Part 2 of #1026, PR 2a of #1081).
//!
//! The counterpart to `foreign_import.rs`: that module knows how a foreign
//! `import numpy` becomes a module object in a global, this one knows how
//! `numpy.pi` becomes a `PyObject_GetAttrString` call on it. Carved out of
//! `lib.rs` for the same reason (AGENTS.md's "Keep source files
//! decomposable"; the tracker for the rest of `lib.rs` is #545), and kept
//! separate from `foreign_import.rs` because the two sit at different
//! layers -- an *item* emitted once from `compile_to_object`'s item loop
//! versus an *expression* emitted wherever `emit_expr` reaches it.
//!
//! **Ownership** (`docs/RUNTIME.md`). `pycc_ext_obj_getattr` returns a new
//! reference and nothing here releases it. Whoever consumes the result
//! decides: since Part 1 of #1092 an *unbound temporary* -- the operand of
//! another object operation, a condition, a conversion, a discarded
//! statement value, the iterable of a `for` or a comprehension -- is
//! released by its consumer (`object_release.rs`), and since Part 1 of
//! #1499 a module global owns a result bound to it and releases it on
//! rebind (`object_slot.rs`), while a result bound to a function-local name
//! or an instance slot, passed to a user function, returned, or boxed is
//! still leaked: this boundary's leak-only rule, which later parts of #1499
//! narrow further. Such a leak inside a
//! loop is trip-count-linear rather than once per process.

use super::*;
use crate::foreign_fail::{ForeignFailEdge, route_null};
use inkwell::builder::Builder;

/// Declares the shim's
/// `PyObject *pycc_ext_obj_getattr(PyObject *, const char *)` once per
/// module, returning the existing declaration on every later call --
/// `foreign_import.rs`'s `obj_import_fn` pattern exactly, and for the same
/// reason: a second declaration of one name is an LLVM module-verifier
/// error.
fn obj_getattr_fn<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
) -> FunctionValue<'ctx> {
    if let Some(existing) = module.get_function(EXT_OBJ_GETATTR_SYMBOL) {
        return existing;
    }
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    module.add_function(
        EXT_OBJ_GETATTR_SYMBOL,
        ptr.fn_type(&[ptr.into(), ptr.into()], false),
        None,
    )
}

/// The object operand of a foreign-object operation -- for example an
/// attribute load's or method call's base, a direct call's callee, a
/// subscript's base, a `for` loop's iterable, or the operand of `len`, a
/// truth test, a conversion or a tuple unpack (`foreign_len`) -- as a
/// `PyObject *`.
///
/// `pycc_mir`'s lowering builds each of those nodes only where the
/// operand's inferred type is `Ty::Object` (for example `pycc_mir::expr`'s
/// `HirExpr::AttrGet` arm, and its `HirExpr::Call` arm for `ObjCall`,
/// #1313), and `ty_to_basic_type` maps that to a pointer, so every other
/// `Scalar` here is a lowering defect rather than a program the front end
/// let through.
pub(super) fn expect_object_pointer(scalar: Scalar<'_>) -> PointerValue<'_> {
    let Scalar::Object(ptr) = scalar else {
        panic!(
            "pycc_codegen: internal error: a foreign-object operand did not evaluate to a \
             CPython object -- pycc_mir lowers every foreign-object operation only for a \
             `Ty::Object` operand"
        )
    };
    ptr
}

/// Emits one `obj.attr` load against a CPython object and yields the
/// attribute's value as an opaque [`Scalar::Object`].
///
/// # Which side owns the "CPython raised" transition
///
/// `pycc_ext_obj_getattr` returns `NULL` with *CPython's* error indicator
/// set. That is a second failure protocol next to pycc's own pending state
/// (D-173, `pycc_rt_exception_active`), and the standard pending-exception
/// guard `emit_expr` emits right after this call -- `expression_can_set_exception`
/// answers `true` for this node -- reads pycc's state, which a CPython-set
/// exception leaves untouched. It therefore takes the no-exception edge and
/// cannot stop the module body on its own. PR 2a's own review found what
/// that costs: `import numpy` followed by `numpy.definitely_not_there` ran
/// the whole module body to completion and CPython reported `SystemError:
/// execution of module n raised unreported exception` instead of the real
/// `AttributeError`.
///
/// So this arm emits the `NULL` check itself and routes the failure through
/// [`ForeignFailEdge`] (#1316), whose edge depends on the function being
/// emitted into:
///
/// - in `pycc_ext_module_exec` with no module-level `try` enclosing the
///   load it is the **module-exec failure edge**: return
///   [`EXT_MODULE_EXEC_FAILED`] immediately, leaving CPython's own
///   exception set and unmodified, so the interpreter reports the real
///   exception;
/// - anywhere else -- inside a module-level `try` (Part 1 of #1096) or in
///   any other function (a user function reading a module-level foreign
///   name) -- it calls `pycc_ext_obj_error_bridge`, which moves CPython's
///   exception into pycc's pending state, and branches to the innermost
///   exception target -- the enclosing `try`'s handler, or the function's
///   own `exception_exit` -- exactly as a failed pycc operation does.
///
/// `foreign_fail::emit_failure` owns that rule.
///
/// The nested case (`a.b.c` nests this node inside itself) is answered
/// twice over: this check stops the outer load from ever seeing the inner
/// `NULL`, and the shim's own NULL-`obj` guard in
/// `src/ext/pycc_ext_module.c` still holds the line for any caller that
/// reaches it another way.
pub(super) fn emit<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    base: Scalar<'ctx>,
    attr: &str,
) -> Scalar<'ctx> {
    let edge = ForeignFailEdge::for_current(builder);
    let getattr = obj_getattr_fn(context, module);
    let base_ptr = expect_object_pointer(base);
    let name = builder
        .build_global_string_ptr(attr, &format!("pycc_foreign_attr_{attr}"))
        .expect("build_global_string_ptr should not fail")
        .as_pointer_value();
    let loaded = builder
        .build_call(getattr, &[base_ptr.into(), name.into()], "foreign_attr")
        .expect("build_call should not fail for pycc_ext_obj_getattr")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_getattr returns PyObject *")
        .into_pointer_value();
    route_null(context, builder, module, rt, edge, loaded, "foreign_attr");
    Scalar::Object(loaded)
}

/// The function `builder` is currently emitting into, once it is known to
/// be the `--ext` module-body entry point.
///
/// Since #1316 only the operations that remain module-body-only call this:
/// `foreign_call.rs`'s `for` loop over a foreign iterable and
/// `foreign_len.rs`'s float-tuple unpack, both of which `pycc_types` still
/// admits only at module scope. Every other foreign operation routes its
/// failure through [`ForeignFailEdge`], which works in any function.
///
/// The guard runs *before* any block is appended: those callers build a
/// [`ForeignFailEdge::ModuleExec`] edge, which returns `i64 -1` when no
/// module-level `try` encloses the operation, so emitting it into a
/// function with a different return type would be an LLVM verifier error
/// rather than a diagnosable one. Reaching it from anywhere else is a
/// front-end defect: the operations are admitted only in a module body.
pub(super) fn expect_module_exec_entry<'ctx>(builder: &Builder<'ctx>) -> FunctionValue<'ctx> {
    let function = builder
        .get_insert_block()
        .expect("the builder is positioned inside a block")
        .get_parent()
        .expect("every basic block belongs to a function");
    if function.get_name().to_bytes() != EXT_MODULE_EXEC_SYMBOL.as_bytes() {
        panic!(
            "pycc_codegen: internal error: a module-body-only operation on a CPython object \
             was emitted outside `{EXT_MODULE_EXEC_SYMBOL}` -- pycc_types admits a foreign \
             `for` loop and a float-tuple unpack only in a module body"
        )
    }
    function
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CompileOptions, EXT_MODULE_EXEC_SYMBOL, compile_to_object_with_observer};
    use inkwell::values::AnyValue;
    use pycc_mir::{MirExpr, MirItem, MirModule, MirStmt, Ty};

    /// `import <module>` followed by one discarded `<module>.<attr>` load.
    ///
    /// A discarded `ExprStmt` is used rather than an assignment because it
    /// was the shape PR 2a admitted end to end: `pycc_types` then refused
    /// binding a CPython object to a name (`I0404`; admitted at module scope
    /// since #1325 and in a function body since Part 1 of #1333), and the
    /// discarded load keeps these tests independent of the store.
    fn load(module: &str, attr: &str) -> Vec<MirItem> {
        vec![
            MirItem::ForeignImport {
                local_name: module.to_string(),
                module_path: module.to_string(),
                from: None,
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::ObjAttrGet {
                base: Box::new(MirExpr::Name {
                    name: module.to_string(),
                    ty: Ty::Object,
                }),
                attr: attr.to_string(),
                ty: Ty::Object,
            })),
        ]
    }

    /// The LLVM text of the module-exec entry point after compiling `items`
    /// as an `ext` object -- `foreign_import.rs`'s own `entry_ir`, which is
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

    /// The call goes to the shared constant's symbol, and the attribute
    /// name reaches it as a global string.
    ///
    /// The symbol is asserted through [`EXT_OBJ_GETATTR_SYMBOL`] rather
    /// than against a literal on purpose: the C definition in
    /// `src/ext/pycc_ext_module.c` and this declaration are resolved
    /// lazily at load time, so a literal spelled twice would be a crash at
    /// first call rather than a link error, and a test that spelled it a
    /// third time would agree with neither.
    #[test]
    fn a_foreign_attribute_load_calls_the_shim_helper_by_its_shared_symbol() {
        let ir = entry_ir("foreign_attr_call", load("numpy", "pi"));
        assert!(ir.contains(EXT_OBJ_GETATTR_SYMBOL), "{ir}");
        assert!(ir.contains("pycc_foreign_attr_pi"), "{ir}");
    }

    /// A failed lookup stops the module body on the module-exec failure
    /// edge, rather than continuing with a `NULL` object.
    ///
    /// PR 2a's review finding 1: without this, `numpy.definitely_not_there`
    /// left CPython's `AttributeError` set, ran the rest of the module
    /// body, and surfaced as `SystemError: execution of module n raised
    /// unreported exception`. The assertion is on the emitted edge --
    /// a null test on the shim's result, and a `ret i64 -1`
    /// ([`EXT_MODULE_EXEC_FAILED`]) on its failure arm, which is the same
    /// edge `foreign_import.rs` takes for a failed import.
    #[test]
    fn a_failed_attribute_load_returns_on_the_module_exec_failure_edge() {
        let ir = entry_ir("foreign_attr_fail_edge", load("numpy", "pi"));
        assert!(ir.contains("foreign_attr_failed"), "{ir}");
        assert!(ir.contains("foreign_attr_fail:"), "{ir}");
        assert!(ir.contains("foreign_attr_cont:"), "{ir}");
        assert!(
            ir.contains(&format!("ret i64 {EXT_MODULE_EXEC_FAILED}")),
            "{ir}"
        );
    }

    /// Two attribute loads in one module share one extern declaration.
    ///
    /// `obj_getattr_fn` returns the existing `FunctionValue` on every call
    /// after the first; a second `add_function` of one name is an LLVM
    /// module-verifier error, so the second load is what proves the early
    /// return is taken rather than merely present.
    #[test]
    fn a_second_attribute_load_reuses_the_one_extern_declaration() {
        let mut items = load("numpy", "pi");
        items.extend(load("scipy", "e"));
        let ir = entry_ir("foreign_attr_twice", items);
        let mut calls = 0usize;
        let mut rest = ir.as_str();
        while let Some(at) = rest.find(EXT_OBJ_GETATTR_SYMBOL) {
            calls += 1;
            rest = &rest[at + EXT_OBJ_GETATTR_SYMBOL.len()..];
        }
        assert_eq!(calls, 2, "one call site per load: {ir}");
    }

    /// The defensive arm in [`expect_object_pointer`]: only `pycc_mir`'s
    /// own lowering builds this node, and only over a `Ty::Object` base, so
    /// any other `Scalar` is a lowering defect. Reached here by handing the
    /// node an `int` base directly, which no `lower_expr` path produces.
    #[test]
    #[should_panic(expected = "did not evaluate to a CPython object")]
    fn a_non_object_base_is_an_internal_error() {
        entry_ir(
            "foreign_attr_bad_base",
            vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(
                MirExpr::ObjAttrGet {
                    base: Box::new(MirExpr::IntLiteral(1)),
                    attr: "pi".to_string(),
                    ty: Ty::Object,
                },
            ))],
        );
    }
}
