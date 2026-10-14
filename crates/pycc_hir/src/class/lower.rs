//! `lower_class`: lowering a class definition to `HirClassDef`.

use super::*;

/// Lowers a module-level `class Foo: ...` statement (D-154). Returns the
/// class's own declared shape (for `HirModule::class_defs`) alongside every
/// method it defines, already lowered into ordinary mangled
/// `HirItem::Function`s ready to append to `HirModule::items` -- see this
/// module's own doc comment for why a class has no `HirItem` of its own.
///
/// `defined_classes` is the list of already-lowered class definitions (name
/// → `HirClassDef`), in source order. Inheritance validation (unknown base,
/// circular inheritance, generic-class-as-base) uses this to check each base
/// class against the classes defined earlier in the module, matching Python's
/// own source-order class-body execution (a base class must be defined before
/// the derived class that inherits from it).
///
/// Every check below is a `C0001` capability diagnostic, not a design
/// question this PR resolves: a class decorator is out of scope entirely
/// (dataclasses/`dataclass_transform` are unrelated later PRs' own scope).
/// Generic classes (`class C[T]:`) with a single type parameter ARE now
/// supported by #387 (see the `type_params` handling below). Inheritance
/// (`class C(Base):`) is now supported by #432 (Part 1).
///
/// `base_class_asts` (#585) pairs every already-lowered class name in this
/// module with its original `StmtClassDef`, so this call can re-inspect an
/// *earlier* class's own `__init_subclass__` body when the class being
/// lowered here inherits, rather than overrides, that hook -- `defined_classes`
/// alone only carries the already-lowered `HirClassDef`, which has no method
/// bodies left to re-validate. A base absent from this slice (a synthetic
/// builtin-exception class, or any base this crate cannot introspect) is
/// simply treated as not defining a validatable inherited hook, matching
/// this file's existing "unintrospectable -> unrestricted" posture elsewhere.
///
/// `staticmethod_rebound` (#1345) is whether the module body binds
/// `staticmethod` anywhere, computed once by `module::lower_module`.
///
/// `type_vars` (#1394) are the module-level `T = TypeVar("T")` declarations
/// lowered before this class, the names a `Generic[...]` base may list
/// besides foreign-imported ones (see `generic_base`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_class(
    def: &pycc_ast::StmtClassDef,
    aliases: &[(String, Ty)],
    defined_classes: &[(String, HirClassDef)],
    module_items: &[HirItem],
    base_class_asts: &[(String, &pycc_ast::StmtClassDef)],
    imports: &[ImportBinding],
    type_vars: &[String],
    signatures: &SignatureTable,
    staticmethod_rebound: bool,
) -> Result<(HirClassDef, Vec<HirItem>), Diagnostic> {
    // #380 (PR-20): build the projected class slice `annotation_to_ty` uses
    // to resolve cross-class annotations (including protocol-typed ones);
    // #693 added `class_getitem_return`, resolved from `module_items` (every
    // `HirItem::Function` lowered by an earlier class or top-level `def` in
    // this module, in source order).
    let mut class_name_defs = class_annotation_infos(defined_classes, module_items);
    // PEP 3129/557/681/544 (#378/#380, PR-18/PR-20): classify the class's
    // decorator list. `@dataclass`/`@dataclass_transform(...)` and
    // `@runtime_checkable` are recognized; any other class decorator is
    // rejected with C0001.
    let decorator_info = classify_class_decorator(&def.decorator_list, def.range.into())?;
    let is_dataclass = decorator_info.is_dataclass;
    // PEP 695 (#387): a generic class (`class C[T]:`) with exactly one type
    // parameter is now supported, reusing PR-13's `Ty::Param` call-site-
    // substitution mechanism (D-133/D-134). More than one type parameter is
    // rejected, matching PR-13's own generic-function scoping.
    let type_param: Option<String> = match def.type_params.as_deref() {
        None => None,
        Some(type_params) => match type_params.type_params.as_slice() {
            [single] => Some(crate::type_param_name(single, def.range)?.to_string()),
            _ => {
                return Err(unsupported(
                    "generic classes with more than one type parameter are not supported yet",
                    def.range,
                ));
            }
        },
    };
    // #432: parse base class names from the class header's positional
    // arguments (`class C(A, B):` → `["A", "B"]`); a `Generic[T, ...]` base
    // is checked and erased there (Part 1 of #886, #1394). Keyword
    // arguments (e.g. `metaclass=Meta`) and every other non-name base
    // (`class C(SomeMod.Base):`, `class C(Base[int]):`) are still rejected.
    let mut bases = generic_base::lower_base_names(def, imports, type_vars, type_param.is_some())?;
    let class_name = def.name.to_string();
    // #379 (PR-19): PEP 435 enum detection. A class whose single base is
    // the bare name `Enum` (`class Color(Enum):`) is an enum class. `Enum`
    // is a builtin base name (see `is_enum_base_name`), not a user-defined
    // class in `defined_classes` -- so it must be intercepted here, before
    // the unknown-base rejection below. An enum class has `bases = []` and
    // `mro = [self_name]` (the `Enum` base is consumed as a marker, not
    // recorded as a real base), no `__init__` requirement, and its body is
    // assignments only (`RED = 1`), not method definitions. Multiple bases
    // with `Enum` (e.g. `class C(Enum, Other):`) are rejected -- `Enum`
    // must be the sole base. A generic enum (`class C[T](Enum):`) is also
    // rejected -- enums are never generic.
    let is_enum = bases.len() == 1 && crate::is_enum_base_name(&bases[0]);
    // #892: `StrEnum` fixes the class's member-value type to `str` before
    // any member is read, so the marker base's own spelling has to be
    // captured here -- `bases.clear()` below consumes it, and
    // `lower_enum_class` is not called until well after that point.
    let enum_marker = if is_enum { bases[0].clone() } else { String::new() };
    if is_enum {
        if type_param.is_some() {
            return Err(unsupported(
                format!(
                    "generic enum class `{class_name}` is not supported -- enums cannot have \
                     type parameters"
                ),
                def.range,
            ));
        }
        // Consume the `Enum` base as a marker: clear `bases` so the class
        // has no real inheritance, and set `mro = [self_name]`.
        bases.clear();
    }
    // #380 (PR-20): PEP 544 protocol detection. A class whose sole base is
    // the bare name `Protocol` (`class P(Protocol):`) is a protocol class.
    // `Protocol` is a builtin base name (see `is_protocol_base_name`), not
    // a user-defined class — intercepted here, before the unknown-base
    // rejection below. A protocol class has `bases = []` and
    // `mro = [self_name]` (the `Protocol` base is consumed as a marker,
    // like `Enum`). Multiple bases with `Protocol` are rejected —
    // `Protocol` must be the sole base. A generic protocol is also
    // rejected. A protocol class with `@dataclass` is rejected (a protocol
    // is a compile-time-only interface, not an instantiable class with
    // fields).
    let is_protocol = bases.len() == 1 && crate::is_protocol_base_name(&bases[0]);
    if is_protocol {
        if type_param.is_some() {
            return Err(unsupported(
                format!(
                    "generic protocol class `{class_name}` is not supported -- protocols cannot \
                     have type parameters in this version"
                ),
                def.range,
            ));
        }
        if is_dataclass {
            return Err(unsupported(
                format!(
                    "protocol class `{class_name}` cannot be a `@dataclass` -- a protocol is a \
                     compile-time-only interface description, not an instantiable class with fields"
                ),
                def.range,
            ));
        }
        // Consume the `Protocol` base as a marker: clear `bases` so the
        // class has no real inheritance, and set `mro = [self_name]`.
        bases.clear();
    }
    // #380 (PR-20): PEP 3119 ABC detection. A class with `ABC` among its
    // bases (`class C(ABC):` or `class C(ABC, Base):`) is abstract. `ABC`
    // is a builtin base name (see `is_abc_base_name`), consumed as a
    // marker — removed from `bases` so it is not treated as a real base
    // class. Unlike `Enum`/`Protocol`, `ABC` can coexist with other bases
    // (`class C(ABC, Base):` is valid — `Base` is a real base, `ABC` is
    // just a marker).
    let is_abstract = bases.iter().any(|b| crate::is_abc_base_name(b));
    if is_abstract {
        // Remove `ABC` from bases — it is a marker, not a real base.
        bases.retain(|b| !crate::is_abc_base_name(b));
    }
    // #380 (PR-20): `@runtime_checkable` on a non-protocol class is
    // rejected — `runtime_checkable` is only valid on protocol classes.
    if decorator_info.runtime_checkable && !is_protocol {
        return Err(unsupported(
            format!(
                "`@runtime_checkable` on class `{class_name}` is not valid -- \
                 `@runtime_checkable` is only valid on protocol classes (`class P(Protocol):`)"
            ),
            def.range,
        ));
    }
    // #432: A generic class (from #387) with base classes is not supported
    // yet — `instantiate_generic_class_methods` creates a monomorphized
    // `HirClassDef` with empty `bases`/`mro`, silently dropping inheritance.
    // Reject this early with a clear error rather than letting it fail with
    // a confusing T0044 downstream.
    if type_param.is_some() && !bases.is_empty() {
        return Err(unsupported(
            format!(
                "generic class `{class_name}` with base classes is not supported yet -- \
                 generic classes cannot inherit from other classes in this version"
            ),
            def.range,
        ));
    }
    validate_bases(
        &class_name,
        &bases,
        defined_classes,
        aliases,
        imports,
        module_items,
        def.range.into(),
    )?;
    // #380 (PR-20): protocol inheritance detection. A class that inherits
    // from a user-defined protocol (`class Q(P):` where `P` is a protocol)
    // is itself a protocol. This check runs after the `Protocol` marker
    // base consumption above (which handles `class P(Protocol):`) and
    // after the base validation loop (which ensures every base is a known
    // class). If any base is a protocol, this class is a protocol too —
    // it inherits the base's `protocol_members` and adds its own.
    let inherits_protocol = bases.iter().any(|base_name| {
        defined_classes
            .iter()
            .any(|(n, d)| n == base_name && d.is_protocol)
    });
    let is_protocol = is_protocol || inherits_protocol;
    if inherits_protocol {
        // A protocol inheriting from a protocol cannot also be a dataclass.
        if is_dataclass {
            return Err(unsupported(
                format!(
                    "protocol class `{class_name}` cannot be a `@dataclass` -- a protocol is a \
                     compile-time-only interface description, not an instantiable class with fields"
                ),
                def.range,
            ));
        }
        // `@runtime_checkable` is valid on a protocol inheriting from
        // another protocol — no extra check needed here.
    }
    let mro = resolve_mro(&class_name, &bases, defined_classes, def.range.into())?;
    // PEP 560 (#611, narrowed by #1130): the class currently being lowered
    // is not in `defined_classes` yet, so without this entry a
    // self-referential `C[int]` annotation inside `C`'s own body would not
    // find its own class at all. Since #1130 there is no subscriptability
    // gate left for it to bypass -- a subscripted annotation whose base
    // resolves to a class is accepted with the type argument erased -- so
    // the entry's remaining purpose is the `class_name` carve-out in
    // `annotation_to_ty`'s subscript arm: the direct `class_defs` lookup is
    // skipped for the six names the `Expr::Name` arm answers before
    // `class_defs` (`name_resolves_before_class_defs`) *except* when the base
    // is the enclosing class's own name, and that exception resolves through
    // this entry.
    class_name_defs.push(ClassAnnotationInfo {
        name: class_name.clone(),
        is_protocol,
        // #693: the hook's return type is not resolvable yet at this
        // point -- an own hook has not been lowered into an
        // `HirItem::Function` yet, and this self-referential entry exists
        // specifically to make a same-body annotation name its own class,
        // not to type it precisely. Such an annotation (rare:
        // `ClassName[x]` referring to the very class whose body it appears
        // in) falls back to `Ty::Instance`, exactly as it did before this
        // field existed.
        class_getitem_return: None,
    });
    let enum_members: Vec<(String, EnumMemberValue)> = Vec::new();
    // #379 (PR-19): an enum class body is assignments only (`RED = 1`),
    // not method definitions. Handle this in a separate function before the
    // regular class-body loop below, which rejects non-`def` statements.
    if is_enum {
        return lower_enum_class(def, class_name, bases, mro, type_param, &enum_marker);
    }
    // #380 (PR-20): a protocol class body contains method definitions
    // (with `...` or `pass` bodies) and annotated assignments
    // (`x: int`). Methods are recorded as `ProtocolMember::Method` and
    // are NOT lowered to `HirItem::Function`s. Annotated assignments are
    // recorded as `ProtocolMember::Attribute`. A protocol method with an
    // implementation body (anything other than `...` or `pass`) is
    // rejected with `C0001`. A protocol `__init__` is rejected with
    // `C0001`. Protocol members from base protocols are inherited.
    if is_protocol {
        return lower_protocol_class(
            def,
            class_name,
            bases,
            mro,
            type_param,
            decorator_info.runtime_checkable,
            defined_classes,
            &class_name_defs,
        );
    }
    // #378 (PR-18): a `@dataclass` class inheriting from a non-dataclass
    // base is rejected -- the base must also be a dataclass for field
    // merging to work. A `@dataclass` with no bases is always valid.
    if is_dataclass {
        for base_name in &bases {
            let base_def = defined_classes
                .iter()
                .find(|(name, _)| name == base_name)
                .map(|(_, def)| def)
                .expect("base class must be defined before the derived class");
            if !base_def.is_dataclass {
                return Err(unsupported(
                    format!(
                        "a `@dataclass` class `{class_name}` cannot inherit from non-dataclass \
                         class `{base_name}` -- dataclass inheritance requires all bases to also \
                         be dataclasses in this version"
                    ),
                    def.range,
                ));
            }
        }
    }
    let ClassBodyOutput {
        mut methods,
        mut items,
        mut attrs,
        properties,
        static_methods,
        class_methods,
        mut dataclass_fields,
        abstract_methods,
        class_attrs,
        method_defaults,
    } = walk_class_body(&ClassBodyInput {
        body: &def.body,
        class_name: &class_name,
        is_dataclass,
        type_param: type_param.as_deref(),
        aliases,
        class_name_defs: &class_name_defs,
        mro: &mro,
        defined_classes,
        imports,
        signatures,
        staticmethod_rebound,
    })?;
    // #911: the class-attribute/instance-slot collision check runs here,
    // after the walk, not at the `AnnAssign` site: `attrs` is populated only
    // when the walk reaches `__init__`, so a class attribute declared before
    // `__init__` would see an empty `attrs` and slip through.
    reject_class_attr_collisions(&ClassAttrCollisionInput {
        class_attrs: &class_attrs,
        attrs: &attrs,
        properties: &properties,
        methods: &methods,
        static_methods: &static_methods,
        class_methods: &class_methods,
        class_name: &class_name,
        mro: &mro,
        defined_classes,
        range: def.range.into(),
    })?;
    // #378 (PR-18): synthesize `__init__`, `__eq__`, and `__repr__` for a
    // `@dataclass` class from its (merged) field list. The synthesized
    // methods flow through the existing method infrastructure as ordinary
    // mangled `HirItem::Function`s, exactly like hand-written methods.
    if is_dataclass {
        // Merge parent dataclass fields (via MRO, in reverse order so
        // parent fields come first) with own fields. The MRO is ordered
        // most-derived-first, so walking it in reverse gives
        // least-derived-first (parent fields before own fields), matching
        // PEP 557's field ordering for inheritance.
        let mut merged_fields: Vec<(String, Ty)> = Vec::new();
        for mro_class in mro.iter().skip(1).rev() {
            // Every class in the MRO (other than the class itself, which
            // `skip(1)` removes) was defined earlier in the module -- the
            // C3 MRO is built from already-lowered class definitions, and
            // dataclass inheritance requires all bases to be dataclasses.
            // Using `.expect` (whose panic path lives in libcore, outside
            // this crate's instrumented regions) avoids an `if let Some`
            // whose `else` branch is structurally unreachable and would
            // show up as a permanently uncovered line under D-014's 100%
            // line-coverage gate.
            let (_, base_def) = defined_classes
                .iter()
                .find(|(name, _)| name == mro_class)
                .expect("MRO class must be defined before the derived class");
            for (name, ty) in &base_def.dataclass_fields {
                match merged_fields.iter().find(|(n, _)| n == name) {
                    Some((_, existing_ty)) if existing_ty != ty => {
                        return Err(Diagnostic::error(
                            "T0052",
                            format!(
                                "attribute `{name}` is declared as `{}` and as `{}` by different dataclasses in the method resolution order of class `{class_name}`",
                                existing_ty.name(),
                                ty.name(),
                            ),
                            Span::new(u32::from(def.range.start()), u32::from(def.range.end())),
                        ));
                    }
                    Some(_) => {}
                    None => merged_fields.push((name.clone(), ty.clone())),
                }
            }
        }
        for (name, ty) in &dataclass_fields {
            match merged_fields.iter().find(|(n, _)| n == name) {
                Some((_, existing_ty)) if existing_ty != ty => {
                    return Err(Diagnostic::error(
                        "T0052",
                        format!(
                            "attribute `{name}` is declared as `{}` in a base class and as `{}` in dataclass `{class_name}`, both in the same method resolution order",
                            existing_ty.name(),
                            ty.name(),
                        ),
                        Span::new(u32::from(def.range.start()), u32::from(def.range.end())),
                    ));
                }
                Some(_) => {}
                None => merged_fields.push((name.clone(), ty.clone())),
            }
        }
        // #913/D-235: a merged dataclass field must not share its name with a
        // `ClassVar` declared anywhere in the MRO. The policy and its three
        // reachable shapes live with the rest of the class-attribute
        // collision rules in `attrs`.
        reject_dataclass_field_class_var_collisions(
            &merged_fields,
            &class_attrs,
            &class_name,
            &mro,
            defined_classes,
            def.range.into(),
        )?;
        // Populate `attrs` from the merged field list (the dataclass's
        // attribute slots are exactly its fields, in declaration order).
        attrs = merged_fields.clone();
        // Synthesize `__init__`: `def __init__(self, f1: T1, f2: T2, ...):
        // self.f1 = f1; self.f2 = f2; ...`
        let init_item = synthesize_dataclass_init(&class_name, &merged_fields);
        let init_mangled = format!("{class_name}.__init__");
        methods.push(("__init__".to_string(), init_mangled));
        items.push(init_item);
        // Synthesize `__eq__`: `def __eq__(self, other: ClassName) -> bool:
        // return self.f1 == other.f1 and self.f2 == other.f2 and ...`
        let eq_item = synthesize_dataclass_eq(&class_name, &merged_fields);
        let eq_mangled = format!("{class_name}.__eq__");
        methods.push(("__eq__".to_string(), eq_mangled));
        items.push(eq_item);
        // Synthesize `__repr__`: `def __repr__(self) -> str: return
        // "ClassName(f1=" + str(self.f1) + ", f2=" + str(self.f2) + ...)"`
        let repr_item = synthesize_dataclass_repr(&class_name, &merged_fields);
        let repr_mangled = format!("{class_name}.__repr__");
        methods.push(("__repr__".to_string(), repr_mangled));
        items.push(repr_item);
        // Store the merged field list for downstream consumers.
        dataclass_fields = merged_fields;
    }
    // #380 (PR-20, PEP 3119): collect inherited abstract methods from the
    // MRO. A concrete (non-abstract) class must override every inherited
    // abstract method — a concrete subclass missing an override is
    // rejected with `C0001`. An abstract class (`is_abstract`) can have
    // unimplemented abstract methods (they are inherited by subclasses).
    let mut all_abstract_methods: Vec<String> = abstract_methods.clone();
    // Iterate over MRO entries (beyond the class itself), finding each in
    // `defined_classes`.  Using `filter_map` avoids a conditional with an
    // unreachable `else` branch — every MRO class is always in
    // `defined_classes`, but `filter_map` skips any that aren't without
    // creating a permanently-uncovered region under D-014.
    for (_, base_def) in mro
        .iter()
        .skip(1)
        .filter_map(|mro_class| defined_classes.iter().find(|(n, _)| n == mro_class))
    {
        for am in &base_def.abstract_methods {
            if !all_abstract_methods.contains(am) {
                all_abstract_methods.push(am.clone());
            }
        }
    }
    // A concrete class must override every inherited abstract method.
    // "Override" means the method name appears in this class's own
    // `methods` (not just inherited). `is_abstract` classes are exempt.
    if !is_abstract {
        for am in &all_abstract_methods {
            // Check if this class overrides the abstract method with a
            // concrete (non-abstract) method.
            let overridden =
                methods.iter().any(|(name, _)| name == am) && !abstract_methods.contains(am);
            if !overridden {
                return Err(unsupported(
                    format!(
                        "concrete class `{class_name}` does not override abstract method \
                         `{am}` inherited from a base class -- all abstract methods must be \
                         overridden in a concrete class"
                    ),
                    def.range,
                ));
            }
        }
    }
    // #966: set only here, from `ensure_init`'s own report -- never from
    // the shape of the constructor it synthesized, and never on the
    // dataclass path, whose `__init__` is a real constructor.
    let mut implicit_object_init = false;
    if !is_dataclass && !methods.iter().any(|(name, _)| name == "__init__") {
        // #432 / #912: the class declares no `__init__` of its own, so it
        // either inherits one from its MRO or gets a synthesized implicit
        // zero-argument constructor.
        implicit_object_init =
            ensure_init(&class_name, &mro, defined_classes, &mut methods, &mut items);
    }
    // #435 (Part B, __init_subclass__) / #585 / #854: PEP 487's
    // `__init_subclass__` hook is recognized as a valid method name. In
    // CPython, the hook is invoked via
    // `super(new_cls, new_cls).__init_subclass__(**kwargs)` at class-creation
    // time -- an MRO lookup that starts immediately *after* the class being
    // created, so it is always the *nearest ancestor's* definition that
    // runs, never the class's own definition (a class's own override only
    // matters later, if and when something else subclasses *it*). This
    // lookup is therefore unconditional: it runs regardless of whether the
    // current class also defines its own `__init_subclass__`, and it always
    // validates the nearest ancestor's body, never the current class's own.
    //
    // (#854 fixed two bugs in an earlier `if`/`else if` version of this
    // guard that instead validated the *current* class's own body whenever
    // the current class also defined an override: doing so (a) let a
    // side-effecting ancestor hook slip through unrejected whenever the
    // subclass added any override at all, however trivial, and (b) as a
    // consequence, missed detecting a `@classmethod`-decorated ancestor
    // hook, since a decorated `__init_subclass__` is registered in a
    // class's `class_methods` table, not `methods`, and the old own-hook
    // check queried only `methods`. The ancestor lookup below scans raw AST
    // `Stmt::FunctionDef` nodes by name -- decorators live on
    // `StmtFunctionDef.decorator_list` and never change the AST variant, so
    // this lookup was already decorator-agnostic; unifying to a single
    // unconditional ancestor lookup closes both gaps as one structural
    // change rather than two independent fixes. See D-214.)
    if let Some(base_ast) = mro.iter().skip(1).find_map(|mro_class| {
        // Only a base this crate can still see the original source of (an
        // earlier class in this same module, per `base_class_asts`) can be
        // re-validated; a base this crate cannot introspect (e.g. a
        // synthetic builtin-exception class, which is never in
        // `base_class_asts`) is left unrestricted, matching this file's
        // existing posture elsewhere. Looking the base up directly in
        // `base_class_asts` (rather than via `defined_classes` first) means
        // a base this crate cannot introspect is filtered out by the same
        // `?` that filters out one lacking `__init_subclass__`, instead of
        // needing a second, separately-unreachable fallback path.
        let (_, base_ast) = base_class_asts.iter().find(|(name, _)| name == mro_class)?;
        base_ast
            .body
            .iter()
            .any(|s| matches!(s, Stmt::FunctionDef(fd) if fd.name.as_str() == "__init_subclass__"))
            .then_some(*base_ast)
    }) {
        // #585 / #854: the nearest ancestor's `__init_subclass__` must be
        // statically evaluable, since CPython invokes it automatically at
        // *this* subclass's creation -- unlike a base class that is merely
        // defined and never subclassed, which must stay legal to compile.
        // Find the last `__init_subclass__` def in the ancestor's body
        // (rebind semantics: the last definition is the one actually bound)
        // and validate its body against this subclass's own creation site
        // (`def.range`), matching D-213's "Alternatives" reasoning that the
        // diagnostic belongs where CPython actually invokes the hook.
        for stmt in base_ast.body.iter().rev() {
            match stmt {
                Stmt::FunctionDef(fd) if fd.name.as_str() == "__init_subclass__" => {
                    validate_init_subclass_body(&fd.body, def.range.into())?;
                    break;
                }
                _ => {}
            }
        }
    }
    let class_def = HirClassDef {
        class_attrs,
        exception_type_tag: None,
        name: class_name,
        bases,
        mro,
        attrs,
        methods,
        properties,
        static_methods,
        class_methods,
        type_param,
        is_enum: false,
        implicit_object_init,
        method_defaults,
        enum_members,
        is_dataclass,
        dataclass_fields,
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: all_abstract_methods,
        is_abstract,
    };
    // #969: the layout gate runs last, on the finished `HirClassDef` -- it
    // needs `attrs` complete (a `@dataclass` fills it from the merged field
    // list well below `walk_class_body`) and every base's own `attrs` final
    // (a base is always defined earlier in the module, so it is).
    validate_mro_slot_layout(&class_def, defined_classes, def.range.into())?;
    // #1337 (WI-6b): a user dunder a raised exception value would ignore.
    exception_dunders::reject_ignored_exception_dunders(
        &class_def,
        defined_classes,
        def.range.into(),
    )?;
    Ok((class_def, items))
}
