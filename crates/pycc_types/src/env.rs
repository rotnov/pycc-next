//! The type checker's scope model: [`BindingState`] and [`Environment`].
//!
//! Every pass in this crate threads an [`Environment`] -- the flat,
//! name-keyed table of value bindings, declared annotations, function
//! signatures, generic definitions, and class definitions that one scope
//! can see -- and reads a name's [`BindingState`] to distinguish a name
//! that is *definitely* assigned from one that is only *maybe* assigned
//! on some path (the T0021/T0041 distinction).
//!
//! It was extracted from [`lib.rs`](crate) per the repository's
//! source-file decomposition rule (AGENTS.md "Keep source files
//! decomposable", tracked by issue #544), as a pure relocation: no
//! scope-model behavior changed with the move. The passes that *use* an
//! environment -- expression inference, statement checking, function
//! checking, the constraint solver -- all stay where they were, so this
//! module is the environment seam only.

use crate::is_assignable;
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{HirClassDef, HirItem, Ty};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Issue #118 Part 1: tracks whether a name is *definitely* assigned on
/// every path reaching this point, or only *maybe* assigned (bound on some
/// paths but not all). A `Definitely` binding is readable; a `Maybe` binding
/// is not -- reading it raises `T0041` (possibly-unbound read), matching
/// CPython's own `NameError`/`UnboundLocalError` for the same control-flow
/// shapes. The join lattice is: `Definitely` > `Maybe` > unbound. An `if`
/// with both branches binding the same name compatibly joins to `Definitely`;
/// one branch only (or a no-else `if`) joins to `Maybe`. A `while`/`for` body
/// may execute zero times, so every body-only binding joins back as `Maybe`,
/// and a `for` loop's own target variable is `Maybe` after the loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BindingState {
    Definitely(Ty),
    Maybe(Ty),
}

impl BindingState {
    /// Returns the inner `Ty` regardless of whether the binding is
    /// `Definitely` or `Maybe` assigned.
    pub(crate) fn ty(&self) -> &Ty {
        match self {
            BindingState::Definitely(t) | BindingState::Maybe(t) => t,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Environment {
    pub(crate) bindings: HashMap<String, BindingState>,
    /// Names declared by a value-less `AnnAssign` (`x: int` with no `= ...`)
    /// that have not yet been definitely assigned (issue #245). Distinct
    /// from `bindings`: an entry here records a known static type with *no*
    /// runtime value, matching CPython's own "declared, not yet assigned"
    /// semantics -- a read of such a name must still raise the existing
    /// `T0021` unbound-local diagnostic, which it does, because `lookup`
    /// only ever consults `bindings`. An entry is consumed (removed) the
    /// moment a real assignment establishes a definite binding; see
    /// `check_assignment`'s declared-consult branch.
    pub(crate) declared: HashMap<String, Ty>,
    pub(crate) functions: Arc<HashMap<String, (Vec<Ty>, Ty)>>,
    /// Names whose *net* source-order top-level binding is currently a
    /// `def` (D-110). Tracked separately from `bindings` on purpose: the
    /// representation record in `bindings` must survive a `def` so that
    /// D-040's sticky-representation rule (`check_assignment`) still rejects
    /// a later incompatible value reassignment -- PR #252's round-4 review
    /// caught a version that cleared `bindings` at a `def` and thereby let
    /// `helper = 1; def helper(): ...; helper = "leaked"` reach codegen with
    /// an `int`-allocated slot stored as `str`.
    pub(crate) def_rebound: HashSet<String>,
    /// Part 1 of #883 (#962): every `(alias, module)` pair the module's
    /// import table binds (`import math as m` -> `("m", Math)`), from
    /// `std_receiver::bind_std_module_aliases`. Read only by the stdlib
    /// receiver shadow check (`std_receiver::shadowed_std_receiver`) so
    /// that a local named after an alias rejects `m.sqrt(m)` exactly as a
    /// local named `math` rejects `math.sqrt(math)`. Filled once, at
    /// `module::check_with_environment_all`'s entry; legitimately empty
    /// for the test-only entry points `check_function` /
    /// `check_generic_function` and the post-check `Environment::new()` in
    /// `monomorphize`, which have no module context at all.
    pub(crate) std_module_aliases: Vec<(String, pycc_std::StdModule)>,
    /// Issue #22: function names whose `def` has been encountered so far in
    /// top-level source order. In top-level code, a call to a function not
    /// yet in this set is a static error (matching CPython's `NameError` for
    /// call-before-`def`). Function bodies see all module functions
    /// regardless of order (Python's late binding: a function body is
    /// evaluated at call time, by which point all module-level `def`s have
    /// typically executed), so `child_for_function` seeds this set with
    /// every function name.
    pub(crate) defined_functions: HashSet<String>,
    /// PR-13 Task 3 (D-133/D-134): the subset of `functions` whose signature
    /// contains a `Ty::Param` (a PEP 695 generic function), keyed by its
    /// *original* (un-mangled) name and carrying the full `HirItem` its
    /// body was lowered to. `functions` alone only has room for a resolved
    /// `(Vec<Ty>, Ty)` signature, which cannot express "resolve this call
    /// site by substituting `T`, don't just check assignability against
    /// `Ty::Param` structurally" -- `infer_expr_in`'s `HirExpr::Call` arm
    /// consults this map first, before falling through to the ordinary
    /// `functions` lookup, so a call to a generic function is dispatched to
    /// `instantiate_generic_call` instead of being rejected as an ordinary
    /// argument-type mismatch against an uninstantiated `Ty::Param`.
    pub(crate) generics: Arc<HashMap<String, HirItem>>,
    /// D-154 (Part 1 of #375): class name -> declared shape, mirroring
    /// `generics`'s own `Arc`-wrapped shape exactly (`Environment` is
    /// `Clone`d per function via `child_for_function`, so this needs the
    /// same cheap-clone property `functions`/`generics` already rely on).
    /// Populated once, from `HirModule::class_defs`, by every `Environment`
    /// constructor this crate has (`check_with_signatures_all`,
    /// `concrete_function_environment`) -- see `class::bind_classes`.
    pub(crate) classes: Arc<HashMap<String, HirClassDef>>,
    /// Part 1 of #541 (extending D-173): the subset of `classes` whose
    /// entries were *synthesized* by HIR lowering for the seven builtin
    /// exception names rather than written by the user
    /// (`pycc_hir::builtin_exception_class_defs`). Kept as a side table
    /// instead of a `HirClassDef` flag on purpose: "who authored this
    /// definition" is a fact about the environment's provenance, not part
    /// of a class's declared shape, and a flag would have to be threaded
    /// through every `HirClassDef` literal in the tree.
    ///
    /// Needed because `is_unshadowed_builtin_exception` used to read
    /// "`ValueError` is absent from `classes`" as "the user has not
    /// shadowed `ValueError`". Once the frontend seeds the builtin classes
    /// that inference inverts: every builtin exception would look shadowed
    /// and every `except ValueError:`/`raise ValueError("x")` would be
    /// rejected. Membership here restores the intended meaning -- present
    /// *and* not synthetic is what "shadowed" means.
    ///
    /// Maintained by [`Self::bind_class`] and [`Self::bind_synthetic_class`],
    /// together the sole mutators of `classes`, so the two tables cannot
    /// drift apart regardless of which environment constructor or
    /// monomorphization path registered a class.
    pub(crate) synthetic_classes: Arc<HashSet<String>>,
    /// PEP 695 (#387): the name of the generic *function* currently being
    /// body-checked, if any. Set by `check_function_in` from the function's
    /// own signature via `generic_type_param_name`; `None` for a non-generic
    /// function (and for the module-level environment). Used by
    /// `check_stmt_in_function`'s `Return` arm to reject a generic function
    /// returning its *own* `Ty::Param` as a concrete scalar
    /// (`def bad[T](x: T) -> int: return x`): `is_assignable`'s
    /// `from == Ty::Param` clause must remain (a non-generic function reading
    /// a generic class instance's attribute still needs `T` → `int` to pass
    /// during type checking, before monomorphization substitutes `T`), so the
    /// function-owned-vs-class-owned distinction is enforced here instead.
    /// `Option<String>` (not `&str`) because `Environment` is owned/cloned
    /// per function via `child_for_function`/`clone`.
    pub(crate) own_type_param: Option<String>,
    /// #433: the name of the class whose method body is currently being
    /// type-checked, if any. Set by `check_function_in` from the method's
    /// own mangled `<ClassName>.<method>` name (the `.` separator is unique
    /// to mangled method names — a real Python identifier can never contain
    /// one, so the prefix before the first `.` is unambiguously the class
    /// name). `None` for a top-level function and for the module-level
    /// environment. Used by `infer_expr_in`'s `HirExpr::Super` arm and by
    /// `resolve_method_call`/`resolve_attr_get` when the base is `Super` to
    /// resolve the next class in the MRO after this one (D-006 static
    /// dispatch, per the #433 ADR — no vtable, no runtime dispatch).
    ///
    /// For a receiver-exact copy of an inherited body (#1337, D-254) this is
    /// the copy's *anchor* -- the class that defines the source body, read
    /// through `pycc_hir::inherited_copy_origin` -- never the receiver the
    /// copy is compiled for; the `super()` arms read the receiver from
    /// `self`'s type and continue along the receiver's MRO after the anchor.
    pub(crate) current_class: Option<String>,
    /// PEP 591 (#383): names declared `Final` (variable-level only, not
    /// parameters or class attributes). Populated from `HirStmt::AnnAssign`'s
    /// `is_final` flag in `check_stmt`/`check_stmt_in_function`'s `AnnAssign`
    /// arms, *after* the initial `check_assignment` call so the initial
    /// binding is not rejected. `check_assignment` consults this set before
    /// its `lookup_any`/`declared` branches: if the target is in `finals`
    /// and already has a runtime binding in `bindings`, this is a
    /// reassignment and is rejected with `T0045`. Mirrors `declared`'s
    /// `child_for_function` clearing behavior — a function-local `Final`
    /// declaration for a name shadowing a module-level `Final` does not
    /// inherit the module-level constraint.
    pub(crate) finals: HashSet<String>,
    /// #382 (PR-22 Part 1): `true` when the statement being checked is
    /// inside an `except` handler body. Used to validate bare `raise`
    /// (re-raise) — only valid inside an except handler. Set to `true`
    /// before checking a handler body, reset to the previous value after.
    pub(crate) in_except_handler: bool,
    /// Issue #769 (Part 2 of #747): flow-sensitive `Optional[T]` narrowing
    /// overlay -- maps a name currently narrowed to `T` (from `T | None`)
    /// in the code being checked *right now* to that inner `T`. Deliberately
    /// a separate map from `bindings`, never merged into it and never
    /// consulted by `join_if_branches` -- see `crate::narrow`'s module doc
    /// comment for why an overlay, not a mutation of the branch-local
    /// clone's own `bindings` entry, is the only leak-safe design. Consulted
    /// by `infer_expr_in`'s `HirExpr::Name` arm (a *read* of a narrowed name
    /// gets the inner type); never consulted by `check_assignment` (an
    /// assignment *target* is still checked against the name's real,
    /// un-narrowed type) -- `check_assignment` clears an entry here for its
    /// own target instead, killing the narrowing the moment the name is
    /// reassigned anywhere inside a narrowed region.
    pub(crate) narrowed: HashMap<String, Ty>,
    /// #974: `true` in an environment produced by [`Self::child_for_function`],
    /// i.e. while a *function body* is being checked rather than a top-level
    /// statement.
    ///
    /// `bindings` means two different things in those two positions.
    /// `check_and_resolve_all_keyed`'s pass 2 walks the module's top-level
    /// statements sequentially, so during that pass a `binding_state` answer
    /// is the binding active *at that point in the source*. Pass 3 then
    /// checks every function body against the environment as it stands after
    /// **all** top-level code (D-041 late binding), so the same answer is
    /// ordering-blind there: a module-level rebinding of a class's own name
    /// looks identical whether it executes before or after the read. The
    /// class-name dispatch guard (`crate::expr::class_name_dispatch`) needs
    /// to tell those apart, and this flag is what tells it which environment
    /// it is looking at.
    pub(crate) in_function_body: bool,
    /// Part 2b of #1142 (#1164), review round 5: whether the function body
    /// being checked contains a `return` lexically inside any `finally`
    /// clause, at any nesting depth.
    ///
    /// Set once per function from `pycc_hir::body_returns_inside_finally`
    /// (see that predicate for why the shape is refused and why the
    /// pre-existing PEP 765 `L0001` rule cannot answer this question), and
    /// read only at the buffer-egress admission in
    /// `crate::check_stmt_in_function`'s `HirStmt::Return` arm. `false` in
    /// the module-level environment, which has no function body to walk.
    ///
    /// A whole-function constant rather than a per-statement context, so the
    /// clones this struct takes across branch joins carry it unchanged and
    /// no join rule is owed for it.
    pub(crate) returns_inside_finally: bool,
    /// #1344 (Part 2 of #1336): the function's own HIR return type is
    /// `Ty::Infer` -- an unannotated private helper whose return the solver
    /// inferred.
    ///
    /// Set once per function in `crate::check_function_in`, and read only in
    /// `crate::check_stmt_in_function`'s `HirStmt::Return` arm, where
    /// `crate::comprehension::inferred_set_return_limit` relabels the false
    /// `T0022` an inferred `set[int]` return meets against a `set[C]` value
    /// as an honest `C0001`. `false` in the module-level environment, which
    /// has no return. A whole-function constant, like
    /// `returns_inside_finally`, so no join rule is owed for it.
    pub(crate) return_inferred: bool,
    /// #1435: the function being checked is a `@classmethod` (its mangled
    /// name ends in `.classmethod`). Its `cls` parameter is typed as an
    /// instance of the class but carries no instance -- `pycc_mir` passes a
    /// null `NullInstance` for it -- so `crate::foreign::check_object_call_args`
    /// refuses `cls` as an argument to a CPython object's call rather than
    /// let the carrier packer meet a null pointer.
    ///
    /// Set once per function in `crate::check_function_in`. A
    /// whole-function constant, like `returns_inside_finally`, so no join
    /// rule is owed for it; `false` in the module-level environment.
    pub(crate) in_classmethod: bool,
    /// Part 2a of #1142 (#1165): the names whose `Ty::MemoryView` binding is
    /// storage **this artifact allocated** (`a = ndarray(n)`), as opposed to
    /// a buffer parameter the `pycc build --ext` wrapper borrowed from the
    /// host for one call.
    ///
    /// A per-name fact rather than a second `Ty` variant, for the reason
    /// `BindingState` itself is one: the distinction is about where a *name*
    /// got its value, not about the type. A `Ty::OwnedBuffer` would ripple
    /// through every exhaustive `match` on `Ty` in the workspace, through
    /// `boundary_carrier`, `render_ty`, `annotation_to_ty`, `ty_to_basic_type`
    /// and the wrapper tables, and would make the annotation spelling
    /// ambiguous -- all to express something none of those sites asks.
    ///
    /// Flat and flow-insensitive on purpose: `crate::buffer`'s
    /// `buffer_parameter_rebinding` refuses the one shape that would make a
    /// name change provenance mid-function, so a set is enough and no
    /// per-program-point map is needed. Merged as a union at every
    /// control-flow join, matching `bindings`' own `Maybe` widening: a name
    /// owned on either path is owned for the epilogue's purposes, and the
    /// runtime free is a documented no-op on the null a never-taken path
    /// leaves in the slot.
    pub(crate) owned_buffers: HashSet<String>,
    /// #1316: the local names of every foreign import in the module, at any
    /// site (`HirModule::imports`, top-level and block imports alike),
    /// minus the names a function body declares locally
    /// ([`Self::child_for_function`] removes them).
    ///
    /// Since Part 1 of #1333 `expr.rs` no longer gates a function-body read
    /// on this set: every definitely-bound `Ty::Object` name is readable
    /// there. The one remaining consumer is `class/foreign_static.rs`'s
    /// `check_use_site`, whose shadowing test asks whether a class body's
    /// receiver root is a module-level foreign import no local shadows.
    ///
    /// Position-blind on purpose, like pass 3 itself (D-041): a member
    /// counts only while `bindings` also holds it `Definitely` as
    /// `Ty::Object`, so a maybe-bound import still fails `T0041` and a read
    /// that runs before the import at run time raises `NameError` from
    /// codegen's unbound-global branch (`pycc_codegen`'s `foreign_fail.rs`).
    pub(crate) foreign_globals: HashSet<String>,
}

impl Environment {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn lookup(&self, name: &str) -> Option<Ty> {
        match self.bindings.get(name) {
            Some(BindingState::Definitely(ty)) => Some(ty.clone()),
            // A `Maybe` binding is not readable -- `lookup` returns `None` so
            // the caller's existing unbound-local / not-defined logic fires,
            // and `infer_expr_in`'s `Name` arm can distinguish it from a
            // truly-unbound name via `binding_state` (issue #118 Part 1).
            _ => None,
        }
    }

    /// Returns the type of `name` regardless of whether it is `Definitely` or
    /// `Maybe` bound (issue #118 Part 1). Used by `check_assignment`'s
    /// first-assignment-wins rule: a maybe-bound name being reassigned on the
    /// current path becomes definite, but the representation (type) from the
    /// maybe-binding must be retained.
    pub fn lookup_any(&self, name: &str) -> Option<Ty> {
        self.bindings.get(name).map(|state| match state {
            BindingState::Definitely(ty) | BindingState::Maybe(ty) => ty.clone(),
        })
    }

    /// Returns the `BindingState` of `name` (issue #118 Part 1). Used by
    /// `infer_expr_in`'s `Name` arm and `lookup_bound_name` to distinguish
    /// three states: `Some(Definitely(ty))` -> readable, `Some(Maybe(ty))` ->
    /// `T0041` possibly-unbound read, `None` -> `T0021` unbound.
    pub(crate) fn binding_state(&self, name: &str) -> Option<&BindingState> {
        self.bindings.get(name)
    }

    /// Records `name` as declared with `ty` by a value-less `AnnAssign`
    /// (issue #245), without binding it -- a subsequent read still raises
    /// `T0021` via `lookup`, which never consults this map. First
    /// declaration wins: a second, still-unbound declaration for the same
    /// name is validated with `is_assignable(ty, existing)` (same direction
    /// `check_assignment` uses for an ordinary reassignment) and rejected on
    /// mismatch, but the earlier entry is what stays on success -- see
    /// `check_assignment`'s comment for the worked `bool`/`int` example.
    pub fn declare(&mut self, name: String, ty: Ty) -> Result<(), Diagnostic> {
        // Already definitely assigned (e.g. `x = 1; x: int`): unchanged
        // pre-existing behavior, out of scope for issue #245 -- leave
        // `bindings` as the sole authority and never shadow it with a
        // `declared` entry that `check_assignment` would never consult
        // anyway (its declared-consult branch only fires when `lookup`
        // already returned `None`).
        if self.bindings.contains_key(&name) {
            return Ok(());
        }
        if let Some(existing) = self.declared.get(&name) {
            if !is_assignable(ty.clone(), existing.clone()) {
                return Err(Diagnostic::error(
                    "T0026",
                    format!(
                        "cannot declare `{name}: {}`, previously declared as `{name}: {}`",
                        ty.name(),
                        existing.name()
                    ),
                    Span::new(0, 0),
                ));
            }
            return Ok(());
        }
        self.declared.insert(name, ty);
        Ok(())
    }

    pub(crate) fn declared_ty(&self, name: &str) -> Option<Ty> {
        self.declared.get(name).cloned()
    }

    pub fn bind(&mut self, name: String, ty: Ty) {
        // A value assignment makes the name's net binding a value again,
        // whatever `def`s came before it (D-110).
        self.def_rebound.remove(name.as_str());
        self.bindings.insert(name, BindingState::Definitely(ty));
    }

    /// Records `name` as *maybe* bound to `ty` (issue #118 Part 1) -- the name
    /// was assigned on some but not all paths reaching this point. A
    /// subsequent definite assignment on the current path upgrades it to
    /// `Definitely` (via `bind`); a read before that raises `T0041`.
    pub fn bind_maybe(&mut self, name: String, ty: Ty) {
        self.bindings.insert(name, BindingState::Maybe(ty));
    }

    pub fn bind_function(&mut self, name: String, param_tys: Vec<Ty>, return_ty: Ty) {
        Arc::make_mut(&mut self.functions).insert(name.clone(), (param_tys, return_ty));
        // Issue #22: by default, binding a function also marks it as
        // defined. This makes standalone `infer_expr` / `check_function`
        // calls work without callers needing to separately track
        // `defined_functions`. `check_with_environment_all` clears this set
        // before its top-level source-order pass so call-before-`def` is
        // caught there.
        self.defined_functions.insert(name);
    }

    pub fn lookup_function(&self, name: &str) -> Option<&(Vec<Ty>, Ty)> {
        self.functions.get(name)
    }

    /// Registers `name` as a PEP 695 generic function whose original,
    /// un-substituted body is `item` (D-133/D-134). Call sites resolve
    /// through [`Self::lookup_generic`], not `lookup_function`, since a
    /// generic function's call-site behavior (substitute `T`, then
    /// type-check) cannot be expressed by a plain `(Vec<Ty>, Ty)` signature.
    pub fn bind_generic(&mut self, name: String, item: HirItem) {
        Arc::make_mut(&mut self.generics).insert(name, item);
    }

    /// Looks up `name`'s original generic-function `HirItem`, if `name` was
    /// registered via [`Self::bind_generic`].
    pub fn lookup_generic(&self, name: &str) -> Option<&HirItem> {
        self.generics.get(name)
    }

    /// Registers `name` as a declared class with shape `def` (D-154, Part 1
    /// of #375), mirroring [`Self::bind_generic`]'s own shape exactly.
    /// Part 1 of #541: registers `name` as *user-authored*, clearing any
    /// earlier synthetic marking for that name and keeping
    /// `synthetic_classes` in step with `classes`. Every caller that is not
    /// replaying HIR lowering's own seeding belongs here; the one caller
    /// that is uses [`Self::bind_synthetic_class`] instead.
    pub fn bind_class(&mut self, name: String, def: HirClassDef) {
        Arc::make_mut(&mut self.synthetic_classes).remove(&name);
        Arc::make_mut(&mut self.classes).insert(name, def);
    }

    /// Part 1 of #541: registers `name` as a class *this compiler's own HIR
    /// lowering synthesized* (D-188), recording it in `synthetic_classes`.
    ///
    /// Only `class::bind_classes` calls this, and only for a name it has
    /// established was seeded by `lower_checked`. The visibility is
    /// `pub(crate)` so that restriction is enforced by the compiler rather
    /// than by convention: marking an arbitrary name synthetic would
    /// silently exclude a user-authored class from `is_user_defined_class`,
    /// which is exactly the defect D-188's provenance record exists to
    /// prevent -- provenance is carried
    /// from the lowering step through `HirModule`, never re-derived from a
    /// definition's shape, because a user-authored class can be
    /// structurally identical to a synthetic one.
    pub(crate) fn bind_synthetic_class(&mut self, name: String, def: HirClassDef) {
        Arc::make_mut(&mut self.synthetic_classes).insert(name.clone());
        Arc::make_mut(&mut self.classes).insert(name, def);
    }

    /// Part 1 of #541: whether `name`'s registered class shape was
    /// synthesized by HIR lowering for a builtin exception class rather
    /// than written by the user. `false` for an unregistered name and for
    /// every user-authored class, including one that shadows a builtin
    /// exception name.
    pub(crate) fn is_synthetic_class(&self, name: &str) -> bool {
        self.synthetic_classes.contains(name)
    }

    /// Looks up `name`'s declared class shape, if `name` was registered via
    /// [`Self::bind_class`].
    pub fn lookup_class(&self, name: &str) -> Option<&HirClassDef> {
        self.classes.get(name)
    }

    /// #433: Returns the name of the class whose method body is currently
    /// being type-checked, if any. Set by `check_function_in` from the
    /// method's mangled `<ClassName>.<method>` name (the anchor class for a
    /// receiver-exact copy, #1337); `None` for a top-level function or the
    /// module-level environment.
    pub(crate) fn current_class(&self) -> Option<&str> {
        self.current_class.as_deref()
    }

    /// Issue #769 (Part 2 of #747): the inner type `name` is currently
    /// narrowed to, if any. Returns `None` outside a narrowed region -- the
    /// overlay is populated only on branch-local clones (in-branch
    /// narrowing) and on the shared env directly for the early-return
    /// continuation shape (see `crate::narrow`), and is never merged back
    /// into `bindings`, so this is `None` again as soon as the narrowed
    /// region ends.
    pub(crate) fn narrowed_ty(&self, name: &str) -> Option<Ty> {
        self.narrowed.get(name).cloned()
    }

    pub(crate) fn child_for_function(&self, local_names: &[&str]) -> Self {
        let mut child = self.clone();
        // #974: everything below this point is a function body, where a
        // `binding_state` answer is the *final* module environment rather
        // than a point-in-time one -- see the field's own doc comment.
        child.in_function_body = true;
        for name in local_names {
            child.bindings.remove(*name);
            child.declared.remove(*name);
            child.finals.remove(*name);
            child.narrowed.remove(*name);
            // Part 2a of #1142 (#1165): a function-local name shadows any
            // module-level one, so it starts with no provenance of its own.
            // Module scope cannot actually own a buffer (`crate::buffer`'s
            // `producer_at_module_scope` refuses production there), so this
            // is hygiene that keeps the set's meaning local rather than a
            // hole being closed.
            child.owned_buffers.remove(*name);
            // #1316: a local of the same name shadows a foreign global.
            child.foreign_globals.remove(*name);
        }
        // Issue #22: a function body may call any module-level function
        // regardless of source order -- Python's late binding evaluates a
        // function body at call time, by which point all module-level
        // `def`s have typically executed. Seed `defined_functions` with
        // every known function name so the call-before-`def` check in
        // `infer_expr_in`'s `HirExpr::Call` arm never fires inside a
        // function body.
        child
            .defined_functions
            .extend(child.functions.keys().cloned());
        child
    }
}
