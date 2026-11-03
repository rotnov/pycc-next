//! A `str` literal that only crosses into CPython becomes a CPython `str`
//! directly (v0.12).
//!
//! A literal's site (`str_truthy::emit_string_literal`) builds a native
//! `PyStrObj` on its first evaluation (`pycc_rt_str_from_literal_cached`:
//! the bytes copied, an FNV table and its mutex for the name-like ones) and
//! a pack (`pycc_ext_obj_pack_str`) then copies its text into a CPython
//! `str`, interned when CPython would intern the text as a code constant.
//! In an `--ext` module body most literals are only ever packed -- a call's
//! argument, a decorator's, a namespace store -- and each body runs once,
//! so every site paid both on its only evaluation: about 1.7M instructions
//! of pydantic's import for 2,100 sites.
//!
//! This pass runs on the generated module before verification: every
//! literal site whose value is only ever handed to `pycc_ext_obj_pack_str`
//! is rewritten to `pycc_ext_obj_str_literal(bytes, len, slot, intern)`,
//! which makes the CPython `str` from the literal's bytes on the first
//! evaluation -- interned exactly when the pack interned it, which the
//! build decides once ([`interned_as_constant`]) -- keeps it in `slot` for
//! the process's life, and answers a new reference, as the pack did. The
//! sites of one text share one slot, as CPython merges a module's equal
//! constants; the native object, its cache and the branch that built it
//! are deleted. What Python code sees is the same text with the same
//! interning; an uninterned text is one object per module, not per site,
//! as in CPython.
//!
//! The pass matches the exact shape `emit_string_literal` emits and leaves
//! anything else alone, so a site whose value is also used natively keeps
//! its native object.

use inkwell::llvm_sys::core::{
    LLVMAddFunction, LLVMAddGlobal, LLVMBuildBr, LLVMBuildCall2, LLVMConstNull,
    LLVMCountIncoming, LLVMCreateBuilderInContext, LLVMDeleteBasicBlock, LLVMDeleteGlobal,
    LLVMDisposeBuilder, LLVMFunctionType, LLVMGetBasicBlockParent, LLVMGetBasicBlockTerminator,
    LLVMGetCalledValue, LLVMGetCondition, LLVMGetFirstBasicBlock, LLVMGetFirstFunction,
    LLVMGetFirstInstruction, LLVMGetFirstUse, LLVMGetIncomingBlock, LLVMGetIncomingValue,
    LLVMGetInstructionParent, LLVMGetModuleContext, LLVMGetNamedFunction, LLVMGetNextBasicBlock,
    LLVMGetNextFunction, LLVMGetNextInstruction, LLVMGetNextUse, LLVMGetNumArgOperands,
    LLVMGetNumOperands, LLVMGetNumSuccessors, LLVMGetOperand, LLVMGetSuccessor, LLVMGetUser,
    LLVMGetValueName2, LLVMInstructionEraseFromParent, LLVMInt64TypeInContext, LLVMIsABranchInst,
    LLVMIsACallInst, LLVMIsAGlobalVariable, LLVMIsAICmpInst, LLVMIsALoadInst, LLVMIsAPHINode,
    LLVMIsConditional, LLVMPointerTypeInContext, LLVMPositionBuilderBefore, LLVMGetAsString,
    LLVMGetInitializer, LLVMInt32TypeInContext, LLVMConstInt,
    LLVMReplaceAllUsesWith, LLVMSetInitializer, LLVMSetLinkage,
};
use inkwell::llvm_sys::prelude::{LLVMBasicBlockRef, LLVMModuleRef, LLVMValueRef};
use inkwell::llvm_sys::LLVMLinkage;
use inkwell::module::Module;

/// The shim's helper the rewritten sites call (`src/ext/module/object_ops.c`).
pub const EXT_OBJ_STR_LITERAL_SYMBOL: &str = "pycc_ext_obj_str_literal";

/// The runtime's literal constructor `emit_string_literal` calls.
const FROM_LITERAL: &str = "pycc_rt_str_from_literal_cached";

/// One literal site of the shape `emit_string_literal` emits:
///
/// ```text
/// entry:  %cached = load ptr, ptr @slot
///         %unbuilt = icmp eq ptr %cached, null
///         br i1 %unbuilt, label %build, label %join
/// build:  %fresh = call ptr @pycc_rt_str_from_literal_cached(ptr @text, i64 len, ptr @slot)
///         br label %join
/// join:   %str = phi ptr [ %cached, %entry ], [ %fresh, %build ]
/// ```
///
/// with every use of `%str` a `pycc_ext_obj_pack_str` call.
struct Site {
    phi: LLVMValueRef,
    cached: LLVMValueRef,
    unbuilt: LLVMValueRef,
    branch: LLVMValueRef,
    build: LLVMBasicBlockRef,
    join: LLVMBasicBlockRef,
    text: LLVMValueRef,
    len: LLVMValueRef,
    slot: LLVMValueRef,
    packs: Vec<LLVMValueRef>,
}

fn name(value: LLVMValueRef) -> &'static [u8] {
    let mut len = 0;
    // SAFETY: `value` is a live value; LLVM answers a pointer to `len` bytes
    // of its name, which the caller only compares before any rename.
    unsafe {
        let ptr = LLVMGetValueName2(value, &mut len);
        if ptr.is_null() {
            return &[];
        }
        std::slice::from_raw_parts(ptr.cast::<u8>(), len)
    }
}

/// The direct callee's name of call `call`, if it has one.
fn callee(call: LLVMValueRef) -> &'static [u8] {
    // SAFETY: `call` is a live call instruction.
    unsafe { name(LLVMGetCalledValue(call)) }
}

/// Every user of `value`.
fn users(value: LLVMValueRef) -> Vec<LLVMValueRef> {
    let mut out = Vec::new();
    // SAFETY: walks `value`'s live use list.
    unsafe {
        let mut use_ = LLVMGetFirstUse(value);
        while !use_.is_null() {
            out.push(LLVMGetUser(use_));
            use_ = LLVMGetNextUse(use_);
        }
    }
    out
}

/// The site whose value is phi `phi`, when it has exactly the shape above
/// and only packs use it.
fn site(phi: LLVMValueRef) -> Option<Site> {
    // SAFETY: queries of live instructions, blocks and globals.
    unsafe {
        if LLVMIsAPHINode(phi).is_null() || LLVMCountIncoming(phi) != 2 {
            return None;
        }
        let (cached, entry) = (LLVMGetIncomingValue(phi, 0), LLVMGetIncomingBlock(phi, 0));
        let (fresh, build) = (LLVMGetIncomingValue(phi, 1), LLVMGetIncomingBlock(phi, 1));
        let join = LLVMGetInstructionParent(phi);
        if LLVMIsALoadInst(cached).is_null()
            || LLVMIsACallInst(fresh).is_null()
            || callee(fresh) != FROM_LITERAL.as_bytes()
            || LLVMGetNumArgOperands(fresh) != 3
            || LLVMGetInstructionParent(fresh) != build
            || LLVMGetInstructionParent(cached) != entry
        {
            return None;
        }
        let slot = LLVMGetOperand(cached, 0);
        let (text, len) = (LLVMGetOperand(fresh, 0), LLVMGetOperand(fresh, 1));
        if LLVMIsAGlobalVariable(slot).is_null()
            || LLVMGetOperand(fresh, 2) != slot
            || LLVMIsAGlobalVariable(text).is_null()
        {
            return None;
        }
        // `build` is the call and its branch to `join`, and nothing else.
        let after = LLVMGetNextInstruction(fresh);
        if LLVMGetFirstInstruction(build) != fresh
            || after.is_null()
            || after != LLVMGetBasicBlockTerminator(build)
            || LLVMIsConditional(after) != 0
            || LLVMGetSuccessor(after, 0) != join
        {
            return None;
        }
        // `entry` ends in the test of the cached value, to `build` or `join`.
        let branch = LLVMGetBasicBlockTerminator(entry);
        if branch.is_null()
            || LLVMIsABranchInst(branch).is_null()
            || LLVMIsConditional(branch) == 0
            || LLVMGetNumSuccessors(branch) != 2
            || LLVMGetSuccessor(branch, 0) != build
            || LLVMGetSuccessor(branch, 1) != join
        {
            return None;
        }
        let unbuilt = LLVMGetCondition(branch);
        if LLVMIsAICmpInst(unbuilt).is_null()
            || LLVMGetNumOperands(unbuilt) != 2
            || LLVMGetOperand(unbuilt, 0) != cached
            || users(unbuilt) != [branch]
            || users(fresh) != [phi]
            || users(build_value(build)) != [branch]
        {
            return None;
        }
        let mut cached_users = users(cached);
        cached_users.sort();
        let mut want = vec![unbuilt, phi];
        want.sort();
        if cached_users != want {
            return None;
        }
        let packs = users(phi);
        if packs.is_empty()
            || !packs.iter().all(|&user| {
                !LLVMIsACallInst(user).is_null()
                    && callee(user) == crate::ext::EXT_OBJ_PACK_STR_SYMBOL.as_bytes()
                    && LLVMGetNumArgOperands(user) == 1
                    && LLVMGetOperand(user, 0) == phi
            })
        {
            return None;
        }
        Some(Site {
            phi,
            cached,
            unbuilt,
            branch,
            build,
            join,
            text,
            len,
            slot,
            packs,
        })
    }
}

/// `build` as a value, for its uses: the branches to it (only the entry's
/// conditional branch, or `build` is reached some other way and the site
/// is not the one emitted).
fn build_value(build: LLVMBasicBlockRef) -> LLVMValueRef {
    // SAFETY: a basic block is a value.
    unsafe { inkwell::llvm_sys::core::LLVMBasicBlockAsValue(build) }
}

/// The bytes of literal global `text` (`emit_string_literal`'s constant
/// array, without a terminator).
fn literal_text(text: LLVMValueRef) -> Vec<u8> {
    let mut len = 0;
    // SAFETY: `text` is a global whose initializer is a constant byte array.
    unsafe {
        let data = LLVMGetAsString(LLVMGetInitializer(text), &mut len);
        if data.is_null() {
            return Vec::new();
        }
        std::slice::from_raw_parts(data.cast::<u8>(), len).to_vec()
    }
}

/// Whether CPython interns `text` as a code constant -- all ASCII letters,
/// digits and `_` (its `all_name_chars`), or one ASCII character or none
/// (singletons) -- as `pycc_ext_obj_pack_str` decides for a literal.
pub(crate) fn interned_as_constant(text: &[u8]) -> bool {
    match text {
        [] => true,
        [only] => only.is_ascii(),
        _ => text.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_'),
    }
}

/// Rewrites every literal site of `module` that only packs its value (see
/// the module docs). Answers how many sites it rewrote.
pub(crate) fn python_literals(module: &Module<'_>) -> usize {
    let raw: LLVMModuleRef = module.as_mut_ptr();
    let mut sites = Vec::new();
    // SAFETY: walks the module's own functions, blocks and instructions.
    unsafe {
        let mut function = LLVMGetFirstFunction(raw);
        while !function.is_null() {
            let mut block = LLVMGetFirstBasicBlock(function);
            while !block.is_null() {
                let mut instruction = LLVMGetFirstInstruction(block);
                while !instruction.is_null() {
                    if let Some(found) = site(instruction) {
                        sites.push(found);
                    }
                    instruction = LLVMGetNextInstruction(instruction);
                }
                block = LLVMGetNextBasicBlock(block);
            }
            function = LLVMGetNextFunction(function);
        }
    }
    if sites.is_empty() {
        return 0;
    }
    // SAFETY: every edit below is to the module's own IR, on values the
    // match above found live; each site's values are its own.
    unsafe {
        let context = LLVMGetModuleContext(raw);
        let ptr = LLVMPointerTypeInContext(context, 0);
        let i64_type = LLVMInt64TypeInContext(context);
        let i32_type = LLVMInt32TypeInContext(context);
        let mut params = [ptr, i64_type, ptr, i32_type];
        let fn_type = LLVMFunctionType(ptr, params.as_mut_ptr(), 4, 0);
        let symbol = std::ffi::CString::new(EXT_OBJ_STR_LITERAL_SYMBOL).expect("no NUL");
        let mut helper = LLVMGetNamedFunction(raw, symbol.as_ptr());
        if helper.is_null() {
            helper = LLVMAddFunction(raw, symbol.as_ptr(), fn_type);
        }
        let builder = LLVMCreateBuilderInContext(context);
        let slot_name = c"py_lit_cache";
        // One slot per text: every site of the module with the same text
        // shares its object, as one code object's equal constants do.
        let mut slots: std::collections::HashMap<Vec<u8>, LLVMValueRef> =
            std::collections::HashMap::new();
        for site in &sites {
            debug_assert_eq!(LLVMGetBasicBlockParent(site.build), LLVMGetBasicBlockParent(site.join));
            let text = literal_text(site.text);
            let interned = LLVMConstInt(i32_type, u64::from(interned_as_constant(&text)), 0);
            let slot = *slots.entry(text).or_insert_with(|| {
                let slot = LLVMAddGlobal(raw, ptr, slot_name.as_ptr());
                LLVMSetInitializer(slot, LLVMConstNull(ptr));
                LLVMSetLinkage(slot, LLVMLinkage::LLVMPrivateLinkage);
                slot
            });
            for &pack in &site.packs {
                LLVMPositionBuilderBefore(builder, pack);
                let mut args = [site.text, site.len, slot, interned];
                // The pack's own name, for the IR's readers.
                let label = std::ffi::CString::new(name(pack)).unwrap_or_default();
                let call =
                    LLVMBuildCall2(builder, fn_type, helper, args.as_mut_ptr(), 4, label.as_ptr());
                LLVMReplaceAllUsesWith(pack, call);
                LLVMInstructionEraseFromParent(pack);
            }
            // `entry` goes straight to `join`; the native object's branch,
            // block, test and cache load go.
            LLVMPositionBuilderBefore(builder, site.branch);
            LLVMBuildBr(builder, site.join);
            LLVMInstructionEraseFromParent(site.branch);
            LLVMInstructionEraseFromParent(site.phi);
            LLVMDeleteBasicBlock(site.build);
            LLVMInstructionEraseFromParent(site.unbuilt);
            LLVMInstructionEraseFromParent(site.cached);
            if LLVMGetFirstUse(site.slot).is_null() {
                LLVMDeleteGlobal(site.slot);
            }
        }
        LLVMDisposeBuilder(builder);
    }
    sites.len()
}
