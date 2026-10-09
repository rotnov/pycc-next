//! `build_at_entry_block` (#1254): the entry-block placement both foreign
//! call slots and an expression comprehension's loop-variable slot share.

use super::*;

/// Builds a function whose entry block is `entry` and whose builder sits in
/// a second block, then places one alloca through the helper.
fn place_alloca(entry_has_instruction: bool) -> (String, String) {
    let context = Context::create();
    let module = context.create_module("entry_block");
    let function = module.add_function("f", context.void_type().fn_type(&[], false), None);
    let entry = context.append_basic_block(function, "entry");
    let body = context.append_basic_block(function, "body");
    let builder = context.create_builder();
    builder.position_at_end(entry);
    if entry_has_instruction {
        builder
            .build_alloca(context.i8_type(), "existing")
            .expect("alloca");
    }
    builder.position_at_end(body);
    let slot = build_at_entry_block(&builder, function, |b| {
        b.build_alloca(context.i64_type(), "slot").expect("alloca")
    });
    assert_eq!(
        slot.as_instruction()
            .and_then(|instruction| instruction.get_parent()),
        Some(entry),
        "the slot is placed in the entry block"
    );
    assert_eq!(
        builder.get_insert_block(),
        Some(body),
        "the builder resumes where it was"
    );
    let names = |block: inkwell::basic_block::BasicBlock<'_>| {
        let mut out = Vec::new();
        let mut next = block.get_first_instruction();
        while let Some(instruction) = next {
            out.push(
                instruction
                    .get_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            );
            next = instruction.get_next_instruction();
        }
        out.join(",")
    };
    (names(entry), names(body))
}

#[test]
fn an_entry_slot_goes_before_the_first_entry_instruction() {
    assert_eq!(
        place_alloca(true),
        ("slot,existing".to_string(), String::new())
    );
}

#[test]
fn an_entry_slot_goes_into_an_empty_entry_block() {
    assert_eq!(place_alloca(false), ("slot".to_string(), String::new()));
}
