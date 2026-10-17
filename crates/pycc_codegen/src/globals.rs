//! Module globals: declaration of module-level slots and enum member singletons.

use super::*;

pub(crate) fn declare_module_globals<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    bindings: &BTreeMap<String, pycc_mir::Ty>,
) -> BTreeMap<String, StorageSlot<'ctx>> {
    bindings
        .iter()
        .map(|(name, ty)| {
            let (storage_ty, initializer): (
                inkwell::types::BasicTypeEnum,
                inkwell::values::BasicValueEnum,
            ) = match ty {
                pycc_mir::Ty::Int | pycc_mir::Ty::Num => (
                    context.i64_type().into(),
                    tag_smallint_const(context, 0).into(),
                ),
                pycc_mir::Ty::Bool => (
                    context.i8_type().into(),
                    context.i8_type().const_zero().into(),
                ),
                // D-075's canonical unit carrier extends to ordinary
                // assignment storage: `None` has no payload, so an `i8 0`
                // is sufficient. The separate `initialized` flag created
                // below, not this zero initializer, distinguishes a binding
                // whose assignment has executed from an uninitialized one.
                pycc_mir::Ty::None => (
                    context.i8_type().into(),
                    context.i8_type().const_zero().into(),
                ),
                pycc_mir::Ty::Float => (
                    context.f64_type().into(),
                    context.f64_type().const_zero().into(),
                ),
                pycc_mir::Ty::Str => (
                    context.ptr_type(inkwell::AddressSpace::default()).into(),
                    context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null()
                        .into(),
                ),
                // Identical storage to `Ty::Str` directly above: an opaque
                // pointer, null until the first assignment stores a real
                // `PyListObj` into it, with the separate `initialized`
                // flag below (which every module global gets) trapping any
                // read that reaches it first.
                //
                // Task 5 (D-089) deliberately left this arm out and let a
                // module-level `list[int]` binding hit the catch-all below,
                // on the grounds that no real source could construct a list
                // value at all yet; its own report flagged the interaction
                // for Task 11 to re-derive. Task 11b makes `x = [1, 2, 3]`
                // constructible from real source, and D-105's first scope
                // cut names module scope as one of the two places a
                // `list[int]` value is expected to live -- so leaving it out
                // now would turn that documented, supported form into an
                // internal compiler panic. No exit-time decref accompanies
                // it (contrast the `Ty::Str` loop in `compile_to_object`):
                // D-107 keeps `list[T]` leak-only for v0.2.
                pycc_mir::Ty::List(_) | pycc_mir::Ty::VarTuple(_) => (
                    context.ptr_type(inkwell::AddressSpace::default()).into(),
                    context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null()
                        .into(),
                ),
                // Identical storage and reasoning to `Ty::List(_)`
                // directly above (PR-11 Task 5): an opaque pointer, null
                // until the first assignment stores a real `PyDictObj`
                // into it, with the separate `initialized` flag below
                // trapping any read that reaches it first. D-123 names
                // module scope as one of the places a `dict[str, int]`
                // value is expected to live, and every one of this task's
                // own CLI repro programs assigns `x = {...}` at module
                // scope -- leaving this arm out would turn that documented,
                // supported form into an internal compiler panic. No
                // exit-time decref accompanies it (contrast the `Ty::Str`
                // loop in `compile_to_object`): D-124 keeps `dict[K, V]`
                // leak-only for v0.2, extending D-107's exact reasoning.
                pycc_mir::Ty::Dict(_) => (
                    context.ptr_type(inkwell::AddressSpace::default()).into(),
                    context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null()
                        .into(),
                ),
                // Identical storage and reasoning to `Ty::List(_)`/
                // `Ty::Dict(_)` directly above (PR-11 Task 9): an opaque
                // pointer, null until the first assignment stores a real
                // `PyIntSetObj` into it, with the separate `initialized`
                // flag below trapping any read that reaches it first.
                // D-123 names module scope as one of the places a
                // `set[int]` value is expected to live, and every one of
                // this task's own CLI repro programs assigns `x = {...}` at
                // module scope -- leaving this arm out would turn that
                // documented, supported form into an internal compiler
                // panic. No exit-time decref accompanies it (contrast the
                // `Ty::Str` loop in `compile_to_object`): D-124 keeps
                // `set[T]` leak-only for v0.2, extending D-107's exact
                // reasoning.
                pycc_mir::Ty::Set(_) | pycc_mir::Ty::FrozenSet(_) => (
                    context.ptr_type(inkwell::AddressSpace::default()).into(),
                    context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null()
                        .into(),
                ),
                // `tuple[...]`'s own storage (PR-11b Task 5, D-115), and
                // the one arm here that is not a nullable pointer: the
                // struct type itself, stored inline in the global, zero-
                // initialized via `const_zero()` (which zero-fills every
                // field whatever the arity and field types -- `i64`/`i8`/
                // `f64` all have a well-defined zero).
                //
                // That zero is deliberately *not* a sentinel, unlike the
                // null pointer the four arms above use. A zeroed struct is
                // indistinguishable from a legitimately-zero tuple, so it
                // could never mark "unassigned" on its own. Nothing is
                // lost: read-before-first-assignment is trapped by the
                // separate `initialized` flag every module global already
                // gets just below -- which is the real guard for the
                // pointer arms too, the null merely being a redundant
                // second signal there. D-116 names module scope as a place
                // a tuple value is expected to live (`x = (1, 2)` at top
                // level is the canonical form), so omitting this arm would
                // turn that supported form into an internal compiler panic.
                // No exit-time decref accompanies it (contrast the
                // `Ty::Str` loop in `compile_to_object`): a tuple owns no
                // allocation to release.
                pycc_mir::Ty::Tuple(elements) if pycc_hir::tuple_is_boxed(elements) => (
                    context.ptr_type(inkwell::AddressSpace::default()).into(),
                    context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null()
                        .into(),
                ),
                pycc_mir::Ty::Tuple(_) => {
                    let struct_ty = ty_to_basic_type(context, ty.clone()).into_struct_type();
                    (struct_ty.into(), struct_ty.const_zero().into())
                }
                // Identical storage and reasoning to `Ty::List(_)`/
                // `Ty::Dict(_)`/`Ty::Set(_)` above (D-154, Part 1 of #375):
                // an opaque pointer, null until the first assignment (an
                // instantiation, or a call/attribute-read returning an
                // instance) stores a real `PyInstanceObj` into it, with the
                // separate `initialized` flag below trapping any read that
                // reaches it first. `p = Point(1, 2)` at module scope is
                // exactly the shape Task 8's own conformance fixture uses --
                // omitting this arm would turn that supported form into an
                // internal compiler panic. No exit-time decref accompanies
                // it: `pycc_rt::instance` is leak-only, mirroring `List`/
                // `Dict`/`Set`.
                pycc_mir::Ty::Instance(_) => (
                    context.ptr_type(inkwell::AddressSpace::default()).into(),
                    context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null()
                        .into(),
                ),
                // Part 1 of #1026: identical storage and reasoning to
                // `Ty::Instance(_)` directly above -- an opaque pointer,
                // null until some store of a CPython object reference
                // fills it -- for example `pycc_ext_obj_import` (the
                // imported module), `pycc_ext_obj_import_from` (a
                // `from X import a` member, #1278), `emit_assign`'s object
                // store (a module-level `x = <object>`, #1325) or a
                // module-level `for` target's own store in the
                // `MirStmt::ForObject` emission -- with the separate `initialized`
                // flag below trapping any read that reaches it first. The
                // slot owns its one reference and a rebind releases the old
                // value (`heap_rc`). No
                // exit-time release accompanies it: the last value is
                // owned for the artifact's lifetime (see
                // `foreign_import.rs` and `docs/RUNTIME.md`).
                pycc_mir::Ty::Object => (
                    context.ptr_type(inkwell::AddressSpace::default()).into(),
                    context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null()
                        .into(),
                ),
                // `int | None` (D-197, #763, Part 1 of #747): stored inline
                // like `Tuple`'s own struct arm immediately above, not as a
                // nullable pointer -- an `Optional[inner]` value is an LLVM
                // aggregate, exactly like a tuple, not a heap object. `x:
                // int | None = 5` at module scope is exactly the shape this
                // PR's own conformance fixture uses, so omitting this arm
                // would turn that supported form into an internal compiler
                // panic, the same reasoning `Tuple`'s own arm gives.
                //
                // Unlike `Tuple`'s plain `const_zero()`, the initializer's
                // payload field is built as a valid D-141-encoded smallint
                // `0` (`tag_smallint_const`), not a raw zero word -- for the
                // identical branch-free-masked-read reason
                // `default_value_for_type`'s own `Ty::Optional` arm and
                // `coerce_scalar_to_type`'s placeholder-normalization arm
                // are: `truthy`'s `Scalar::Optional` arm ANDs the payload's
                // truthiness with the present flag unconditionally rather
                // than branching around it, and a global read before its
                // first real assignment (guarded only by the separate
                // `initialized` flag below, not by never reaching this
                // value at all) must not hand that AND a raw zero word that
                // trips `classify_encoded_int`'s fail-closed panic. The
                // present flag itself is `0` either way, matching every
                // "unassigned" global's own zero-flag/null-pointer pattern
                // above.
                // A pointer-represented `T | None`: the `T` pointer, null.
                pycc_mir::Ty::Optional(inner) if pycc_hir::is_pointer_optional_inner(inner) => (
                    context.ptr_type(inkwell::AddressSpace::default()).into(),
                    context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null()
                        .into(),
                ),
                pycc_mir::Ty::Optional(inner) => {
                    let struct_ty = ty_to_basic_type(context, ty.clone()).into_struct_type();
                    // #809: `default_value_for_type` (below) already draws
                    // this same `Ty::Int`-vs-other-inner-type distinction
                    // for exactly the same reason -- only `Ty::Int`'s
                    // payload field is the D-141 encoded-word
                    // representation `tag_smallint_const` produces; a
                    // `Float`/`Bool` inner type's payload field is a plain
                    // `f64`/`i8` value, and handing `tag_smallint_const`'s
                    // `i64` constant to `const_named_struct` for a struct
                    // whose first field is `f64`/`i8` is an LLVM constant
                    // type/size mismatch (the `Optional[bool]` case was
                    // reproduced crashing the LLVM backend with "invalid
                    // number of bytes" before this fix). Reuse
                    // `default_value_for_type` itself so this initializer
                    // and the per-local entry-block placeholder in
                    // `storage_slot_at_entry` can never drift apart again.
                    let payload = match inner.as_ref() {
                        pycc_mir::Ty::Int => tag_smallint_const(context, 0).into(),
                        other_inner => default_value_for_type(context, other_inner.clone()),
                    };
                    let zeroed = struct_ty
                        .const_named_struct(&[payload, context.i8_type().const_zero().into()]);
                    (struct_ty.into(), zeroed.into())
                }
                // A generator object: a pointer, null until bound.
                pycc_mir::Ty::Generator(_) | pycc_mir::Ty::Callable(_) => (
                    context.ptr_type(inkwell::AddressSpace::default()).into(),
                    context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null()
                        .into(),
                ),
                other => panic!(
                    "pycc_codegen: a `{}`-typed module binding is not supported yet",
                    other.name()
                ),
            };
            let global = module.add_global(storage_ty, None, &format!("pyglobal_{name}"));
            global.set_linkage(Linkage::Internal);
            global.set_initializer(&initializer);
            let initialized =
                module.add_global(context.i8_type(), None, &format!("pyglobal_init_{name}"));
            initialized.set_linkage(Linkage::Internal);
            initialized.set_initializer(&context.i8_type().const_zero());
            (
                name.clone(),
                StorageSlot {
                    ptr: global.as_pointer_value(),
                    ty: ty.clone(),
                    initialized: Some(initialized.as_pointer_value()),
                },
            )
        })
        .collect()
}

/// #379 (PR-19): Emit per-enum-member singleton init sequences. Each enum
/// member is a compile-time singleton instance that must be alive before
/// any top-level code reads it. For each enum class with members, and each
/// member in source order, allocate a fresh 2-slot instance
/// (`pycc_rt_instance_new(2, NULL, 0)` -- no layout descriptor, since both
/// slots are assigned right here, before any code can read them), set slot 0 to the member's `int` or `str`
/// value per its `EnumMemberValue` (#892 widened this from `int` alone),
/// set slot 1 to a string pointer containing the member name, and store
/// the instance pointer into the synthetic global
/// `<Class>.<Member>.enum_member`. Extracted from
/// `compile_to_object_with_observer` to isolate the enum-specific code
/// paths (see cargo-llvm-cov#276 for the coverage instantiation issue).
pub(crate) fn emit_enum_member_inits<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    mir: &MirModule,
    module_globals: &BTreeMap<String, StorageSlot<'ctx>>,
) {
    for (class_name, class_def) in &mir.class_defs {
        for (member_name, member_value) in &class_def.enum_members {
            let global_name = format!("{class_name}.{member_name}.enum_member");
            let slot = &module_globals[&global_name];
            // Allocate a fresh 2-slot instance.
            let count = context.i64_type().const_int(2, false);
            let no_layout = context
                .ptr_type(inkwell::AddressSpace::default())
                .const_null();
            let no_layout_len = context.i64_type().const_zero();
            let instance_ptr = builder
                .build_call(
                    rt.instance_new,
                    &[count.into(), no_layout.into(), no_layout_len.into()],
                    "enum_instance_new",
                )
                .expect("build_call should not fail for a well-formed enum member allocation")
                .try_as_basic_value()
                .expect_basic("pycc_rt_instance_new returns a non-void pointer")
                .into_pointer_value();
            // Set slot 0 to the member's own value literal, carried in
            // `HirClassDef.enum_members` from HIR through MIR to here.
            //
            // For an `int` member (`RED = 1` → 1), `emit_int_constant` folds
            // the value at compile time into the tagged-pointer
            // representation `pycc_rt` uses for small ints, or materializes
            // a heap bigint through `pycc_rt_int_from_i64` when the
            // discriminant is outside the tagged 63-bit range (D-178) --
            // once, here at module init.
            //
            // For a `str` member (#892: `RED = "red"`, or any member of an
            // `enum.StrEnum` subclass), `emit_string_literal` interns the
            // bytes as a private constant and calls
            // `pycc_rt_str_from_literal` -- the same helper slot 1 (the
            // member's `name`) already uses just below.
            let value_scalar = match member_value {
                pycc_mir::EnumMemberValue::Int(value) => {
                    Scalar::Int(emit_int_constant(context, builder, rt, *value))
                }
                pycc_mir::EnumMemberValue::Str(value) => {
                    Scalar::Str(emit_string_literal(context, builder, module, rt, value))
                }
            };
            let value_word = scalar_to_slot_word(context, builder, value_scalar);
            let slot0_index = context.i64_type().const_int(0, false);
            builder
                .build_call(
                    rt.instance_set_slot,
                    &[instance_ptr.into(), slot0_index.into(), value_word.into()],
                    "enum_set_value",
                )
                .expect("build_call should not fail for a well-formed enum value slot write");
            // Set slot 1 to a string pointer containing the member name.
            let name_ptr = emit_string_literal(context, builder, module, rt, member_name);
            let name_scalar = Scalar::Str(name_ptr);
            let name_word = scalar_to_slot_word(context, builder, name_scalar);
            let slot1_index = context.i64_type().const_int(1, false);
            builder
                .build_call(
                    rt.instance_set_slot,
                    &[instance_ptr.into(), slot1_index.into(), name_word.into()],
                    "enum_set_name",
                )
                .expect("build_call should not fail for a well-formed enum name slot write");
            // Store the instance pointer into the synthetic global and
            // mark it initialized. Module globals always have an
            // initialized flag (see `declare_module_globals`), so
            // `slot.initialized` is always `Some` here — `expect` makes
            // that invariant explicit without an unreachable `None` arm
            // that would create a permanently uncovered coverage region.
            builder
                .build_store(slot.ptr, instance_ptr)
                .expect("build_store should not fail for a declared enum member global");
            let initialized_ptr = slot
                .initialized
                .expect("module globals always have an initialized flag");
            builder
                .build_store(initialized_ptr, context.i8_type().const_int(1, false))
                .expect("build_store should not fail for a declared enum member init flag");
        }
    }
}
