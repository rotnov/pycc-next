//! Constructors at the `--ext` host boundary: which published classes are
//! constructible from the host and the `Py_tp_init` descriptor each gets.
//! Moved out of `src/ext_build.rs` unchanged (#1461).

use super::*;

/// What [`resolved_init`] hands back: the constructor's mangled name, its
/// declared parameter list -- receiver still at index 0 -- and its return
/// type. A named alias only because `clippy::type_complexity` refuses the
/// tuple spelled inline; every caller destructures it immediately.
pub(crate) type ResolvedInit<'a> = (&'a str, &'a [(String, Ty)], &'a Ty);

/// The `__init__` a host-side `mod.<Class>(...)` call would run, resolved
/// through `class`'s MRO exactly as instantiation resolves it.
///
/// Two passes, and the second is mandatory (#966, D-232): a D-225 implicit
/// zero-argument constructor lands in its *own* class's method table, where
/// it would otherwise out-rank a real `__init__` declared by a later base.
/// The first pass therefore skips every `implicit_object_init` class and the
/// second accepts one, mirroring `pycc_types::class`' `super().__init__()`
/// ranking and `pycc_mir`'s instantiation lowering. A class whose only
/// constructor *is* the implicit one -- `class C: pass`, the common case --
/// is found by the second pass alone.
///
/// Returns the constructor's *destructured* mangled name, parameter list
/// and return type rather than the `HirItem` itself. Every caller needs all
/// three, and handing back the enum would make each one re-match a variant
/// whose other arms this function has already excluded -- an `else` arm no
/// test could ever execute, which
/// `scripts/check_diff_coverage.py`'s 100%-changed-lines invariant does not
/// admit (D-242 rule 1).
pub(crate) fn resolved_init<'a>(module: &'a HirModule, class: &str) -> Option<ResolvedInit<'a>> {
    let (_, class_def) = module.class_defs.iter().find(|(held, _)| held == class)?;
    let resolve = |skip_implicit: bool| {
        class_def.mro.iter().find_map(|mro_class| {
            let (_, mro_def) = module
                .class_defs
                .iter()
                .find(|(held, _)| held == mro_class)?;
            if skip_implicit && mro_def.implicit_object_init {
                return None;
            }
            mro_def
                .methods
                .iter()
                .find(|(method, _)| method == "__init__")
                .map(|(_, mangled)| mangled.as_str())
        })
    };
    // #1337 (D-254): an inherited constructor runs the copy compiled for
    // `class` when it needed one, as `pycc_mir`'s instantiation does.
    let mangled =
        inherited::receiver_exact_init(module, class, resolve(true).or_else(|| resolve(false))?);
    module.items.iter().find_map(|item| match item {
        HirItem::Function {
            name,
            params,
            return_ty,
            ..
        } if name == mangled => Some((name.as_str(), params.as_slice(), return_ty)),
        _ => None,
    })
}

/// Whether a published class can be constructed from the host -- the
/// canonical statement of D-244's #1145 amendment clause (b), and the
/// predicate every instance-method exclusion cites.
///
/// *Publication* is decided by [`collect_class_publications`], and this
/// predicate is one of its two disjuncts: a [`class_publishable`] class gets
/// a type object when its MRO-resolved method set is non-empty *or* this
/// answers `true` (#1450), so a constructible class is always published --
/// an `__init__`-only class with no exported method included. What this
/// function adds on top is which published class gets a `Py_tp_init`: a
/// class published only for its methods and not constructible gets a type
/// object the host cannot instantiate.
///
/// The four conditions, each a HIR fact rather than a name pattern:
///
/// 1. the class is not abstract, not a `Protocol` and not an `Enum` --
///    `is_abstract` is what removes an `@abstractmethod`'s stub body, whose
///    lowered `HirItem::Function` returns nothing while its `return_ty` says
///    otherwise, and it removes it *totally*: a class carrying its own
///    `@abstractmethod` without an `ABC` base never reaches codegen at all
///    (`crates/pycc_hir/src/class.rs`'s unoverridden-abstract check rejects
///    it with `C0001`), so every surviving abstract stub belongs to an
///    `is_abstract` class;
/// 2. it carries no `exception_type_tag` and is not a seeded synthetic
///    builtin exception class -- [`register_class_c`] already publishes a
///    user exception class under its bare name, and the flat builtins carry
///    no tag of their own;
/// 3. its MRO-resolved `__init__` ([`resolved_init`]) exists and returns
///    `None` -- an unannotated `__init__` infers `Ty::Infer` and is refused
///    here rather than emitting a constructor whose C return type is
///    unspellable;
/// 4. the receiver-free tail of that `__init__`'s parameters is carriable by
///    [`unsupported_boundary_ty`] and contains no `tuple`.
///
/// Condition 4 **reuses `unsupported_boundary_ty`** rather than re-deriving
/// the predicate, so a constructor's admissibility is literally the same
/// statement as every other export's instead of a second one that drifts
/// (`AGENTS.md`'s canonical-statement rule). It also answers correctly, for
/// free, on the case a hand-written predicate would most likely miss: `def
/// __init__(self, other: Grid)` carries a `Ty::Instance`, which since Part 1
/// of #1447 (#1449) is carried exactly when it names a regular class of this
/// module, so such a class is constructible from a host that already holds
/// a `Grid`, while `def __init__(self, c: Color)` with `Color` an enum keeps
/// the class non-constructible. The extra `tuple` refusal
/// is not a second admissibility rule either: `is_ext_exportable_name`
/// answers `false` for `<Class>.__init__` (its second segment starts with
/// `_`), so `ext_thunk_required` emits no `pycc_ext_thunk_` for a
/// constructor and a `tuple` parameter would have no callable C entry point
/// at all.
pub(crate) fn class_constructible(module: &HirModule, class: &str) -> bool {
    ctor_descriptor(module, class).is_some()
}

/// [`class_constructible`]'s conditions 1 and 2 -- the ones about the
/// class's own *shape* rather than about its `__init__` -- factored out so
/// [`instance_method_reachable`] can hold them while relaxing conditions 3
/// and 4, instead of restating them (`AGENTS.md`'s canonical-statement
/// rule).
pub(crate) fn instance_shape_admissible(class_def: &HirClassDef, class: &str) -> bool {
    !class_def.is_abstract
        && !class_def.is_protocol
        && !class_def.is_enum
        && class_def.exception_type_tag.is_none()
        && !is_builtin_exception_class(class)
}

/// Whether the artifact publishes a type object for `class` at all, as far
/// as the class's *name and kind* decide it -- the two conditions
/// [`collect_class_publications`] applies, factored out so
/// [`instance_method_reachable`] can require them of its witness instead
/// of restating them (`AGENTS.md`'s canonical-statement rule).
///
/// Publication's third condition -- that the class's MRO-resolved method
/// set is non-empty *or* the class is [`class_constructible`] (#1450) -- is
/// deliberately *not* here, and a witness does not need it: a witness is
/// required to be [`class_constructible`], which satisfies that condition
/// on its own -- except for a monomorphized `0gen_` witness, which the
/// constructor half skips and which is published through the method half
/// instead, as before #1450: the export it witnesses is in its own resolved
/// set. Stating the method half here would also be circular, since
/// the resolved set is built out of the export set this predicate helps
/// decide.
/// Where each conjunct bites: the name half is what a *witness* needs --
/// a privately named subclass is constructible and unpublished -- and the
/// tag half is what the *publication* site needs, since `class_constructible`
/// already refuses an exception-tagged class through
/// [`instance_shape_admissible`]. Deleting either one turns a test red, but
/// not the same test: the name half is pinned by
/// `a_privately_named_constructible_subclass_witnesses_nothing_for_its_base`
/// and the tag half by `a_private_or_exception_inheriting_class_is_not_published`.
///
/// [`instance_shape_admissible`]'s third exclusion, `is_builtin_exception_class`,
/// is deliberately absent: the synthetic builtin classes `pycc_hir` seeds carry no
/// public method of their own and [`class_constructible`] refuses them through
/// [`instance_shape_admissible`], so neither disjunct of
/// [`collect_class_publications`]' third condition holds for any of them before
/// this predicate's answer could matter.
pub(crate) fn class_publishable(class_def: &HirClassDef, class: &str) -> bool {
    is_public_name(class) && class_def.exception_type_tag.is_none()
}

/// Whether `class`'s own shape admits instance exports **and** the host can
/// actually obtain a receiver for them -- the predicate [`collect_exports_with_hooks`]
/// applies to the bare method spelling, and the canonical statement of
/// D-244 rule 1's #1145 receiver-reachability clause.
///
/// The answer is: [`instance_shape_admissible`] holds of `class` itself,
/// *and* some class the artifact **publishes** ([`class_publishable`])
/// whose MRO contains `class` is [`class_constructible`].
///
/// Wider than [`class_constructible`] alone, and deliberately so. A method
/// is lowered once against its own class's slot layout and is then
/// inherited by every subclass, so `Derived(21).value()` reaches
/// `Base.value`'s compiled body even when `Base` itself can never be built
/// from the host -- an unannotated or `tuple`-carrying `__init__` makes
/// `Base` unconstructible without making its methods unreachable. `mro[0]`
/// is the class itself, so a publishable constructible class answers for
/// its own methods.
///
/// **The witness must also resolve `method` to `class`.** The predicate is
/// per method, not per class, because [`collect_class_publications`] answers
/// a name from the first MRO entry that binds it (see [`namespace_owner`]):
/// a witness whose own body shadows `method` -- with a `@property`, an
/// `@abstractmethod`, or a member of any other kind -- publishes its own
/// binding and never the one compiled here, so this method is as
/// unreachable through that witness as it is through an unpublished one.
/// A witness whose MRO assigns `method` to `self` in any `__init__`
/// disqualifies it for the same reason: the instance answers the name and
/// the compiled body is unreachable through that witness too.
/// Exporting it anyway would emit a `PyMethodDef` row nothing can call and,
/// with an uncarriable signature, fail the whole `--ext` build with a
/// `C0003` for a method no host could ever reach.
///
/// **Both halves of the witness are load-bearing.** Constructibility alone
/// is not enough, because the host names a constructor only through a
/// published type object: a privately named subclass is never published by
/// [`collect_class_publications`], so `mod._Priv(...)` does not exist and
/// no instance reaching `class`'s methods can ever be built. Accepting
/// such a witness would emit a `PyMethodDef` row with no obtainable
/// receiver, or -- with an uncarriable signature -- fail the whole `--ext`
/// build with a `C0003` for a method nothing could ever call.
///
/// [`class_constructible`]'s conditions 3 and 4 are the ones a publishable
/// constructible subclass rescues. Conditions 1 and 2 --
/// [`instance_shape_admissible`] -- are not: an `@abstractmethod`'s stub
/// body returns nothing while its `return_ty` says otherwise, so exporting
/// it from an `is_abstract` base would emit a wrapper over a body that
/// never returns, and an exception class publishes no type object at all.
///
/// A receiver-exact copy of an inherited body (#1337, D-254) never reaches
/// this predicate: it is judged by `inherited::copy_export_verdict` in
/// this predicate's place, and a witness subclass that has a copy of
/// `method` binds that copy instead of this body
/// (`inherited::receiver_exact_export`), so a publishable witness here is
/// one that genuinely runs the body compiled for `class`.
pub(crate) fn instance_method_reachable(module: &HirModule, class: &str, method: &str) -> bool {
    let Some((_, class_def)) = module.class_defs.iter().find(|(held, _)| held == class) else {
        return false;
    };
    if !instance_shape_admissible(class_def, class) {
        return false;
    }
    module.class_defs.iter().any(|(held, def)| {
        def.mro.iter().any(|entry| entry == class)
            && class_publishable(def, held)
            && class_constructible(module, held)
            && namespace_owner(module, &def.mro, method) == Some(class)
    })
}

/// [`class_constructible`]'s answer with the generated `Py_tp_init`'s inputs
/// attached: one function, so the predicate and the descriptor can never
/// disagree about which classes are constructible.
fn ctor_descriptor(module: &HirModule, class: &str) -> Option<ExtCtor> {
    let (_, class_def) = module.class_defs.iter().find(|(held, _)| held == class)?;
    if !instance_shape_admissible(class_def, class) {
        return None;
    }
    let (name, params, return_ty) = resolved_init(module, class)?;
    if *return_ty != Ty::None {
        return None;
    }
    // The constructor descriptor does not come through `collect_exports`,
    // so the receiver is still at index 0 here and has to be split off
    // exactly as the classmethod path splits `cls`: the receiver is the
    // carrier `tp_init` fills, not an argument it unpacks.
    let (_, carried) = params.split_first()?;
    if unsupported_boundary_ty(carried, &Ty::None, &carrier_class_names(module)).is_some()
        || carried.iter().any(|(_, ty)| matches!(ty, Ty::Tuple(_)))
    {
        return None;
    }
    let body = module.items.iter().find_map(|item| match item {
        HirItem::Function {
            name: held, body, ..
        } if held == name => Some(body.as_slice()),
        _ => None,
    });
    // `resolved_init` found `name` in `module.items` to produce the
    // signature above, so the same lookup cannot miss now; `unwrap_or` is
    // the total spelling of that rather than a second panic site.
    let body = body.unwrap_or(&[]);
    Some(ExtCtor {
        class: class.to_string(),
        name: name.to_string(),
        params: carried.iter().map(|(_, ty)| ty.clone()).collect(),
        param_writable: carried
            .iter()
            .map(|(param_name, ty)| {
                *ty == Ty::MemoryView && pycc_hir::body_stores_into(body, param_name)
            })
            .collect(),
        defaults: defaults::carried_defaults(module, name, true),
        slot_names: instance_slot_names(module, class_def),
        keyword_names: None,
    })
}

/// The name of every `pycc_rt_instance_new` slot an instance of
/// `class_def` occupies, in slot order.
///
/// `pycc_hir::flat_attr_layout` is the single definition of what counts as
/// a slot -- merged `@dataclass` fields, an exception class's empty
/// attribute list -- and `pycc_mir` delegates to it too, so the layout the
/// generated `tp_init` allocates is by construction the one
/// `MirExpr::Instantiate` passes for the same class (#1388: the names as
/// well as the count, since they word the `AttributeError` of a slot read
/// before its assignment).
///
/// That parity is what forbids skipping an MRO entry this program does not
/// define. The counterpart on the MIR side is `pycc_mir::class`'s
/// `mro_attrs`/`mro_class_def`, which does not skip such an entry either --
/// it panics with an internal error, pinned by
/// `mro_attrs_with_a_ghost_class_in_the_mro_panics_with_an_internal_error`
/// (`crates/pycc_mir/src/tests/class_mro.rs`). Skipping here while
/// `mro_attrs` panics there would mean under-allocating an instance whose
/// inherited compiled methods then index past its own storage, so this path
/// fails the same way instead. It is unreachable on any program `pycc check`
/// accepts: `validate_bases` rejects a base this module does not define, and
/// the one MRO entry that can be absent from `HirModule::class_defs` -- a
/// synthetic builtin exception base dropped as `pycc_hir::link` concatenates
/// -- only ever appears in the MRO of an exception class, which
/// [`instance_shape_admissible`] already refused above.
fn instance_slot_names(module: &HirModule, class_def: &HirClassDef) -> Vec<String> {
    flat_attr_layout(&mro_class_defs(module, class_def))
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

/// `class_def`'s MRO resolved to the definitions that carry its slots, most
/// derived first -- [`instance_slot_names`]' input, shared with the getset
/// collector (#1442, keyed by carrier class since #1448) so both read one
/// layout. Panics on an MRO entry this program does not define; see
/// [`instance_slot_names`] for why.
pub(crate) fn mro_class_defs<'m>(
    module: &'m HirModule,
    class_def: &HirClassDef,
) -> Vec<&'m HirClassDef> {
    class_def
        .mro
        .iter()
        .map(|mro_class| {
            match module
                .class_defs
                .iter()
                .find(|(held, _)| held == mro_class)
                .map(|(_, def)| def)
            {
                Some(def) => def,
                None => panic!(
                    "pycc: internal error: class `{}` lists `{mro_class}` in its \
                     method resolution order but this program defines no such \
                     class -- pycc_hir::class refuses a base the module does not \
                     define, so this HIR should never have been built",
                    class_def.name
                ),
            }
        })
        .collect()
}

/// One constructible class's generated `Py_tp_init` descriptor.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ExtCtor {
    /// The class's own (unqualified) Python name, which is also the
    /// host-visible type name and the suffix of every C identifier the
    /// generated shim builds for it.
    pub(crate) class: String,
    /// The compiled program's name for the constructor,
    /// `pycc_hir::class`'s mangled `<Owner>.__init__`. `<Owner>` is the
    /// MRO-resolved owner and not necessarily [`ExtCtor::class`].
    pub(crate) name: String,
    /// The constructor's declared parameter types **excluding** the leading
    /// `self`, which the generated shim supplies itself from
    /// `pycc_rt_instance_new`. Their count is the arity `tp_init` checks
    /// `PyTuple_Size(args)` against.
    pub(crate) params: Vec<Ty>,
    /// Per-parameter writability, parallel to [`ExtCtor::params`] and
    /// meaning exactly what [`ExtExport::param_writable`] means.
    ///
    /// A `memoryview` `__init__` parameter is a live ingress path
    /// `collect_exports` never sees -- it refuses `__init__` outright -- so
    /// the flag has to be computed here too, or `Py_tp_init` would acquire
    /// read-only for a constructor body the checker admits a store in.
    pub(crate) param_writable: Vec<bool>,
    /// Each carried parameter's default value, parallel to
    /// [`ExtCtor::params`] and meaning exactly what [`ExtExport::defaults`]
    /// means. Read through [`ExtCtor::name`], so an inherited constructor
    /// carries the defaults of the `__init__` it runs.
    pub(crate) defaults: Vec<Option<pycc_hir::HirExpr>>,
    /// The slot names, in slot order, whose count `pycc_rt_instance_new` is
    /// called with and which, after [`ExtCtor::class`], make up the layout
    /// descriptor it is passed (#1388).
    pub(crate) slot_names: Vec<String>,
    /// The constructor's carried parameter names when `Py_tp_init` binds
    /// keywords (#1461), meaning exactly what [`ExtExport::keyword_names`]
    /// means; `None` keeps the hand-written keyword refusal.
    pub(crate) keyword_names: Option<Vec<String>>,
}

/// The constructible classes among the published ones, in publication order.
///
/// Driven by [`collect_class_publications`] rather than by the export set
/// directly, so a class that declares no exportable member of its own but
/// inherits one -- published since #1145's inheritance fix -- gets its
/// `Py_tp_init` too. Ordered off that list and never off a hash map,
/// because the generated `.inc` must be byte-identical across runs.
pub(crate) fn collect_constructors(
    module: &HirModule,
    publications: &[ExtPublishedClass],
) -> Vec<ExtCtor> {
    publications
        .iter()
        .filter_map(|published| ctor_descriptor(module, &published.class))
        .collect()
}
