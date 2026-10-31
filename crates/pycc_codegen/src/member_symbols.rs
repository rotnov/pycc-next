//! The symbols of a module compiled as one member of a package's extension
//! (v0.8, the whole-package build).
//!
//! Every module of a package is linked into one shared object
//! (`pkg/__init__.abi3.so`) with one copy of the runtime, so two modules may
//! not define a symbol of the same name: each module's object names what it
//! defines, and what its own C unit defines for it, with a prefix unique to
//! the module ([`member_prefix`]). The rename runs on the optimized module,
//! before the backend (and before `split_emit` promotes the module's local
//! symbols, which take the same prefix), so no pass sees a renamed symbol.
//! The symbols compiled code shares with the runtime unit of the shim and
//! with `pycc_rt` keep their names: one definition serves every module.

use inkwell::llvm_sys::core::{
    LLVMGetFirstFunction, LLVMGetFirstGlobal, LLVMGetLinkage, LLVMGetNextFunction,
    LLVMGetNextGlobal, LLVMGetValueName2, LLVMIsDeclaration, LLVMSetValueName2, LLVMSetVisibility,
};
use inkwell::llvm_sys::prelude::LLVMValueRef;
use inkwell::llvm_sys::{LLVMLinkage, LLVMVisibility};
use inkwell::module::Module;

/// What the module unit of the shim defines for compiled code: the entry
/// points over the module's own record (`src/ext/module/module_ctx.c`) and
/// `pycc_ext_publish`. Compiled code calls them by these names, which a
/// member's object and its C unit (`pycc_ext_member.h`) both prefix.
pub const MODULE_UNIT_ENTRIES: [&str; 12] = [
    crate::ext::EXT_PUBLISH_SYMBOL,
    "pycc_ext_ns_dict",
    "pycc_ext_global_sync",
    "pycc_ext_obj_pack_instance",
    "pycc_ext_obj_pack_list",
    "pycc_ext_obj_tuple_rebox",
    "pycc_ext_obj_isinstance_compiled",
    "pycc_ext_obj_unbox_instance",
    crate::ext::EXT_OBJ_IMPORT_SYMBOL,
    crate::ext::EXT_OBJ_IMPORT_DOTTED_SYMBOL,
    crate::ext::EXT_OBJ_IMPORT_FROM_SYMBOL,
    crate::object_coerce::EXT_OBJ_COERCE_INSTANCE_SYMBOL,
];

/// The prefix of the symbols of the module `dotted` (`pkg.sub.mod`): each
/// dotted part as its length and its text, so distinct names never meet
/// (`pycc_P3pkg3sub3mod_`).
pub fn member_prefix(dotted: &str) -> String {
    let mut prefix = String::from("pycc_P");
    for part in dotted.split('.') {
        prefix.push_str(&part.len().to_string());
        prefix.push_str(part);
    }
    prefix.push('_');
    prefix
}

fn name(value: LLVMValueRef) -> String {
    let mut len = 0;
    // SAFETY: `value` is a live global of the module; LLVM returns a
    // pointer to `len` bytes of its name.
    unsafe {
        let ptr = LLVMGetValueName2(value, &mut len);
        if ptr.is_null() {
            return String::new();
        }
        String::from_utf8_lossy(std::slice::from_raw_parts(ptr.cast::<u8>(), len)).into_owned()
    }
}

/// Prefixes, with `prefix`, every symbol `module` defines that the link
/// sees (made hidden: nothing outside the extension names it), and every
/// declaration of a [`MODULE_UNIT_ENTRIES`] name. Local symbols keep their
/// names until `split_emit` promotes them under the same prefix.
pub(crate) fn rename(module: &Module<'_>, prefix: &str) {
    let raw = module.as_mut_ptr();
    let mut values = Vec::new();
    // SAFETY: walks the module's own function and global lists.
    unsafe {
        let mut f = LLVMGetFirstFunction(raw);
        while !f.is_null() {
            values.push(f);
            f = LLVMGetNextFunction(f);
        }
        let mut g = LLVMGetFirstGlobal(raw);
        while !g.is_null() {
            values.push(g);
            g = LLVMGetNextGlobal(g);
        }
    }
    for value in values {
        let old = name(value);
        // SAFETY: queries of a live global value.
        let (declaration, linkage) =
            unsafe { (LLVMIsDeclaration(value) != 0, LLVMGetLinkage(value)) };
        let local = matches!(
            linkage,
            LLVMLinkage::LLVMInternalLinkage | LLVMLinkage::LLVMPrivateLinkage
        );
        let renamed = if declaration {
            MODULE_UNIT_ENTRIES.contains(&old.as_str())
        } else {
            !local && !old.is_empty() && !old.starts_with("llvm.")
        };
        if !renamed {
            continue;
        }
        let new = format!("{prefix}{old}");
        // SAFETY: renames a live global value; LLVM copies the name.
        unsafe {
            LLVMSetValueName2(value, new.as_ptr().cast(), new.len());
            if !declaration {
                LLVMSetVisibility(value, LLVMVisibility::LLVMHiddenVisibility);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_prefixes_keep_distinct_dotted_names_distinct() {
        assert_eq!(member_prefix("pkg.sub.mod"), "pycc_P3pkg3sub3mod_");
        assert_ne!(member_prefix("a.b_c"), member_prefix("a_b.c"));
        assert_ne!(member_prefix("ab.c"), member_prefix("a.bc"));
    }

    #[test]
    fn rename_prefixes_definitions_and_module_unit_entries_only() {
        let context = inkwell::context::Context::create();
        let module = context.create_module("m");
        let void = context.void_type().fn_type(&[], false);
        let defined = module.add_function("fnptr_f", void, None);
        let block = context.append_basic_block(defined, "entry");
        let builder = context.create_builder();
        builder.position_at_end(block);
        builder.build_return(None).unwrap();
        let local = module.add_function("helper", void, Some(inkwell::module::Linkage::Internal));
        let block = context.append_basic_block(local, "entry");
        builder.position_at_end(block);
        builder.build_return(None).unwrap();
        module.add_function("pycc_ext_publish", void, None);
        module.add_function("pycc_ext_obj_getattr", void, None);
        module.add_function("pycc_rt_str_concat", void, None);
        rename(&module, "pycc_P1m_");
        assert!(module.get_function("pycc_P1m_fnptr_f").is_some());
        assert!(module.get_function("helper").is_some());
        assert!(module.get_function("pycc_P1m_pycc_ext_publish").is_some());
        assert!(module.get_function("pycc_ext_obj_getattr").is_some());
        assert!(module.get_function("pycc_rt_str_concat").is_some());
    }
}
