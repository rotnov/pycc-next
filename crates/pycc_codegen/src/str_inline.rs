//! Inline common cases of native `str` operations.
//!
//! A native `str` (`pycc_rt::PyStrObj`) is `repr(C)` and starts with its
//! reference count, a pointer to its text, the text's length in bytes and
//! its cached code-point count (`-1` before counted), at offsets 0, 8, 16
//! and 24 (`pycc_rt::str_core`, which asserts them). A `str` that crosses in
//! from CPython shares the CPython object's UTF-8 buffer and its code-point
//! count, so an ASCII text is indexable at once, with no copy and no count.
//!
//! Each runtime function below that a module declares becomes an internal,
//! always-inlined function of the same signature: it answers the common
//! case from those fields and calls the runtime's own function, renamed
//! nowhere and declared again under its symbol, for everything else.
//!
//! * `pycc_rt_str_getitem(s, i)`: a small `int` index into ASCII text
//!   (code points equal bytes), negative indices included, is the shared
//!   one-character `str` of that byte (`pycc_rt_ascii_chars`) once that
//!   slot is filled.
//! * `pycc_rt_str_cmp(a, b)`: the same object compares equal; two
//!   one-byte texts compare by that byte (UTF-8 orders as code points).
//! * `pycc_rt_str_len(s)`: a counted text's count.
//! * `pycc_rt_str_incref(s)`, `pycc_rt_str_decref(s)`: null and immortal
//!   `str`s (literals and the shared one-character strings) are skipped;
//!   an increment is stored inline, a decrement calls the runtime.
//!
//! Only a `--release` build on a 64-bit little-endian target (every Tier-1
//! target) gets them, and only for a function the module calls.

use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::module::{Linkage, Module};
use inkwell::targets::TargetData;
use inkwell::values::{BasicValue, FunctionValue, IntValue, PointerValue};
use inkwell::{AddressSpace, AtomicOrdering, IntPredicate};

const DATA: u64 = 8;
const LEN: u64 = 16;
const CHARS: u64 = 24;
/// `pycc_rt::str_index::IMMORTAL / 2`: a count at or above it is immortal.
const IMMORTAL_FROM: u64 = 1 << 29;
const ASCII_CHARS_SYMBOL: &str = "pycc_rt_ascii_chars";

/// Gives the module's declared `str` runtime functions inline common cases
/// (module doc). Returns how many it wrapped.
pub(crate) fn inline_str_ops(module: &Module<'_>, target: &TargetData) -> usize {
    if target.get_pointer_byte_size(None) != 8
        || target.get_byte_ordering() != inkwell::targets::ByteOrdering::LittleEndian
    {
        return 0;
    }
    let ops = [
        "pycc_rt_str_getitem",
        "pycc_rt_str_cmp",
        "pycc_rt_str_len",
        "pycc_rt_str_incref",
        "pycc_rt_str_decref",
    ];
    let mut wrapped = 0;
    for name in ops {
        let Some(function) = module
            .get_function(name)
            .filter(|f| f.count_basic_blocks() == 0 && f.as_global_value().as_pointer_value().get_first_use().is_some())
        else {
            continue;
        };
        // Every use keeps pointing at `function`, which becomes the inline
        // copy; the runtime's symbol is declared again for its slow path.
        function.as_global_value().set_name(&format!("__pycc_inline.{name}"));
        let slow = module.add_function(name, function.get_type(), Some(Linkage::External));
        function.set_linkage(Linkage::Internal);
        let context = module.get_context();
        let kind = inkwell::attributes::Attribute::get_named_enum_kind_id("alwaysinline");
        function.add_attribute(
            inkwell::attributes::AttributeLoc::Function,
            context.create_enum_attribute(kind, 0),
        );
        let ops = Ops { module, context: module.get_context(), builder: context.create_builder(), slow };
        match name {
            "pycc_rt_str_getitem" => ops.getitem(function),
            "pycc_rt_str_cmp" => ops.compare(function),
            "pycc_rt_str_len" => ops.len(function),
            "pycc_rt_str_incref" => ops.incref(function),
            _ => ops.decref(function),
        }
        wrapped += 1;
    }
    wrapped
}

struct Ops<'a, 'ctx> {
    module: &'a Module<'ctx>,
    context: inkwell::context::ContextRef<'ctx>,
    builder: Builder<'ctx>,
    slow: FunctionValue<'ctx>,
}

impl<'ctx> Ops<'_, 'ctx> {
    fn block(&self, function: FunctionValue<'ctx>, name: &str) -> BasicBlock<'ctx> {
        self.context.append_basic_block(function, name)
    }

    fn field(&self, object: PointerValue<'ctx>, offset: u64, bits: u32, name: &str) -> IntValue<'ctx> {
        let ty = if bits == 32 { self.context.i32_type() } else { self.context.i64_type() };
        // SAFETY: `offset` is a field of `PyStrObj` (module doc).
        let at = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    object,
                    &[self.context.i64_type().const_int(offset, false)],
                    &format!("{name}_at"),
                )
                .expect("gep")
        };
        self.builder.build_load(ty, at, name).expect("load").into_int_value()
    }

    fn data(&self, object: PointerValue<'ctx>, name: &str) -> PointerValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        // SAFETY: `PyStrObj::data`.
        let at = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    object,
                    &[self.context.i64_type().const_int(DATA, false)],
                    &format!("{name}_at"),
                )
                .expect("gep")
        };
        self.builder.build_load(ptr, at, name).expect("load").into_pointer_value()
    }

    fn byte_at(&self, data: PointerValue<'ctx>, index: IntValue<'ctx>, name: &str) -> IntValue<'ctx> {
        // SAFETY: `index` is within the text, checked by the caller.
        let at = unsafe {
            self.builder
                .build_in_bounds_gep(self.context.i8_type(), data, &[index], &format!("{name}_at"))
                .expect("gep")
        };
        self.builder
            .build_load(self.context.i8_type(), at, name)
            .expect("load")
            .into_int_value()
    }

    fn i64(&self, value: u64) -> IntValue<'ctx> {
        self.context.i64_type().const_int(value, true)
    }

    fn cmp(&self, predicate: IntPredicate, a: IntValue<'ctx>, b: IntValue<'ctx>, name: &str) -> IntValue<'ctx> {
        self.builder.build_int_compare(predicate, a, b, name).expect("icmp")
    }

    fn branch(&self, cond: IntValue<'ctx>, then: BasicBlock<'ctx>, otherwise: BasicBlock<'ctx>) {
        self.builder.build_conditional_branch(cond, then, otherwise).expect("branch");
    }

    /// Fills `slow_block` with the runtime call, returning its result.
    fn call_slow(&self, function: FunctionValue<'ctx>, slow_block: BasicBlock<'ctx>) {
        self.builder.position_at_end(slow_block);
        let args: Vec<_> = function.get_param_iter().map(Into::into).collect();
        let call = self.builder.build_call(self.slow, &args, "slow").expect("call");
        call.set_tail_call(true);
        match call.try_as_basic_value().basic() {
            Some(value) => self.builder.build_return(Some(&value)).expect("return"),
            None => self.builder.build_return(None).expect("return"),
        };
    }

    fn getitem(&self, function: FunctionValue<'ctx>) {
        let s = function.get_nth_param(0).expect("s").into_pointer_value();
        let index = function.get_nth_param(1).expect("index").into_int_value();
        let entry = self.block(function, "entry");
        let ascii = self.block(function, "ascii");
        let found = self.block(function, "found");
        let filled = self.block(function, "filled");
        let slow = self.block(function, "slow");
        self.builder.position_at_end(entry);
        let tag = self.builder.build_and(index, self.i64(1), "tag").expect("and");
        let small = self.cmp(IntPredicate::EQ, tag, self.i64(1), "small");
        let len = self.field(s, LEN, 64, "len");
        let chars = self.field(s, CHARS, 64, "chars");
        let counted_ascii = self.cmp(IntPredicate::EQ, chars, len, "counted_ascii");
        let both = self.builder.build_and(small, counted_ascii, "both").expect("and");
        self.branch(both, ascii, slow);
        self.builder.position_at_end(ascii);
        let i = self.builder.build_right_shift(index, self.i64(1), true, "i").expect("ashr");
        let negative = self.cmp(IntPredicate::SLT, i, self.i64(0), "negative");
        let wrapped = self.builder.build_int_add(i, len, "wrapped").expect("add");
        let i = self
            .builder
            .build_select(negative, wrapped, i, "index")
            .expect("select")
            .into_int_value();
        let in_range = self.cmp(IntPredicate::ULT, i, len, "in_range");
        self.branch(in_range, found, slow);
        self.builder.position_at_end(found);
        let data = self.data(s, "data");
        let byte = self.byte_at(data, i, "byte");
        let byte = self
            .builder
            .build_int_z_extend(byte, self.context.i64_type(), "byte64")
            .expect("zext");
        let table = self.ascii_chars();
        let ptr = self.context.ptr_type(AddressSpace::default());
        // SAFETY: `byte` is ASCII (code points equal bytes), below 128.
        let slot = unsafe {
            self.builder
                .build_in_bounds_gep(ptr, table, &[byte], "slot_at")
                .expect("gep")
        };
        let char_str = self.builder.build_load(ptr, slot, "char").expect("load");
        char_str
            .as_instruction_value()
            .expect("a load")
            .set_atomic_ordering(AtomicOrdering::Acquire)
            .expect("an atomic load");
        char_str
            .as_instruction_value()
            .expect("a load")
            .set_alignment(8)
            .expect("aligned");
        let char_str = char_str.into_pointer_value();
        let empty = self.builder.build_is_null(char_str, "empty").expect("is_null");
        self.branch(empty, slow, filled);
        self.builder.position_at_end(filled);
        self.builder.build_return(Some(&char_str)).expect("return");
        self.call_slow(function, slow);
    }

    fn ascii_chars(&self) -> PointerValue<'ctx> {
        self.module
            .get_global(ASCII_CHARS_SYMBOL)
            .unwrap_or_else(|| {
                let ptr = self.context.ptr_type(AddressSpace::default());
                let global = self.module.add_global(ptr.array_type(128), None, ASCII_CHARS_SYMBOL);
                global.set_linkage(Linkage::External);
                global
            })
            .as_pointer_value()
    }

    fn compare(&self, function: FunctionValue<'ctx>) {
        let a = function.get_nth_param(0).expect("a").into_pointer_value();
        let b = function.get_nth_param(1).expect("b").into_pointer_value();
        let entry = self.block(function, "entry");
        let same = self.block(function, "same");
        let lengths = self.block(function, "lengths");
        let bytes = self.block(function, "bytes");
        let slow = self.block(function, "slow");
        let i32_type = self.context.i32_type();
        self.builder.position_at_end(entry);
        let identical = self.builder.build_int_compare(IntPredicate::EQ, a, b, "identical").expect("icmp");
        self.branch(identical, same, lengths);
        self.builder.position_at_end(same);
        self.builder.build_return(Some(&i32_type.const_zero())).expect("return");
        self.builder.position_at_end(lengths);
        let la = self.field(a, LEN, 64, "la");
        let lb = self.field(b, LEN, 64, "lb");
        let one_a = self.cmp(IntPredicate::EQ, la, self.i64(1), "one_a");
        let one_b = self.cmp(IntPredicate::EQ, lb, self.i64(1), "one_b");
        let ones = self.builder.build_and(one_a, one_b, "ones").expect("and");
        self.branch(ones, bytes, slow);
        self.builder.position_at_end(bytes);
        let ba = self.byte_at(self.data(a, "da"), self.i64(0), "ba");
        let bb = self.byte_at(self.data(b, "db"), self.i64(0), "bb");
        let less = self.cmp(IntPredicate::ULT, ba, bb, "less");
        let greater = self.cmp(IntPredicate::UGT, ba, bb, "greater");
        let less = self.builder.build_int_z_extend(less, i32_type, "less32").expect("zext");
        let greater = self.builder.build_int_z_extend(greater, i32_type, "greater32").expect("zext");
        let order = self.builder.build_int_sub(greater, less, "order").expect("sub");
        self.builder.build_return(Some(&order)).expect("return");
        self.call_slow(function, slow);
    }

    fn len(&self, function: FunctionValue<'ctx>) {
        let s = function.get_nth_param(0).expect("s").into_pointer_value();
        let entry = self.block(function, "entry");
        let counted = self.block(function, "counted");
        let slow = self.block(function, "slow");
        self.builder.position_at_end(entry);
        let chars = self.field(s, CHARS, 64, "chars");
        let known = self.cmp(IntPredicate::SGE, chars, self.i64(0), "known");
        self.branch(known, counted, slow);
        self.builder.position_at_end(counted);
        let shifted = self.builder.build_left_shift(chars, self.i64(1), "shifted").expect("shl");
        let tagged = self.builder.build_or(shifted, self.i64(1), "tagged").expect("or");
        self.builder.build_return(Some(&tagged)).expect("return");
        self.call_slow(function, slow);
    }

    /// Branches to `skip` for a null or immortal `s`, else to `counted`
    /// with the count loaded.
    fn mortal(
        &self,
        s: PointerValue<'ctx>,
        skip: BasicBlock<'ctx>,
        counted: BasicBlock<'ctx>,
        check: BasicBlock<'ctx>,
    ) -> IntValue<'ctx> {
        let null = self.builder.build_is_null(s, "null").expect("is_null");
        self.branch(null, skip, check);
        self.builder.position_at_end(check);
        let rc = self.field(s, 0, 32, "rc");
        let immortal = self.cmp(
            IntPredicate::UGE,
            rc,
            self.context.i32_type().const_int(IMMORTAL_FROM, false),
            "immortal",
        );
        self.branch(immortal, skip, counted);
        rc
    }

    fn incref(&self, function: FunctionValue<'ctx>) {
        let s = function.get_nth_param(0).expect("s").into_pointer_value();
        let entry = self.block(function, "entry");
        let check = self.block(function, "check");
        let count = self.block(function, "count");
        let done = self.block(function, "done");
        self.builder.position_at_end(entry);
        let rc = self.mortal(s, done, count, check);
        self.builder.position_at_end(count);
        let next = self
            .builder
            .build_int_add(rc, self.context.i32_type().const_int(1, false), "next")
            .expect("add");
        self.builder.build_store(s, next).expect("store");
        self.builder.build_unconditional_branch(done).expect("branch");
        self.builder.position_at_end(done);
        self.builder.build_return(None).expect("return");
    }

    fn decref(&self, function: FunctionValue<'ctx>) {
        let s = function.get_nth_param(0).expect("s").into_pointer_value();
        let entry = self.block(function, "entry");
        let check = self.block(function, "check");
        let slow = self.block(function, "slow");
        let done = self.block(function, "done");
        self.builder.position_at_end(entry);
        self.mortal(s, done, slow, check);
        self.builder.position_at_end(done);
        self.builder.build_return(None).expect("return");
        self.call_slow(function, slow);
    }
}
