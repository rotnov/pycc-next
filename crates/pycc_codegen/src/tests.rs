//! Unit tests for the codegen crate root.
//!
//! Relocated verbatim from `lib.rs`'s inline `#[cfg(test)] mod tests` under
//! AGENTS.md's decomposability rule (#545 Part 1). As a child module it still
//! sees the crate root's private items directly, so nothing needed widened
//! visibility; only the module's own location changed.

use super::*;
use pycc_mir::{
    BinOpKind, CmpOpKind, CompSource, InstantiateExpr, MirExceptHandler, MirExceptionValue,
    MirExpr, MirFStringPart, MirItem, MirModule, MirStmt, Ty,
};
use std::process::Command;

// #619 / AGENTS.md decomposability rule: the D-029 mechanical guard's
// checking logic and its automated synthetic-source tests live in their
// own cohesion-driven submodule instead of growing this already
// far-over-threshold file further. See that module's own doc comment.
mod d029_guard;

// Part 2 of #1175 (#1179): the buffer sub-range egress's own tests, in a
// cohesion-driven submodule for the same reason `d029_guard` has one.
mod buffer_slice_egress;

// #1212: chained-comparison codegen, in its own submodule for the same
// reason.
mod compare_chain;

// #1254: the shared entry-block placement helper.
mod entry_block;

// #1298: a caught exception's message ownership (print/f-string renderings
// and borrowed raise messages).
mod exception_message_rc;

// #1325: binding a CPython object value to a module-level name.
mod object_binding;

// Part 1 of #1333: passing a CPython object to a pycc function.
mod object_argument;
// #1418: CPython's `NotImplemented` singleton as a return value.
mod not_implemented;
mod object_compare;
// Part 1 of #1255: a list or set comprehension over a CPython object.
mod object_comprehension;
// Part 2d of #1371: a list display built as a CPython `list`.
mod object_list_display;
// #1457: storing and deleting an attribute of a CPython object.
mod object_attr_store;
// #1475: boxing a native value into an `object` slot.
mod object_box;
mod object_unbox;
// Part 2b of #1371: membership in, and slices of, a CPython object.
mod object_membership_slice;
mod object_raise;
mod object_return;
// Part 1 of #891: the run-time unpack of a CPython object.
mod object_unpack;
// Part 6 of #1371: `and`/`or` with a CPython object operand.
mod object_bool_op;
// Part 10 of #1371: `not` on a CPython object.
mod object_not;
// Part 11 of #1371: `type(o)` on a CPython object.
mod object_type;

// Topic submodules split out of this file.
mod toplevel;
mod print;
mod arith;
mod locals;
mod control_flow;
mod calls;
mod representation;
mod builtin_calls;
mod instances;
mod dicts_sets;
mod tuples;
mod optional_int;
mod optional_float_bool;
mod lists;
mod bool_identity;
mod float_and_str_ops;
mod fstrings;
mod comprehension_list_set;
mod comprehension_dict;
mod list_dict_methods;
mod enums_and_singletons;
mod exceptions_groups;
mod exceptions_try;
mod cpython_object;
mod buffers;
mod str_epilogue;
mod buffer_alloc;

/// `print(<n>)` as a `MirStmt` -- a convenience single-int-argument
/// shape reused by many of this file's older tests (`emit_stmt`'s
/// `print` dispatch itself now handles any number of arguments of any
/// v0.1 scalar type, plus `None` from direct user-function results,
/// D-075 parameter values, and assignment storage; see its own doc comment).
fn call_print(n: i64) -> MirStmt {
    MirStmt::ExprStmt(MirExpr::Call {
        callee: "print".to_string(),
        args: vec![MirExpr::IntLiteral(n)],
        ty: Ty::None,
    })
}

/// A zero-arg call to a user-defined function as a `MirStmt`.
fn call_user_fn(name: &str) -> MirStmt {
    MirStmt::ExprStmt(MirExpr::Call {
        callee: name.to_string(),
        args: vec![],
        ty: Ty::None,
    })
}

/// Builds the minimal `Context`/`Module`/`Builder`/`RtFns` set the
/// `Scalar::List` defensive-panic tests below need. None of them build
/// any IR -- every one panics inside its function's own `match` before
/// reaching a `build_*` call -- so unlike the hand-built-`StorageSlot`
/// tests above, none needs a function or a positioned basic block.
pub(crate) fn list_scalar_panic_fixture(
    context: &Context,
) -> (inkwell::module::Module<'_>, RtFns<'_>) {
    let module = context.create_module("test");
    let rt = declare_rt_functions(context, &module);
    (module, rt)
}

/// Wraps `body` in a `None`-returning function `f` that is then called
/// at top level -- the shape every `list[int]` MIR fixture below needs,
/// since a `list[int]` binding only becomes a function-local (rather
/// than a module global) when its assignment lives inside a function
/// body. Kept as a helper because each of these tests differs only in
/// the statements it puts inside.
pub(crate) fn list_fixture_module(body: Vec<MirStmt>) -> MirModule {
    MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body,
            },
            MirItem::TopLevelStmt(call_user_fn("f")),
        ],
        class_defs: Vec::new(),
    }
}

/// `xs = [1, 2, 3]` as a `MirStmt`, the prelude most of the `list[int]`
/// fixtures below open with.
fn assign_list_literal(target: &str) -> MirStmt {
    MirStmt::Assign {
        target: target.to_string(),
        value: MirExpr::ListLiteral(vec![
            MirExpr::IntLiteral(1),
            MirExpr::IntLiteral(2),
            MirExpr::IntLiteral(3),
        ]),
    }
}

/// Builds `print(<a bare expression>)` as a `MirStmt`, for the dict
/// codegen tests below -- like `call_print` above but for an arbitrary
/// argument expression rather than only an int literal.
fn print_expr(arg: MirExpr) -> MirStmt {
    MirStmt::ExprStmt(MirExpr::Call {
        callee: "print".to_string(),
        args: vec![arg],
        ty: Ty::None,
    })
}

fn dict_str_int() -> Ty {
    Ty::Dict(Box::new((Ty::Str, Ty::Int)))
}

fn dict_name(name: &str) -> MirExpr {
    MirExpr::Name {
        name: name.to_string(),
        ty: dict_str_int(),
    }
}

fn set_int() -> Ty {
    Ty::Set(Box::new(Ty::Int))
}

fn set_name(name: &str) -> MirExpr {
    MirExpr::Name {
        name: name.to_string(),
        ty: set_int(),
    }
}

fn instance_ty(class_name: &str) -> Ty {
    Ty::Instance(Box::new(class_name.to_string()))
}

/// A hand-built `Scalar::Tuple` for the four direct-call panic tests
/// below. The `list`/`dict`/`set` fixtures all use
/// `ptr_type().const_null()`, which has no tuple analogue -- a tuple is
/// a by-value struct, not a pointer (D-115) -- so this builds an
/// `undef` single-field struct instead. No builder is needed: `undef`
/// is a constant, and every function under test rejects the value
/// before doing anything with its contents.
fn tuple_scalar(context: &Context) -> Scalar<'_> {
    Scalar::Tuple(
        context
            .struct_type(&[context.i64_type().into()], false)
            .get_undef(),
    )
}

// --- `Optional[int]` (D-197, #763, Part 1 of #747) ---------------------

/// `int | None`, the one `Optional` shape real, type-checked source ever
/// constructs in this PR (`T0049` rejects every other inner type).
fn optional_int() -> Ty {
    Ty::Optional(Box::new(Ty::Int))
}

/// Builds `MirStmt::ForList { var: "v", list: <list>, body: [print(v)] }`,
/// the shape every list-comprehension test below uses to read its own
/// result back out: container `to_str`/`truthy` are unimplemented
/// (D-107/D-124), so a comprehension's own produced list cannot be
/// printed directly and must be walked element-by-element instead.
fn print_each_int(list: &str) -> MirStmt {
    MirStmt::ForList {
        var_ty: pycc_mir::Ty::Int,
        var: "v".to_string(),
        list: list.to_string(),
        body: vec![MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "v".to_string(),
                ty: Ty::Int,
            }],
            ty: Ty::None,
        })],
    }
}

/// Test-only linking helper. `pycc`'s real CLI (Task 8) does this via
/// `cc`/clang (see `src/main.rs`'s `linker_command`/`effective_link_target`/
/// `add_windows_system_libs`/`add_linux_system_libs`); duplicated
/// minimally here so pycc_codegen's own tests can prove the object file
/// it produces actually links and runs, without depending on the `pycc`
/// binary crate (that would be a dependency cycle: pycc depends on
/// pycc_codegen, not the other way around). Needs the same Windows
/// handling as `main.rs`, and for the same reasons: there's no default
/// `cc` there (D-028) -- on this runner it silently resolved to
/// MinGW's `gcc`, which cannot link the MSVC-ABI `pycc_rt.lib` (the
/// exact "undefined reference to `__imp_...`"/`collect2` wall D-028
/// already diagnosed for `main.rs`, reproduced here because this
/// helper wasn't covered by that fix); clang's bare-invocation default
/// target also proved unreliable (D-028), so `-target` must be
/// explicit too. Needs the same Linux handling too, for the same
/// reason `main.rs` does (`f64::powf` -> libm's `pow`, not linked by
/// GCC's/clang's default driver invocation): this helper's own
/// `undefined reference to 'pow'` failure on both Linux architectures
/// wasn't covered by that fix either, since it's a separate linker
/// invocation from `main.rs`'s.
pub(crate) fn link_object_with_runtime(obj_path: &std::path::Path, bin_path: &std::path::Path) {
    // `CARGO_MANIFEST_DIR` is this *crate*'s directory, so the
    // workspace root is two levels up; `parent()` twice rather than a
    // `..` join keeps the rendered path readable in linker errors.
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate manifest dir has a workspace-root grandparent");
    let target_root = crate::artifact_layout::resolve_cargo_target_root(
        workspace_root,
        crate::artifact_layout::cargo_target_dir_from_env,
    );
    let rt_lib_dir = crate::artifact_layout::find_pycc_rt_lib_dir_in(
        &target_root,
        None,
        false,
        std::path::Path::exists,
    )
    .expect("pycc_rt debug build must exist before these link-and-run tests");

    #[cfg(windows)]
    let mut cmd = {
        let clang = std::path::Path::new(env!("LLVM_SYS_221_PREFIX"))
            .join("bin")
            .join("clang.exe");
        let mut cmd = Command::new(clang);
        cmd.arg("-target").arg("x86_64-pc-windows-msvc");
        cmd
    };
    #[cfg(not(windows))]
    let mut cmd = Command::new("cc");

    cmd.arg(obj_path)
        .arg("-L")
        .arg(&rt_lib_dir)
        .arg("-lpycc_rt")
        .arg("-o")
        .arg(bin_path);

    #[cfg(windows)]
    for lib in [
        "ws2_32",
        "ntdll",
        "userenv",
        "advapi32",
        "shell32",
        "ole32",
        "uuid",
        "psapi",
        "dbghelp",
        "kernel32",
        "legacy_stdio_definitions",
    ] {
        cmd.arg(format!("-l{lib}"));
    }

    #[cfg(target_os = "linux")]
    cmd.arg("-lm");

    let status = cmd.status().expect("the linker driver should run");
    assert!(status.success(), "linking failed");
}

/// Helper: a `MirStmt::Return(Some(MirExpr::IntLiteral(n)))` — used in
/// function bodies to terminate the current block so that the enclosing
/// try's `*_falls_through` check is false.
fn return_int(n: i64) -> MirStmt {
    MirStmt::Return(Some(MirExpr::IntLiteral(n)))
}

// ---------------------------------------------------------------------------
// D-244, #1026: the `Scalar::Object` arms.
//
// Most arms below are defensive: `pycc_types`' Part 2 refusal migration
// rejects a `Ty::Object` operand at those consuming sites (see
// `docs/TYPE_SYSTEM.md`'s `object` row), so no type-checked program reaches
// them. Each is pinned directly with a hand-built `Scalar::Object` carrying
// a null `PyObject *` -- the same convention the `Scalar::List` defensive
// tests above use, and for the same reason: it pins the panic to the
// function that owns the gap rather than to whichever caller reaches it
// first. The two *reachable* `Ty::Object` paths (`MirStmt::Return`'s
// pass-through and `call_result_scalar`'s arm) are covered by real MIR at
// the end of this section instead.
//
// `truthy`'s arm is the exception, and PR 3a of #1082 is what made it one:
// it no longer panics, because the ten condition-position refusals that
// justified the panic are gone. Its test below therefore asserts the shim
// call it emits rather than a message.
// ---------------------------------------------------------------------------

/// A null `PyObject *` as a [`Scalar::Object`]. None of the defensive tests
/// below dereferences it -- each panics inside its function's own `match`
/// before any `build_*` call runs.
fn null_object_scalar(context: &Context) -> Scalar<'_> {
    Scalar::Object(
        context
            .ptr_type(inkwell::AddressSpace::default())
            .const_null(),
    )
}

/// `import numpy` spliced ahead of `items`, so a `MirExpr::Name` read of
/// `numpy` typed `Ty::Object` resolves to the module global the foreign
/// import emits.
fn with_foreign_numpy(mut items: Vec<MirItem>) -> Vec<MirItem> {
    items.insert(
        0,
        MirItem::ForeignImport {
            local_name: "numpy".to_string(),
            module_path: "numpy".to_string(),
            from: None,
        },
    );
    items
}

/// `numpy.pi` as a `MirExpr`, the one producer shape PR 2a emits.
fn numpy_pi() -> MirExpr {
    MirExpr::ObjAttrGet {
        base: Box::new(MirExpr::Name {
            name: "numpy".to_string(),
            ty: Ty::Object,
        }),
        attr: "pi".to_string(),
        ty: Ty::Object,
    }
}

/// Compiles `items` as a D-244 `ext` object, which is the only mode a
/// foreign import is emitted in.
fn compile_ext_items(label: &str, items: Vec<MirItem>) {
    let dir = pycc_scratch::ScratchDir::new(label).expect("failed to create scratch dir");
    compile_to_object_with_options(
        &MirModule {
            items,
            ..Default::default()
        },
        &dir.join(format!("{label}.o")),
        &CompileOptions {
            ext: true,
            ..CompileOptions::default()
        },
    )
    .expect("ext codegen should succeed");
}

/// An `--ext` module holding one function whose first parameter is a
/// `memoryview`, which is the only shape that binding can have.
fn buffer_fn_items(body: Vec<MirStmt>, return_ty: Ty) -> Vec<MirItem> {
    vec![MirItem::Function {
        name: "element".to_string(),
        params: vec![
            ("b".to_string(), Ty::MemoryView),
            ("i".to_string(), Ty::Int),
        ],
        return_ty,
        body,
    }]
}

/// `b[i]` as a `MirExpr`, the one buffer shape Part 2 of #1027 emits.
fn buffer_get_b_i() -> MirExpr {
    MirExpr::BufferGet {
        base: Box::new(MirExpr::Name {
            name: "b".to_string(),
            ty: Ty::MemoryView,
        }),
        index: Box::new(MirExpr::Name {
            name: "i".to_string(),
            ty: Ty::Int,
        }),
    }
}

/// Compiles `items` as a D-244 `ext` object and hands the whole module's IR
/// to `check`.
fn compile_ext_items_checking_ir(label: &str, items: Vec<MirItem>, check: impl Fn(&str)) {
    let dir = pycc_scratch::ScratchDir::new(label).expect("failed to create scratch dir");
    let mut seen = String::new();
    // D-029: never let the `LLVMString` temporary drop -- route it through
    // `llvm_string_to_owned`, exactly as this crate's error paths do.
    let mut observer = |module: &inkwell::module::Module<'_>, _| {
        seen = llvm_string_to_owned(module.print_to_string());
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
    check(&seen);
}

// ---------------------------------------------------------------------------
// #1054: every entry-block `str` slot is released on every exit path.
//
// These live in-crate deliberately. The hosted end-to-end probe
// (`tests/issue_1054_ext_str_release.rs`) is `#[ignore]`d and `cargo
// llvm-cov` runs without `--include-ignored`, so it contributes no line
// coverage at all; D-014/D-242's 100%-changed-line gate over the new
// `pycc_str_epilogue` emission is satisfied here and nowhere else. One test
// per exit family, matching the plan's own case list (a)-(j).
// ---------------------------------------------------------------------------

/// Compiles `items` as an ordinary (non-`ext`) object and hands the whole
/// module's IR to `check`. The non-`ext` sibling of
/// `compile_ext_items_checking_ir` above, and it observes the same D-029
/// rule: the `LLVMString` temporary must never be dropped.
fn compile_items_checking_ir(label: &str, items: Vec<MirItem>, check: impl Fn(&str)) {
    let dir = pycc_scratch::ScratchDir::new(label).expect("failed to create scratch dir");
    let mut seen = String::new();
    let mut observer = |module: &inkwell::module::Module<'_>, _| {
        seen = llvm_string_to_owned(module.print_to_string());
    };
    compile_to_object_with_observer(
        &MirModule {
            items,
            ..Default::default()
        },
        &dir.join(format!("{label}.o")),
        &CompileOptions::default(),
        Some(&mut observer),
    )
    .expect("the #1054 str-epilogue fixtures must all compile");
    check(&seen);
}

fn str_name(name: &str) -> MirExpr {
    MirExpr::Name {
        name: name.to_string(),
        ty: Ty::Str,
    }
}

// ---------------------------------------------------------------------------
// Part 2a of #1142 (#1165): `MirExpr::BufferAlloc` and the owned-slot frame.
//
// The second source of a `Ty::MemoryView` value, and the first that the
// artifact itself owns. What has to hold is a lifecycle rather than a single
// emission: the slot is null-initialized at entry, the allocation stores into
// it, a reassignment frees the old view first, and the epilogue frees
// whatever the slot holds on every return path -- including a path on which
// the allocation never ran, where the slot still holds the entry null and
// `pycc_rt_buffer_f64_free`'s documented null no-op is what makes it safe.
// ---------------------------------------------------------------------------

/// `a = ndarray(4)` as a `MirStmt`, the one producer shape #1165 emits.
fn buffer_alloc_a() -> MirStmt {
    MirStmt::Assign {
        target: "a".to_string(),
        value: MirExpr::BufferAlloc {
            len: Box::new(MirExpr::IntLiteral(4)),
        },
    }
}
