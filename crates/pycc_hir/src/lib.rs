use pycc_diag::{Diagnostic, Span};

mod boolop;
mod buffer_store;
pub mod builtins;
mod class;
mod compare_chain;
mod container;
mod dunder_name;
mod exception;
mod expr;
mod func;
mod hir_module;
mod if_exp;
mod import;
mod int_boundary;
pub mod int_enum;
mod isinstance_narrow;
pub mod locate;
mod module;
pub mod ndarray;
mod not_implemented;
mod post_if;
pub mod positions;
pub use positions::SrcPos;
pub mod printf;
mod program;
mod return_coverage;
mod stmt;
mod stop_iteration;
mod typecheck;
mod uses_objects;

// HIR node definitions split out of the crate root (mechanical move).
mod hir_expr;
mod hir_stmt;
pub use hir_expr::*;
pub use hir_stmt::*;

pub use boolop::{BoolOpKind, bool_op_result_ty};
pub use buffer_store::{body_returns_inside_finally, body_returns_slice_of, body_stores_into};
pub use class::enum_call::enum_class_call_message;
pub use class::{external_slot_ty, set_external_attribute_reads};
pub use class::{
    ClassAttrValue, ClassNamespaceWinner, CopiedMemberKind, EnumMemberValue, EqRefusal,
    ForeignCallableRef, HOISTED_CLASS_ATTR_PREFIX, HashRefusal, HirClassDef, InheritedCopy,
    InstanceEq, InstanceHash, InstanceHashLowering, PropertyDef, ProtocolMember,
    SUPER_TARGET_MARKER, binds_member, class_name_foreign_static, class_namespace_winner,
    dataclass_repr_body, declares_name_outside_class_attrs, first_definer, flat_attr_layout,
    class_lookup, inherited_copy_name, inherited_copy_origin, instance_foreign_static,
    instance_rc_kind, is_exception_object_class, is_refcounted_class,
    method_shadows_foreign_static, mro_has_instance_slot, resolve_instance_eq,
    resolve_instance_hash, slot_behind_foreign_static_meets_property, slot_rc_kind,
    subclass_divergence,
};
pub use compare_chain::{CompareLink, chain_as_conjunction, compare_chain_operands};
pub use container::{
    DisplayKind, check_container_ty, check_tuple_element_ty, display_is_object,
    empty_object_constructor, homogeneous_tuple_element, is_hashable_ty, is_list_element_ty,
    is_shaped_list_display, object_comp_collects_natively,
    is_var_tuple_element_ty, sequence_element, set_element_message, tuple_fits_var_tuple,
    tuple_is_boxed,
};
pub use dunder_name::DUNDER_NAME;
pub use exception::{
    BUILTIN_EXCEPTION_CLASSES, EXCEPTION_GROUP_TYPE_TAG, EXCEPTION_INIT_MANGLED_NAME,
    FIRST_USER_EXCEPTION_TYPE_TAG, FOREIGN_BASE_EXCEPTION_TYPE_TAG, HirExceptHandler,
    MAX_USER_EXCEPTION_CLASSES, builtin_exception_class_defs, builtin_exception_init_item,
    builtin_exception_parent, except_handler_binding_type_name, is_builtin_exception_class,
    is_flat_builtin_exception_class,
};
pub use expr::object_keyword_call::{
    KEYWORD_CALL_UNSUPPORTED, STAR_ARG, STAR_ARGS, STAR_KWARGS, is_unpacking,
};
pub use expr::receiver_takes_method_path;
pub(crate) use func::{
    annotation_to_ty, lower_arg_list, lower_function, lower_return_annotation, type_param_name,
    with_bare_container_advice,
};
pub use hir_module::{
    ForeignImportSite, FromImport, HirModule, ImportBinding, ProjectBindingKind,
    foreign_binds_root, foreign_bound_module, foreign_bound_object, foreign_import_statement,
    killed_names, opens_foreign_statement, top_level_bound_names,
};
pub use if_exp::if_exp_result_ty;
pub use import::{
    ProjectImportRequest, ResolvedImport, ResolvedImports, ResolvedModule, is_native_std_module,
    names_used_as_values, project_import_requests,
};
pub(crate) use import::{
    import_local_name, lower_import_stmt, lower_legacy_type_alias_ann_assign, lower_type_alias_stmt,
};
pub use isinstance_narrow::{IsInstancePolarity, isinstance_narrow_target, isinstance_test};
pub use module::{LoweredModule, last_item_origins, lower_all, lower_checked, lower_module};
pub use not_implemented::{WIDENED_RETURN_HELP, body_returns_not_implemented};
pub use post_if::continuation_narrows;
pub use program::{
    LinkInput, LinkLayout, Placement, finalize, link, link_layout, set_exception_tag_base,
};
/// The callee of a nested function used as a value (`pycc_ast::desugar`).
pub use pycc_ast::CLOSURE_MARKER;
pub use pycc_ast::function_qualname;
pub use pycc_ast::namespace;
pub use return_coverage::{block_always_returns, block_never_completes, loop_never_ends_normally};
pub use stmt::del::{deleted_names, mentioned_names};
pub use stop_iteration::{stop_iteration_sources, stop_iteration_value_ty};
pub use typecheck::{
    ExtractClassNamesError, eval_isinstance_single, eval_issubclass_single, extract_class_names,
    is_abc_base_name, is_builtin_type_name, is_enum_base_name, is_protocol_base_name,
};
pub use uses_objects::declares_object_slot;

/// D-038's visibility predicate: a name is public unless it begins with an
/// underscore.
///
/// One canonical home for a rule four call sites used to spell inline
/// (`func.rs`, `class.rs`, and twice in `class/protocol.rs`). It is `pub`
/// because the rule now has a consumer outside HIR lowering as well: D-244
/// rule 1 exports *every public module-level function* of an `--ext` build,
/// and a public `@staticmethod` or `@classmethod` of a public class, and
/// the driver derives that export set from the same predicate the
/// frontend's own `T0001` annotation requirement uses -- applied
/// unchanged to the class name and the method name alike, never forked --
/// so "public" can only ever mean one thing across the compiler.
///
/// The name is the whole input on purpose. This is a *visibility* predicate
/// over a source-level identifier, unrelated to `pycc_types`'
/// `is_private_solver_scalar`, which is a `Ty`-shape predicate that happens
/// to carry "private" in its name.
#[must_use]
pub fn is_public_name(name: &str) -> bool {
    !name.starts_with('_')
}

/// The prefix of a per-call-site copy of a nested helper
/// (`pycc_types::static_isinstance`, issue #7): private, compiler-made, and
/// with no `def` statement of its own, so codegen calls it directly, as it
/// does a monomorphized generic (`0gen_...`). No source name can spell it
/// (`pycc_ast` reserves `_pycc_`).
pub const SITE_COPY_PREFIX: &str = "_pycc_site";

/// The prefix of the function a module of a package built whole calls in
/// place of a compiled function of another module of the package (v0.8,
/// `pycc_codegen::SiblingTarget`): `_pycc_sib_<imported name>`. Like a
/// per-call-site copy it is compiler-made and called directly; no source
/// name can spell it (`pycc_ast` reserves `_pycc_`).
pub const SIBLING_CALL_PREFIX: &str = "_pycc_sib_";

/// The prefix of the class a module of a package built whole declares in
/// place of a compiled class of another module of the package whose
/// instances it reads natively (v0.8, `pycc_codegen::SiblingClass`):
/// `_pycc_sibc_<class>`. Its slots are the other module's, in the same
/// order and of the same types; its methods are the other module's
/// compiled ones, which codegen calls directly. No source name can spell
/// it (`pycc_ast` reserves `_pycc_`).
pub const SIBLING_CLASS_PREFIX: &str = "_pycc_sibc_";

thread_local! {
    /// Imported names an annotation resolves to a sibling class
    /// ([`SIBLING_CLASS_PREFIX`]) of the module being lowered, or to
    /// `object` (`None`).
    static SIBLING_CLASS_NAMES: std::cell::RefCell<Vec<(String, Option<String>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Makes an annotation naming `name` (a name a foreign import binds to a
/// class another module of the package compiled) resolve to the sibling
/// class `class`, or to `object` when `class` is `None` (the class is read
/// through CPython), for each `(name, class)` pair, on this thread until
/// the next call.
pub fn set_sibling_class_names(names: Vec<(String, Option<String>)>) {
    SIBLING_CLASS_NAMES.with(|cell| *cell.borrow_mut() = names);
}

/// What an annotation naming `name` resolves to, if
/// [`set_sibling_class_names`] named it: `Some(None)` for `object`.
pub(crate) fn sibling_class_for(name: &str) -> Option<Option<String>> {
    SIBLING_CLASS_NAMES.with(|cell| {
        cell.borrow()
            .iter()
            .find(|(local, _)| local == name)
            .map(|(_, class)| class.clone())
    })
}

/// The callee of the call `pycc_hir` lowers a `yield value` statement to
/// (`HirStmt::ExprStmt`); not a Python identifier, so no source name can
/// spell it.
pub const YIELD_MARKER: &str = "<yield>";

/// The callee of the one-argument call that makes a number an
/// `int | float` value (`pycc_types`' `int | float` widening wraps the
/// elements of a list it widens in it); not a Python identifier, so no
/// source name can spell it.
pub const NUM_MARKER: &str = "<int | float>";

/// The callee of the one-argument call that makes an `int` or `None` an
/// `int | None` element (`pycc_types` wraps the elements of a list it
/// widens to `list[int | None]` in it); not a Python identifier.
pub const OPTIONAL_INT_MARKER: &str = "<int | None>";

/// The callee of the call `pycc_hir` lowers `yield from sub` to (as a
/// statement or a value); typed as `sub`'s return type.
pub const YIELD_FROM_MARKER: &str = "<yield from>";

/// The callee of the statement `pycc_hir` lowers a generator's `return
/// value` to (followed by a bare `return`): the value becomes
/// `StopIteration.value`.
pub const GEN_RETURN_MARKER: &str = "<generator return>";

/// Whether `body` holds a call of [`YIELD_MARKER`] anywhere: a generator
/// function's body (`func::lower_function` marks its return type).
pub fn body_yields(body: &[HirStmt]) -> bool {
    fn expr_yields(expr: &HirExpr) -> bool {
        matches!(expr, HirExpr::Call { callee, .. } if callee == YIELD_MARKER || callee == YIELD_FROM_MARKER)
    }
    body.iter().any(|stmt| match stmt {
        HirStmt::ExprStmt(expr) => expr_yields(expr),
        HirStmt::If { body, orelse, .. } => body_yields(body) || body_yields(orelse),
        HirStmt::While { body, .. }
        | HirStmt::ForRange { body, .. }
        | HirStmt::ForList { body, .. }
        | HirStmt::ForObject { body, .. } => body_yields(body),
        HirStmt::LoopElse { loop_stmt, orelse } => {
            body_yields(std::slice::from_ref(loop_stmt.as_ref())) || body_yields(orelse)
        }
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }
        | HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            body_yields(body)
                || handlers.iter().any(|h| body_yields(&h.body))
                || body_yields(orelse)
                || body_yields(finalbody)
        }
        HirStmt::Match { cases, .. } => cases.iter().any(|c| body_yields(&c.body)),
        _ => false,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    Int,
    Float,
    /// `int | float`: a value that is an `int` at one point and a `float`
    /// at another (`best = float('inf')` later replaced by an `int`, an
    /// accumulator that starts as `0` and receives floats), spelled
    /// `int | float` in an annotation or inferred by `pycc_types` where
    /// one name is bound to both. Every operation has CPython's semantics
    /// for whichever the value is at run time. One NaN-boxed word
    /// (`pycc_rt::num`): a `float` is its own bits, an `int` is inline up
    /// to 50 bits and a counted heap bigint beyond.
    Num,
    Bool,
    Str,
    None,
    Infer,
    /// A generic function's own type parameter (PEP 695, D-133), e.g. the `T`
    /// in `def f[T](x: T) -> T`. Distinct from `Ty::Infer`: this is resolved by
    /// call-site substitution (D-134), not by unification, and must never
    /// reach `pycc_mir` unsubstituted (same invariant `Ty::Infer` already
    /// holds, see the assertion near this enum's other pre-MIR checks).
    /// Boxed as `Box<String>`, not `Box<str>`: `str` is unsized, so
    /// `Box<str>` is itself a fat (16-byte, data-ptr + length) pointer,
    /// which measured `size_of::<Ty>() == 24` here -- it broke the
    /// niche-filling layout uniformity the other thin-pointer variants
    /// rely on (see `Tuple`'s doc comment above for the same phenomenon
    /// with `Box<[Ty]>` vs `Box<Vec<Ty>>`). `Box<String>` is a single
    /// (8-byte) thin pointer to a further heap-indirected `String` --
    /// confirmed by measurement to restore `size_of::<Ty>() == 16`,
    /// matching `Tuple`'s `Box<Vec<Ty>>` shape. (D-133's ADR text says
    /// `Box<str>`; this is a deliberate, measurement-driven deviation --
    /// `Box<str>` does not in fact keep `Ty` at the D-109 ceiling.)
    Param(Box<String>),
    /// `list[T]`. One generic runtime representation serves every element
    /// type [`is_list_element_ty`] admits (`int`, `float`, `bool`, `str`, a
    /// user-class instance, a nested list); every other `T` is `T0034`.
    List(Box<Ty>),
    /// `dict[K, V]`. No v0.2 code path constructs this yet (PR-11's own
    /// scope per `docs/DELIVERY_PLAN.md`) -- the variant exists now only
    /// because D-089 decided `Ty`'s full recursive shape up front, so
    /// every later PR's match arms are additive, not migratory again.
    /// The key/value pair is boxed together as a single pointer (D-109):
    /// this variant was not itself the dominant contributor to `Ty`'s
    /// pre-fix 24-byte size (`Tuple(Vec<Ty>)` was), but once `Tuple`
    /// shrinks to a single 8-byte pointer too, `Dict`'s original
    /// two-separate-`Box<Ty>` shape (16 bytes) would become the new
    /// ceiling -- boxing it avoids that, not because `dict[K,V]` needs
    /// its own codegen yet.
    Dict(Box<(Ty, Ty)>),
    /// `set[T]`. Same status as `Dict` above -- PR-11's own scope.
    Set(Box<Ty>),
    /// `frozenset[T]` (Part 1 of #1319). The immutable sibling of `Set`:
    /// only `frozenset[int]` is compiled (D-122's one-combination gate,
    /// `T0038`), and it shares `set[int]`'s run-time representation
    /// (`PyIntSetObj`); immutability is enforced by `pycc_types` alone.
    /// `Box<Ty>` is a thin pointer, the same shape as `Set`, so this
    /// variant keeps `size_of::<Ty>()` at 16 bytes, inside the D-109
    /// ceiling (measured, and pinned by `ty_size_stays_within_d109_ceiling`).
    FrozenSet(Box<Ty>),
    /// `tuple[T, ...]`, the homogeneous variadic tuple (v0.5), for a
    /// counted `T` ([`is_var_tuple_element_ty`]: `str`, an instance, a
    /// container). Its run-time object is the boxed tuple object every
    /// tuple of counted elements already is (`pycc_rt::tuple`), so a
    /// fixed-arity `tuple[T, T]` *is* a `tuple[T, ...]` with no conversion
    /// ([`tuple_fits_var_tuple`]). It reads like a list -- `len`, `t[i]`,
    /// iteration are the list's inline paths over the shared `{len, data}`
    /// prefix, and the list runtime's slice, `+`, `*` and `in` handle a
    /// tuple object -- and renders, compares and hashes as a tuple;
    /// `pycc_types` admits no operation that mutates it. It crosses into
    /// CPython as a `tuple` copy, which immutability makes unobservable
    /// except through `is`. `Box<Ty>`, a thin pointer, keeps
    /// `size_of::<Ty>()` at 16 bytes.
    VarTuple(Box<Ty>),
    /// `tuple[A, B, ...]`. Same status as `Dict` above -- PR-11's own
    /// scope. Boxed (D-109) as `Box<Vec<Ty>>` -- a second indirection: a
    /// thin (8-byte) pointer to a heap-allocated `Vec<Ty>` -- not as
    /// `Box<[Ty]>` (a 16-byte fat pointer: data ptr + length), which was
    /// tried first and measured `size_of::<Ty>() == 24`, no reduction at
    /// all from the pre-fix size (confirmed independently in-crate and
    /// via a standalone `rustc` reproduction). `Box<Vec<Ty>>` measured
    /// `size_of::<Ty>() == 16` instead (`align_of::<Ty>()` stayed `8` in
    /// every configuration measured). The most plausible explanation is
    /// that rustc's niche-filling enum-layout optimization (the trick
    /// behind `size_of::<Option<Box<T>>>() == size_of::<Box<T>>()`) needs
    /// every dataful variant to share a uniform pointer shape to collapse
    /// the discriminant for free -- `Box<[Ty]>`'s fat pointer broke that
    /// uniformity against `List`/`Dict`/`Set`'s thin ones, `Box<Vec<Ty>>`
    /// restores it -- but this project has not independently re-derived
    /// rustc's exact layout algorithm against every configuration (in
    /// particular, the pre-fix shape already had non-uniform dataful
    /// variant sizes yet still measured 24, not the 32 bytes a naive
    /// "tag plus largest payload" rule would predict). Treat the measured
    /// numbers above as the authoritative facts and this paragraph's
    /// mechanism as a plausible, not proven, explanation.
    Tuple(Box<Vec<Ty>>),
    /// A list display whose elements have different types (`[name, i]`, a
    /// `str` and an `int`): a list whose positions keep fixed types, the
    /// shape (v0.8 accept 5). Only `pycc_types` sees it. What it admits is
    /// what is valid for every list of that shape: a literal index, `len`,
    /// comparison with a list of the same shape (lexicographic, CPython's
    /// list `<`), `str`/`repr`/`print`, unpacking, and being an element of
    /// a list (which sorts). A store, a method, iteration, `+`/`*`, hashing
    /// and a CPython boundary are refused. After the check it is erased to
    /// [`Ty::Tuple`] ([`erase_shaped_lists`]): its run-time object is a
    /// tuple object flagged to render and compare as the list it is. Only a
    /// boxed shape ([`tuple_is_boxed`]) is one; `[1, 2.5]` stays
    /// `list[int | float]`.
    ShapedList(Box<Vec<Ty>>),
    /// A single-class instance (no inheritance in this PR -- see D-154,
    /// "class instance layout"). Carries only the originating class's name,
    /// not its shape: attribute-name -> slot-index mapping and method
    /// signatures live in a module-level side table
    /// (`HirModule::class_defs`, `pycc_hir::class::HirClassDef`), the same
    /// shape `type_aliases`/`imports` already use for compile-time-only
    /// side information that isn't part of a `HirItem`/`HirStmt` value
    /// itself. Boxed as `Box<String>`, not `Box<str>`, mirroring
    /// `Ty::Param`'s own documented reasoning exactly: `Box<str>` is a fat
    /// (16-byte) pointer because `str` is unsized, which would break the
    /// niche-filling layout uniformity every other dataful `Ty` variant
    /// relies on to stay at a single thin (8-byte) pointer; `Box<String>`
    /// is a single thin pointer to a further heap-indirected `String`, so
    /// this variant does not move `size_of::<Ty>()` past the D-109 16-byte
    /// ceiling (see `ty_size_stays_within_d109_ceiling` below).
    Instance(Box<String>),
    /// A protocol-typed value (PEP 544, #380, PR-20). Carries the protocol
    /// class name. Distinct from `Ty::Instance` so that `is_assignable` and
    /// every match arm can distinguish "this value is an instance of a
    /// concrete class" from "this value is typed as a protocol." A
    /// protocol-typed value's concrete representation is determined by the
    /// first assignment (D-040 sticky-representation rule) or by
    /// monomorphization at each call site (D-006/D-134). Boxed as
    /// `Box<String>`, mirroring `Ty::Instance`'s own documented reasoning
    /// exactly — maintains the D-109 16-byte `size_of::<Ty>()` ceiling.
    Protocol(Box<String>),
    /// `T | None` (PEP 604, D-197), scoped in this PR to `Optional[Ty::Int]`
    /// for codegen (see `crates/pycc_codegen`'s niche-optimized `int | None`
    /// representation) -- every other inner type type-checks but is
    /// rejected pre-lowering by a dedicated diagnostic. A general
    /// `Ty::Union(Vec<Ty>)` was considered and rejected for this PR: Python's
    /// `X | Y | None` general union has no agreed-on runtime representation
    /// here yet, while `Optional[T]` alone has a well-known niche-filling
    /// encoding (mirroring `Option<Box<T>>` in Rust itself). Boxed as
    /// `Box<Ty>`, the same thin-pointer shape as `List`/`Set`, so this
    /// variant does not move `size_of::<Ty>()` past the D-109 16-byte
    /// ceiling.
    Optional(Box<Ty>),
    /// An opaque CPython object (Part 1 of #1026): the value a foreign
    /// `import numpy` binds. Carries no shape at all -- pycc knows only
    /// that the value is a `PyObject *` owned by the CPython runtime.
    /// `pycc_types` admits only the operations lowered through the CPython C
    /// API (`docs/TYPE_SYSTEM.md`'s `object` row) and refuses every other one
    /// with `I0404`. In a `native` build `object` itself is unspellable in
    /// an annotation: `annotation_to_ty` rejects it with `C0001`, and `Any`
    /// with `T0002`. In every module of a `pycc build --ext` artifact (D-258,
    /// #1397) `typing.Any`, the builtin `object`, a bare
    /// `list`/`dict`/`tuple`/`set` and a container with an object argument
    /// all spell this variant, and a public signature carries it across the
    /// export boundary as the `PyObject *` itself. Since Part 1 of #1367 the name of a
    /// class a foreign import binds resolves to this variant in an
    /// annotation in either mode (`docs/TYPE_SYSTEM.md`'s `object` row). The
    /// variant is deliberately unit-shaped.
    /// A unit variant adds no payload, so `size_of::<Ty>()` stays at the
    /// D-109 16-byte ceiling.
    Object,
    /// A one-dimensional buffer over contiguous `float` (C `double`)
    /// elements, admitted only at a `pycc build --ext` boundary (Part 1 of
    /// #1027, D-244). Carries no shape: what the boundary guarantees is
    /// exactly what the four refusal arms in `src/ext_build.rs` check at
    /// run time -- exports a buffer, C-contiguous, `ndim == 1`, format
    /// `"d"` -- so no payload could add information the wrapper has not
    /// already proved. A unit variant adds no payload, so `size_of::<Ty>()`
    /// stays at the D-109 16-byte ceiling.
    ///
    /// Three source spellings lower to this one variant: `memoryview`,
    /// since #1129 `ndarray`, and since #1134 `NDArray`. They are one type
    /// because their run-time contract is identical -- the boundary admits
    /// any object exporting a conforming buffer, whichever way the parameter
    /// was spelled -- so diagnostics render the canonical `memoryview` for
    /// all three, exactly as they already do for an alias of it.
    MemoryView,
    /// A typed numpy array (`NDArray[np.float64]` and friends, see
    /// `crate::ndarray`): a `numpy.ndarray` whose dtype and number of
    /// dimensions are known statically. Represented at run time by the
    /// array's own `PyObject *`; `pycc_mir` erases it to `Ty::Object`
    /// after lowering the array operations. `NdTy` is two bytes, so
    /// `size_of::<Ty>()` stays 16.
    NdArray(crate::ndarray::NdTy),
    /// A generator object yielding values of the boxed type: what calling
    /// a generator function (a `def` whose body contains `yield`) returns,
    /// spelled `Iterator[T]`, `Iterable[T]` or `Generator[T, None, None]`
    /// in an annotation. A reference-counted pointer to a
    /// `pycc_rt::generator::PyGenObj`, which owns the function's LLVM
    /// coroutine frame (`pycc_codegen::generator`). Boxed as a thin
    /// pointer, like `List`, so `size_of::<Ty>()` stays 16 bytes.
    ///
    /// The tuple is `(yield, send, return)`: `Generator[Y, S, R]`.
    /// `Iterator[T]` and `Iterable[T]` are `Generator[T, None, None]`.
    Generator(Box<(Ty, Ty, Ty)>),
    /// A function value, `Callable[[A, B], R]`: a module-level function
    /// used as a value, or a nested function closed over its enclosing
    /// variables (`pycc_ast::desugar::closures`). A reference-counted
    /// pointer to a `pycc_rt::closure::PyClosureObj` holding the captured
    /// values and the function that receives them (`pycc_codegen::closure`).
    /// Two callables are the same type when their parameter and return
    /// types are. Boxed as one thin pointer, like `Dict`.
    Callable(Box<(Vec<Ty>, Ty)>),
}

thread_local! {
    static NON_STRICT_PROGRAM: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Records whether the program this thread compiles is built in
/// non-strict mode, for the later stages that lower it (`pycc_mir`): the
/// driver sets it through `pycc_types::set_non_strict`, before checking.
pub fn set_non_strict_program(non_strict: bool) {
    NON_STRICT_PROGRAM.with(|cell| cell.set(non_strict));
}

/// Whether the program this thread compiles is built in non-strict mode
/// ([`set_non_strict_program`]).
#[must_use]
pub fn is_non_strict_program() -> bool {
    NON_STRICT_PROGRAM.with(std::cell::Cell::get)
}

thread_local! {
    static HOST_CLASSES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Records whether CPython code can subclass the classes of the program
/// this thread compiles (v0.9): an `--ext` module's or a non-strict
/// program's, whose classes are published. The driver sets it before
/// lowering; `pycc_mir`'s dispatchers then run a Python subclass's
/// overrides.
pub fn set_host_classes(host: bool) {
    HOST_CLASSES.with(|cell| cell.set(host));
}

/// Whether CPython code can subclass the program's classes
/// ([`set_host_classes`]).
#[must_use]
pub fn host_classes() -> bool {
    HOST_CLASSES.with(std::cell::Cell::get)
}

/// Whether `T | None` holds a `T` as a pointer that is null for `None`: a
/// `str`, a list, dict or set, a variadic or boxed tuple, or a class
/// instance. Every one is one pointer word whose reference counting is
/// null-safe, so `pycc_codegen` represents the optional as the bare pointer
/// (`int`, `float` and `bool` payloads carry a presence flag instead).
pub fn is_pointer_optional_inner(ty: &Ty) -> bool {
    match ty {
        Ty::Str
        | Ty::List(_)
        | Ty::Dict(_)
        | Ty::Set(_)
        | Ty::FrozenSet(_)
        | Ty::VarTuple(_)
        | Ty::Instance(_) => true,
        Ty::Tuple(elements) => tuple_is_boxed(elements),
        _ => false,
    }
}

/// The `int | float` two numbers of different types join to: an `int` (or
/// `bool`) with a `float`, or anything numeric with an `int | float`.
/// `None` for any other pair, including `int` with `bool` (one `int`).
#[must_use]
pub fn num_join(a: &Ty, b: &Ty) -> Option<Ty> {
    num_join_inner(a, b)
}

/// The `T | None` a list display's elements `a` and `b` join to: `None`
/// with a `T` (or a `T | None`), for a `T` a container may hold optional
/// (`crate::container::is_optional_element_ty`). `None` for any other pair.
#[must_use]
pub fn optional_element_join(a: &Ty, b: &Ty) -> Option<Ty> {
    let joined = match (a, b) {
        (Ty::None, Ty::None) => return None,
        (Ty::None, other) | (other, Ty::None) => match other {
            Ty::Optional(_) => other.clone(),
            other => Ty::Optional(Box::new(other.clone())),
        },
        (Ty::Optional(inner), other) | (other, Ty::Optional(inner)) if **inner == *other => {
            Ty::Optional(inner.clone())
        }
        _ => return None,
    };
    crate::container::is_optional_element_ty(&joined).then_some(joined)
}

fn num_join_inner(a: &Ty, b: &Ty) -> Option<Ty> {
    let number = |ty: &Ty| matches!(ty, Ty::Int | Ty::Bool | Ty::Float | Ty::Num);
    let floating = |ty: &Ty| matches!(ty, Ty::Float | Ty::Num);
    (a != b && number(a) && number(b) && (floating(a) || floating(b))).then_some(Ty::Num)
}

impl Ty {
    /// `Generator[element, None, None]` (`Iterator[element]`).
    pub fn generator(element: Ty) -> Ty {
        Ty::Generator(Box::new((element, Ty::None, Ty::None)))
    }

    /// `Generator[yielded, sent, returned]`.
    pub fn generator_of(yielded: Ty, sent: Ty, returned: Ty) -> Ty {
        Ty::Generator(Box::new((yielded, sent, returned)))
    }

    pub fn name(&self) -> String {
        match self {
            Ty::Int => "int".to_string(),
            Ty::Float => "float".to_string(),
            Ty::Num => "int | float".to_string(),
            Ty::Bool => "bool".to_string(),
            Ty::Str => "str".to_string(),
            Ty::None => "None".to_string(),
            Ty::Infer => "<inferred>".to_string(),
            Ty::Param(name) => name.to_string(),
            Ty::List(elem) => format!("list[{}]", elem.name()),
            Ty::Dict(kv) => format!("dict[{}, {}]", kv.0.name(), kv.1.name()),
            Ty::Set(elem) => format!("set[{}]", elem.name()),
            Ty::FrozenSet(elem) => format!("frozenset[{}]", elem.name()),
            Ty::VarTuple(elem) => format!("tuple[{}, ...]", elem.name()),
            Ty::Tuple(elems) => format!(
                "tuple[{}]",
                elems.iter().map(Ty::name).collect::<Vec<_>>().join(", ")
            ),
            Ty::ShapedList(elems) => format!(
                "list[{}] (a fixed shape)",
                elems.iter().map(Ty::name).collect::<Vec<_>>().join(", ")
            ),
            Ty::Instance(class_name) => class_name.to_string(),
            Ty::Protocol(name) => name.to_string(),
            Ty::Optional(inner) => format!("{} | None", inner.name()),
            Ty::Object => "object".to_string(),
            Ty::MemoryView => "memoryview".to_string(),
            Ty::NdArray(nd) => nd.name(),
            Ty::Generator(sig) => format!(
                "Generator[{}, {}, {}]",
                sig.0.name(),
                sig.1.name(),
                sig.2.name()
            ),
            Ty::Callable(sig) => format!(
                "Callable[[{}], {}]",
                sig.0.iter().map(Ty::name).collect::<Vec<_>>().join(", "),
                sig.1.name()
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOpKind {
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Pow,
    LShift,
    RShift,
    BitAnd,
    BitOr,
    BitXor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOpKind {
    Eq,
    NotEq,
    Lt,
    LtE,
    Gt,
    GtE,
    /// `is` (D-197, #763, Part 1 of #747). This compiler's first `is`/`is
    /// not` support of any kind -- deliberately scoped at HIR-lowering to
    /// exactly the case #763 needs: one operand is syntactically
    /// `Expr::NoneLiteral`, the other anything. Since Part 1 of #1371 a
    /// single comparison between two non-literal operands lowers too, and
    /// `pycc_types` admits it only between two CPython objects; every other
    /// pair keeps the `C0001` "comparison operator not supported yet"
    /// rejection (see `crate::compare_chain::lower_cmp_op`). Since Part 11
    /// of #1371 a chain link between two non-literal operands lowers the
    /// same way, and `pycc_types` refuses the chain: the object chain's
    /// `I0404` when an operand is an object, that `C0001` otherwise.
    Is,
    /// `is not`. Same scoping as `Is` above.
    IsNot,
    /// `item in container` (Part 2b of #1371). Lowered only for a single
    /// comparison whose right operand is not syntactically a literal or a
    /// display (see `crate::compare_chain::lower_cmp_op`); `pycc_types`
    /// admits it only with a CPython-object container and refuses every
    /// other pair with the same `C0001` "comparison operator not supported
    /// yet" message. A chain keeps that `C0001` here.
    In,
    /// `item not in container`. Same scoping as `In` above.
    NotIn,
}

/// The unary operators this compiler supports (#603 Part 2, #604 Part 3, of
/// #573).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOpKind {
    /// `-x`.
    USub,
    /// `+x`. Not a no-op: `+True` is the integer `1` in Python, so a
    /// `bool` operand crosses into `int` here (see
    /// `pycc_types::unop::unary_result_type`).
    UAdd,
    /// `not x`. Defined by truthiness: the result is always `bool`, for
    /// every operand type this compiler can evaluate a truth value for
    /// (#604, Part 3 of #573).
    Not,
    /// `~x`. Bitwise complement, defined only for `int` (`bool` included,
    /// since it is a numeric subtype); `~x == -x - 1` (#604, Part 3 of
    /// #573).
    Invert,
}

impl UnaryOpKind {
    /// The operator's Python source spelling, for diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            UnaryOpKind::USub => "-",
            UnaryOpKind::UAdd => "+",
            UnaryOpKind::Not => "not",
            UnaryOpKind::Invert => "~",
        }
    }
}

fn body_jumps(body: &[HirStmt], is_jump: &dyn Fn(&HirStmt) -> bool) -> bool {
    body.iter().any(|stmt| stmt_jumps(stmt, is_jump))
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirItem {
    Function {
        name: String,
        params: Vec<(String, Ty)>,
        return_ty: Ty,
        body: Vec<HirStmt>,
    },
    TopLevelStmt(HirStmt),
}

fn unsupported<R>(message: impl Into<String>, range: R) -> Diagnostic
where
    std::ops::Range<u32>: From<R>,
{
    let range = std::ops::Range::<u32>::from(range);
    let message = message.into();
    // Issue #890: a `C0001` message must name the rejected construct in
    // Python terms (`pycc_ast::expr_kind_name`/`stmt_kind_name`), never
    // render an AST node's Rust `Debug` form. Every `ruff_python_ast`
    // node's `Debug` output contains `node_index: NodeIndex(`, so this
    // catches a reintroduced `{:?}` of a node in every test build. The
    // assertion is deliberately message-less: a messaged `debug_assert!`
    // leaves its never-formatted message literal as an uncovered region
    // under D-014's 100%-region gate, and this comment says why it fired.
    debug_assert!(!message.contains("NodeIndex("));
    Diagnostic::error("C0001", message, Span::new(range.start, range.end))
}

/// `C0002`: "stdlib symbol not supported yet" (D-136) -- distinct from
/// `C0001`. Used only when the *module* of a `from ... import ...`
/// statement is recognized but a specific imported name inside it is not
/// registered (e.g. `from math import isnan`), as opposed to `C0001`'s
/// "we don't recognize this import shape/module at all."
fn unresolved_symbol<R>(message: impl Into<String>, range: R) -> Diagnostic
where
    std::ops::Range<u32>: From<R>,
{
    let range = std::ops::Range::<u32>::from(range);
    Diagnostic::error("C0002", message, Span::new(range.start, range.end))
}

/// `L0001`: reused (not a new code, D-148) for a post-parse statement- or
/// expression-context violation caught during HIR lowering -- a construct
/// that is syntactically well-formed but only valid Python in a context the
/// enclosing statement or expression isn't in (`break`/`continue` outside a
/// loop, `async for` outside an async function -- D-148; `yield`/`yield
/// from` outside a function -- D-149). CPython itself raises `SyntaxError`
/// for all of these, the same failure class `L0001` already covers for the
/// parser's own grammar violations; unlike a genuine parser `L0001`, one
/// emitted from here carries no "expected set" in its message.
fn context_invalid<R>(message: impl Into<String>, range: R) -> Diagnostic
where
    std::ops::Range<u32>: From<R>,
{
    let range = std::ops::Range::<u32>::from(range);
    Diagnostic::error("L0001", message, Span::new(range.start, range.end))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod pycc_parser_test_helper {
    pub fn parse(source: &str) -> pycc_ast::ModModule {
        pycc_parser::parse(source).expect("test fixture must parse")
    }
}
