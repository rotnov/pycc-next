//! Type-based alias analysis (`!tbaa`) tags for the memory compiled code
//! addresses directly, so LLVM can keep values in registers across stores
//! that provably cannot reach them.
//!
//! Each tag names one kind of location, and every location is only ever
//! accessed by compiled code through its one tag:
//!
//! - an instance slot's word or assigned flag, by slot index: slot `k` sits
//!   at a fixed offset of every instance (`attr_slot`), instances never
//!   overlap, so a store to slot `k` of any instance cannot change slot
//!   `m != k` of any other, nor any flag;
//! - a list's `len` and `data` header fields, and its elements (`i64`
//!   words, or `list[bool]`'s bytes), which live in a separate buffer;
//! - the pending-exception flag `pycc_rt_exc_pending`, which only the
//!   runtime writes.
//!
//! The tags are scalar type nodes under one root, so any two distinct tags
//! never alias. Untagged accesses, and calls into the runtime, still alias
//! everything, which keeps every runtime write visible.

use inkwell::context::Context;
use inkwell::values::InstructionValue;

/// Tags the load or store `instruction` with the TBAA type `name`.
pub(super) fn tag<'ctx>(context: &'ctx Context, instruction: InstructionValue<'ctx>, name: &str) {
    let i64_type = context.i64_type();
    let root = context.metadata_node(&[context.metadata_string("pycc tbaa").into()]);
    let ty = context.metadata_node(&[
        context.metadata_string(name).into(),
        root.into(),
        i64_type.const_zero().into(),
    ]);
    let access = context.metadata_node(&[ty.into(), ty.into(), i64_type.const_zero().into()]);
    instruction
        .set_metadata(access, context.get_kind_id("tbaa"))
        .expect("a load or store accepts `!tbaa` metadata");
}

/// The tag of instance slot `slot`'s word (`flag == false`) or assigned flag.
pub(super) fn slot_name(slot: u64, flag: bool) -> String {
    format!("pycc slot {slot} {}", if flag { "flag" } else { "word" })
}

/// The tag of a list header field: `0` is `len`, `1` is `data`.
pub(super) fn list_field_name(field: u32) -> &'static str {
    if field == 0 { "pycc list len" } else { "pycc list data" }
}

/// The tag of a list element of `width_bits` (8 or 64).
pub(super) fn list_element_name(width_bits: u32) -> &'static str {
    if width_bits == 8 { "pycc list byte" } else { "pycc list word" }
}

/// The tag of the pending-exception flag.
pub(super) const EXC_PENDING: &str = "pycc exc pending";
