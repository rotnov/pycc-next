//! The rich-comparison and hash slots of a carrier type (#1427): a compiled
//! class's `__eq__`, `__ne__`, `__lt__`, `__le__`, `__gt__`, `__ge__` and
//! `__hash__`, installed as `Py_tp_richcompare` and `Py_tp_hash` so the
//! host's `==`, `<`, `in`, `hash()` and dict lookups run them.
//!
//! **Why a slot and not a method row.** CPython never looks these names up
//! in an instance's method table: `PyObject_RichCompare` and
//! `PyObject_Hash` call the type's slots. Before #1427 a carrier type had
//! neither, so it inherited `object`'s, and `c == None` compared identity
//! even where the compiled `__eq__` answered `True`. `PyType_Ready`'s
//! `add_operators` publishes the slot wrappers named `__eq__` and so on
//! from the slots themselves, so these exports never enter a `PyMethodDef`
//! table.
//!
//! **Which class answers.** The seven names are resolved by type lookup:
//! the first class of the MRO whose own namespace binds the name, whatever
//! kind of binding it is. That is deliberately not
//! [`super::namespace_owner`], whose instance-slot rule does not apply
//! here: CPython looks a special method up on the type, never on the
//! instance. A binding that is not an instance method, or one whose
//! signature the boundary cannot carry, is a `C0003`
//! ([`slot_dunder_gap`]), never a silent omission -- an omitted slot is the
//! identity bypass this module exists to remove.
//!
//! **Which classes get slots.** Every carrier class
//! ([`super::collect_carrier_classes`]) that resolves at least one of the
//! seven names, except a monomorphized specialization (`0gen_`), whose name
//! never appears in a layout descriptor, and a `Protocol` class, which has
//! no instances. A published class gets the slots on its type object
//! ([`super::method_types_c`]). Any other carrier class gets a hidden
//! carrier type the generated registration creates and enters in the
//! shim's carrier-type cache up front ([`hidden_carrier_types_c`]), so the
//! first crossing of such an instance finds a type with the slots rather
//! than creating a slotless one on demand. Since #1448 the same hidden type
//! also carries the class's field descriptors (`super::getset`), and a
//! class with descriptors but no slot gets one too.
//!
//! **Both artifacts (#1470).** The embedded executable runs the same shim
//! and companion, so its `__main__` carriers get the same slots (but none of
//! the #1448 descriptors). It compiles with no export thunks, so a slot
//! dunder whose wrapper would call one is refused there too
//! ([`SlotArtifact::Embedded`]).

use pycc_diag::{Diagnostic, Severity};
use pycc_hir::{HirClassDef, HirItem, HirModule};

use super::export_name::ExtReceiver;
use super::getset::{ExtClassGetsets, getset_c};
use super::{
    CARRIABLE_TYPES, EXT_CAPABILITY_CODE, ExtCarrierClass, ExtExport, ExtPublishedClass,
    carrier_class_names, defaults, inherited, unsupported_boundary_ty, wrapper_for,
};

/// The seven special methods this module installs. The lexical mirror in
/// `pycc_codegen::is_ext_exportable_name` copies this list, and the export
/// parity test pins the two together.
pub(crate) const SLOT_DUNDERS: [&str; 7] = [
    "__lt__", "__le__", "__eq__", "__ne__", "__gt__", "__ge__", "__hash__",
];

/// The six comparisons and the `Py_LT`... operation each one answers.
const COMPARISONS: [(&str, &str); 6] = [
    ("__lt__", "Py_LT"),
    ("__le__", "Py_LE"),
    ("__eq__", "Py_EQ"),
    ("__ne__", "Py_NE"),
    ("__gt__", "Py_GT"),
    ("__ge__", "Py_GE"),
];

/// Whether the compiled item `name` is spelled `<Class>.<one of the seven>`
/// for a class that is not a monomorphized specialization: the lexical
/// half of "this item may be installed as a slot", which codegen's thunk
/// predicate mirrors.
pub(crate) fn is_slot_dunder_method(name: &str) -> bool {
    let mut segments = name.split('.');
    let class = segments.next().unwrap_or_default();
    let Some(method) = segments.next() else {
        return false;
    };
    !class.is_empty()
        && !class.starts_with("0gen_")
        && segments.next().is_none()
        && SLOT_DUNDERS.contains(&method)
}

/// Which artifact the slots are installed in (#1470): the refusal text
/// names it, and an embedded executable additionally refuses a slot whose
/// wrapper would call an export thunk, because it is compiled without them
/// (`pycc_codegen::CompileOptions::suppress_export_thunks`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlotArtifact {
    /// A `pycc build --ext` extension module.
    Ext,
    /// An embedded executable's `__main__` (Part 1 of #1028).
    Embedded,
}

/// What a carrier type's `Py_tp_hash` slot is.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SlotHash {
    /// The first MRO class binding `__hash__` or `__eq__` binds `__hash__`:
    /// the compiled method, called through this instance export's wrapper.
    Compiled(Box<ExtExport>),
    /// That class binds `__eq__` alone, so the data model makes the type
    /// unhashable: `PyObject_HashNotImplemented`, which `PyType_Ready`
    /// publishes as `__hash__ = None`.
    NotImplemented,
    /// Neither name is bound, but a comparison is: `object`'s identity hash,
    /// installed explicitly because `inherit_slots` never inherits
    /// `tp_hash` next to an overridden `tp_richcompare`.
    Identity,
}

/// One carrier class's comparison and hash slots.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ExtSlotDunders {
    /// The class whose carrier type carries the slots.
    pub(crate) class: String,
    /// Each comparison the class resolves, as `(dunder name, export)`, in
    /// [`COMPARISONS`] order. Empty when only `__hash__` resolves, in which
    /// case no `Py_tp_richcompare` is installed.
    pub(crate) comparisons: Vec<(&'static str, ExtExport)>,
    /// The `Py_tp_hash` slot, always installed.
    pub(crate) hash: SlotHash,
}

/// How a class's own namespace binds one of the seven names.
enum Binding<'m> {
    /// An ordinary instance method, by its mangled item name.
    Method(&'m str),
    /// Any other kind of binding, described for the `C0003`.
    Other(&'static str),
}

fn lookup_class<'m>(module: &'m HirModule, class: &str) -> Option<&'m HirClassDef> {
    module
        .class_defs
        .iter()
        .find(|(held, _)| held == class)
        .map(|(_, def)| def)
}

/// How `def`'s own body binds `name`, if it does: the kinds
/// `publication::class_member_names` reads, minus a `Protocol` member,
/// which only a `Protocol` class carries and [`collect_slot_dunders`] skips
/// those.
fn own_binding<'m>(def: &'m HirClassDef, name: &str) -> Option<Binding<'m>> {
    let lookup = |table: &'m [(String, String)]| table.iter().any(|(held, _)| held == name);
    if let Some((_, mangled)) = def.methods.iter().find(|(held, _)| held == name) {
        Some(Binding::Method(mangled))
    } else if def.properties.iter().any(|prop| prop.name == name) {
        Some(Binding::Other("a `@property`"))
    } else if lookup(&def.static_methods) {
        Some(Binding::Other("a `@staticmethod`"))
    } else if lookup(&def.class_methods) {
        Some(Binding::Other("a `@classmethod`"))
    } else if def.class_attrs.iter().any(|(held, _, _)| held == name) {
        Some(Binding::Other("a class attribute"))
    } else {
        None
    }
}

/// The `C0003` for a slot dunder the artifact cannot install, with the
/// instance-method remedy. Span-less for the reason `capability_gap` gives.
fn slot_dunder_gap(
    artifact: SlotArtifact,
    subject: &str,
    name: &str,
    class: &str,
    why: &str,
) -> Diagnostic {
    let fix = format!(
        "define it as an instance method `def {name}(self, ...)` whose signature the boundary \
         carries ({CARRIABLE_TYPES})"
    );
    slot_dunder_gap_with(artifact, subject, name, class, why, &fix)
}

/// [`slot_dunder_gap`] with its own `fix`, for a refusal an instance
/// method does not cure (a PEP 695 generic class).
///
/// The `--ext` text is #1427's, byte for byte. An embedded executable is
/// built whenever the program imports CPython, so it has no `--ext` to
/// drop and its text names itself instead (#1470).
fn slot_dunder_gap_with(
    artifact: SlotArtifact,
    subject: &str,
    name: &str,
    class: &str,
    why: &str,
    fix: &str,
) -> Diagnostic {
    let (who, remedy) = match artifact {
        SlotArtifact::Ext => ("--ext", ", or build without --ext (#1427)"),
        SlotArtifact::Embedded => ("an embedded executable", " (#1427, #1470)"),
    };
    Diagnostic {
        code: EXT_CAPABILITY_CODE,
        severity: Severity::Error,
        message: format!(
            "{who} cannot install `{subject}` as the host-visible `{name}` of `{class}` \
             instances: {why} -- CPython calls `{name}` implicitly (for `==`, `<`, `in`, \
             `hash()` or a dict key), so leaving it out of the artifact would silently answer \
             by identity instead -- {fix}{remedy}"
        ),
        span: None,
        label: None,
        help: None,
    }
}

/// The instance export the slot `name` of `class_def` calls, `None` when no
/// class of its MRO binds the name, or the `C0003` refusing it.
fn slot_export(
    artifact: SlotArtifact,
    module: &HirModule,
    class_def: &HirClassDef,
    name: &str,
) -> Result<Option<ExtExport>, Diagnostic> {
    let class = class_def.name.as_str();
    let Some((owner, binding)) = class_def.mro.iter().find_map(|ancestor| {
        let def = lookup_class(module, ancestor)?;
        own_binding(def, name).map(|binding| (def, binding))
    }) else {
        return Ok(None);
    };
    let subject = format!("{}.{name}", owner.name);
    let mangled = match binding {
        Binding::Other(kind) => {
            let why = format!("it is bound as {kind}, not as an instance method");
            return Err(slot_dunder_gap(artifact, &subject, name, class, &why));
        }
        Binding::Method(mangled) => mangled,
    };
    // A PEP 695 template's specializations all cross as a carrier named
    // after the template, so one slot cannot pick the right compiled copy
    // -- the reason `CopyRefusal::GenericLayout` exists.
    if class_def
        .mro
        .iter()
        .any(|ancestor| lookup_class(module, ancestor).is_some_and(|def| def.type_param.is_some()))
    {
        let why = format!(
            "`{class}` is or derives from a PEP 695 generic class, whose specializations all \
             cross as one carrier type"
        );
        let fix = "declare the class with an erased `Generic[T]` base instead of PEP 695 syntax";
        return Err(slot_dunder_gap_with(
            artifact, &subject, name, class, &why, fix,
        ));
    }
    let item = inherited::receiver_exact_member(module, class, name, mangled);
    // The wrapper's call form must match codegen's: an item this predicate
    // refuses would get no thunk, and a tuple-carrying one would then take
    // the `fnptr_` cast form measured to fault on aarch64.
    debug_assert!(
        is_slot_dunder_method(item),
        "codegen's thunk mirror must admit the slot item `{item}`"
    );
    let (params, return_ty, body) = module
        .items
        .iter()
        .rev()
        .find_map(|held| match held {
            HirItem::Function {
                name,
                params,
                return_ty,
                body,
                ..
            } if name == item => Some((params, return_ty, body)),
            _ => None,
        })
        .expect("a class's method table names a lowered function");
    // `pycc_hir::class` requires a regular method to lead with its
    // receiver, as `collect_exports_with_hooks` states.
    let carried = params.get(1..).unwrap_or_default();
    if let Some(offender) =
        unsupported_boundary_ty(carried, return_ty, &carrier_class_names(module))
    {
        let why =
            format!("its {offender} is not a type this pycc version's CPython boundary can carry");
        return Err(slot_dunder_gap(artifact, &subject, name, class, &why));
    }
    let export = ExtExport {
        name: item.to_string(),
        class: Some(class.to_string()),
        method: Some(name.to_string()),
        returns_buffer_slice: carried.iter().any(|(param, ty)| {
            *ty == pycc_hir::Ty::MemoryView && pycc_hir::body_returns_slice_of(body, param)
        }),
        receiver: ExtReceiver::SelfInstance,
        params: carried.iter().map(|(_, ty)| ty.clone()).collect(),
        param_writable: carried
            .iter()
            .map(|(param, ty)| {
                *ty == pycc_hir::Ty::MemoryView && pycc_hir::body_stores_into(body, param)
            })
            .collect(),
        defaults: defaults::carried_defaults(module, item, true),
        return_ty: return_ty.clone(),
        keyword_names: None,
    };
    // #1470: an embedded executable is compiled without export thunks, so
    // a wrapper that would call one (`wrappers::wrapper_for`'s own
    // predicate) would name an undefined symbol.
    let needs_thunk = pycc_codegen::ext_thunk_required(
        item,
        &export.params,
        &export.return_ty,
        export.returns_buffer_slice,
    );
    if artifact == SlotArtifact::Embedded && needs_thunk {
        let why = "its signature needs an export thunk (it carries a `tuple` or returns a \
                   slice of a `memoryview` argument), and an embedded executable is \
                   compiled without one";
        let fix = "take and return only scalars, instances of a class compiled in this program, \
                   or `None`";
        return Err(slot_dunder_gap_with(
            artifact, &subject, name, class, why, fix,
        ));
    }
    Ok(Some(export))
}

/// The comparison and hash slots of every carrier class in `carriers` that
/// resolves at least one of [`SLOT_DUNDERS`], in `carriers` order, or every
/// `C0003` refusing one, worded for `artifact`.
pub(crate) fn collect_slot_dunders(
    module: &HirModule,
    carriers: &[ExtCarrierClass],
    artifact: SlotArtifact,
) -> Result<Vec<ExtSlotDunders>, Vec<Diagnostic>> {
    let mut out = Vec::new();
    let mut gaps: Vec<Diagnostic> = Vec::new();
    for carrier in carriers {
        let def = lookup_class(module, &carrier.class).expect("a carrier class is a module class");
        if carrier.class.starts_with("0gen_") || def.is_protocol {
            continue;
        }
        let mut comparisons = Vec::new();
        let mut refused = Vec::new();
        for (name, _) in COMPARISONS {
            match slot_export(artifact, module, def, name) {
                Ok(Some(export)) => comparisons.push((name, export)),
                Ok(None) => {}
                Err(gap) => refused.push(gap),
            }
        }
        // The data model's hash rule: the first class binding either name
        // decides, and one binding `__eq__` without `__hash__` makes the
        // type unhashable.
        let decider = def.mro.iter().find_map(|ancestor| {
            let held = lookup_class(module, ancestor)?;
            ["__hash__", "__eq__"]
                .into_iter()
                .find(|name| own_binding(held, name).is_some())
        });
        let hash = match decider {
            Some("__hash__") => match slot_export(artifact, module, def, "__hash__") {
                Ok(export) => export.map(|export| SlotHash::Compiled(Box::new(export))),
                Err(gap) => {
                    refused.push(gap);
                    None
                }
            },
            Some(_) => Some(SlotHash::NotImplemented),
            None => Some(SlotHash::Identity),
        };
        gaps.extend(refused);
        // A class resolving none of the seven keeps `object`'s slots: no
        // entry, so its generated C is unchanged.
        if let Some(hash) = hash
            && (matches!(hash, SlotHash::Compiled(_)) || !comparisons.is_empty())
        {
            out.push(ExtSlotDunders {
                class: carrier.class.clone(),
                comparisons,
                hash,
            });
        }
    }
    if gaps.is_empty() { Ok(out) } else { Err(gaps) }
}

/// The C text one class's slots need ahead of its slot array: each
/// resolved method's `METH_FASTCALL` wrapper (once per artifact, through
/// `emitted`, shared with `getset_c` for the same redefinition reason), the
/// generated `tp_richcompare`, and the generated `tp_hash` when `__hash__`
/// is compiled.
///
/// The `tp_richcompare` is CPython's `object_richcompare` with the class's
/// own methods in it: a resolved comparison returns its method's result
/// unchanged, `NotImplemented` included; an unresolved `__ne__` inverts
/// the type's own `==` unless that is `NotImplemented` or raised; an
/// unresolved `__eq__` answers `True` for the same object and
/// `NotImplemented` otherwise; every other unresolved comparison is
/// `NotImplemented`, so the host tries the reflected operand.
pub(crate) fn slot_functions_c(slots: &ExtSlotDunders, emitted: &mut Vec<String>) -> String {
    let class = &slots.class;
    let mut out = String::new();
    let hash_export = match &slots.hash {
        SlotHash::Compiled(export) => Some(&**export),
        _ => None,
    };
    for export in slots
        .comparisons
        .iter()
        .map(|(_, export)| export)
        .chain(hash_export)
    {
        if !emitted.contains(&export.name) {
            emitted.push(export.name.clone());
            out.push_str(&wrapper_for(export));
        }
    }
    if !slots.comparisons.is_empty() {
        out.push_str(&format!(
            "static PyObject *pycc_ext_richcompare_{class}(PyObject *self, PyObject *other, \
             int op)\n{{\n"
        ));
        let defined = |name: &str| slots.comparisons.iter().any(|(held, _)| *held == name);
        for (name, export) in &slots.comparisons {
            let op = COMPARISONS
                .iter()
                .find(|(held, _)| held == name)
                .map(|(_, op)| *op)
                .expect("a resolved comparison is one of the six");
            // A parameter annotated with a compiled class: an operand of any
            // other type is `NotImplemented`, not the ingress `TypeError`.
            let guard = match export.params.first() {
                Some(pycc_hir::Ty::Instance(operand)) => format!(
                    "        if (!pycc_ext_slot_operand_is(other, \"{operand}\")) {{\n            \
                     Py_RETURN_NOTIMPLEMENTED;\n        }}\n"
                ),
                _ => String::new(),
            };
            out.push_str(&format!(
                "    if (op == {op}) {{\n{guard}        \
                 return pycc_ext_wrap_{symbol}(self, &other, 1);\n    }}\n",
                symbol = pycc_codegen::mangle_ext_name(&export.name)
            ));
        }
        if !defined("__ne__") {
            out.push_str(&format!(
                "    if (op == Py_NE) {{\n        \
                 PyObject *res = pycc_ext_richcompare_{class}(self, other, Py_EQ);\n        \
                 int truth;\n        \
                 if (res == NULL || res == Py_NotImplemented) {{\n            \
                 return res;\n        }}\n        \
                 truth = PyObject_IsTrue(res);\n        Py_DECREF(res);\n        \
                 if (truth < 0) {{\n            return NULL;\n        }}\n        \
                 return PyBool_FromLong(!truth);\n    }}\n"
            ));
        }
        if !defined("__eq__") {
            out.push_str(
                "    if (op == Py_EQ && self == other) {\n        Py_RETURN_TRUE;\n    }\n",
            );
        }
        out.push_str("    Py_RETURN_NOTIMPLEMENTED;\n}\n\n");
    }
    if let Some(export) = hash_export {
        out.push_str(&format!(
            "static Py_hash_t pycc_ext_hash_{class}(PyObject *self)\n{{\n    \
             return pycc_ext_finish_hash(pycc_ext_wrap_{symbol}(self, NULL, 0));\n}}\n\n",
            symbol = pycc_codegen::mangle_ext_name(&export.name)
        ));
    }
    out
}

/// The `PyType_Slot` rows that install `slots` on a carrier type.
pub(crate) fn slot_rows(slots: &ExtSlotDunders) -> String {
    let class = &slots.class;
    let mut out = String::new();
    if !slots.comparisons.is_empty() {
        out.push_str(&format!(
            "    {{Py_tp_richcompare, pycc_ext_richcompare_{class}}},\n"
        ));
    }
    let hash = match &slots.hash {
        SlotHash::Compiled(_) => format!("pycc_ext_hash_{class}"),
        SlotHash::NotImplemented => "pycc_ext_unhashable".to_string(),
        SlotHash::Identity => "pycc_ext_identity_hash".to_string(),
    };
    out.push_str(&format!("    {{Py_tp_hash, {hash}}},\n"));
    out
}

/// The registration line that swaps the `pycc_ext_unhashable` marker in
/// `array` for `PyObject_HashNotImplemented` before `PyType_FromSpec`
/// reads it, empty unless `slots` makes the type unhashable.
///
/// `PyType_Ready` publishes `__hash__ = None` only when `tp_hash` *is*
/// `PyObject_HashNotImplemented`, and on Windows a static initializer
/// naming a function imported from `python3.dll` holds the import thunk's
/// address instead, so the slot is filled at run time, where the address
/// comes from the import table.
pub(crate) fn unhashable_fixup(slots: &ExtSlotDunders, array: &str) -> String {
    match slots.hash {
        SlotHash::NotImplemented => format!("    pycc_ext_unhashable_slots({array});\n"),
        SlotHash::Compiled(_) | SlotHash::Identity => String::new(),
    }
}

/// The hidden carrier types (#1427) for the classes in `getsets` (#1448)
/// or `slots` that `publications` does not publish, as `(definitions,
/// registration)`: the classes of `getsets` first, then those only `slots`
/// names, so a program without descriptors keeps #1427's order.
///
/// Each type is what the shim's `pycc_ext_carrier_type` would create on
/// demand -- `<module>.<Class>`, the shared deallocator and `__copy__`,
/// `Py_TPFLAGS_DISALLOW_INSTANTIATION` -- plus the class's field
/// descriptors and slots. The registration enters it in the carrier-type
/// cache under the class name and does not add it to the module: the class
/// publishes nothing, so the host can only meet its instances, never name
/// the class. A class with neither still crosses on the on-demand type,
/// whose shared slot array is exactly what it needs.
pub(crate) fn hidden_carrier_types_c(
    publications: &[ExtPublishedClass],
    getsets: &[ExtClassGetsets],
    slots: &[ExtSlotDunders],
    emitted: &mut Vec<String>,
) -> (String, String) {
    let mut defs = String::new();
    let mut registration = String::new();
    let mut classes: Vec<&str> = getsets.iter().map(|table| table.class.as_str()).collect();
    for entry in slots {
        if !classes.contains(&entry.class.as_str()) {
            classes.push(&entry.class);
        }
    }
    for class in classes
        .into_iter()
        .filter(|class| !publications.iter().any(|held| held.class == *class))
    {
        let table = getsets.iter().find(|table| table.class == class);
        let entry = slots.iter().find(|entry| entry.class == class);
        let mut rows = String::new();
        if let Some(table) = table {
            defs.push_str(&getset_c(table, emitted));
            rows.push_str(&format!(
                "    {{Py_tp_getset, pycc_ext_type_getset_{class}}},\n"
            ));
        }
        if let Some(entry) = entry {
            defs.push_str(&slot_functions_c(entry, emitted));
            rows.push_str(&slot_rows(entry));
            registration.push_str(&unhashable_fixup(
                entry,
                &format!("pycc_ext_carrier_slots_{class}"),
            ));
        }
        defs.push_str(&format!(
            "static PyMethodDef pycc_ext_carrier_methods_{class}[] = {{\n    \
             {{\"__copy__\", (PyCFunction)(void (*)(void))pycc_ext_instance_copy, \
             METH_NOARGS, NULL}},\n    {{NULL, NULL, 0, NULL}},\n}};\n\n\
             static PyType_Slot pycc_ext_carrier_slots_{class}[] = {{\n    \
             {{Py_tp_dealloc, pycc_ext_instance_dealloc}},\n    \
             {{Py_tp_methods, pycc_ext_carrier_methods_{class}}},\n{rows}    {{0, NULL}},\n}};\n\n\
             static PyType_Spec pycc_ext_carrier_spec_{class} = {{\n    \
             PYCC_EXT_MODULE_NAME_STR \".{class}\",\n    sizeof(PyccExtInstance),\n    0,\n    \
             Py_TPFLAGS_DEFAULT | Py_TPFLAGS_DISALLOW_INSTANTIATION | \
             Py_TPFLAGS_IMMUTABLETYPE,\n    pycc_ext_carrier_slots_{class},\n}};\n\n"
        ));
        registration.push_str(&format!(
            "    type = PyType_FromSpec(&pycc_ext_carrier_spec_{class});\n    \
             if (type == NULL) {{\n        return -1;\n    }}\n    \
             if (pycc_ext_carrier_register(\"{class}\", type) < 0) {{\n        \
             Py_DECREF(type);\n        return -1;\n    }}\n    Py_DECREF(type);\n"
        ));
    }
    (defs, registration)
}

#[cfg(test)]
#[path = "richcompare_tests.rs"]
mod tests;
