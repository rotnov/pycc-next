use super::*;
use pycc_mir::{MirExpr, MirItem, MirModule, MirStmt, Ty};

// -----------------------------------------------------------------
// #624 (D-180) codegen-depth: the inline bigint-refcount guard.
//
// The behavioral tests for this feature live in
// `tests/issue_146_bigint_release.rs` (real source through the real
// CLI, plus two peak-RSS oracles). Those prove the *net effect* -- a
// loop's peak RSS stops tracking its trip count -- but they cannot see
// the two structural properties the emitter is actually responsible
// for, because both are invisible in a program's output and in its RSS:
//
//   1. Every `pycc_rt_bigint_retain`/`_release` call is reached only
//      through `emit_bigint_refcount_call`'s inline `(word & 0b11) == 0
//      && word != 0` guard, on the *same* word the call receives. An
//      unguarded call is correct-but-slow, so it regresses D-084/D-140's
//      nbody throughput floor silently rather than failing a test.
//   2. That guard splits the current basic block, so any caller
//      recording a block for a phi incoming edge must re-read
//      `get_insert_block()` afterwards. A stale phi predecessor is
//      malformed IR that LLVM's own verifier rejects.
//
// These are in-crate rather than in `tests/` on purpose:
// `compile_to_object_with_observer`, the only way to reach the emitted
// module before it becomes an object file, is private -- so this
// follows `an_oversized_int_literal_materializes_a_runtime_bigint` in
// the crate root's own test module (`tests.rs`), which is this
// repository's actual
// IR-inspection convention, rather than
// `tests/slice1_codegen_depth.rs`'s
// hand-built-MIR-through-`compile_to_object` one.
//
// They live beside the emitter they exercise rather than in the crate
// root's test module, under AGENTS.md's decomposability rule: this
// cluster is
// self-contained -- no test outside it uses `RefcountCall`,
// `guarded_bigint_refcount_calls`, `refcount_calls_in` or `int_name` --
// so it is exactly the cohesive unit that rule points at.
// -----------------------------------------------------------------

/// One `pycc_rt_bigint_retain`/`_release` call recovered from emitted
/// LLVM IR: the callee's bare symbol name and the encoded word it is
/// actually passed.
#[derive(Debug, PartialEq, Eq)]
struct RefcountCall {
    callee: String,
    word: String,
}

/// Returns every `pycc_rt_bigint_*` refcount call in `ir`, and panics
/// unless each one is dominated by exactly the guard
/// `emit_bigint_refcount_call` emits, evaluated on the same operand the
/// call receives.
///
/// The chain proved per call, backwards from the call:
/// the call sits in a `bigint_rc_call*` block; that block is the taken
/// arm of `br i1 %c, label %bigint_rc_call*, label %bigint_rc_cont*`;
/// `%c = and i1 %a, %b`; `%a = icmp eq i64 %t, 0` with `%t = and i64
/// %w, 3`; `%b = icmp ne i64 %w, 0`; and the call's own argument is
/// that same `%w`.
fn guarded_bigint_refcount_calls(ir: &str) -> Vec<RefcountCall> {
    // Every name `emit_bigint_refcount_call` produces is a fixed
    // literal, and LLVM disambiguates those only *within* a function --
    // the first guard of two different functions is `%bigint_is_heap`
    // in both. So each function body is analyzed on its own maps, and a
    // call can never be validated against a guard chain read out of
    // some other function. `split` yields the module header first,
    // which `skip(1)` drops.
    ir.split("\ndefine ")
        .skip(1)
        .flat_map(guarded_bigint_refcount_calls_in_one_function)
        .collect()
}

/// `guarded_bigint_refcount_calls` for a single `define` body.
fn guarded_bigint_refcount_calls_in_one_function(ir: &str) -> Vec<RefcountCall> {
    // `%name = <rhs>` for every SSA value in the function.
    let mut defs: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    // `taken block -> (condition, not-taken block)` for every `br i1`.
    let mut branches: std::collections::HashMap<&str, (&str, &str)> =
        std::collections::HashMap::new();
    // Every refcount call, tagged with the block it was found in.
    let mut calls: Vec<(String, RefcountCall)> = Vec::new();
    let mut block = String::new();

    for raw in ir.lines() {
        let line = raw.trim();
        // A block label is the only unindented `name:` line inside a
        // function body; LLVM appends a `; preds = ...` comment to it.
        // The leftover of the `define` line itself is the only such
        // line here, and it carries no colon.
        if !raw.starts_with([' ', '@', '%', '}', '!']) {
            if let Some((label, _)) = line.split_once(':') {
                block = label.to_string();
            }
            continue;
        }
        if let Some((name, rhs)) = line.split_once(" = ") {
            defs.insert(name, rhs);
        }
        if let Some(rest) = line.strip_prefix("br i1 ") {
            let parts: Vec<&str> = rest.split(", label ").collect();
            assert_eq!(
                parts.len(),
                3,
                "a conditional branch always has a condition and two targets"
            );
            branches.insert(parts[1], (parts[0], parts[2]));
        }
        if let Some(rest) = line.strip_prefix("call void @pycc_rt_bigint_") {
            let (op, args) = rest
                .split_once('(')
                .expect("a call instruction always has an argument list");
            let word = args
                .trim_end_matches(')')
                .strip_prefix("i64 ")
                .expect("both refcount entry points take exactly one i64")
                .to_string();
            calls.push((
                block.clone(),
                RefcountCall {
                    callee: format!("pycc_rt_bigint_{op}"),
                    word,
                },
            ));
        }
    }

    // Only `assert*!` and `expect` are used from here on, never a
    // local `unwrap_or_else(|| panic!(..))` closure: an error closure
    // that never runs is an uncovered function under D-014's region
    // gate, whereas `assert!`'s cold arm lives in `core`.
    for (block, call) in &calls {
        assert!(
            block.starts_with("bigint_rc_call"),
            "{} is emitted in block `{block}`, not a guard's own call block",
            call.callee
        );
        let key: &str = &format!("%{block}");
        let (cond, cont) = *branches
            .get(key)
            .expect("a guard's call block is always the target of its own branch");
        assert!(
            cont.starts_with("%bigint_rc_cont"),
            "`{block}`'s not-taken arm is `{cont}`, not a guard continuation"
        );
        let (aligned, non_zero) = defs[cond]
            .strip_prefix("and i1 ")
            .and_then(|rest| rest.split_once(", "))
            .expect("the guard condition is an `and` of its two halves");
        let low_tag = defs[aligned]
            .strip_prefix("icmp eq i64 ")
            .and_then(|rest| rest.strip_suffix(", 0"))
            .expect("the guard's alignment half compares a masked word against 0");
        let tagged_word = defs[low_tag]
            .strip_prefix("and i64 ")
            .and_then(|rest| rest.strip_suffix(", 3"))
            .expect("the guard's alignment half masks the word with 0b11");
        let tested_word = defs[non_zero]
            .strip_prefix("icmp ne i64 ")
            .and_then(|rest| rest.strip_suffix(", 0"))
            .expect("the guard's non-zero half compares the word against 0");
        assert_eq!(
            tagged_word, tested_word,
            "the guard's two halves test different words"
        );
        assert_eq!(
            tagged_word, call.word,
            "{} is passed a word the guard never tested",
            call.callee
        );
    }

    calls.into_iter().map(|(_, call)| call).collect()
}

/// Compiles `mir` and runs `guarded_bigint_refcount_calls` over the
/// emitted module.
///
/// LLVM's own verifier is what catches a phi whose incoming block
/// stopped being a real predecessor after `emit_bigint_refcount_call`
/// split it -- but this helper deliberately does not call it. Callers do
/// not need to: `compile_to_object_with_observer` already runs
/// `verify_module` on this exact module *before* it invokes the
/// observer, so simply reaching this closure means the module already
/// passed. Calling `module.verify()` a second time here would bypass
/// `verify_module`'s `#[cfg(windows)]` skip and reintroduce D-029:
/// inkwell's `Module::verify` calls `LLVMDisposeMessage` on its
/// *success* path too, which faults against the prebuilt LLVM 22.1.1
/// the Windows runner uses -- an access violation that kills the whole
/// test binary rather than failing one test.
fn refcount_calls_in(label: &str, mir: &MirModule) -> Vec<RefcountCall> {
    refcount_calls_with_options(label, mir, &CompileOptions::default())
}

/// `refcount_calls_in` with an explicit [`CompileOptions`], for the one
/// fixture whose MIR only compiles under `ext: true` (a `memoryview`
/// parameter, which D-244 admits nowhere else).
fn refcount_calls_with_options(
    label: &str,
    mir: &MirModule,
    options: &CompileOptions,
) -> Vec<RefcountCall> {
    let dir = pycc_scratch::ScratchDir::new(label).expect("failed to create scratch dir");
    let obj_path = dir.join(format!("{label}.o"));
    let mut calls = Vec::new();
    // D-029: never let `print_to_string`'s `LLVMString` drop; route it
    // through `llvm_string_to_owned` exactly as the sibling tests do.
    let mut observer = |module: &inkwell::module::Module<'_>, _| {
        calls = guarded_bigint_refcount_calls(&llvm_string_to_owned(module.print_to_string()));
    };
    compile_to_object_with_observer(mir, &obj_path, options, Some(&mut observer))
        .expect("codegen should succeed");
    calls
}

fn int_name(name: &str) -> MirExpr {
    MirExpr::Name {
        name: name.to_string(),
        ty: Ty::Int,
    }
}

// Topic submodules split out of this file.
mod operands;
mod slots;
