//! Instance attribute slot words (D-154): the encoding between a `Scalar`
//! and the raw `i64` word `pycc_rt_instance_get_slot`/`set_slot` carry, and
//! the release of a slot's previous `str`/`int` value before a store.
//!
//! Extracted from `lib.rs` per AGENTS.md's decomposition rule when #1262
//! (Part 1 of #1218) added the `list[int]`/`dict[str, int]` pointer word.

use crate::Scalar;
use crate::bigint_rc::{BigIntRefcount, emit_bigint_refcount_call};
use crate::rt_fns::RtFns;
use inkwell::context::Context;
use inkwell::values::{IntValue, PointerValue};

/// The layout descriptor `pycc_rt_instance_new` takes (#1388): `class_name`
/// then each of `slot_names`, NUL-separated, as a private constant interned
/// once per distinct descriptor and returned with its byte length. The
/// global is keyed on the descriptor itself, never on the constructor: a
/// subclass that inherits `__init__` shares its base's constructor but not
/// its class name. `pycc_rt` reads it
/// only to word the `AttributeError` of a slot read before its assignment,
/// so it never needs a terminating NUL.
pub(crate) fn instance_layout_constant<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    class_name: &str,
    slot_names: &[String],
) -> (PointerValue<'ctx>, IntValue<'ctx>) {
    let mut layout = class_name.to_string();
    for name in slot_names {
        layout.push('\0');
        layout.push_str(name);
    }
    let len = context.i64_type().const_int(layout.len() as u64, false);
    // A NUL cannot appear in an LLVM global's name, and `/` cannot appear
    // in a Python identifier, so the mapping keeps distinct descriptors
    // distinct.
    let global_name = format!("pycc_instance_layout.{}", layout.replace('\0', "/"));
    if let Some(existing) = module.get_global(&global_name) {
        return (existing.as_pointer_value(), len);
    }
    let (ptr, _) =
        crate::exception_value::emit_str_bytes_constant(context, module, &layout, &global_name);
    (ptr, len)
}

/// Reinterprets a raw `i64` slot word read from `pycc_rt_instance_get_slot`
/// as the `Scalar` its declared attribute `ty` names (D-154, Part 1 of
/// #375; see `pycc_rt::instance`'s own doc comment for the slot
/// representation this mirrors exactly): `int` passes through unchanged;
/// `bool` truncates to `pycc_codegen`'s own `i8` `Scalar::Bool` carrier;
/// `float` bit-reinterprets the same 8 bytes as `f64` (never a numeric
/// conversion -- the word *is* a float's bit pattern, written by
/// `scalar_to_slot_word`'s own mirror-image `float` arm); `str` reinterprets
/// the word as a `*mut PyStrObj` pointer; and -- since #1262 -- a
/// `list[int]` or `dict[str, int]` reinterprets the word as the container's
/// pointer, the same `inttoptr` `str` uses. Containers are leak-only (D-107,
/// D-124), so the read hands back the very object the slot holds, which is
/// CPython's aliasing. Since Part 1 of #1367 an `object` (a class a foreign
/// import binds) reinterprets the word as its `PyObject*` the same way, and
/// since #1389 an instance of a class of this program (`Ty::Instance`) as its
/// instance pointer -- never freed by `pycc_rt`, so again the read aliases
/// the stored object. Only these eight `Ty`s can ever reach here, because
/// the two places a slot gets its type admit no other: an undeclared
/// attribute's `pycc_hir::class::init_slot::slot_ty_from_init_rhs` (a
/// scalar parameter or literal, or a `list[int]`/`dict[str, int]`/`object`/
/// class-instance parameter) and a class-body declaration's
/// `pycc_hir::class::declared_attrs` gate (the same set plus a type
/// parameter, which monomorphisation replaces), so a
/// `Set`/`Tuple`/`Protocol`/`Param`/`Infer`/`MemoryView`-typed attribute can
/// never be constructed from real, type-checked source. Since #1388 a
/// declared attribute's right-hand side may be any expression; its type is
/// the declared one, which `pycc_types::check_attr_set` holds the value to.
pub(crate) fn slot_word_to_scalar<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    raw: IntValue<'ctx>,
    ty: &pycc_mir::Ty,
) -> Scalar<'ctx> {
    match ty {
        pycc_mir::Ty::Int => Scalar::Int(raw),
        pycc_mir::Ty::Bool => Scalar::Bool(
            builder
                .build_int_truncate(raw, context.i8_type(), "attr_bool_trunc")
                .expect("build_int_truncate should not fail truncating i64 to i8"),
        ),
        pycc_mir::Ty::Float => Scalar::Float(
            builder
                .build_bit_cast(raw, context.f64_type(), "attr_float_bitcast")
                .expect("build_bit_cast should not fail reinterpreting i64 bits as f64")
                .into_float_value(),
        ),
        pycc_mir::Ty::Str => Scalar::Str(
            builder
                .build_int_to_ptr(
                    raw,
                    context.ptr_type(inkwell::AddressSpace::default()),
                    "attr_str_inttoptr",
                )
                .expect("build_int_to_ptr should not fail reinterpreting an i64 as a pointer"),
        ),
        // #1262: one `inttoptr` for both containers, then an asymmetric
        // wrapper choice -- two sibling arms would be the structurally
        // identical regions that have lost coverage counts in this code
        // before (see `init_slot::slot_ty_from_init_rhs`'s own comment).
        //
        // Part 1 of #1367: a foreign object (`Ty::Object`) shares the same
        // `inttoptr` -- D-154 already stores `str`/instance pointers
        // reinterpreted as `i64`, and a `PyObject*` is one more pointer.
        // Every `object` producer hands back a reference that is never
        // released and `pycc_rt::instance` never releases a slot word, so
        // the read needs no refcount traffic. A slot read before its
        // `__init__` assignment never reaches here: the checked read raises
        // `AttributeError` first (#1388).
        //
        // #1389: an instance of a class of this program (`Ty::Instance`)
        // shares it too. `pycc_rt` never frees an instance, so the read
        // aliases the stored one with no refcount traffic.
        pycc_mir::Ty::List(_)
        | pycc_mir::Ty::Dict(..)
        | pycc_mir::Ty::Object
        | pycc_mir::Ty::Instance(_) => {
            let ptr = builder
                .build_int_to_ptr(
                    raw,
                    context.ptr_type(inkwell::AddressSpace::default()),
                    "attr_container_inttoptr",
                )
                .expect("build_int_to_ptr should not fail reinterpreting an i64 as a pointer");
            match ty {
                pycc_mir::Ty::List(_) => Scalar::List(ptr),
                pycc_mir::Ty::Dict(..) => Scalar::Dict(ptr),
                pycc_mir::Ty::Instance(_) => Scalar::Instance(ptr),
                _ => Scalar::Object(ptr),
            }
        }
        other => panic!(
            "pycc_codegen: internal error: an instance attribute of type `{}` is not \
             supported yet -- pycc_hir's init-slot and declaration gates should have \
             rejected this before codegen",
            other.name()
        ),
    }
}

/// Mirror image of [`slot_word_to_scalar`]: encodes a `Scalar` as the raw
/// `i64` word `pycc_rt_instance_set_slot` stores. See that function's own
/// doc comment for why only `Int`/`Bool`/`Float`/`Str`/`List`/`Dict`/`Object`/
/// `Instance` are ever reachable here.
pub(crate) fn scalar_to_slot_word<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    scalar: Scalar<'ctx>,
) -> IntValue<'ctx> {
    match scalar {
        Scalar::Int(v) => v,
        Scalar::Bool(v) => builder
            .build_int_z_extend(v, context.i64_type(), "attr_bool_zext")
            .expect("build_int_z_extend should not fail widening i8 to i64"),
        Scalar::Float(v) => builder
            .build_bit_cast(v, context.i64_type(), "attr_float_bitcast")
            .expect("build_bit_cast should not fail reinterpreting f64 bits as i64")
            .into_int_value(),
        Scalar::Str(v) => builder
            .build_ptr_to_int(v, context.i64_type(), "attr_str_ptrtoint")
            .expect("build_ptr_to_int should not fail reinterpreting a pointer as i64"),
        // #1262: a `list[int]`/`dict[str, int]` slot holds the container's
        // pointer. No refcount traffic: containers are leak-only (D-107,
        // D-124), and `MirStmt::AttrSet`'s release calls are gated on a
        // `str`/`int` value type.
        // Part 1 of #1367: a foreign object's `PyObject*` is stored the
        // same way, with no refcount traffic here (see
        // `slot_word_to_scalar`). Since Part 1 of #1499 the
        // `MirStmt::AttrSet` arm retains a borrowed object before this
        // conversion (`object_slot::retain_if_borrowed`).
        // #1389: so is an instance of a class of this program.
        Scalar::List(v) | Scalar::Dict(v) | Scalar::Object(v) | Scalar::Instance(v) => builder
            .build_ptr_to_int(v, context.i64_type(), "attr_container_ptrtoint")
            .expect("build_ptr_to_int should not fail reinterpreting a pointer as i64"),
        Scalar::Set(_)
        | Scalar::Tuple(_)
        // D-197, #763, Part 1 of #747: an `Optional[int]`-typed instance
        // attribute joins the same defensive arm as every other
        // multi-word/aggregate `Scalar` above -- this raw-`i64`-word slot
        // encoding has no room for the `{ i64, i8 }` struct's extra
        // present/absent byte, and this PR ships no class-attribute use of
        // `Optional[int]` for `slot_ty_from_init_rhs` to have exercised.
        | Scalar::Optional(_)
        // Part 2 of #1027: a `memoryview` joins the same or-pattern for
        // the identical reason -- neither `slot_ty_from_init_rhs` nor a
        // class-body declaration admits a `memoryview` slot. Part 2a of #1142 (#1165) gave
        // the type its one storable position, a *local* slot bound by
        // `a = ndarray(n)`; an instance attribute is not that position and
        // stays refused, so no such attribute is ever built.
        | Scalar::MemoryView(_) => panic!(
            "pycc_codegen: internal error: cannot store this value into an instance \
             attribute slot -- pycc_hir's init-slot and declaration gates should have \
             rejected this before codegen"
        ),
    }
}

/// Mirror of [`crate::decref_str_slot_before_store`] for an instance attribute slot
/// rather than a local's own alloca (D-154, Part 1 of #375): only
/// meaningful for a `Ty::Str` attribute -- reads the slot's *current* raw
/// word through the same opaque `pycc_rt_instance_get_slot` accessor
/// `MirExpr::AttrGet` itself uses, reinterprets it as a `str` pointer, and
/// decrefs it before the new value overwrites the slot. The read is the
/// unchecked one: a freshly allocated instance's slots start unassigned
/// (`pycc_rt::instance::new_instance`), which that read returns as the `0`
/// word -- a null pointer whose runtime decref is a documented
/// no-op (`pycc_rt_str_decref`'s own null check) -- exactly like a local's
/// null-initialized string slot -- so the same call is correct for both
/// `__init__`'s first assignment and any later reassignment.
///
/// Unlike `decref_str_slot_before_store`, this function has no runtime
/// assertion that the target slot's own declared type is actually
/// `Ty::Str` -- its one caller (`MirStmt::AttrSet`'s own codegen) only
/// invokes it when `value`'s type is `Ty::Str`, and `pycc_types::class::
/// check_attr_set`'s `is_assignable(value_ty, attr_ty)` gate (`T0021`)
/// already rejects a `str` value targeting a non-`str` attribute before
/// codegen ever runs -- so this slot's declared type is `Ty::Str` too on
/// every reachable call, by construction, not merely by convention left
/// unchecked (D-068 review finding, PR #385).
pub(crate) fn decref_str_attr_slot_before_store<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    base_ptr: PointerValue<'ctx>,
    slot_index: IntValue<'ctx>,
) {
    let raw = builder
        .build_call(
            rt.instance_get_slot,
            &[base_ptr.into(), slot_index.into()],
            "instance_get_slot_old",
        )
        .expect("build_call should not fail for a well-formed attribute read")
        .try_as_basic_value()
        .expect_basic("pycc_rt_instance_get_slot returns a non-void i64")
        .into_int_value();
    let old = builder
        .build_int_to_ptr(
            raw,
            context.ptr_type(inkwell::AddressSpace::default()),
            "attr_str_inttoptr_old",
        )
        .expect("build_int_to_ptr should not fail reinterpreting an i64 as a pointer");
    builder
        .build_call(rt.str_decref, &[old.into()], "str_decref_old_attr")
        .expect("build_call should not fail for a well-formed decref");
}

/// [`crate::bigint_rc::release_int_slot_before_store`]'s counterpart for an instance
/// attribute slot, and the exact `int` mirror of
/// [`decref_str_attr_slot_before_store`] directly above: reads the slot's
/// current raw word through `pycc_rt_instance_get_slot` and releases it
/// before the new value overwrites it. A freshly allocated instance's slots
/// start unassigned (`pycc_rt::instance::new_instance`), which the
/// unchecked read returns as the `0` word, and `0` is the word
/// `pycc_rt_bigint_release` returns on without classifying, so the
/// same call is correct for `__init__`'s first assignment and every later
/// reassignment.
pub(crate) fn release_int_attr_slot_before_store<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    base_ptr: PointerValue<'ctx>,
    slot_index: IntValue<'ctx>,
) {
    let old = builder
        .build_call(
            rt.instance_get_slot,
            &[base_ptr.into(), slot_index.into()],
            "instance_get_slot_old_int",
        )
        .expect("build_call should not fail for a well-formed attribute read")
        .try_as_basic_value()
        .expect_basic("pycc_rt_instance_get_slot returns a non-void i64")
        .into_int_value();
    emit_bigint_refcount_call(context, builder, rt, old, BigIntRefcount::Release);
}

#[cfg(test)]
mod tests {
    use crate::{CompileOptions, compile_to_object_with_observer, llvm_string_to_owned};
    use pycc_mir::{MirExpr, MirItem, MirModule, MirStmt, Ty};

    fn name(name: &str, ty: Ty) -> MirExpr {
        MirExpr::Name {
            name: name.to_string(),
            ty,
        }
    }

    /// The IR of the one function that stores and reads the container
    /// slots (and, since Part 1 of #1367, a foreign `object` slot, and since
    /// #1389 a class-instance slot), compiled
    /// through the real pipeline.
    fn container_slot_function_ir() -> String {
        let self_ty = Ty::Instance(Box::new("Holder".to_string()));
        let list_ty = Ty::List(Box::new(Ty::Int));
        let dict_ty = Ty::Dict(Box::new((Ty::Str, Ty::Int)));
        let object_ty = Ty::Object;
        let leaf_ty = Ty::Instance(Box::new("Leaf".to_string()));
        let get = |slot: usize, ty: &Ty| MirExpr::AttrGet {
            base: Box::new(name("self", self_ty.clone())),
            slot,
            ty: ty.clone(),
        };
        let init = MirItem::Function {
            name: "Holder.__init__".to_string(),
            params: vec![
                ("self".to_string(), self_ty.clone()),
                ("xs".to_string(), list_ty.clone()),
                ("d".to_string(), dict_ty.clone()),
                ("o".to_string(), object_ty.clone()),
                ("leaf".to_string(), leaf_ty.clone()),
            ],
            return_ty: Ty::None,
            body: vec![
                MirStmt::AttrSet {
                    base: name("self", self_ty.clone()),
                    slot: 0,
                    value: name("xs", list_ty.clone()),
                },
                MirStmt::AttrSet {
                    base: name("self", self_ty.clone()),
                    slot: 1,
                    value: name("d", dict_ty.clone()),
                },
                MirStmt::AttrSet {
                    base: name("self", self_ty.clone()),
                    slot: 2,
                    value: name("o", object_ty.clone()),
                },
                MirStmt::AttrSet {
                    base: name("self", self_ty.clone()),
                    slot: 3,
                    value: name("leaf", leaf_ty.clone()),
                },
                MirStmt::Assign {
                    target: "ys".to_string(),
                    value: get(0, &list_ty),
                },
                MirStmt::Assign {
                    target: "e".to_string(),
                    value: get(1, &dict_ty),
                },
                MirStmt::Assign {
                    target: "p".to_string(),
                    value: get(2, &object_ty),
                },
                MirStmt::Assign {
                    target: "l".to_string(),
                    value: get(3, &leaf_ty),
                },
                MirStmt::Return(None),
            ],
        };
        let mir = MirModule {
            items: vec![init],
            class_defs: Vec::new(),
        };
        let dir = pycc_scratch::ScratchDir::new("attr_slot_container").expect("scratch dir");
        let obj_path = dir.join("attr_slot_container.o");
        let mut ir = String::new();
        // D-029: route `print_to_string`'s `LLVMString` through
        // `llvm_string_to_owned`, as every sibling IR test does.
        let mut observer = |module: &inkwell::module::Module<'_>, _| {
            ir = llvm_string_to_owned(module.print_to_string());
        };
        compile_to_object_with_observer(
            &mir,
            &obj_path,
            &CompileOptions::default(),
            Some(&mut observer),
        )
        .expect("codegen should succeed");
        ir.split("\ndefine ")
            .find(|function| function.contains("attr_container_ptrtoint"))
            .expect("the container slot store should be emitted")
            .to_string()
    }

    #[test]
    fn a_container_slot_is_stored_and_read_as_a_pointer_word_with_no_refcount_traffic() {
        // #1262: `list[int]`/`dict[str, int]` slots hold the container's
        // pointer word (one `ptrtoint` per store, one `inttoptr` per read).
        // Containers are leak-only (D-107, D-124), so neither store may
        // release the slot's previous value: `MirStmt::AttrSet` gates its
        // `str` decref and `int` release on the value's type. Part 1 of
        // #1367: an `object` slot holds its `PyObject*` the same way, with
        // no refcount traffic either (every `object` producer's reference
        // is never released). #1389: so does an instance of a class of this
        // program, which `pycc_rt` never frees.
        let ir = container_slot_function_ir();
        let defined = |prefix: &str| {
            ir.lines()
                .filter(|line| line.trim_start().starts_with(prefix))
                .count()
        };
        assert_eq!(defined("%attr_container_ptrtoint"), 4, "{ir}");
        assert_eq!(defined("%attr_container_inttoptr"), 4, "{ir}");
        assert!(!ir.contains("Py_DecRef"), "{ir}");
        assert!(!ir.contains("pycc_rt_str_decref"), "{ir}");
        assert!(!ir.contains("pycc_rt_bigint_release"), "{ir}");
    }
}
