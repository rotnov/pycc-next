//! Class records (v0.10): one named global per compiled class, which every
//! instance of the class points at.
//!
//! A record is the class's layout descriptor (the bytes
//! `pycc_rt_instance_new` takes: the class name, each slot name and, for a
//! reference-counted class, the slot kinds, NUL-separated) preceded by a
//! fixed header:
//!
//! ```text
//! offset  0  ptr  base     the base class's descriptor, or null
//! offset  8  ptr  vtable   reserved for cross-module dispatch, null
//! offset 16  i64  entries  the vtable's length, 0
//! offset 24  i64  len      the descriptor's length in bytes
//! offset 32  [len x i8]    the descriptor
//! ```
//!
//! An instance's layout pointer is the descriptor's address (record + 32),
//! so the instance's run-time class is that pointer: a dispatcher tests a
//! receiver's class with one load and one compare (`pycc_mir::dispatch`),
//! whatever the classes are named. The global is external (hidden), never
//! `unnamed_addr`, so every object of a split emit and the shim's generated
//! C (`tp_init`, `tp_new`, which construct instances from Python calls)
//! name the same one: [`record_symbol`].

use std::collections::HashMap;

use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::values::{GlobalValue, IntValue, PointerValue};

/// The prefix of every class record's symbol.
pub const CLASS_RECORD_PREFIX: &str = "pycc_class_";

/// Bytes from a record's start to its descriptor.
pub const CLASS_RECORD_HEADER: u64 = 32;

/// The symbol of class `class`'s record: [`CLASS_RECORD_PREFIX`] and the
/// name with every byte outside `[A-Za-z0-9]` escaped (`_` as `__`, any
/// other as `_` and two hex digits), so it is a C identifier and distinct
/// names stay distinct.
#[must_use]
pub fn record_symbol(class: &str) -> String {
    let mut out = String::from(CLASS_RECORD_PREFIX);
    for byte in class.bytes() {
        match byte {
            b'_' => out.push_str("__"),
            b if b.is_ascii_alphanumeric() => out.push(b as char),
            b => out.push_str(&format!("_{b:02x}")),
        }
    }
    out
}

/// The descriptor of class `name` with slots `slots` and, when known, its
/// slot kinds.
fn descriptor(name: &str, slots: &[String], kinds: Option<&str>) -> String {
    let mut layout = name.to_string();
    for slot in slots {
        layout.push('\0');
        layout.push_str(slot);
    }
    if let Some(kinds) = kinds {
        layout.push('\0');
        layout.push_str(kinds);
    }
    layout
}

/// One class's record as emitted: its global and its descriptor's length.
#[derive(Clone, Copy)]
pub(crate) struct Record<'ctx> {
    global: GlobalValue<'ctx>,
    len: u64,
    slots: usize,
}

impl<'ctx> Record<'ctx> {
    /// The descriptor's address: the class's identity.
    pub(crate) fn descriptor(&self, context: &'ctx Context) -> PointerValue<'ctx> {
        let i8_type = context.i8_type();
        // SAFETY: the offset lies inside the record global.
        unsafe {
            self.global.as_pointer_value().const_gep(
                i8_type,
                &[context.i64_type().const_int(CLASS_RECORD_HEADER, false)],
            )
        }
    }

    pub(crate) fn len(&self, context: &'ctx Context) -> IntValue<'ctx> {
        context.i64_type().const_int(self.len, false)
    }

    pub(crate) fn slots(&self) -> usize {
        self.slots
    }
}

/// Emits a record for every class of `class_defs` whose values are
/// `pycc_rt` instances (every class but an exception class raised as an
/// exception object, and the sibling classes standing for another
/// module's, whose instances carry that module's records), keyed by class
/// name.
pub(crate) fn emit_records<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    class_defs: &[(String, pycc_hir::HirClassDef)],
    kinds: &HashMap<String, String>,
    exception_objects: &std::collections::HashSet<String>,
) -> HashMap<String, Record<'ctx>> {
    let by_name: HashMap<&str, &pycc_hir::HirClassDef> = class_defs
        .iter()
        .map(|(name, def)| (name.as_str(), def))
        .collect();
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i64_type = context.i64_type();
    let mut records: HashMap<String, Record<'ctx>> = HashMap::new();
    // Bases first, so a record's `base` field names an emitted record.
    let mut ordered: Vec<&(String, pycc_hir::HirClassDef)> = class_defs.iter().collect();
    ordered.sort_by_key(|(_, def)| def.mro.len());
    for (name, def) in ordered {
        if name.starts_with(pycc_hir::SIBLING_CLASS_PREFIX) || exception_objects.contains(name) {
            continue;
        }
        let mro: Vec<&pycc_hir::HirClassDef> = def
            .mro
            .iter()
            .filter_map(|entry| by_name.get(entry.as_str()).copied())
            .collect();
        if mro.len() != def.mro.len() {
            continue;
        }
        let slots: Vec<String> = pycc_hir::flat_attr_layout(&mro)
            .into_iter()
            .map(|(slot, _)| slot)
            .collect();
        let layout = descriptor(name, &slots, kinds.get(name).map(String::as_str));
        let bytes = context.const_string(layout.as_bytes(), false);
        let record_type = context.struct_type(
            &[
                ptr.into(),
                ptr.into(),
                i64_type.into(),
                i64_type.into(),
                bytes.get_type().into(),
            ],
            false,
        );
        let base = def
            .mro
            .iter()
            .skip(1)
            .find_map(|base| records.get(base))
            .map_or_else(|| ptr.const_null(), |base| base.descriptor(context));
        let global = module.add_global(record_type, None, &record_symbol(name));
        global.set_initializer(&context.const_struct(
            &[
                base.into(),
                ptr.const_null().into(),
                i64_type.const_zero().into(),
                i64_type.const_int(layout.len() as u64, false).into(),
                bytes.into(),
            ],
            false,
        ));
        global.set_constant(true);
        global.set_linkage(Linkage::External);
        global.set_visibility(inkwell::GlobalVisibility::Hidden);
        global.set_alignment(8);
        records.insert(
            name.clone(),
            Record {
                global,
                len: layout.len() as u64,
                slots: slots.len(),
            },
        );
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_symbols_are_c_identifiers_and_distinct() {
        assert_eq!(record_symbol("Grid"), "pycc_class_Grid");
        assert_eq!(record_symbol("my_cls"), "pycc_class_my__cls");
        assert_eq!(record_symbol("A.b"), "pycc_class_A_2eb");
        assert_ne!(record_symbol("a_2eb"), record_symbol("a.b"));
        assert_eq!(record_symbol("é"), "pycc_class__c3_a9");
    }

    #[test]
    fn a_descriptor_names_the_class_its_slots_and_their_kinds() {
        assert_eq!(
            descriptor("C", &["x".to_string(), "y".to_string()], Some("si")),
            "C\0x\0y\0si"
        );
        assert_eq!(descriptor("E", &[], None), "E");
    }
}
