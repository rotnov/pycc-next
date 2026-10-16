//! Parallel object emission (v0.5 accept 4).
//!
//! LLVM's backend (instruction selection, register allocation) runs one
//! function at a time, and for a large optimized module it is as slow as
//! the whole-module `default<O3>` pipeline before it: a few seconds for
//! diff-match-patch. Once the module is optimized it is split by function
//! into a few partitions that are compiled to objects on separate threads,
//! the way a C compiler handles several translation units, and the link
//! takes all of them. Nothing about the code changes: the optimizer ran on
//! the whole module before the split, and each function is emitted exactly
//! once.
//!
//! The split ([`plan`]):
//! - every module-local function and global (`internal`/`private`) becomes
//!   a hidden external symbol under a reserved name (`__pycc_local.`), so a
//!   reference from one partition reaches its definition in another, and
//!   stays invisible outside the artifact. A local constant whose address
//!   is insignificant (`unnamed_addr`: string and table data) is copied
//!   into every partition instead.
//! - each function definition is owned by one partition, largest first to
//!   the least loaded one; every global variable by the first.
//!
//! Each partition ([`emit`]) is a copy of the module, parsed from its
//! bitcode in a context of its own, in which every definition another
//! partition owns becomes `available_externally` and is then removed by
//! `elim-avail-extern`; `globaldce` drops the unused copied constants.

use super::*;
use inkwell::llvm_sys::core::{
    LLVMDeleteGlobal, LLVMGetFirstBasicBlock, LLVMGetFirstFunction, LLVMGetFirstGlobal,
    LLVMGetFirstGlobalAlias, LLVMGetFirstGlobalIFunc, LLVMGetFirstInstruction, LLVMGetLinkage,
    LLVMGetNextBasicBlock, LLVMGetNextFunction, LLVMGetNextGlobal, LLVMGetNextInstruction,
    LLVMGetUnnamedAddress, LLVMGetValueName2, LLVMIsDeclaration, LLVMIsGlobalConstant,
    LLVMIsThreadLocal, LLVMSetLinkage, LLVMSetValueName2, LLVMSetVisibility,
};
use inkwell::llvm_sys::prelude::{LLVMModuleRef, LLVMValueRef};
use inkwell::llvm_sys::{LLVMLinkage, LLVMUnnamedAddr, LLVMVisibility};
use inkwell::memory_buffer::MemoryBuffer;
use inkwell::module::Module;
use std::path::PathBuf;

/// Below this many instructions a module is emitted whole: splitting
/// copies the module once per partition, which costs more than it saves.
const MIN_INSTRUCTIONS: usize = 20_000;
/// The least work worth a partition of its own.
const MIN_PARTITION_INSTRUCTIONS: usize = 8_000;
/// At most this many partitions (threads).
const MAX_PARTITIONS: usize = 8;
/// The prefix of a promoted module-local symbol: reserved, so the
/// promotion cannot collide with a symbol of the runtime or the shim.
const LOCAL_PREFIX: &str = "__pycc_local.";
/// The backend recurses over expression trees; a partition's thread gets
/// the stack the main thread has.
const PARTITION_STACK: usize = 64 * 1024 * 1024;

/// Which partition owns each function definition, by name.
pub(crate) struct Partitions {
    count: usize,
    owner: HashMap<String, usize>,
}

fn name(value: LLVMValueRef) -> String {
    let mut len = 0;
    // SAFETY: `value` is a live global of the module being split; LLVM
    // returns a pointer to `len` bytes of its name, valid until renamed.
    unsafe {
        let ptr = LLVMGetValueName2(value, &mut len);
        if ptr.is_null() {
            return String::new();
        }
        String::from_utf8_lossy(std::slice::from_raw_parts(ptr.cast::<u8>(), len)).into_owned()
    }
}

fn set_name(value: LLVMValueRef, name: &str) {
    // SAFETY: `value` is a live global; LLVM copies the name.
    unsafe { LLVMSetValueName2(value, name.as_ptr().cast(), name.len()) }
}

/// Every function of `module`, then every global variable.
fn functions(module: LLVMModuleRef) -> Vec<LLVMValueRef> {
    let mut all = Vec::new();
    // SAFETY: walks the module's own function list.
    unsafe {
        let mut f = LLVMGetFirstFunction(module);
        while !f.is_null() {
            all.push(f);
            f = LLVMGetNextFunction(f);
        }
    }
    all
}

fn globals(module: LLVMModuleRef) -> Vec<LLVMValueRef> {
    let mut all = Vec::new();
    // SAFETY: walks the module's own global variable list.
    unsafe {
        let mut g = LLVMGetFirstGlobal(module);
        while !g.is_null() {
            all.push(g);
            g = LLVMGetNextGlobal(g);
        }
    }
    all
}

fn instruction_count(function: LLVMValueRef) -> usize {
    let mut count = 0;
    // SAFETY: walks the blocks and instructions of a function definition.
    unsafe {
        let mut block = LLVMGetFirstBasicBlock(function);
        while !block.is_null() {
            let mut inst = LLVMGetFirstInstruction(block);
            while !inst.is_null() {
                count += 1;
                inst = LLVMGetNextInstruction(inst);
            }
            block = LLVMGetNextBasicBlock(block);
        }
    }
    count
}

fn is_local(linkage: LLVMLinkage) -> bool {
    matches!(
        linkage,
        LLVMLinkage::LLVMInternalLinkage | LLVMLinkage::LLVMPrivateLinkage
    )
}

/// A local constant whose address nothing can observe: copied into every
/// partition rather than shared.
fn is_copied_constant(global: LLVMValueRef) -> bool {
    // SAFETY: queries of a live global variable.
    unsafe {
        is_local(LLVMGetLinkage(global))
            && LLVMIsGlobalConstant(global) != 0
            && LLVMIsThreadLocal(global) == 0
            && LLVMGetUnnamedAddress(global) != LLVMUnnamedAddr::LLVMNoUnnamedAddr
    }
}

/// How many partitions `module` should be emitted as, or `None` to emit it
/// whole; when split, its local symbols are promoted (module doc).
pub(crate) fn plan(module: &Module<'_>) -> Option<Partitions> {
    if cfg!(windows) {
        // D-029: object emission on Windows stays on the single path the
        // Windows tests pin.
        return None;
    }
    let threads = std::thread::available_parallelism().map_or(1, usize::from);
    plan_for(module, threads.min(MAX_PARTITIONS))
}

fn plan_for(module: &Module<'_>, threads: usize) -> Option<Partitions> {
    let raw = module.as_mut_ptr();
    // SAFETY: queries of the module's alias and ifunc lists.
    let has_aliases = unsafe {
        !LLVMGetFirstGlobalAlias(raw).is_null() || !LLVMGetFirstGlobalIFunc(raw).is_null()
    };
    if threads < 2 || has_aliases {
        return None;
    }
    let mut sizes: Vec<(LLVMValueRef, usize)> = Vec::new();
    for f in functions(raw) {
        // SAFETY: queries of a live function.
        let (declaration, linkage) = unsafe { (LLVMIsDeclaration(f) != 0, LLVMGetLinkage(f)) };
        if !declaration
            && matches!(
                linkage,
                LLVMLinkage::LLVMExternalLinkage
                    | LLVMLinkage::LLVMInternalLinkage
                    | LLVMLinkage::LLVMPrivateLinkage
            )
        {
            sizes.push((f, instruction_count(f)));
        }
    }
    let total: usize = sizes.iter().map(|(_, n)| n).sum();
    let count = threads.min(total / MIN_PARTITION_INSTRUCTIONS);
    if total < MIN_INSTRUCTIONS || count < 2 {
        return None;
    }
    promote_locals(raw);
    // Largest first, each to the least loaded partition (ties: the lowest).
    sizes.sort_by(|(fa, a), (fb, b)| b.cmp(a).then_with(|| name(*fa).cmp(&name(*fb))));
    let mut load = vec![0usize; count];
    let mut owner = HashMap::new();
    for (f, size) in sizes {
        let (part, _) = load
            .iter()
            .enumerate()
            .min_by_key(|(index, load)| (**load, *index))
            .expect("at least two partitions");
        load[part] += size;
        owner.insert(name(f), part);
    }
    Some(Partitions { count, owner })
}

/// Makes every local function and global (but a copied constant) a hidden
/// external symbol with a reserved name; an unnamed one gets a name.
fn promote_locals(module: LLVMModuleRef) {
    let mut anonymous = 0usize;
    let mut promote = |value: LLVMValueRef| {
        let old = name(value);
        let new = if old.is_empty() {
            anonymous += 1;
            format!("{LOCAL_PREFIX}anon.{anonymous}")
        } else {
            format!("{LOCAL_PREFIX}{old}")
        };
        set_name(value, &new);
        // SAFETY: relinks a live global of the module.
        unsafe {
            LLVMSetLinkage(value, LLVMLinkage::LLVMExternalLinkage);
            LLVMSetVisibility(value, LLVMVisibility::LLVMHiddenVisibility);
        }
    };
    for f in functions(module) {
        // SAFETY: queries of a live function.
        if unsafe { LLVMIsDeclaration(f) == 0 && is_local(LLVMGetLinkage(f)) } {
            promote(f);
        }
    }
    for g in globals(module) {
        // SAFETY: queries of a live global variable.
        if unsafe { LLVMIsDeclaration(g) == 0 && is_local(LLVMGetLinkage(g)) }
            && !is_copied_constant(g)
        {
            promote(g);
        }
    }
}

/// The object file of partition `index`: `output_path` itself for the first.
fn partition_path(output_path: &Path, index: usize) -> PathBuf {
    if index == 0 {
        return output_path.to_path_buf();
    }
    let stem = output_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    output_path.with_file_name(format!("{stem}.part{index}.o"))
}

/// Keeps in the copy `module` only what partition `index` owns.
fn keep_partition(module: LLVMModuleRef, index: usize, parts: &Partitions) {
    for f in functions(module) {
        // SAFETY: queries of a live function.
        if unsafe { LLVMIsDeclaration(f) != 0 } {
            continue;
        }
        if parts
            .owner
            .get(&name(f))
            .is_some_and(|owner| *owner != index)
        {
            // SAFETY: an owned-elsewhere external definition becomes an
            // available_externally one, which `elim-avail-extern` removes.
            unsafe { LLVMSetLinkage(f, LLVMLinkage::LLVMAvailableExternallyLinkage) };
        }
    }
    if index == 0 {
        return;
    }
    for g in globals(module) {
        // SAFETY: queries and relinking of a live global variable; an
        // appending (`llvm.used`, constructors) global is the first
        // partition's alone and nothing in the module uses it.
        unsafe {
            if LLVMIsDeclaration(g) != 0 {
                continue;
            }
            match LLVMGetLinkage(g) {
                LLVMLinkage::LLVMAppendingLinkage => LLVMDeleteGlobal(g),
                LLVMLinkage::LLVMExternalLinkage => {
                    LLVMSetLinkage(g, LLVMLinkage::LLVMAvailableExternallyLinkage)
                }
                _ => {}
            }
        }
    }
}

/// Emits partition `index` of the module whose bitcode is `bitcode`.
fn emit_partition(
    bitcode: &[u8],
    index: usize,
    parts: &Partitions,
    path: &Path,
    target_triple: Option<&str>,
    release: bool,
) -> Result<(), String> {
    let context = Context::create();
    let buffer = MemoryBuffer::create_from_memory_range_copy(bitcode, "pycc_partition");
    let module = Module::parse_bitcode_from_buffer(&buffer, &context).map_err(|e| {
        format!(
            "pycc_codegen: reading a partition back: {}",
            llvm_string_to_owned(e)
        )
    })?;
    keep_partition(module.as_mut_ptr(), index, parts);
    let target_machine = target_machine::target_machine_for(target_triple, release)?;
    module
        .run_passes(
            "elim-avail-extern,globaldce",
            &target_machine,
            PassBuilderOptions::create(),
        )
        .expect("a fixed, valid pipeline over a module read back from LLVM's own bitcode");
    target_machine
        .write_to_file(&module, FileType::Object, path)
        .map_err(llvm_string_to_owned)
}

/// Writes `module`, already promoted by [`plan`], as `parts.count` objects
/// compiled in parallel; returns their paths, `output_path` first.
pub(crate) fn emit(
    module: &Module<'_>,
    parts: &Partitions,
    output_path: &Path,
    target_triple: Option<&str>,
    release: bool,
) -> Result<Vec<PathBuf>, String> {
    let bitcode = module.write_bitcode_to_memory();
    let bitcode = bitcode.as_slice();
    let paths: Vec<PathBuf> = (0..parts.count)
        .map(|index| partition_path(output_path, index))
        .collect();
    std::thread::scope(|scope| {
        let workers: Vec<_> = paths
            .iter()
            .enumerate()
            .map(|(index, path)| {
                std::thread::Builder::new()
                    .stack_size(PARTITION_STACK)
                    .spawn_scoped(scope, move || {
                        emit_partition(bitcode, index, parts, path, target_triple, release)
                    })
                    .map_err(|e| format!("pycc_codegen: starting a backend thread: {e}"))
            })
            .collect();
        for worker in workers {
            worker?
                .join()
                .map_err(|_| "pycc_codegen: a backend thread panicked".to_string())??;
        }
        Ok::<(), String>(())
    })?;
    Ok(paths)
}

#[cfg(test)]
#[path = "split_emit_tests.rs"]
mod tests;
