//! A `frozenset` of characters as its ASCII mask: the shim's two-way cache
//! of masks (`fast_ops.c`) and the fast paths that test a character's code
//! against one.

use super::*;

/// The shim's `PyccAsciiMask pycc_ext_ascii_masks[]` (`fast_ops.c`): two
/// ways per index of `MASK_WAYS`, each entry `MASK_SIZE` bytes: its set, its
/// two 64-bit words of ASCII members, whether it is usable.
const EXT_ASCII_MASKS_SYMBOL: &str = "pycc_ext_ascii_masks";
/// `long long pycc_ext_ascii_mask_fill(PyObject *frozenset, long long c)`:
/// after a miss in both ways, fills the entry and answers whether ASCII `c`
/// is a member (1 or 0), or -1 when the set is not usable.
const EXT_ASCII_MASK_FILL_SYMBOL: &str = "pycc_ext_ascii_mask_fill";
/// `PyccAsciiMask *pycc_ext_ascii_mask_entry(PyObject *frozenset)`: the
/// set's usable entry, computed on a miss, or `NULL` ([`Fast::ascii_mask_words`]).
const EXT_ASCII_MASK_ENTRY_SYMBOL: &str = "pycc_ext_ascii_mask_entry";
const MASK_WAYS: u64 = 64;
const MASK_SIZE: u64 = 32;
const MASK_BITS: u64 = 8;
const MASK_USABLE: u64 = 24;

impl<'ctx> Fast<'_, 'ctx> {
    /// `item in set` as an `i32` 0 or 1, requiring an exact `frozenset`
    /// whose members are all exact `str`s and a one-character ASCII `str`:
    /// a bit test against the set's ASCII mask, read from the shim's
    /// two-way cache ([`EXT_ASCII_MASKS_SYMBOL`]) and computed on a miss
    /// ([`EXT_ASCII_MASK_FILL_SYMBOL`], `fast_ops.c`).
    pub(crate) fn ascii_member(&self, set: PointerValue<'ctx>, item: PointerValue<'ctx>, name: &str) -> IntValue<'ctx> {
        let length = self.ascii_str(item, &format!("{name}_item"));
        let single = self
            .builder
            .build_int_compare(IntPredicate::EQ, length, self.i64(1), &format!("{name}_single"))
            .expect("icmp");
        self.require(single, &format!("{name}_single"));
        let byte = self.ascii_byte(item, self.i64(0), &format!("{name}_char"));
        self.ascii_byte_member(set, byte, name)
    }

    /// The two cache entries ([`EXT_ASCII_MASKS_SYMBOL`]) where the mask of
    /// `set` may be, requiring an exact `frozenset`.
    fn mask_ways(&self, set: PointerValue<'ctx>, name: &str) -> (PointerValue<'ctx>, PointerValue<'ctx>) {
        let builder = self.builder;
        let i64_type = self.context.i64_type();
        self.require_class_in(set, &[FROZENSET], &format!("{name}_set"));
        let masks = self
            .module
            .get_global(EXT_ASCII_MASKS_SYMBOL)
            .unwrap_or_else(|| {
                let global = self.module.add_global(
                    self.context.i8_type().array_type((MASK_WAYS * 2 * MASK_SIZE) as u32),
                    None,
                    EXT_ASCII_MASKS_SYMBOL,
                );
                global.set_linkage(Linkage::External);
                global
            })
            .as_pointer_value();
        let address = builder
            .build_ptr_to_int(set, i64_type, &format!("{name}_address"))
            .expect("ptrtoint");
        let shifted = builder
            .build_right_shift(address, self.i64(4), false, &format!("{name}_shifted"))
            .expect("lshr");
        let index = builder
            .build_and(shifted, self.i64(MASK_WAYS - 1), &format!("{name}_index"))
            .expect("and");
        let offset = builder
            .build_int_mul(index, self.i64(2 * MASK_SIZE), &format!("{name}_offset"))
            .expect("mul");
        // SAFETY: `index` is below `MASK_WAYS`, an entry pair of the cache.
        let first = unsafe {
            builder
                .build_in_bounds_gep(self.context.i8_type(), masks, &[offset], &format!("{name}_first"))
                .expect("gep")
        };
        let second = self.at(first, MASK_SIZE, &format!("{name}_second"));
        (first, second)
    }

    /// The two 64-bit words of the ASCII mask of `set` (bit `b` of the
    /// pair set when `chr(b)` is a member), requiring an exact `frozenset`
    /// whose members are all exact `str`s: read from the shim's cache, or
    /// from the entry `pycc_ext_ascii_mask_entry` computes on a miss
    /// ([`EXT_ASCII_MASK_ENTRY_SYMBOL`]). The words stay the set's answer
    /// while it lives, since a `frozenset` cannot change, whatever later
    /// becomes of the cache entry.
    pub(crate) fn ascii_mask_words(&self, set: PointerValue<'ctx>, name: &str) -> (IntValue<'ctx>, IntValue<'ctx>) {
        let builder = self.builder;
        let i64_type = self.context.i64_type();
        let ptr = self.context.ptr_type(AddressSpace::default());
        let (first, second) = self.mask_ways(set, name);
        let entry_block = builder.get_insert_block().expect("positioned");
        let try_second = self.context.append_basic_block(self.function, &format!("{name}_try_second"));
        let fill = self.context.append_basic_block(self.function, &format!("{name}_fill"));
        let found = self.context.append_basic_block(self.function, &format!("{name}_entry"));
        let first_set = self.load(ptr.into(), first, false, &format!("{name}_first_set")).into_pointer_value();
        let first_hit = builder
            .build_int_compare(IntPredicate::EQ, first_set, set, &format!("{name}_first_hit"))
            .expect("icmp");
        let branch = builder
            .build_conditional_branch(first_hit, found, try_second)
            .expect("build_conditional_branch should not fail for an i1");
        crate::int_fast::weight_branch(self.context, branch, true);
        builder.position_at_end(try_second);
        let second_set = self.load(ptr.into(), second, false, &format!("{name}_second_set")).into_pointer_value();
        let second_hit = builder
            .build_int_compare(IntPredicate::EQ, second_set, set, &format!("{name}_second_hit"))
            .expect("icmp");
        builder
            .build_conditional_branch(second_hit, found, fill)
            .expect("build_conditional_branch should not fail for an i1");
        builder.position_at_end(fill);
        let entry_fn = self.module.get_function(EXT_ASCII_MASK_ENTRY_SYMBOL).unwrap_or_else(|| {
            self.module
                .add_function(EXT_ASCII_MASK_ENTRY_SYMBOL, ptr.fn_type(&[ptr.into()], false), Some(Linkage::External))
        });
        let filled = builder
            .build_call(entry_fn, &[set.into()], &format!("{name}_filled"))
            .expect("build_call should not fail for pycc_ext_ascii_mask_entry")
            .try_as_basic_value()
            .expect_basic("pycc_ext_ascii_mask_entry returns the entry")
            .into_pointer_value();
        builder.build_unconditional_branch(found).expect("branch");
        builder.position_at_end(found);
        let entry = builder.build_phi(ptr, &format!("{name}_mask")).expect("phi");
        entry.add_incoming(&[(&first, entry_block), (&second, try_second), (&filled, fill)]);
        let entry = entry.as_basic_value().into_pointer_value();
        let present = builder
            .build_is_not_null(entry, &format!("{name}_present"))
            .expect("build_is_not_null should not fail");
        self.require(present, &format!("{name}_present"));
        let usable = self
            .load(i64_type.into(), self.at(entry, MASK_USABLE, &format!("{name}_usable_at")), false, &format!("{name}_usable"))
            .into_int_value();
        let usable = builder
            .build_int_compare(IntPredicate::NE, usable, self.i64(0), &format!("{name}_is_usable"))
            .expect("icmp");
        self.require(usable, &format!("{name}_is_usable"));
        let bits = self.at(entry, MASK_BITS, &format!("{name}_bits"));
        let low = self.load(i64_type.into(), bits, false, &format!("{name}_low")).into_int_value();
        let high = self
            .load(i64_type.into(), self.at(bits, 8, &format!("{name}_high_at")), false, &format!("{name}_high"))
            .into_int_value();
        (low, high)
    }

    /// [`Fast::ascii_member`] for the one-character ASCII `str` of code
    /// `byte` (below 128), requiring an exact `frozenset` of exact `str`s.
    pub(crate) fn ascii_byte_member(&self, set: PointerValue<'ctx>, byte: IntValue<'ctx>, name: &str) -> IntValue<'ctx> {
        let builder = self.builder;
        let i64_type = self.context.i64_type();
        let ptr = self.context.ptr_type(AddressSpace::default());
        let (first, second) = self.mask_ways(set, name);
        let entry_block = builder.get_insert_block().expect("positioned");
        let try_second = self.context.append_basic_block(self.function, &format!("{name}_try_second"));
        let fill = self.context.append_basic_block(self.function, &format!("{name}_fill"));
        let found = self.context.append_basic_block(self.function, &format!("{name}_entry"));
        let first_set = self.load(ptr.into(), first, false, &format!("{name}_first_set")).into_pointer_value();
        let first_hit = builder
            .build_int_compare(IntPredicate::EQ, first_set, set, &format!("{name}_first_hit"))
            .expect("icmp");
        let branch = builder
            .build_conditional_branch(first_hit, found, try_second)
            .expect("build_conditional_branch should not fail for an i1");
        crate::int_fast::weight_branch(self.context, branch, true);
        builder.position_at_end(try_second);
        let second_set = self.load(ptr.into(), second, false, &format!("{name}_second_set")).into_pointer_value();
        let second_hit = builder
            .build_int_compare(IntPredicate::EQ, second_set, set, &format!("{name}_second_hit"))
            .expect("icmp");
        builder
            .build_conditional_branch(second_hit, found, fill)
            .expect("build_conditional_branch should not fail for an i1");
        builder.position_at_end(found);
        let entry = builder.build_phi(ptr, &format!("{name}_mask")).expect("phi");
        entry.add_incoming(&[(&first, entry_block), (&second, try_second)]);
        let entry = entry.as_basic_value().into_pointer_value();
        let usable = self
            .load(i64_type.into(), self.at(entry, MASK_USABLE, &format!("{name}_usable_at")), false, &format!("{name}_usable"))
            .into_int_value();
        let usable = builder
            .build_int_compare(IntPredicate::NE, usable, self.i64(0), &format!("{name}_is_usable"))
            .expect("icmp");
        let word_index = builder
            .build_right_shift(byte, self.i64(6), false, &format!("{name}_word_index"))
            .expect("lshr");
        let word_offset = builder
            .build_int_mul(word_index, self.i64(8), &format!("{name}_word_offset"))
            .expect("mul");
        let bits = self.at(entry, MASK_BITS, &format!("{name}_bits"));
        let word_at = self.byte_at(bits, word_offset, &format!("{name}_word_at"));
        let word = self.load(i64_type.into(), word_at, false, &format!("{name}_word")).into_int_value();
        let bit = builder.build_and(byte, self.i64(63), &format!("{name}_bit")).expect("and");
        let shifted = builder.build_right_shift(word, bit, false, &format!("{name}_word_shifted")).expect("lshr");
        let member = builder.build_and(shifted, self.i64(1), &format!("{name}_member")).expect("and");
        let cached = builder
            .build_select(usable, member, i64_type.const_all_ones(), &format!("{name}_cached"))
            .expect("select")
            .into_int_value();
        let done = self.context.append_basic_block(self.function, &format!("{name}_done"));
        builder.build_unconditional_branch(done).expect("branch");
        // A miss answers from the entry it computes, before the evicted set
        // is released: that release may run Python code (a weakref
        // callback) that refills this very bucket.
        builder.position_at_end(fill);
        let fill_fn = self.module.get_function(EXT_ASCII_MASK_FILL_SYMBOL).unwrap_or_else(|| {
            self.module.add_function(
                EXT_ASCII_MASK_FILL_SYMBOL,
                i64_type.fn_type(&[ptr.into(), i64_type.into()], false),
                Some(Linkage::External),
            )
        });
        let filled = builder
            .build_call(fill_fn, &[set.into(), byte.into()], &format!("{name}_filled"))
            .expect("build_call should not fail for pycc_ext_ascii_mask_fill")
            .try_as_basic_value()
            .expect_basic("pycc_ext_ascii_mask_fill returns the answer")
            .into_int_value();
        builder.build_unconditional_branch(done).expect("branch");
        builder.position_at_end(done);
        let status = builder.build_phi(i64_type, &format!("{name}_answer")).expect("phi");
        status.add_incoming(&[(&cached, found), (&filled, fill)]);
        let status = status.as_basic_value().into_int_value();
        let known = builder
            .build_int_compare(IntPredicate::SGE, status, self.i64(0), &format!("{name}_known"))
            .expect("icmp");
        self.require(known, &format!("{name}_known"));
        builder
            .build_int_truncate(status, self.context.i32_type(), &format!("{name}_status"))
            .expect("trunc")
    }
}
