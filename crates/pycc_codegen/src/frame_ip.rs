//! Which of a live frame's `instr_ptr` stores a function keeps (`frames`).
//!
//! A live frame's current line is what CPython reads while the function
//! calls out -- `sys._getframe().f_lineno`, `inspect.stack()`, a warning's
//! line, a frame object taken over when the function returns. Codegen
//! stores the position at every statement and site that moves the line;
//! a store is needed only when some path from it reaches a call that can
//! run CPython code before the next store. A hot native loop calls none,
//! so its stores go: the rest are erased once the function is complete.
//!
//! A call can run CPython code unless it is an LLVM intrinsic, a `pycc_rt`
//! function (the native runtime), or one of the shim's helpers that run
//! no Python code that looks at frames: packing a value, a reference
//! count (a finalizer a release runs sees the line of the last call out),
//! an error's bridge, a global's re-read, the frame helpers. Traceback
//! records carry their position themselves.

use inkwell::basic_block::BasicBlock;
use inkwell::values::{FunctionValue, InstructionOpcode, InstructionValue, Operand};
use std::collections::{HashMap, HashSet};

/// Whether `instruction` (a call) can run CPython code.
fn reaches_cpython(instruction: InstructionValue<'_>) -> bool {
    let count = instruction.get_num_operands();
    let Some(Operand::Value(callee)) = count
        .checked_sub(1)
        .and_then(|at| instruction.get_operand(at))
    else {
        return true;
    };
    let name = callee.get_name().to_string_lossy().into_owned();
    !(name.starts_with("llvm.")
        || name.starts_with("pycc_rt_")
        || name.starts_with("pycc_ext_obj_pack_")
        || matches!(
            name.as_str(),
            "pycc_ext_frame_enter"
                | "pycc_ext_tb_here_live"
                | "pycc_ext_obj_error_bridge"
                | "pycc_ext_obj_retain"
                | "pycc_ext_obj_release"
                | "pycc_ext_name_error"
                | "pycc_ext_global_sync"
        ))
}

/// What a block does first, scanning from its start.
#[derive(Clone, Copy, PartialEq, Eq)]
enum First {
    /// Calls out before any store: the line must be current on entry.
    Reads,
    /// Stores before any such call.
    Stores,
    /// Neither.
    Nothing,
}

/// The successors of `block`.
fn successors(block: BasicBlock<'_>) -> Vec<BasicBlock<'_>> {
    let Some(terminator) = block.get_terminator() else {
        return Vec::new();
    };
    (0..terminator.get_num_operands())
        .filter_map(|at| match terminator.get_operand(at) {
            Some(Operand::Block(target)) => Some(target),
            _ => None,
        })
        .collect()
}

/// Erases each of `stores` (the `instr_ptr` stores of `function`'s live
/// frame) after which no call that can run CPython code comes before the
/// next one.
pub(crate) fn prune<'ctx>(function: FunctionValue<'ctx>, stores: &[InstructionValue<'ctx>]) {
    let stores: HashSet<InstructionValue<'ctx>> = stores.iter().copied().collect();
    if stores.is_empty() {
        return;
    }
    let blocks = function.get_basic_blocks();
    let is_call = |i: InstructionValue<'ctx>| i.get_opcode() == InstructionOpcode::Call;
    let first: HashMap<BasicBlock<'ctx>, First> = blocks
        .iter()
        .map(|&block| {
            let mut at = block.get_first_instruction();
            let mut first = First::Nothing;
            while let Some(i) = at {
                if stores.contains(&i) {
                    first = First::Stores;
                    break;
                }
                if is_call(i) && reaches_cpython(i) {
                    first = First::Reads;
                    break;
                }
                at = i.get_next_instruction();
            }
            (block, first)
        })
        .collect();
    let succ: HashMap<BasicBlock<'ctx>, Vec<BasicBlock<'ctx>>> =
        blocks.iter().map(|&b| (b, successors(b))).collect();
    // `live_in`: entering the block, the line may be read before a store.
    let mut live_in: HashMap<BasicBlock<'ctx>, bool> = blocks
        .iter()
        .map(|&b| (b, first[&b] == First::Reads))
        .collect();
    let live_out = |b: BasicBlock<'ctx>, live_in: &HashMap<BasicBlock<'ctx>, bool>| {
        succ[&b]
            .iter()
            .any(|s| live_in.get(s).copied().unwrap_or(true))
    };
    loop {
        let mut changed = false;
        for &block in blocks.iter().rev() {
            if first[&block] == First::Nothing && !live_in[&block] && live_out(block, &live_in) {
                live_in.insert(block, true);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut dead = Vec::new();
    for &block in &blocks {
        let out = live_out(block, &live_in);
        // Scanning backward: whether the line is read before the next store.
        let mut needed = out;
        let mut at = block.get_last_instruction();
        while let Some(i) = at {
            if stores.contains(&i) {
                if !needed {
                    dead.push(i);
                }
                needed = false;
            } else if is_call(i) && reaches_cpython(i) {
                needed = true;
            }
            at = i.get_previous_instruction();
        }
    }
    for store in dead {
        store.erase_from_basic_block();
    }
}
