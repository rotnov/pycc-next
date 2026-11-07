//! Inline fast paths for operations on CPython objects in a generic body
//! (non-strict mode).
//!
//! Every operation on an `object` value is a call into a shim helper over
//! CPython's C API (`foreign_arith.rs`, `foreign_compare.rs`, ...). In a body
//! whose values are all objects -- an unannotated function compiled whole --
//! that made `pos += 1`, `pos < n`, `src[pos]` and `char == "x"` each a call
//! that dispatches on the operands' types again, where CPython's
//! specializing interpreter runs an inline path for exact `int` and `str`
//! (`BINARY_OP_ADD_INT`, `COMPARE_OP_INT`, `BINARY_SUBSCR_STR_INT`,
//! `COMPARE_OP_STR`).
//!
//! The emitters here put the same inline path at the operation's site: the
//! operands' exact types are checked, the operation is done in place when
//! they are a compact `int` or a compact ASCII `str`, and anything else --
//! another type, a big `int`, a non-ASCII `str`, an index out of range --
//! branches to the helper call, which keeps CPython's result and every
//! exception ([`guarded`]). The layouts read are the ones the shim's
//! `src/ext/module/fast_ops.c` checks at module exec; it publishes the type
//! pointers the checks compare with (`pycc_ext_fast`) only when they hold,
//! so on any other build every check fails and every operation takes its
//! helper.
//!
//! The loads of an object's type and of an `int`'s or `str`'s value are
//! `!invariant.load`: an object's type can change (`__class__` assignment)
//! only between two heap classes, never to or from `int` or `str`, and an
//! `int`'s or `str`'s value never changes. So a check on a value a loop does
//! not rebind is loop-invariant, and LLVM hoists it out of the loop.

use crate::rt_fns::RtFns;
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicValue, BasicValueEnum, FunctionValue, IntValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate};

/// The shim's `PyccExtFast pycc_ext_fast` (`fast_ops.c`).
pub const EXT_FAST_SYMBOL: &str = "pycc_ext_fast";
/// `PyObject *pycc_ext_obj_pack_i64(long long)`: a compact result as a new
/// `int` (`PyLong_FromLongLong`), or `NULL` with the exception set.
pub const EXT_OBJ_PACK_I64_SYMBOL: &str = "pycc_ext_obj_pack_i64";

/// `PyObject *pycc_ext_ns_getitem_cached(PyObject *ns, PyObject *k,
/// PyccNsReadCache *cache)` (`src/ext/module/module_ctx.c`): a global read
/// that fills its site's cache ([`ns_read`]).
pub const EXT_NS_GETITEM_CACHED_SYMBOL: &str = "pycc_ext_ns_getitem_cached";

/// `PyccExtFast`'s fields, by byte offset.
const STR_TYPE: u64 = 0;
const INT_TYPE: u64 = 8;
const CHARS: u64 = 16;
const CHAR_COUNT: u64 = 128;
/// The built-in classes by `pycc_ext_obj_isinstance`'s selector: `int`,
/// `str`, `float`, `bool`, `list`, `dict`, `tuple`; then `set` and
/// `frozenset`, `bytes` and `bytearray` ([`SET`], [`FROZENSET`], [`BYTES`],
/// [`BYTEARRAY`]).
const CLASSES: u64 = CHARS + CHAR_COUNT * 8;
const CLASS_COUNT: u64 = 12;
/// The selectors of `str`, `set` and `frozenset` in the classes.
pub(crate) const STR: u64 = 1;
pub(crate) const SET: u64 = 7;
pub(crate) const FROZENSET: u64 = 8;
pub(crate) const BYTES: u64 = 9;
pub(crate) const BYTEARRAY: u64 = 10;
/// `bytes`'s and `bytearray`'s length, and where their bytes are.
const BYTES_LENGTH: u64 = 16;
const BYTES_DATA: u64 = 32;
const BYTEARRAY_START: u64 = 40;
const FAST_SIZE: u64 = CLASSES + CLASS_COUNT * 8;

/// The object layouts `fast_ops.c` checks (its file comment).
const OB_TYPE: u64 = 8;
const INT_TAG: u64 = 16;
const INT_DIGIT: u64 = 24;
/// `lv_tag` below this: no digits or one.
const INT_COMPACT_BELOW: u64 = 16;
const STR_LENGTH: u64 = 16;
const STR_HASH: u64 = 24;
const STR_STATE: u64 = 32;
const STR_DATA: u64 = 40;
const STR_COMPACT_ASCII: u64 = 0x60;

/// The fast path under construction: each [`Fast::require`] continues it
/// only while its condition holds, and branches to the helper call
/// otherwise.
pub(crate) struct Fast<'a, 'ctx> {
    context: &'ctx Context,
    builder: &'a Builder<'ctx>,
    module: &'a Module<'ctx>,
    function: FunctionValue<'ctx>,
    slow: BasicBlock<'ctx>,
}

/// `fast`, then `slow` (the helper call) where any of `fast`'s checks
/// failed, merged: the value of whichever ran. Both return the same type.
pub(crate) fn guarded<'a, 'ctx>(
    context: &'ctx Context,
    builder: &'a Builder<'ctx>,
    module: &'a Module<'ctx>,
    name: &str,
    fast: impl FnOnce(&Fast<'a, 'ctx>) -> BasicValueEnum<'ctx>,
    slow: impl FnOnce() -> BasicValueEnum<'ctx>,
) -> BasicValueEnum<'ctx> {
    let function = builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .expect("a fast path is emitted inside a function body");
    let slow_block = context.append_basic_block(function, &format!("{name}_slow"));
    let join = context.append_basic_block(function, &format!("{name}_join"));
    let path = Fast {
        context,
        builder,
        module,
        function,
        slow: slow_block,
    };
    let fast_value = fast(&path);
    let fast_end = builder.get_insert_block().expect("positioned");
    builder.build_unconditional_branch(join).expect("branch");
    builder.position_at_end(slow_block);
    let slow_value = slow();
    let slow_end = builder.get_insert_block().expect("positioned");
    builder.build_unconditional_branch(join).expect("branch");
    builder.position_at_end(join);
    let phi = builder
        .build_phi(fast_value.get_type(), name)
        .expect("build_phi should not fail");
    phi.add_incoming(&[(&fast_value, fast_end), (&slow_value, slow_end)]);
    phi.as_basic_value()
}

impl<'a, 'ctx> Fast<'a, 'ctx> {
    /// Continues the fast path only when `cond` holds (the likely edge).
    pub(crate) fn require(&self, cond: IntValue<'ctx>, name: &str) {
        let next = self.context.append_basic_block(self.function, name);
        let branch = self
            .builder
            .build_conditional_branch(cond, next, self.slow)
            .expect("build_conditional_branch should not fail for an i1");
        crate::int_fast::weight_branch(self.context, branch, true);
        self.builder.position_at_end(next);
    }

    fn i64(&self, value: u64) -> IntValue<'ctx> {
        self.context.i64_type().const_int(value, false)
    }

    fn at(&self, base: PointerValue<'ctx>, offset: u64, name: &str) -> PointerValue<'ctx> {
        // SAFETY: a byte offset into an object or the struct whose layout
        // `fast_ops.c` confirmed before any check here can pass.
        unsafe {
            self.builder
                .build_in_bounds_gep(self.context.i8_type(), base, &[self.i64(offset)], name)
                .expect("build_in_bounds_gep should not fail")
        }
    }

    fn load(&self, ty: BasicTypeEnum<'ctx>, at: PointerValue<'ctx>, invariant: bool, name: &str) -> BasicValueEnum<'ctx> {
        let value = self.builder.build_load(ty, at, name).expect("build_load should not fail");
        if invariant {
            let node = self.context.metadata_node(&[]);
            value
                .as_instruction_value()
                .expect("a load is an instruction")
                .set_metadata(node, self.context.get_kind_id("invariant.load"))
                .expect("a load accepts `!invariant.load`");
        }
        value
    }

    /// `pycc_ext_fast`, declared once per module.
    fn fast_global(&self) -> PointerValue<'ctx> {
        self.module
            .get_global(EXT_FAST_SYMBOL)
            .unwrap_or_else(|| {
                let global = self.module.add_global(
                    self.context.i8_type().array_type(FAST_SIZE as u32),
                    None,
                    EXT_FAST_SYMBOL,
                );
                global.set_linkage(Linkage::External);
                global
            })
            .as_pointer_value()
    }

    fn fast_field(&self, offset: u64, name: &str) -> PointerValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.load(ptr.into(), self.at(self.fast_global(), offset, name), true, name)
            .into_pointer_value()
    }

    /// Requires `object` to be non-NULL and an instance of exactly one of
    /// the classes `selectors` name.
    pub(crate) fn require_class_in(&self, object: PointerValue<'ctx>, selectors: &[u64], name: &str) {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let present = self
            .builder
            .build_is_not_null(object, &format!("{name}_present"))
            .expect("build_is_not_null should not fail");
        self.require(present, &format!("{name}_present"));
        let actual = self
            .load(ptr.into(), self.at(object, OB_TYPE, &format!("{name}_type_at")), true, &format!("{name}_type"))
            .into_pointer_value();
        let mut any = None;
        for &selector in selectors {
            assert!(selector < CLASS_COUNT, "pycc_codegen: internal error: no class {selector}");
            let expected = self.fast_field(CLASSES + selector * 8, &format!("{name}_expected"));
            let exact = self
                .builder
                .build_int_compare(IntPredicate::EQ, actual, expected, &format!("{name}_exact"))
                .expect("icmp of two pointers");
            any = Some(match any {
                None => exact,
                Some(previous) => self.builder.build_or(previous, exact, &format!("{name}_any")).expect("or"),
            });
        }
        self.require(any.expect("at least one class"), &format!("{name}_exact"));
    }

    /// Requires `object` to be non-NULL and of exactly the type at `which`.
    fn require_exact(&self, object: PointerValue<'ctx>, which: u64, name: &str) {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let present = self
            .builder
            .build_is_not_null(object, &format!("{name}_present"))
            .expect("build_is_not_null should not fail");
        self.require(present, &format!("{name}_present"));
        let expected = self.fast_field(which, &format!("{name}_expected"));
        let actual = self
            .load(ptr.into(), self.at(object, OB_TYPE, &format!("{name}_type_at")), true, &format!("{name}_type"))
            .into_pointer_value();
        let exact = self
            .builder
            .build_int_compare(IntPredicate::EQ, actual, expected, &format!("{name}_exact"))
            .expect("icmp of two pointers");
        self.require(exact, &format!("{name}_exact"));
    }

    /// `isinstance(object, C)` for the built-in class `C` that
    /// `pycc_ext_obj_isinstance` selects by `selector`, as an `i32` 0 or 1,
    /// requiring `object` to be non-NULL and of exactly one of the classes
    /// of the table (`NoneType` included). Such an object's `__class__` is
    /// its type, and of those classes only `bool` derives from another
    /// (`int`), so the answer is the type's identity.
    pub(crate) fn builtin_isinstance(&self, object: PointerValue<'ctx>, selector: u64, name: &str) -> IntValue<'ctx> {
        assert!(selector < CLASS_COUNT, "pycc_codegen: internal error: no built-in class {selector}");
        let ptr = self.context.ptr_type(AddressSpace::default());
        let present = self
            .builder
            .build_is_not_null(object, &format!("{name}_present"))
            .expect("build_is_not_null should not fail");
        self.require(present, &format!("{name}_present"));
        let actual = self
            .load(ptr.into(), self.at(object, OB_TYPE, &format!("{name}_type_at")), true, &format!("{name}_type"))
            .into_pointer_value();
        let mut known = None;
        let mut answer = None;
        for class in 0..CLASS_COUNT {
            let expected = self.fast_field(CLASSES + class * 8, &format!("{name}_class"));
            let exact = self
                .builder
                .build_int_compare(IntPredicate::EQ, actual, expected, &format!("{name}_is"))
                .expect("icmp of two pointers");
            known = Some(match known {
                None => exact,
                Some(previous) => self.builder.build_or(previous, exact, &format!("{name}_known")).expect("or"),
            });
            // `bool` (selector 3) is an `int` (selector 0).
            if class == selector || (selector == 0 && class == 3) {
                answer = Some(match answer {
                    None => exact,
                    Some(previous) => self.builder.build_or(previous, exact, &format!("{name}_answer")).expect("or"),
                });
            }
        }
        self.require(known.expect("the table has classes"), &format!("{name}_known"));
        self.builder
            .build_int_z_extend(
                answer.expect("the selected class is in the table"),
                self.context.i32_type(),
                &format!("{name}_status"),
            )
            .expect("zext")
    }


    /// The value of `object` as an `i64`, requiring an exact compact `int`.
    pub(crate) fn compact_int(&self, object: PointerValue<'ctx>, name: &str) -> IntValue<'ctx> {
        self.require_exact(object, INT_TYPE, name);
        let i64_type = self.context.i64_type();
        let tag = self
            .load(i64_type.into(), self.at(object, INT_TAG, &format!("{name}_tag_at")), true, &format!("{name}_tag"))
            .into_int_value();
        let compact = self
            .builder
            .build_int_compare(IntPredicate::ULT, tag, self.i64(INT_COMPACT_BELOW), &format!("{name}_compact"))
            .expect("icmp");
        self.require(compact, &format!("{name}_compact"));
        let digit = self
            .load(
                self.context.i32_type().into(),
                self.at(object, INT_DIGIT, &format!("{name}_digit_at")),
                true,
                &format!("{name}_digit"),
            )
            .into_int_value();
        let digit = self
            .builder
            .build_int_z_extend(digit, i64_type, &format!("{name}_digit64"))
            .expect("zext");
        let sign_bits = self.builder.build_and(tag, self.i64(3), &format!("{name}_sign_bits")).expect("and");
        let sign = self.builder.build_int_sub(self.i64(1), sign_bits, &format!("{name}_sign")).expect("sub");
        self.builder.build_int_mul(sign, digit, &format!("{name}_value")).expect("mul")
    }

    /// The value of a native `int` word, requiring a small one (odd; a
    /// bool marker or a big `int` takes the helper).
    pub(crate) fn small_word(&self, word: IntValue<'ctx>, name: &str) -> IntValue<'ctx> {
        let small = crate::int_fast::is_small(self.context, self.builder, word);
        self.require(small, &format!("{name}_small"));
        self.builder
            .build_right_shift(word, self.i64(1), true, &format!("{name}_value"))
            .expect("ashr")
    }

    /// The length of `object`, requiring an exact compact ASCII `str`.
    pub(crate) fn ascii_str(&self, object: PointerValue<'ctx>, name: &str) -> IntValue<'ctx> {
        self.require_exact(object, STR_TYPE, name);
        let i32_type = self.context.i32_type();
        let state = self
            .load(i32_type.into(), self.at(object, STR_STATE, &format!("{name}_state_at")), true, &format!("{name}_state"))
            .into_int_value();
        let mask = i32_type.const_int(STR_COMPACT_ASCII, false);
        let bits = self.builder.build_and(state, mask, &format!("{name}_bits")).expect("and");
        let ascii = self
            .builder
            .build_int_compare(IntPredicate::EQ, bits, mask, &format!("{name}_ascii"))
            .expect("icmp");
        self.require(ascii, &format!("{name}_ascii"));
        self.load(
            self.context.i64_type().into(),
            self.at(object, STR_LENGTH, &format!("{name}_length_at")),
            true,
            &format!("{name}_length"),
        )
        .into_int_value()
    }

    /// The `index`th byte of a compact ASCII `str`, as an `i64`.
    pub(crate) fn ascii_byte(&self, object: PointerValue<'ctx>, index: IntValue<'ctx>, name: &str) -> IntValue<'ctx> {
        let data = self.at(object, STR_DATA, &format!("{name}_data"));
        // SAFETY: `index` is within the `str`'s length, checked by the caller.
        let at = unsafe {
            self.builder
                .build_in_bounds_gep(self.context.i8_type(), data, &[index], &format!("{name}_at"))
                .expect("gep")
        };
        let byte = self
            .load(self.context.i8_type().into(), at, true, &format!("{name}_byte"))
            .into_int_value();
        self.builder
            .build_int_z_extend(byte, self.context.i64_type(), &format!("{name}_byte64"))
            .expect("zext")
    }

    /// Whether a compact ASCII `str` of `length` holds exactly `bytes`
    /// (`text`, a constant with those bytes), as an `i1`: its bytes are read
    /// only when the lengths match.
    pub(crate) fn ascii_equals(
        &self,
        object: PointerValue<'ctx>,
        length: IntValue<'ctx>,
        text: PointerValue<'ctx>,
        bytes: &[u8],
        name: &str,
    ) -> IntValue<'ctx> {
        let builder = self.builder;
        let bool_type = self.context.bool_type();
        let same_length = builder
            .build_int_compare(IntPredicate::EQ, length, self.i64(bytes.len() as u64), &format!("{name}_same_length"))
            .expect("icmp");
        let Some((&first, _)) = bytes.split_first() else {
            return same_length;
        };
        let entry = builder.get_insert_block().expect("positioned");
        let compare = self.context.append_basic_block(self.function, &format!("{name}_bytes"));
        let done = self.context.append_basic_block(self.function, &format!("{name}_compared"));
        builder
            .build_conditional_branch(same_length, compare, done)
            .expect("build_conditional_branch should not fail for an i1");
        builder.position_at_end(compare);
        let data = self.at(object, STR_DATA, &format!("{name}_data"));
        let equal = if bytes.len() == 1 {
            let byte = self
                .load(self.context.i8_type().into(), data, true, &format!("{name}_byte"))
                .into_int_value();
            builder
                .build_int_compare(
                    IntPredicate::EQ,
                    byte,
                    self.context.i8_type().const_int(u64::from(first), false),
                    &format!("{name}_byte_equal"),
                )
                .expect("icmp")
        } else {
            // A constant-size `memcmp` against zero, which LLVM expands into
            // a few loads.
            let i32_type = self.context.i32_type();
            let ptr = self.context.ptr_type(AddressSpace::default());
            let memcmp = self.module.get_function("memcmp").unwrap_or_else(|| {
                self.module.add_function(
                    "memcmp",
                    i32_type.fn_type(&[ptr.into(), ptr.into(), self.context.i64_type().into()], false),
                    Some(Linkage::External),
                )
            });
            let order = builder
                .build_call(memcmp, &[data.into(), text.into(), self.i64(bytes.len() as u64).into()], &format!("{name}_memcmp"))
                .expect("build_call should not fail for memcmp")
                .try_as_basic_value()
                .expect_basic("memcmp returns int")
                .into_int_value();
            builder
                .build_int_compare(IntPredicate::EQ, order, i32_type.const_zero(), &format!("{name}_bytes_equal"))
                .expect("icmp")
        };
        let compare_end = builder.get_insert_block().expect("positioned");
        builder.build_unconditional_branch(done).expect("branch");
        builder.position_at_end(done);
        let phi = builder.build_phi(bool_type, &format!("{name}_equal")).expect("phi");
        phi.add_incoming(&[(&bool_type.const_zero(), entry), (&equal, compare_end)]);
        phi.as_basic_value().into_int_value()
    }

    /// The length of `object` and a pointer to its first byte, requiring
    /// an exact `bytes` (`bytes_too`) or `bytearray`. A `bytearray`'s
    /// length and buffer are read anew each time: it can be resized.
    pub(crate) fn byte_buffer(&self, object: PointerValue<'ctx>, bytes_too: bool, name: &str) -> (IntValue<'ctx>, PointerValue<'ctx>) {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        if !bytes_too {
            self.require_class_in(object, &[BYTEARRAY], name);
            let length = self
                .load(i64_type.into(), self.at(object, BYTES_LENGTH, &format!("{name}_length_at")), false, &format!("{name}_length"))
                .into_int_value();
            let start = self
                .load(ptr.into(), self.at(object, BYTEARRAY_START, &format!("{name}_start_at")), false, &format!("{name}_start"))
                .into_pointer_value();
            return (length, start);
        }
        self.require_class_in(object, &[BYTES, BYTEARRAY], name);
        let actual = self
            .load(ptr.into(), self.at(object, OB_TYPE, &format!("{name}_type_at")), true, &format!("{name}_type"))
            .into_pointer_value();
        let bytes_type = self.fast_field(CLASSES + BYTES * 8, &format!("{name}_bytes_type"));
        let is_bytes = self
            .builder
            .build_int_compare(IntPredicate::EQ, actual, bytes_type, &format!("{name}_is_bytes"))
            .expect("icmp");
        let length = self
            .load(i64_type.into(), self.at(object, BYTES_LENGTH, &format!("{name}_length_at")), false, &format!("{name}_length"))
            .into_int_value();
        let inline = self.at(object, BYTES_DATA, &format!("{name}_inline"));
        // A `bytearray`'s pointer to its bytes; of a `bytes`, which holds its
        // bytes inline, a field every object has is read instead, unused.
        let field = self
            .builder
            .build_select(
                is_bytes,
                self.at(object, OB_TYPE, &format!("{name}_unused_at")),
                self.at(object, BYTEARRAY_START, &format!("{name}_start_at")),
                &format!("{name}_field"),
            )
            .expect("select")
            .into_pointer_value();
        let loaded = self.load(ptr.into(), field, false, &format!("{name}_loaded")).into_pointer_value();
        let start = self
            .builder
            .build_select(is_bytes, inline, loaded, &format!("{name}_start"))
            .expect("select")
            .into_pointer_value();
        (length, start)
    }

    /// `at` adjusted as a Python index into `length` items (a negative one
    /// from the end), requiring it to be in range.
    pub(crate) fn index_within(&self, at: IntValue<'ctx>, length: IntValue<'ctx>, name: &str) -> IntValue<'ctx> {
        let builder = self.builder;
        let negative = builder
            .build_int_compare(IntPredicate::SLT, at, self.i64(0), &format!("{name}_negative"))
            .expect("icmp");
        let from_end = builder.build_int_add(at, length, &format!("{name}_from_end")).expect("add");
        let at = builder
            .build_select(negative, from_end, at, &format!("{name}_at"))
            .expect("select")
            .into_int_value();
        let within = builder
            .build_int_compare(IntPredicate::ULT, at, length, &format!("{name}_within"))
            .expect("icmp");
        self.require(within, &format!("{name}_within"));
        at
    }

    /// A pointer `offset` bytes past `base`.
    pub(crate) fn byte_at(&self, base: PointerValue<'ctx>, offset: IntValue<'ctx>, name: &str) -> PointerValue<'ctx> {
        // SAFETY: `offset` is within the buffer, checked by the caller.
        unsafe {
            self.builder
                .build_in_bounds_gep(self.context.i8_type(), base, &[offset], name)
                .expect("gep")
        }
    }

    /// A compact ASCII `str`'s cached hash (-1 before it is computed).
    pub(crate) fn str_hash(&self, object: PointerValue<'ctx>, name: &str) -> IntValue<'ctx> {
        self.load(self.context.i64_type().into(), self.at(object, STR_HASH, &format!("{name}_hash_at")), false, &format!("{name}_hash"))
            .into_int_value()
    }

    /// The one-character `str` of ASCII code `byte`, a new reference.
    pub(crate) fn char_str(&self, rt: &RtFns<'ctx>, byte: IntValue<'ctx>, name: &str) -> PointerValue<'ctx> {
        let i64_type = self.context.i64_type();
        let offset = self
            .builder
            .build_int_mul(byte, i64_type.const_int(8, false), &format!("{name}_offset"))
            .expect("mul");
        let offset = self
            .builder
            .build_int_add(offset, i64_type.const_int(CHARS, false), &format!("{name}_field"))
            .expect("add");
        let global = self.fast_global();
        // SAFETY: `byte` is below 128, an index into `chars`.
        let at = unsafe {
            self.builder
                .build_in_bounds_gep(self.context.i8_type(), global, &[offset], &format!("{name}_at"))
                .expect("gep")
        };
        let ptr = self.context.ptr_type(AddressSpace::default());
        let text = self.load(ptr.into(), at, true, name).into_pointer_value();
        self.builder
            .build_call(rt.obj_retain, &[text.into()], "")
            .expect("build_call should not fail for pycc_ext_obj_retain");
        text
    }

    /// A compact `int` result as a new `int` object (`NULL` with the
    /// exception set when it cannot be made).
    pub(crate) fn int_object(&self, value: IntValue<'ctx>, name: &str) -> PointerValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let pack = crate::foreign_pack::shim_fn(
            self.module,
            EXT_OBJ_PACK_I64_SYMBOL,
            ptr.fn_type(&[self.context.i64_type().into()], false),
        );
        self.builder
            .build_call(pack, &[value.into()], name)
            .expect("build_call should not fail for pycc_ext_obj_pack_i64")
            .try_as_basic_value()
            .expect_basic("pycc_ext_obj_pack_i64 returns PyObject *")
            .into_pointer_value()
    }
}

/// `PyccNsReadCache`'s fields, by byte offset: the namespace dict the entry
/// is for, where its version is read, the version seen, the value read.
const NS_DICT: u64 = 0;
const NS_VERSION: u64 = 8;
const NS_SEEN: u64 = 16;
const NS_VALUE: u64 = 24;
const NS_CACHE_SIZE: u64 = 32;

/// A global read `ns[name]` of the module's namespace dict through a cache
/// of the site's own (`LOAD_GLOBAL`'s): while the dict is the one the cache
/// was filled from and its version is unchanged, the cached value (a new
/// reference); otherwise `call(cache)`, the helper that reads and refills
/// it. A new reference, or `NULL` with the exception set.
pub(crate) fn ns_read<'a, 'ctx>(
    context: &'ctx Context,
    builder: &'a Builder<'ctx>,
    module: &'a Module<'ctx>,
    rt: &RtFns<'ctx>,
    ns: PointerValue<'ctx>,
    call: impl FnOnce(PointerValue<'ctx>) -> BasicValueEnum<'ctx>,
) -> PointerValue<'ctx> {
    let cache_type = context.i8_type().array_type(NS_CACHE_SIZE as u32);
    let cache = module.add_global(cache_type, None, "pycc_ns_read_cache");
    cache.set_linkage(Linkage::Private);
    cache.set_initializer(&cache_type.const_zero());
    cache.set_alignment(8);
    let cache = cache.as_pointer_value();
    guarded(
        context,
        builder,
        module,
        "fast_ns_read",
        |fast| {
            let ptr = context.ptr_type(AddressSpace::default());
            let i64_type = context.i64_type();
            let dict = fast
                .load(ptr.into(), fast.at(cache, NS_DICT, "fast_ns_dict_at"), false, "fast_ns_dict")
                .into_pointer_value();
            let same = builder
                .build_int_compare(IntPredicate::EQ, dict, ns, "fast_ns_same")
                .expect("icmp of two pointers");
            fast.require(same, "fast_ns_cached");
            let version = fast
                .load(ptr.into(), fast.at(cache, NS_VERSION, "fast_ns_version_at"), false, "fast_ns_version")
                .into_pointer_value();
            let now = fast.load(i64_type.into(), version, false, "fast_ns_now").into_int_value();
            let seen = fast
                .load(i64_type.into(), fast.at(cache, NS_SEEN, "fast_ns_seen_at"), false, "fast_ns_seen")
                .into_int_value();
            let current = builder
                .build_int_compare(IntPredicate::EQ, now, seen, "fast_ns_current")
                .expect("icmp");
            fast.require(current, "fast_ns_current");
            let value = fast
                .load(ptr.into(), fast.at(cache, NS_VALUE, "fast_ns_value_at"), false, "fast_ns_value")
                .into_pointer_value();
            builder
                .build_call(rt.obj_retain, &[value.into()], "")
                .expect("build_call should not fail for pycc_ext_obj_retain");
            value.into()
        },
        || call(cache),
    )
    .into_pointer_value()
}
