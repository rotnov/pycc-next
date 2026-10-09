//! The export set (D-244 rule 6): which compiled functions and methods the
//! `--ext` artifact publishes, and the `C0003` gaps that keep the others
//! out. Moved out of `src/ext_build.rs` unchanged (#1461).

use super::*;

/// One export: a public module-level function, or a public
/// `@staticmethod`/`@classmethod` of a public class.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ExtExport {
    /// The compiled program's own name for the function -- a plain
    /// identifier for a module-level `def`, and `pycc_hir::class`'s mangled
    /// `<Class>.<method>.static` / `<Class>.<method>.classmethod` for a
    /// method. Every C identifier built from it goes through
    /// `pycc_codegen::mangle_ext_name` first; it is *not* the host-visible
    /// name for a method.
    pub(crate) name: String,
    /// The owning class, or `None` for a module-level function. A method's
    /// host-visible name is `mod.<class>.<method>` -- a `PyMethodDef` entry
    /// in that class's own table -- so two classes may carry the same
    /// [`ExtExport::method`] without colliding.
    pub(crate) class: Option<String>,
    /// The bare method name, which is the `PyMethodDef` `ml_name` for a
    /// method. `None` exactly when [`ExtExport::class`] is `None`, in which
    /// case [`ExtExport::name`] is itself the `ml_name`.
    pub(crate) method: Option<String>,
    /// Whether this export's body returns a **sub-range** of one of its
    /// `memoryview` parameters (Part 2 of #1175, #1179).
    ///
    /// Not a function of [`ExtExport::return_ty`]: a declared
    /// `-> memoryview` is the same signature for a bare `return b`
    /// (Part 1 of #1175), an artifact-owned `return a` (Part 2b of #1142)
    /// and `return b[i:j]`, and only the last of the three carries the
    /// three trailing `long long *` out-pointers
    /// `pycc_codegen::ext_thunk_out_tys` describes. Keying the wrapper on
    /// the declared type instead would move every existing
    /// buffer-returning export onto the thunk path and change generated C
    /// this task does not touch.
    pub(crate) returns_buffer_slice: bool,
    /// Which leading receiver pointer the compiled function takes, and what
    /// the wrapper must supply for it.
    ///
    /// [`ExtReceiver::NullCls`] for a `@classmethod`: `pycc_hir::class`
    /// injects `cls: Ty::Instance(Class)` as its first parameter, and
    /// `MirExpr::NullInstance` records that every native `Class.method(...)`
    /// call site passes a null pointer for it, because a method compiled for
    /// one class resolves `cls.attr` at compile time and never dereferences
    /// it. The wrapper does the same, and so discards the *type object*
    /// CPython hands `METH_CLASS` in `self` -- writing that pointer into a
    /// slot typed `Ty::Instance` would be type confusion even though nothing
    /// dereferences it today.
    ///
    /// [`ExtReceiver::SelfInstance`] for an instance method (#1145), whose
    /// leading `self` *is* dereferenced: the wrapper unwraps the host
    /// carrier object's inner `PyInstanceObj` and passes that. Both spell
    /// the same `void *` in the declaration; only the call argument differs.
    ///
    /// [`ExtReceiver::None`] for a module-level `def` and a
    /// `@staticmethod`, which declare no receiver at all.
    pub(crate) receiver: ExtReceiver,
    /// The declared parameter types, in order, **excluding** a
    /// [`ExtExport::receiver`]. Their count is the arity the
    /// `METH_FASTCALL` wrapper checks, and each one alone picks that
    /// argument's C local, its `pycc_ext_unpack_*` helper and its slot in
    /// the indirect call's cast.
    ///
    /// The receiver is dropped here and reinstated *textually* in
    /// [`wrapper_for`], because every consumer of this field is
    /// arity-shaped or carrier-shaped and neither can represent it. Since
    /// Part 1 of #1447 (#1449) `boundary_carrier` does map `Ty::Instance`,
    /// but to an *argument* carrier that unpacks a host object at a
    /// `METH_FASTCALL` position: `self` is not such a position at all, and a
    /// `@classmethod`'s `cls` must be a null pointer rather than an unpacked
    /// carrier, so leaving the receiver in would shift every arity by one
    /// and unpack the wrong object. It is reinstated rather than simply dropped because
    /// `pycc_codegen`'s thunk builds its own parameter list from the MIR
    /// function's parameters, which *do* include the receiver -- declaring
    /// different arities on the two sides of one symbol is the silent ABI
    /// mismatch the thunk exists to prevent.
    pub(crate) params: Vec<Ty>,
    /// Per-parameter writability, parallel to [`ExtExport::params`] and
    /// carrying `true` exactly where that parameter is a `memoryview` the
    /// body stores into (Part 1 of #1142, `pycc_hir::body_stores_into`).
    ///
    /// Carried here rather than derived in [`boundary_carrier`], which is a
    /// pure function of a `Ty` and cannot see a body, and rather than
    /// folded into [`ExtExport::params`], which every other consumer reads
    /// as a plain type list. `false` for every non-buffer parameter, where
    /// it is inert.
    pub(crate) param_writable: Vec<bool>,
    /// Each carried parameter's default value, parallel to
    /// [`ExtExport::params`], or empty when the export declares none (the
    /// method part of #1140; see `defaults`). Non-empty only for a method:
    /// a module-level export keeps the exact arity check (#1194).
    pub(crate) defaults: Vec<Option<pycc_hir::HirExpr>>,
    /// The declared return type, which picks the cast's return type and the
    /// egress: a `pycc_ext_pack_*` call, or a new reference to `None` for `-> None`.
    pub(crate) return_ty: Ty,
    /// The source names of the carried parameters, parallel to
    /// [`ExtExport::params`], when a host call may pass them as keywords
    /// (#1461), or `None` when every keyword is refused.
    ///
    /// `Some` selects `METH_FASTCALL | METH_KEYWORDS` and the four-argument
    /// wrapper signature together, so the flag and the signature can never
    /// disagree. Set by [`super::keywords::bind_keyword_names`]; see that
    /// module for when an export qualifies.
    pub(crate) keyword_names: Option<Vec<String>>,
}

/// The names of the module's classes whose instances may cross the `--ext`
/// boundary at a parameter or return position (Part 1 of #1447, #1449):
/// exactly [`collect_carrier_classes`]'s rows, so a name this admits is
/// always one the generated `pycc_ext_carrier_class_isinstance` table that
/// ingress consults has a row for.
pub(crate) fn carrier_class_names(module: &HirModule) -> BTreeSet<String> {
    collect_carrier_classes(module)
        .into_iter()
        .map(|carrier| carrier.class)
        .collect()
}

/// Derives the export set from the typed program, per D-244 rule 1: every
/// public module-level function is exported. "The compiled module" there is
/// the linked program D-222 produces -- the entry file plus its whole import
/// closure -- so a public function defined in an imported project module is
/// exported too, and renaming it private is how a project keeps it off the
/// artifact's CPython surface.
///
/// D-244 rule 1's export set also reaches a public `@staticmethod`,
/// `@classmethod` and -- since #1145 -- instance method of a public,
/// non-exception class. Such a method reaches `HirItem::Function` under
/// `pycc_hir::class`'s mangled `<Class>.<method>.static` /
/// `<Class>.<method>.classmethod` / bare `<Class>.<method>` name, and its
/// host-visible name is `mod.<Class>.<method>` -- a `PyMethodDef` entry in
/// that class's own `PyType_FromSpec` type object, never a flat
/// `mod.<Class>.<method>` module attribute. [`classify_export_name`] owns
/// the lexical half of that verdict.
///
/// An instance method is exported only from a class some host-obtainable
/// instance can be a receiver for -- [`instance_method_reachable`] is the
/// canonical statement of that predicate -- because a method no instance
/// can ever reach would be an unreachable entry. That ordering is also what
/// bounds the new `C0003` set: a class no constructible class inherits
/// contributes no new gaps at all, so an `@abstractmethod`'s stub body and
/// a `@property` getter are **excluded as representation, before
/// [`unsupported_boundary_ty`] is consulted**, exactly as the suffixed
/// spellings that preceded them were.
///
/// "Public" is D-038's predicate, `pycc_hir::is_public_name`, applied to
/// the class name and the method name alike. The exclusions that are not
/// policy but representation:
///
/// * a monomorphized generic specialization carries the `0gen_` prefix and
///   has no `fnptr_` global to call through (codegen dispatches those
///   directly);
/// * `<Class>.<property>.setter` -- a `@property` is attribute syntax on
///   the host side, not a method;
/// * a `@property` **getter**, which shares the bare `<Class>.<method>`
///   spelling with an ordinary instance method and is told apart here by
///   `HirClassDef::properties`, whose `getter` field holds exactly that
///   mangled name;
/// * every instance method of a class [`instance_method_reachable`]
///   refuses, which is what removes an `@abstractmethod`'s stub body: such
///   a method survives lowering only on an `is_abstract` class (a class
///   carrying its own `@abstractmethod` without an `ABC` base is rejected
///   with `C0001` by `crates/pycc_hir/src/class.rs`'s unoverridden-abstract
///   check), and that predicate refuses an `is_abstract` class outright --
///   unlike an unconstructible `__init__` shape, which a constructible
///   subclass does rescue. The exclusion story is therefore *"excluded
///   because no host instance can ever receive it"*, never *"excluded
///   because the method is abstract"*.
///
/// Routing any of them through the gap collector instead would turn a
/// public ABC into a build failure on a signature the boundary carries
/// perfectly well.
///
/// A class whose HIR carries an `exception_type_tag` publishes no type
/// object and exports no method. [`register_class_c`] already publishes
/// such a class under its **bare class name** as a module attribute, so a
/// second `PyModule_AddObjectRef` under that name would replace a working
/// exception class with a non-instantiable type -- the host then gets
/// `TypeError: catching classes that do not inherit from BaseException`.
/// The exclusion is scoped on the HIR tag and deliberately *not* on
/// [`collect_user_exception_classes`]' selector, which additionally drops
/// group-derived classes: a `class MyGroup(ExceptionGroup)` is never
/// registered, so scoping there would publish `mod.MyGroup` as a
/// non-instantiable non-`BaseException` type standing in for a user
/// exception class -- the same wrong-published-name defect from the other
/// side. This filter is applied in the driver, layered *on top of*
/// [`classify_export_name`]'s lexical verdict rather than inside it, which
/// keeps that verdict mirror-comparable with
/// `pycc_codegen::is_ext_exportable_name` (work item 8's parity test) and
/// makes the mirror a harmless superset. The entry module's PEP 562 hooks
/// (#1467) are layered on the same way, by [`collect_exports_with_hooks`]
/// from its `hooks` set rather than from `classify_export_name`, and the
/// comparison and hash slot dunders (#1427) by `super::collect_slot_dunders`,
/// so the parity test compares the mirror with that verdict
/// `|| is_module_hook(name) || is_slot_dunder_method(name)`.
///
/// The boundary carries `int`, `float`, `bool`, `str` and a `tuple` of
/// those scalars in either direction, and `None` as a return type only
/// (#1036, #1048, #1049, #1050). `docs/RUNTIME.md`'s admissibility matrix
/// is the canonical statement of that set, including the narrowings each
/// admitted type carries. Every other public signature is a
/// [`EXT_CAPABILITY_CODE`] capability gap, and *all* of them are collected
/// before returning -- one `--ext` build should not have to be re-run once
/// per unsupported function.
///
/// A `tuple` is admitted by its elements and not by its own name: the
/// boundary carries it by spreading it into one scalar slot per element
/// (see [`boundary_carrier`]), so a nested or container-carrying `tuple`
/// stays a gap. That is a restatement of D-116's model -- a tuple type has
/// a fixed arity of `int`/`bool`/`float` elements -- and not a second
/// admissibility rule.
///
/// The export set is derived here, in the driver, rather than carried as a
/// new field on `HirItem::Function`/`MirItem::Function`: those two patterns
/// are constructed at 693 and 174 sites across this workspace, and a new
/// field would put hundreds of mechanically-updated non-test lines into the
/// 100%-coverage denominator for no behavioural gain.
///
/// Since #1467 this is the hook-free form the unit tests call; the build
/// calls [`collect_exports_with_hooks`] with the entry module's hooks.
#[cfg(test)]
pub(crate) fn collect_exports(module: &HirModule) -> Result<Vec<ExtExport>, Vec<Diagnostic>> {
    collect_exports_with_hooks(module, &BTreeSet::new())
}

/// The export set the hook-free `collect_exports` above documents, plus
/// the PEP 562 module
/// hooks the entry module defines (#1467, [`super::module_hooks`]): a
/// compiled function named in `hooks` is a module-level export although
/// [`classify_export_name`] refuses every dunder.
///
/// `hooks` comes from the entry module's own source
/// ([`super::EntryHooks::defined`]), never from the linked program: a
/// helper module's `def __getattr__` is a `HirItem::Function` of the same
/// name, and CPython never applies it to the entry module. A hook defined
/// in both is already a `C0001` of D-222's import closure, so the one
/// compiled item a name in `hooks` matches is the entry module's own.
///
/// Every rule below applies to a hook unchanged -- last-wins dedup, the
/// buffer-slice union, the boundary check -- except the gap's wording: a
/// hook's remedy is not "rename it private" ([`module_hook_gap`]).
pub(crate) fn collect_exports_with_hooks(
    module: &HirModule,
    hooks: &BTreeSet<String>,
) -> Result<Vec<ExtExport>, Vec<Diagnostic>> {
    let mut exports: Vec<ExtExport> = Vec::new();
    // Every compiled name any definition of which returns a buffer
    // sub-range; see the union pass after the loop for why the fact is
    // per-name rather than last-wins.
    let mut slice_widened_names: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();
    let mut gaps = Vec::new();
    // Part 1 of #1447 (#1449): the classes whose instances the boundary may
    // carry at a parameter or return position -- the same set the generated
    // carrier `isinstance` table is built from, so ingress can never admit
    // a class that table cannot answer for.
    let carrier_classes = carrier_class_names(module);
    for item in &module.items {
        let HirItem::Function {
            name,
            params,
            return_ty,
            body,
            ..
        } = item
        else {
            continue;
        };
        let Some(spelling) = classify_export_name(name)
            .or_else(|| hooks.contains(name).then_some(ExportName::ModuleLevel))
        else {
            continue;
        };
        // The driver-only exception-class filter, layered on top of the
        // lexical verdict rather than inside it -- see this function's doc.
        if let ExportName::Method { class, .. } = &spelling
            && module
                .class_defs
                .iter()
                .any(|(held, def)| held == class && def.exception_type_tag.is_some())
        {
            continue;
        }
        // #1145's two driver filters, in the same position and form as the
        // exception-class one above and, like it, *before*
        // `unsupported_boundary_ty` -- so neither exclusion can become a
        // `C0003`. Both apply only to the bare spelling: a `@staticmethod`
        // and a `@classmethod` are published on a non-constructible class
        // exactly as Part 1 published them.
        if let ExportName::Method {
            class,
            method,
            receiver: ExtReceiver::SelfInstance,
        } = &spelling
        {
            // #1337 (D-254): a receiver-exact copy of an inherited body is
            // absent from the class tables both filters below read, so it
            // is judged by its own rule (`inherited::copy_export_verdict`).
            match inherited::copy_export_verdict(module, name) {
                inherited::CopyVerdict::Drop => continue,
                inherited::CopyVerdict::Publish => {}
                inherited::CopyVerdict::NotACopy => {
                    // A `@property` getter shares the bare spelling with an
                    // ordinary instance method. `HirClassDef::properties`
                    // holds the getter's own mangled name, so this is an
                    // identity test rather than a name pattern.
                    if module.class_defs.iter().any(|(held, def)| {
                        held == class && def.properties.iter().any(|prop| prop.getter == *name)
                    }) {
                        continue;
                    }
                    if !instance_method_reachable(module, class, method) {
                        continue;
                    }
                }
            }
        }
        // A `@classmethod`'s leading `cls` never crosses the boundary (the
        // wrapper passes a C `NULL` for it) and an instance method's
        // leading `self` crosses it as an opaque pointer the wrapper
        // unwraps, not as a carried argument. Both are split off here,
        // before `unsupported_boundary_ty` runs: since Part 1 of #1447
        // (#1449) a `Ty::Instance` *is* carried, but as a host argument the
        // wrapper unpacks from a `METH_FASTCALL` position, which neither
        // receiver is (see `ExtExport::params`).
        let receiver = match &spelling {
            ExportName::ModuleLevel => ExtReceiver::None,
            ExportName::Method { receiver, .. } => *receiver,
        };
        // `receiver` is decided lexically, from the mangled suffix alone,
        // because `classify_export_name` cannot see HIR. The guarantee that
        // such a function really leads with a receiver lives in another
        // crate -- `crates/pycc_hir/src/class.rs` refuses a `@classmethod`
        // that does not take `cls` first, and requires a regular method to
        // declare a receiver as its first parameter, lowering it under the
        // canonical name `self` whatever the source spelled it (#1181;
        // `crates/pycc_hir/src/class/receiver.rs`). The invariant this site
        // rests on is *positional*, so the #1181 relaxation of the source
        // spelling does not weaken it -- this site states that cross-crate
        // invariant instead of slicing on the strength of it.
        let carried_params = if receiver == ExtReceiver::None {
            &params[..]
        } else {
            match params.split_first() {
                Some((_, tail)) => tail,
                None => panic!(
                    "pycc: internal error: `{name}` is spelled as a method with a \
                     receiver but has no parameters -- pycc_hir::class refuses a \
                     `@classmethod` without a leading `cls` and a regular method \
                     without a leading receiver parameter, so this HIR should never \
                     have been built"
                ),
            }
        };
        if let Some(offender) = unsupported_boundary_ty(carried_params, return_ty, &carrier_classes)
        {
            gaps.push(if hooks.contains(name) {
                module_hook_gap(name, &offender)
            } else {
                capability_gap(name, &offender)
            });
            continue;
        }
        let (class, method) = match &spelling {
            ExportName::ModuleLevel => (None, None),
            ExportName::Method { class, method, .. } => (Some(class.clone()), Some(method.clone())),
        };
        let export = ExtExport {
            name: name.clone(),
            class,
            method,
            receiver,
            params: carried_params.iter().map(|(_, ty)| ty.clone()).collect(),
            // Part 1 of #1142. The walk runs over the *post-split* carried
            // tail, so the flags line up with `params` even for a method
            // whose receiver was dropped above, and it is keyed on the
            // parameter's own source name because that is what
            // `HirStmt::DictSet` carries.
            param_writable: carried_params
                .iter()
                .map(|(param_name, ty)| {
                    *ty == Ty::MemoryView && pycc_hir::body_stores_into(body, param_name)
                })
                .collect(),
            // Part 2 of #1175 (#1179). The driver's half of the per-export
            // "this body carries a buffer sub-range egress" fact; codegen
            // recomputes the same fact from MIR
            // (`pycc_codegen::body_returns_buffer_slice`) because
            // `ExtExport` never crosses the crate boundary, and a parity
            // test pins the two answers together. A divergence is not a
            // wrong diagnostic: it is a generated C call form that does not
            // match the compiled function's own signature.
            //
            // Keyed on the carried parameter's own source name, exactly as
            // `param_writable` above is, and asked only of `memoryview`
            // parameters -- which is the provenance half
            // `pycc_hir::body_returns_slice_of` deliberately leaves to its
            // caller.
            returns_buffer_slice: carried_params.iter().any(|(param_name, ty)| {
                *ty == Ty::MemoryView && pycc_hir::body_returns_slice_of(body, param_name)
            }),
            defaults: defaults::carried_defaults(module, name, receiver != ExtReceiver::None),
            return_ty: return_ty.clone(),
            keyword_names: None,
        };
        // A module may rebind a public name -- two `def`s, a `def` over an
        // imported name, or two `@staticmethod def f` in one class body,
        // which `pycc check` accepts. Codegen emits exactly one
        // `fnptr_<name>` global and binds it to the *last* definition, so
        // the wrapper table must carry exactly one entry per C function
        // definition, with that definition's signature: a second entry
        // generates a second `pycc_ext_wrap_<name>` and the C compiler
        // rejects the redefinition outright. Replacing in place rather than
        // appending keeps the table in definition order, which is what the
        // generated `.inc` fixtures assert. Cross-*module* collisions cannot
        // reach here -- `pycc_hir`'s import closure rejects a name defined by
        // two inputs with `C0001` first.
        //
        // The key is `(class, method)` for a method and the plain name for a
        // module-level function -- **not** the host-visible name alone. On a
        // type object `Grid.scale` and `Other.scale` both publish `ml_name`
        // `"scale"` and belong to different `PyMethodDef` tables, so a
        // host-visible key would collapse two distinct exports into one;
        // while keying on the compiled `name` alone would let one class
        // publish `ml_name` `"f"` twice, once from `f.static` and once from
        // `f.classmethod`, which Python's own class body cannot mean.
        // Replacing keeps the last definition, which is what `Grid.f` binds
        // to in Python and what the shared `fnptr_` slot holds.
        if export.returns_buffer_slice {
            slice_widened_names.insert(export.name.clone());
        }
        let key = export_dedup_key(&export);
        match exports
            .iter_mut()
            .find(|held| export_dedup_key(held) == key)
        {
            Some(held) => *held = export,
            None => exports.push(export),
        }
    }
    // Part 2 of #1175 (#1179), review round 2. The one field that is *not*
    // resolved last-wins: whether the compiled function carries the three
    // buffer-sub-range out-pointers is a property of the shared
    // `fnptr_<name>` slot's single signature, not of the definition
    // currently bound to it, so it unions over every definition of the
    // compiled name exactly as `pycc_codegen::buffer_slice_out_names` does.
    //
    // Keyed on the compiled `name` rather than on `export_dedup_key`
    // deliberately: the name is what codegen groups by, and a class that
    // publishes one `ml_name` from two differently-mangled definitions
    // shares the dedup key without sharing a signature.
    //
    // Taking the last definition's answer instead is what let
    // `def f: return b[1:]` / `def f: return b` declare one arity in the
    // generated C and compile another -- an ill-typed call across the
    // object boundary that no compiler on either side can see. Widening a
    // name whose active definition only does a bare `return b` is harmless:
    // that return stores `has_slice = 0` and the wrapper hands back the
    // whole view.
    for export in &mut exports {
        export.returns_buffer_slice = slice_widened_names.contains(&export.name);
    }
    if gaps.is_empty() {
        Ok(exports)
    } else {
        Err(gaps)
    }
}
