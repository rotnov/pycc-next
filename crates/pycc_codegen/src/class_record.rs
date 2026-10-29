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
//! offset  8  ptr  vtable   null, or the class's vtable (below)
//! offset 16  i64  entries  the vtable's length
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
//!
//! **Vtables.** A class compiled over a sibling class -- a subclass of a
//! class another module of a package built whole compiled
//! (`crate::sibling_class`) -- carries a vtable: one function pointer per
//! entry of that class's method table (its length and each listed method's
//! index come from the other module's interface). An entry is the function
//! the subclass resolves the method to -- its own override (or its
//! receiver-exact copy of an inherited one), or the other module's body --
//! when its signature is the other module's; null for an override of
//! another signature, which the other module's dispatchers then reach
//! through CPython. Its `base` is the other module's record, an external
//! symbol the link resolves.

use std::collections::HashMap;

use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::values::{FunctionValue, GlobalValue, IntValue, PointerValue};

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

/// The descriptor's address inside the record `global`.
pub(crate) fn descriptor_of<'ctx>(
    context: &'ctx Context,
    global: GlobalValue<'ctx>,
) -> PointerValue<'ctx> {
    // SAFETY: the offset lies inside the record global.
    unsafe {
        global.as_pointer_value().const_gep(
            context.i8_type(),
            &[context.i64_type().const_int(CLASS_RECORD_HEADER, false)],
        )
    }
}

/// A record global `symbol` holding `layout` behind a header naming `base`
/// (null for none) and `vtable` (its address and length); constant,
/// 8-aligned, never `unnamed_addr`. The caller sets its linkage.
pub(crate) fn record_global<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    symbol: &str,
    layout: &str,
    base: PointerValue<'ctx>,
    vtable: Option<(PointerValue<'ctx>, usize)>,
) -> GlobalValue<'ctx> {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i64_type = context.i64_type();
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
    let (table, entries) = vtable.unwrap_or((ptr.const_null(), 0));
    let global = module.add_global(record_type, None, symbol);
    global.set_initializer(&context.const_struct(
        &[
            base.into(),
            table.into(),
            i64_type.const_int(entries as u64, false).into(),
            i64_type.const_int(layout.len() as u64, false).into(),
            bytes.into(),
        ],
        false,
    ));
    global.set_constant(true);
    global.set_alignment(8);
    global
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
        descriptor_of(context, self.global)
    }

    pub(crate) fn len(&self, context: &'ctx Context) -> IntValue<'ctx> {
        context.i64_type().const_int(self.len, false)
    }

    pub(crate) fn slots(&self) -> usize {
        self.slots
    }
}

/// A function's MIR signature: its parameters after the receiver, and its
/// return.
pub(crate) type Signature = (Vec<pycc_mir::Ty>, pycc_mir::Ty);

/// What [`emit_records`] reads of the module's functions: each function's
/// LLVM value and MIR signature (parameters after the receiver, return), by
/// MIR name.
pub(crate) struct Functions<'a, 'ctx> {
    pub(crate) value: &'a dyn Fn(&str) -> Option<FunctionValue<'ctx>>,
    pub(crate) signature: &'a dyn Fn(&str) -> Option<Signature>,
}

/// The method `class` resolves `method` to on its MRO, as the function it
/// runs: the defining class and the receiver-exact copy for `class` when
/// one exists (`pycc_hir::inherited_copy_name`), else the definition.
fn resolve<'c>(
    class: &'c pycc_hir::HirClassDef,
    method: &str,
    by_name: &HashMap<&str, &'c pycc_hir::HirClassDef>,
    functions: &Functions<'_, '_>,
) -> Option<(&'c str, String)> {
    // The first class of the MRO binding the name, in any of its
    // namespaces, wins: a static or class method, a property, a class
    // attribute or an instance attribute there is no instance method, so
    // the entry stays null and the call goes through CPython.
    let (owner, mangled) = class.mro.iter().find_map(|entry| {
        let def = by_name.get(entry.as_str())?;
        if let Some((_, mangled)) = def.methods.iter().find(|(name, _)| name == method) {
            return Some(Some((def.name.as_str(), mangled.clone())));
        }
        let other = def.static_methods.iter().any(|(name, _)| name == method)
            || def.class_methods.iter().any(|(name, _)| name == method)
            || def.properties.iter().any(|prop| prop.name == method)
            || def.class_attrs.iter().any(|(name, _, _)| name == method)
            || def.attrs.iter().any(|(name, _)| name == method);
        other.then_some(None)
    })??;
    let class_of = |name: &str| by_name.get(name).copied();
    let copy = pycc_hir::inherited_copy_name(class, owner, &mangled, &class_of)
        .filter(|copy| owner != class.name && (functions.value)(copy).is_some());
    Some((owner, copy.unwrap_or(mangled)))
}

/// The vtable of `class`, compiled over the sibling class `sibling`
/// (module doc), as a global array.
fn vtable<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    name: &str,
    class: &pycc_hir::HirClassDef,
    sibling: &crate::SiblingClass,
    by_name: &HashMap<&str, &pycc_hir::HirClassDef>,
    functions: &Functions<'_, 'ctx>,
) -> (PointerValue<'ctx>, usize) {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let entries = sibling.entries.max(1);
    let mut table = vec![ptr.const_null(); entries];
    for (index, stub_method, body) in &sibling.vtable {
        let Some(at) = table.get_mut(*index) else {
            continue;
        };
        let method = stub_method
            .rsplit_once('.')
            .map_or(stub_method.as_str(), |(_, m)| m);
        let Some((owner, function)) = resolve(class, method, by_name, functions) else {
            continue;
        };
        // The other module's own body, of its own signature.
        if owner == sibling.class {
            if let Some(stub) = (functions.value)(stub_method) {
                let body = module
                    .get_function(body)
                    .unwrap_or_else(|| module.add_function(body, stub.get_type(), Some(Linkage::External)));
                *at = body.as_global_value().as_pointer_value();
            }
            continue;
        }
        // An override: only of the other module's signature.
        let same = (functions.signature)(&function)
            .zip((functions.signature)(stub_method))
            .is_some_and(|(own, theirs)| own == theirs);
        if let (true, Some(value)) = (same, (functions.value)(&function)) {
            *at = value.as_global_value().as_pointer_value();
        }
    }
    let array_type = ptr.array_type(entries as u32);
    let global = module.add_global(array_type, None, &format!("{}.vtable", record_symbol(name)));
    global.set_initializer(&ptr.const_array(&table));
    global.set_constant(true);
    global.set_linkage(Linkage::Private);
    (global.as_pointer_value(), entries)
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
    functions: &Functions<'_, 'ctx>,
) -> HashMap<String, Record<'ctx>> {
    let by_name: HashMap<&str, &pycc_hir::HirClassDef> = class_defs
        .iter()
        .map(|(name, def)| (name.as_str(), def))
        .collect();
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
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
        // The nearest base with a record of this module, else the other
        // module's record of the sibling class it derives from.
        let sibling = def
            .mro
            .iter()
            .find_map(|entry| crate::sibling_class::get(entry));
        let base = def.mro.iter().skip(1).find_map(|base| {
            if let Some(record) = records.get(base) {
                return Some(record.descriptor(context));
            }
            let sibling = crate::sibling_class::get(base)?;
            let global = module.get_global(&sibling.record).unwrap_or_else(|| {
                let global = module.add_global(context.i8_type(), None, &sibling.record);
                global.set_linkage(Linkage::External);
                global
            });
            Some(descriptor_of(context, global))
        });
        let table = sibling
            .as_ref()
            .map(|sibling| vtable(context, module, name, def, sibling, &by_name, functions));
        let global = record_global(
            context,
            module,
            &record_symbol(name),
            &layout,
            base.unwrap_or_else(|| ptr.const_null()),
            table,
        );
        global.set_linkage(Linkage::External);
        global.set_visibility(inkwell::GlobalVisibility::Hidden);
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
