//! The `Py_tp_getset` descriptors a carrier type carries (#1442, every
//! carrier type since #1448): one per carriable instance-attribute slot and one per carriable
//! `@property`, so `instance.field` reads the compiled value on the CPython
//! side, and (Part 1 of #1443) a slot's `instance.field = v` and
//! `del instance.field` store into it, while (#1458) a property's
//! `instance.prop = v` runs its compiled setter.
//!
//! **Why the host needs them.** D-258 makes an unannotated or `Any`/`object`
//! operand the opaque CPython object, and compiled code reads an attribute
//! of such an operand through `PyObject_GetAttr` (`pycc_ext_obj_getattr`).
//! Before #1442 a published type carried only `Py_tp_methods`, so
//! `other.state_stack` inside a compiled `__eq__(self, other)` raised
//! `AttributeError` for an `other` that really was a compiled instance --
//! the shape lark's `ParserState.__eq__` has. A getset descriptor is the
//! attribute the host's own lookup finds, so the same read now answers the
//! compiled field, and a host program reading `obj.field` gets it too.
//!
//! **Slot stores (Part 1 of #1443).** A slot descriptor has a setter
//! ([`setter`]): a store converts the value by the slot type's parameter row
//! and releases the replaced word as a compiled store does; a `del`
//! un-assigns the slot. A class whose MRO defines `__setattr__` or
//! `__delattr__` never reaches this table: the frontend refuses either name
//! in a class body (`C0001`, #1459), because a raw slot store -- or a
//! direct call of a property setter -- would silently bypass the method.
//! The read side is the same: a class binding `__getattribute__` or
//! `__getattr__` never reaches the getters here either (`C0001`, #1465),
//! because a getter answering the read directly would bypass the method.
//!
//! **Property stores (#1458).** A property with a compiled setter routes a
//! store through the setter's `METH_FASTCALL` wrapper, the way the getter
//! is routed, so the value is converted by the setter parameter's row of
//! the boundary table ([`PropertySetter::Compiled`]). A getter-only
//! property answers CPython's own `property '<p>' of '<C>' object has no
//! setter`, and every property answers `del` with `... has no deleter`
//! (pycc has no `@<p>.deleter`). A setter whose value or return type the
//! boundary does not carry leaves the descriptor read-only, the same
//! silently-partial policy as an uncarried getter.
//!
//! **Instance-typed fields (#1453).** A slot or property declared as a
//! regular class compiled in the same module -- lark's
//! `ParserState.parse_conf: ParseConf` -- is packed through the #1449
//! egress, `pycc_ext_pack_instance`, so the stored instance's live carrier
//! is returned again: `s.parse_conf is s.parse_conf`, and a host-constructed
//! instance reads back as the host's own object. The admitted classes are exactly the ones a
//! published signature may name ([`super::carrier_class_names`]): an enum
//! or exception-class field still gets no descriptor.
//!
//! **Silently partial.** A slot or property whose declared type the boundary
//! cannot pack from one machine word (a `list[int]` slot, an enum- or
//! exception-class-typed one, a `tuple`- or `memoryview`-returning getter)
//! simply gets no descriptor; it is never a `C0003`. The descriptors widen what a host can
//! observe of an object it already holds, and refusing a whole build because
//! one field cannot be observed would turn an additive capability into a
//! regression for every program that has such a field today. An optional
//! instance slot (`C | None`) never reaches this table: its annotation is
//! still refused at compile time with `T0049`.
//!
//! **Every carrier type (#1448).** The table is keyed by carrier class
//! ([`collect_carrier_getsets`]), not by constructor, because since #1435
//! every type that boxes an instance for the host -- a constructible
//! published type, a non-constructible published one, and the carrier type
//! of a class that publishes nothing -- wraps a live `inst`. A published
//! class's type object installs the table ([`super::method_types_c`]); any
//! other class with a table gets a hidden carrier type registered in the
//! shim's carrier-type cache at module exec
//! ([`super::richcompare::hidden_carrier_types_c`], #1427's mechanism), so
//! no instance of it crosses on the shim's descriptor-less on-demand type.
//! The rules above apply unchanged, setters included. A class no instance
//! is ever an instance *of* -- an abstract or `Protocol` class, a
//! monomorphized `0gen_` specialization -- gets no table, and neither does
//! one with a PEP 695 generic in its MRO that #1442 did not already cover:
//! its specializations all cross as one carrier named after the template
//! with per-specialization layouts (the reason `richcompare` refuses slots
//! there), so it is silently partial in the sense above. A constructible
//! published generic template keeps the table #1442 gave it, so #1448
//! narrows nothing #1442 published.

use std::collections::BTreeSet;

use pycc_hir::{HirClassDef, HirItem, HirModule, Ty, flat_attr_layout};

mod setter;

use super::export_name::ExtReceiver;
use super::{
    ExtCarrierClass, ExtExport, carrier_class_names, class_constructible, class_publishable,
    inherited, instance_shape_admissible, mro_class_defs, namespace_owner, wrapper_for,
};

/// One attribute descriptor a carrier type exposes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ExtGetset {
    /// An instance-attribute slot, read straight out of the inner
    /// `PyInstanceObj` with the same checked accessor a compiled `self.x`
    /// read uses (#1388), so an unassigned slot raises the same
    /// `AttributeError`.
    Slot {
        /// The attribute name, which is the host-visible descriptor name.
        name: String,
        /// The slot index in `pycc_hir::flat_attr_layout` order -- the
        /// layout every instance of the class is allocated with
        /// (`ExtCtor::slot_names` for a host-constructed one).
        index: usize,
        /// The slot's declared type, which picks the packer and the
        /// setter's unpack helper. Every slot descriptor has a setter
        /// (Part 1 of #1443).
        ty: Ty,
    },
    /// A `@property`, read by calling its compiled getter through the same
    /// `METH_FASTCALL` wrapper an instance method gets.
    Property {
        /// The property name, which is the host-visible descriptor name.
        name: String,
        /// The zero-argument instance export the getter wrapper is rendered
        /// from. It is never in the export set ([`super::collect_exports_with_hooks`]
        /// excludes a getter as representation), so the wrapper is emitted
        /// here and nowhere else.
        getter: ExtExport,
        /// What a host store or `del` runs (#1458).
        setter: PropertySetter,
    },
}

/// What a property descriptor's setter slot does (#1458).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PropertySetter {
    /// The property has a compiled setter whose value and return types the
    /// boundary carries: a store calls it through its `METH_FASTCALL`
    /// wrapper, rendered from this one-argument instance export. Like the
    /// getter's, the export is never in the export set
    /// ([`super::collect_exports_with_hooks`] refuses a `.setter` spelling), so its
    /// wrapper is emitted only by [`getset_c`]. Boxed, so a `Refuse` or
    /// `ReadOnly` descriptor does not carry an export's footprint.
    Compiled(Box<ExtExport>),
    /// A getter-only property: a store raises CPython's `property '<p>' of
    /// '<C>' object has no setter`.
    Refuse,
    /// No setter at all (`NULL` in the table), so CPython's generic
    /// `attribute '<p>' of '<mod>.<C>' objects is not writable`: a compiled
    /// setter whose value or return type is not carried.
    ReadOnly,
}

impl ExtGetset {
    /// The host-visible attribute name.
    pub(crate) fn name(&self) -> &str {
        match self {
            ExtGetset::Slot { name, .. } | ExtGetset::Property { name, .. } => name,
        }
    }
}

/// One carrier class's descriptor table (#1448): what
/// [`super::method_types_c`] installs as `Py_tp_getset` on its published
/// type object or on its hidden carrier type.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ExtClassGetsets {
    /// The class whose carrier type carries the table.
    pub(crate) class: String,
    /// Its descriptors, in [`collect_getsets`] order; never empty.
    pub(crate) getsets: Vec<ExtGetset>,
}

/// The descriptor table of every class in `carriers` that has at least one
/// descriptor and whose name can be some instance's run-time class, in
/// `carriers` order (the module doc names the classes skipped).
pub(crate) fn collect_carrier_getsets(
    module: &HirModule,
    carriers: &[ExtCarrierClass],
) -> Vec<ExtClassGetsets> {
    let lookup = |class: &str| {
        module
            .class_defs
            .iter()
            .find(|(held, _)| held == class)
            .map(|(_, def)| def)
    };
    carriers
        .iter()
        .filter_map(|carrier| {
            let class = carrier.class.as_str();
            let def = lookup(class).expect("a carrier class is a module class");
            // A constructible published generic template kept the table
            // #1442 built for it from its constructor, so only the classes
            // #1448 newly covers are held to the generic exclusion.
            let generic = def
                .mro
                .iter()
                .any(|ancestor| lookup(ancestor).is_some_and(|held| held.type_param.is_some()))
                && !(class_publishable(def, class) && class_constructible(module, class));
            if class.starts_with("0gen_") || !instance_shape_admissible(def, class) || generic {
                return None;
            }
            let getsets = collect_getsets(module, def, &mro_class_defs(module, def));
            (!getsets.is_empty()).then(|| ExtClassGetsets {
                class: class.to_string(),
                getsets,
            })
        })
        .collect()
}

/// Whether a slot or getter of type `ty` gets a descriptor: the five types
/// whose value the boundary packs from one machine word, and (#1453) an
/// instance of one of `carrier_classes`, the module's regular classes,
/// whose word is the instance pointer #1449's egress packs.
fn carried(ty: &Ty, carrier_classes: &BTreeSet<String>) -> bool {
    match ty {
        Ty::Int | Ty::Float | Ty::Bool | Ty::Str | Ty::Object => true,
        Ty::Instance(class) => carrier_classes.contains(class.as_str()),
        _ => false,
    }
}

/// The getset descriptors the carrier class `class` exposes, given
/// its definition and its MRO's definitions (most derived first): its
/// carriable slots in slot order, then its carriable properties in MRO
/// order.
///
/// A property is published only where it *wins the namespace walk*
/// ([`namespace_owner`]): a slot of the same name, or a nearer class's
/// member of that name, answers the attribute instead, exactly as it does
/// in Python and in [`super::collect_class_publications`].
pub(crate) fn collect_getsets(
    module: &HirModule,
    class_def: &HirClassDef,
    mro_defs: &[&HirClassDef],
) -> Vec<ExtGetset> {
    let class = class_def.name.as_str();
    let carrier_classes = carrier_class_names(module);
    let mut out: Vec<ExtGetset> = flat_attr_layout(mro_defs)
        .into_iter()
        .enumerate()
        .filter(|(_, (_, ty))| carried(ty, &carrier_classes))
        .map(|(index, (name, ty))| ExtGetset::Slot { name, index, ty })
        .collect();
    for def in mro_defs {
        for prop in &def.properties {
            if namespace_owner(module, &class_def.mro, &prop.name) != Some(def.name.as_str())
                || out.iter().any(|getset| getset.name() == prop.name)
            {
                continue;
            }
            let getter = inherited::receiver_exact_member(module, class, &prop.name, &prop.getter);
            if let Some(export) = getter_export(module, class, &prop.name, getter, &carrier_classes)
            {
                let setter = match &prop.setter {
                    None => PropertySetter::Refuse,
                    Some(mangled) => {
                        let member = format!("{}.setter", prop.name);
                        let setter =
                            inherited::receiver_exact_member(module, class, &member, mangled);
                        setter_export(module, class, &prop.name, setter, &carrier_classes)
                            .map_or(PropertySetter::ReadOnly, |export| {
                                PropertySetter::Compiled(Box::new(export))
                            })
                    }
                };
                out.push(ExtGetset::Property {
                    name: prop.name.clone(),
                    getter: export,
                    setter,
                });
            }
        }
    }
    out
}

/// The zero-argument instance export for the compiled getter `getter`, or
/// `None` when its return type gets no descriptor. The *last* definition of
/// the name is the one codegen binds `fnptr_<name>` to, so it is the one
/// read.
fn getter_export(
    module: &HirModule,
    class: &str,
    prop: &str,
    getter: &str,
    carrier_classes: &BTreeSet<String>,
) -> Option<ExtExport> {
    let return_ty = module.items.iter().rev().find_map(|item| match item {
        HirItem::Function {
            name, return_ty, ..
        } if name == getter => Some(return_ty),
        _ => None,
    })?;
    carried(return_ty, carrier_classes).then(|| ExtExport {
        name: getter.to_string(),
        class: Some(class.to_string()),
        method: Some(prop.to_string()),
        returns_buffer_slice: false,
        receiver: ExtReceiver::SelfInstance,
        params: Vec::new(),
        param_writable: Vec::new(),
        defaults: Vec::new(),
        return_ty: return_ty.clone(),
        keyword_names: None,
    })
}

/// The one-argument instance export for the compiled setter `setter`, or
/// `None` when the boundary cannot carry its value parameter or its return
/// type (#1458). The value is converted by that parameter's row, as an
/// argument of the same type is; a non-`None` return is packed by the
/// wrapper and discarded by the descriptor. As for the getter, the last
/// definition of the name is the one `fnptr_<name>` is bound to.
fn setter_export(
    module: &HirModule,
    class: &str,
    prop: &str,
    setter: &str,
    carrier_classes: &BTreeSet<String>,
) -> Option<ExtExport> {
    let (params, return_ty) = module.items.iter().rev().find_map(|item| match item {
        HirItem::Function {
            name,
            params,
            return_ty,
            ..
        } if name == setter => Some((params, return_ty)),
        _ => None,
    })?;
    // The frontend refuses a setter that is not exactly `(self, value)`
    // (`C0001`), so the value is the second parameter.
    let (_, value_ty) = params.get(1)?;
    (carried(value_ty, carrier_classes)
        && (*return_ty == Ty::None || carried(return_ty, carrier_classes)))
    .then(|| ExtExport {
        name: setter.to_string(),
        class: Some(class.to_string()),
        method: Some(prop.to_string()),
        returns_buffer_slice: false,
        receiver: ExtReceiver::SelfInstance,
        params: vec![value_ty.clone()],
        param_writable: vec![false],
        defaults: Vec::new(),
        return_ty: return_ty.clone(),
        keyword_names: None,
    })
}

/// The C getter function name for `class`'s attribute `name`. Each part is
/// length-prefixed, so class `A_b` attribute `c` and class `A` attribute
/// `b_c` cannot collide.
fn getter_symbol(class: &str, name: &str) -> String {
    format!("pycc_ext_get_{}_{class}_{}_{name}", class.len(), name.len())
}

/// The packing statements for a slot word of type `ty`, already read into
/// the local `word`. The slot keeps its own reference, so the two packers
/// that discharge one -- `int` for a heap bigint (D-180 rule 6), `str` for
/// its `PyStrObj` -- are handed a reference taken here first; the object
/// packer takes its own new reference.
fn pack_slot_word(class: &str, name: &str, ty: &Ty) -> String {
    match ty {
        Ty::Int => format!(
            "    pycc_rt_bigint_retain(word);\n    \
             return pycc_ext_pack_int(\"{class}.{name}\", word);\n"
        ),
        Ty::Float => "    memcpy(&value, &word, sizeof value);\n    \
                      return pycc_ext_pack_float(value);\n"
            .to_string(),
        Ty::Bool => "    return pycc_ext_pack_bool((char)word);\n".to_string(),
        Ty::Str => "    pycc_rt_str_incref((void *)(intptr_t)word);\n    \
                    return pycc_ext_pack_str((void *)(intptr_t)word);\n"
            .to_string(),
        // #1453: the word is the stored instance's pointer. The instance is
        // never freed (D-107, D-154) and the packer takes no reference on
        // it, only a new one on the carrier it returns -- the live one when
        // the instance has one, so a repeated read is the same object.
        Ty::Instance(_) => {
            "    return pycc_ext_pack_instance((void *)(intptr_t)word);\n".to_string()
        }
        // `collect_getsets` admits exactly the `carried` types, so the
        // remaining one is the opaque object.
        _ => "    return pycc_ext_pack_object((void *)(intptr_t)word);\n".to_string(),
    }
}

/// The C text for one carrier class's descriptors: each property's
/// getter wrapper, each descriptor's getter function, each slot's setter
/// function, each property's setter function unless it is
/// [`PropertySetter::ReadOnly`] (#1458), and the
/// `pycc_ext_type_getset_<Class>` table [`super::method_types_c`] installs
/// as `Py_tp_getset` on a published type or a hidden carrier type (#1448).
///
/// `emitted` holds the compiled getters and setters whose wrapper an
/// earlier class already rendered: a `Derived` that inherits `Base`'s `@property` with no
/// receiver-exact copy resolves to the same `Base.<name>` item, and a
/// second `pycc_ext_wrap_` definition of one symbol is a C redefinition
/// error, so each wrapper is rendered once per artifact.
pub(crate) fn getset_c(table: &ExtClassGetsets, emitted: &mut Vec<String>) -> String {
    let class = &table.class;
    let mut out = String::new();
    for getset in &table.getsets {
        let name = getset.name();
        let symbol = getter_symbol(class, name);
        match getset {
            ExtGetset::Slot { index, ty, .. } => {
                // A carrier `PyType_GenericNew` zeroed and `tp_init` never
                // filled (`mod.Class.__new__(mod.Class)`) has no instance, so
                // every slot of it is unassigned: the same `AttributeError`
                // the checked accessor raises for one unassigned slot.
                let float_local = if *ty == Ty::Float {
                    "    double value;\n"
                } else {
                    ""
                };
                out.push_str(&format!(
                    "static PyObject *{symbol}(PyObject *self, void *closure)\n{{\n    \
                     void *inst = ((PyccExtInstance *)self)->inst;\n    long long word;\n\
                     {float_local}    (void)closure;\n    if (inst == NULL) {{\n        \
                     PyErr_SetString(PyExc_AttributeError, \
                     \"'{class}' object has no attribute '{name}'\");\n        \
                     return NULL;\n    }}\n    \
                     word = pycc_rt_instance_get_slot_checked(inst, {index});\n    \
                     if (pycc_rt_ext_pending_type() >= 0) {{\n        \
                     pycc_ext_raise_pending();\n        return NULL;\n    }}\n{}}}\n\n",
                    pack_slot_word(class, name, ty)
                ));
                out.push_str(&setter::slot_setter_c(class, name, *index, ty));
            }
            ExtGetset::Property { getter, setter, .. } => {
                if !emitted.contains(&getter.name) {
                    emitted.push(getter.name.clone());
                    out.push_str(&wrapper_for(getter));
                }
                if let PropertySetter::Compiled(export) = setter
                    && !emitted.contains(&export.name)
                {
                    emitted.push(export.name.clone());
                    out.push_str(&wrapper_for(export));
                }
                // The NULL-`inst` guard answers `AttributeError` before the
                // wrapper's own `TypeError` guard is reached, so `hasattr`
                // and `getattr(o, name, default)` on a carrier `tp_init`
                // never filled behave as they do for any missing attribute.
                out.push_str(&format!(
                    "static PyObject *{symbol}(PyObject *self, void *closure)\n{{\n    \
                     (void)closure;\n    if (((PyccExtInstance *)self)->inst == NULL) {{\n        \
                     PyErr_SetString(PyExc_AttributeError, \
                     \"'{class}' object has no attribute '{name}'\");\n        \
                     return NULL;\n    }}\n    \
                     return pycc_ext_wrap_{wrapped}(self, NULL, 0);\n}}\n\n",
                    wrapped = pycc_codegen::mangle_ext_name(&getter.name)
                ));
                match setter {
                    PropertySetter::Compiled(export) => {
                        out.push_str(&setter::property_setter_c(class, name, Some(export)));
                    }
                    PropertySetter::Refuse => {
                        out.push_str(&setter::property_setter_c(class, name, None));
                    }
                    PropertySetter::ReadOnly => {}
                }
            }
        }
    }
    out.push_str(&format!(
        "static PyGetSetDef pycc_ext_type_getset_{class}[] = {{\n"
    ));
    for getset in &table.getsets {
        let name = getset.name();
        let setter = match getset {
            ExtGetset::Slot { .. }
            | ExtGetset::Property {
                setter: PropertySetter::Compiled(_) | PropertySetter::Refuse,
                ..
            } => setter::setter_symbol(class, name),
            _ => "NULL".to_string(),
        };
        out.push_str(&format!(
            "    {{\"{name}\", {}, {setter}, NULL, NULL}},\n",
            getter_symbol(class, name)
        ));
    }
    out.push_str("    {NULL, NULL, NULL, NULL, NULL},\n};\n\n");
    out
}
