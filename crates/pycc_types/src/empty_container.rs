//! Empty-container element-type resolution (#1021, D-245).
//!
//! An empty `[]` or `{}` carries no element type of its own, and this
//! compiler has no bidirectional/expected-type inference to hand one down
//! from the surrounding context. Before this pass existed, *every* empty
//! container literal was a hard error -- including the annotated
//! `xs: list[int] = []` form (#927).
//!
//! This module is the single place that *infers* one. (The one other
//! construction site is syntactic: `pycc_hir`'s annotated attribute target
//! `self.xs: list[int] = []` builds the typed node straight from its written
//! annotation, #1264 and D-245's 2026-09-24 amendment; such a node is already
//! typed and this pass leaves it alone.) It is an infallible
//! HIR-to-HIR rewrite that runs **once, at the top of both
//! [`crate::check_all_keyed`] and [`crate::check_and_resolve_all_keyed`]**,
//! replacing a resolvable `HirExpr::ListLiteral(vec![])` with
//! [`HirExpr::EmptyList`] and a resolvable `HirExpr::DictLiteral(vec![])`
//! with [`HirExpr::EmptyDict`]. Running *before* checking, rather than as a
//! post-check resolve phase, is what makes the pass's key safety property
//! structural instead of merely asserted: `pycc check` and `pycc build`
//! consume the identical rewritten module, so `pycc check` cannot accept a
//! program that `pycc build` then panics on in `MirExpr::ty()`. See
//! `docs/decisions/D-245-*.md` for the alternatives this rejected.
//!
//! The element type comes from exactly three sources, in priority order:
//!
//! 1. the annotation on an `AnnAssign` target (`xs: list[int] = []`) --
//!    purely syntactic, needing no environment at all;
//! 2. *any* successfully-inferred binding for the target anywhere in the
//!    enclosing function -- not only one that precedes the literal;
//! 3. a forward scan of the enclosing function body for the first
//!    *producer* use of the target -- `xs.append(v)` or `d[k] = v`, in
//!    statement position only (see [`find_producer`]). The scan descends into
//!    every block statement the rewrite walk itself descends into, so the two
//!    halves of the pass never disagree about which statements exist.
//!
//! Sources 2 and 3 are best-effort: they use the same
//! `bind_local_types_in_body` pre-pass environment the protocol
//! monomorphization pass already builds, and they swallow inference
//! failures exactly as that function does. A container this pass cannot
//! resolve is left as the empty literal it was, and the check phase then
//! reports `T0003`.
//!
//! **Source 2 is order-insensitive, not a backward scan.** The whole-function
//! environment is built by a single forward `bind_local_types_in_body` pass
//! that completes *before* any rewriting, and is then reused for every
//! occurrence regardless of position, so `xs = [1]; ...; xs = []` and
//! `xs = []; xs = [1]; ...` resolve identically. That is safe because this
//! pass resolves but never accepts: `pycc_hir::check_container_ty` admits
//! only `list[int]` and `dict[str, int]`, so a resolution is either the one
//! element type the program could have compiled with or a `T0034`/`T0036`,
//! and the check phase re-validates the function in true program order
//! (D-040's sticky-representation rule included). Restricting source 2 to
//! strictly-prior bindings could only turn some compilable programs into
//! `T0003`; it could not change which accepted program is produced. The
//! "only a fully concrete `Ty` is ever stored" invariant is enforced here, by
//! [`resolve`] discarding any resolution containing `Ty::Infer` -- the
//! private-helper solver's placeholder, which this pass can observe because
//! it runs before that solver and which nothing downstream substitutes into a
//! rewritten node.
//!
//! The same completed-before-rewriting property is why
//! `bind_local_types_in_stmt`'s `AnnAssign` arm seeds the declared annotation
//! when the value does not infer: `xs: list[int] = []`'s raw literal is not
//! yet rewritten when the environment is built, so without that fallback the
//! environment never learned `xs: list[int]` and a resolution derived from
//! `xs` (`for x in xs: ys.append(x)`) reported a spurious `T0003`.
//!
//! **Producers, not consumers.** `for x in xs`, `xs[0]`, `len(xs)` and
//! `xs.pop()` *read* an element type that is already known; in a single
//! forward synthesis pass with no backward unification they cannot supply
//! one. Only `HirExpr::ListAppend` and `HirStmt::DictSet` can. `HirExpr::
//! SetAdd` is structurally dead as a producer -- `{}` parses as a dict and
//! `set()` is rejected at `pycc_hir` lowering -- so there is deliberately no
//! set path here at all.
//!
//! **Scope.** Function bodies only (class methods included: they lower to
//! ordinary mangled `HirItem::Function` items). Module-level statements keep
//! failing, now with `T0003`.
//!
//! **Object slots (Part 2d of #1371).** A list display bound to a CPython
//! object -- `x: object = [1, "a"]`, or `s = []` where `s` is bound to an
//! object elsewhere -- is not a native `list[T]` at all, and is rewritten
//! into [`HirExpr::ObjectList`] ahead of the three sources above;
//! [`object_slot`] owns which displays qualify and why. Since #1421 an
//! object annotation also reaches every value position of the assigned
//! expression (either operand of a value-producing `and`/`or`, either
//! branch of a conditional expression), and the class phase below rewrites
//! a display stored into an object attribute slot the same way.
//!
//! **Instance attributes (#1265).** An attribute store is not a function
//! local, so the three sources above never type one. `pycc_hir` lowers an
//! unannotated `self.xs = []` at the top level of `__init__` to a
//! *provisional* `list[<Ty::Infer>]` slot, and a class phase ([`attr_slot`])
//! runs ahead of the per-function phase: it resolves each provisional slot
//! from an inherited concrete slot or from the first `self.xs.append(v)` in
//! the class's own methods, then types every `self.<attr> = []`/`{}` in those
//! methods from the class's slot layout; a store into a slot whose layout
//! type is the object has its value-position list displays rewritten into
//! [`HirExpr::ObjectList`] instead (#1421). A provisional slot it cannot
//! resolve is refused with `T0003` by [`attr_slot::reject_unresolved_attr_slots`].
//! An unannotated `self.d = {}` establishing the attribute in `__init__`
//! stays `C0001` in `pycc_hir` (a later reset in a method is typed from the
//! slot like `[]`, or left for `T0003` on a shape mismatch): its producer,
//! `self.d[k] = v`, is not lowerable yet (#891). A class-body declaration
//! (`d: dict[str, int]`, #1266) gives the slot its concrete type in
//! `pycc_hir` instead, so the establishing literal is typed like a reset,
//! and a wrong-shape one is left for `T0003`. D-245's 2026-09-24 amendments
//! for #1265 and #1266 are the canonical statement.
//!
//! Within a function body the pass walks *every* block statement this HIR has
//! -- `if`/`else`, `while`, both `for` forms, every `match` case body, and
//! every `try`/`except`/`except*`/`else`/`finally` suite (see
//! [`nested_bodies`]) -- so source 1, which is purely syntactic and reads no
//! environment at all, works identically at any nesting depth.
//!
//! Sources 2 and 3 are the ones that do not, and they share one cause rather
//! than differing: both read the flat whole-function `Environment` built by
//! `bind_local_types_in_body`, a pre-existing pass shared with protocol
//! monomorphization whose own statement walk has no `match` arm. Source
//! 2 reads a binding out of that environment directly; source 3 infers the
//! producer's value *in* it ([`find_producer`] calls `infer_expr_in` with
//! exactly that environment). So a name bound only inside a `match` case is
//! invisible to both, and `case y: xs = []; xs.append(y)` is not resolved
//! even though `case y: xs: list[int] = []; xs.append(y)` is. That costs a
//! missed resolution, never a wrong element type. The binder walks every
//! `try`/`except`/`except*`/`else`/`finally` suite since #1445 (lark's
//! `action, arg = ...` inside a `try`, whose `arg` the object slice bound of
//! `s = value_stack[-size:]` derives from), so a name bound in one of those
//! is visible like an `if` branch's, demoted and re-promoted by the same
//! rules below.
//!
//! What a miss then *reports* is not always `T0003`. A miss leaves the
//! concrete path failing, which routes the module into the private-helper
//! constraint solver, whose own `HirStmt::Match` arm never binds a pattern's
//! capture names into the case environment -- so the capture is reported as
//! `T0021` with the "local name is not bound before this use" message,
//! and D-220's
//! solver-first merge lets that win over the true `T0003`. That solver
//! behavior predates this pass (`origin/main` emits the identical diagnostic
//! for the same program, and for a `match` program containing no empty
//! container at all) and is untouched here; it is tracked as issue #1046.

use super::*;

pub(crate) mod attr_slot;
mod object_slot;
mod producer;

use producer::find_producer;

/// What a resolution produced: the *element* type of a list, or the
/// key/value pair of a dict.
enum Resolution {
    List(Ty),
    Dict(Ty, Ty),
}

impl Resolution {
    /// The rewritten node this resolution produces.
    fn into_expr(self) -> HirExpr {
        match self {
            Resolution::List(element) => HirExpr::EmptyList(element),
            Resolution::Dict(key, value) => HirExpr::EmptyDict(Box::new((key, value))),
        }
    }

    /// Whether this resolution's shape matches the literal being rewritten.
    /// An `xs: list[int] = {}` annotation resolves to a list while the
    /// literal is a dict. Rewriting `{}` into a typed empty *list* would
    /// silently repair a program the user got wrong, so the literal is left
    /// alone and the check phase reports `T0003` against it.
    fn matches(&self, literal: EmptyLiteral) -> bool {
        matches!(
            (self, literal),
            (Resolution::List(_), EmptyLiteral::List) | (Resolution::Dict(..), EmptyLiteral::Dict)
        )
    }
}

/// Which empty literal shape a node is, when it is one at all.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EmptyLiteral {
    List,
    Dict,
}

fn empty_literal(expr: &HirExpr) -> Option<EmptyLiteral> {
    match expr {
        HirExpr::ListLiteral(elements) if elements.is_empty() => Some(EmptyLiteral::List),
        HirExpr::DictLiteral(pairs) if pairs.is_empty() => Some(EmptyLiteral::Dict),
        _ => None,
    }
}

/// Rewrites every resolvable empty container in `hir`, and every list
/// display bound to an object slot (Part 2d of #1371, #1421), returning
/// `None` when the module contains no empty container literal and no
/// object-annotated display in a function body at all, no provisional
/// attribute slot (#1265) and no display stored into an attribute while
/// some class has an object slot -- which is the overwhelmingly common
/// case, and which keeps this pass from adding a whole-module clone to
/// `check_all_keyed`, a path that does not otherwise clone.
pub(crate) fn resolve_empty_containers(hir: &HirModule) -> Option<HirModule> {
    let class_phase = attr_slot::needs_class_phase(hir);
    if !class_phase
        && !hir.items.iter().any(|item| match item {
            HirItem::Function { body, .. } => body_has_empty_literal(body),
            HirItem::TopLevelStmt(_) => false,
        })
    {
        return None;
    }
    let local_names = crate::module::module_function_local_names(hir);
    let mut resolved = hir.clone();
    // #1265: the class phase runs first, so the per-function phase below
    // builds its module scope from the class-resolved module -- a local
    // producer reading `self.xs` then sees the resolved `list[int]` rather
    // than the provisional `list[<inferred>]`, which `concrete` would discard.
    if class_phase {
        attr_slot::resolve_class_slots(hir, &mut resolved, &local_names);
    }
    let module_env = module_environment(&resolved);
    for (index, item) in resolved.items.iter_mut().enumerate() {
        let HirItem::Function {
            name, params, body, ..
        } = item
        else {
            continue;
        };
        let names = &local_names[index];
        let env = function_environment(&module_env, name, params, body, names);
        let producers = body.clone();
        rewrite_body(body, &producers, &env, names);
    }
    Some(resolved)
}

/// The module scope every producer inference in `hir` reads: the annotated
/// function table, the std-module aliases, the foreign names, and the
/// module-level bindings in source order.
fn module_environment(hir: &HirModule) -> Environment {
    // #1021 review round 5: build the module scope from the *annotated*
    // functions rather than through `concrete_function_environment`, which
    // refuses the whole module the moment any one signature still carries
    // `Ty::Infer` -- true for essentially every program this pass exists to
    // serve, since an unannotated private helper is exactly what the D-146
    // solver is for. Its `unwrap_or_default()` therefore handed this pass an
    // environment with no function table and no classes, so a module-level
    // global initialized from an annotated helper (`VALUE = _base()`) failed
    // to resolve a container that the equivalent non-empty literal
    // (`xs = [VALUE]`) resolves fine.
    //
    // #1021 review round 13 refuted this call's original rationale, which
    // read that a *partial* table is safe because an annotated signature is
    // authoritative. `annotated_function_environment` does not build a
    // partial table: it registers every signature, `Ty::Infer` included, and
    // must, because its own `bind_classes` call records every class member
    // unconditionally and the class resolvers panic on a member with no
    // ordinary-function registration. That function's `# Registry invariant`
    // section is the canonical statement of what it guarantees and why
    // dropping an entry would be unsound rather than merely lossy; this call
    // site relies on it rather than restating it. What holds here is the
    // narrower property the pass actually needs: an `Ty::Infer` signature
    // yields `Ty::Infer` at the call, which `resolve`'s `concrete` gate
    // discards, so such a call costs a missed resolution and never a wrong
    // element type -- and the D-228 container gate still runs downstream
    // either way.
    let mut module_env = annotated_function_environment(hir);
    // #1021 review round 5: seed the module scope the way
    // `check_with_environment_all` does before it checks any function body,
    // so a producer that reads a module-level global resolves here exactly
    // as the equivalent non-empty literal already does. Without this,
    // `VALUE = 1` / `def f(): xs = []; xs.append(VALUE)` reported a
    // diagnostic while `xs = [VALUE]` compiled -- an asymmetry introduced by
    // this pass's own feature, and one that reappears one source further out
    // for every module-level binding form, not just a plain `Assign`.
    // `bind_local_types_in_body` is the same infallible binder the
    // per-function loop below already uses: it handles `Assign`, `AnnAssign`
    // and walrus targets uniformly and swallows every inference failure, so
    // the pass stays infallible and a global whose own type does not infer
    // simply leaves the container unresolved for the check phase to report.
    // The alias table is seeded for env parity with
    // `check_with_environment_all`, not for a defect reachable today: the
    // registry's only aliasable `math` symbols are `sqrt`/`pi`, both
    // `float`, and D-228's admit set is `list[int]`/`dict[str, int]`, so no
    // aliased-std producer can currently yield an admitted element type.
    // Parity is the property worth holding -- this pass must never see a
    // narrower module scope than the checker that follows it -- and the line
    // keeps that true as the registry grows.
    module_env.std_module_aliases = crate::std_receiver::bind_std_module_aliases(&hir.imports);
    // Part 1 of #1026, for the same env-parity reason as the alias table
    // above: this pass must never see a narrower module scope than the
    // checker that follows it. A foreign name is `Ty::Object`, which is
    // not an admitted container element type, so no producer here can
    // resolve a container through it -- parity is the property, not a
    // reachable defect.
    //
    // PR 2a of #1081 removed the checker's matching pre-seed, so the two
    // are no longer identical: this pass binds every foreign name up
    // front, while the checker binds each at its own recorded position
    // (`crate::foreign::bind_foreign_objects_at`). The difference is
    // entirely in the safe direction. It can only make this scope *wider*,
    // which is the side the invariant above permits, and this pass reports
    // nothing -- it either rewrites a resolvable empty container or hands
    // the module back for the checker to judge -- so a name bound here
    // earlier than the checker would bind it cannot turn into a
    // diagnostic, only into a container resolution that `Ty::Object` can
    // never supply.
    crate::foreign::bind_foreign_objects(&mut module_env, &hir.imports);
    let top_level_stmts: Vec<HirStmt> = hir
        .items
        .iter()
        .filter_map(|item| match item {
            HirItem::TopLevelStmt(stmt) => Some(stmt.clone()),
            HirItem::Function { .. } => None,
        })
        .collect();
    let top_level_names = crate::function_local_names(&[], &top_level_stmts);
    // Walk the items in source order rather than binding the collected
    // statements in a batch: a `def` only becomes callable at its own
    // position, and `infer_expr_in` rejects a call to a name that is not yet
    // in `defined_functions` (`expr.rs`'s callee gate, issue #22). Mirroring
    // `check_with_environment_all`'s pass 2 here is what lets `VALUE =
    // _base()` infer, and it keeps a genuine forward reference unresolved in
    // this pass exactly as the checker rejects it. Function bodies are
    // unaffected either way: `child_for_function` re-seeds the whole set.
    for item in &hir.items {
        match item {
            HirItem::Function { name, .. } => {
                // Both sets, exactly as `check_with_environment_all`'s pass 2
                // does: `def_rebound` is what D-110's non-callable-binding
                // gate consults, so a `def` that shadows an earlier value of
                // the same name (`helper = 1` / `def helper() -> int:`) is
                // callable from its own position onward here too. Seeding
                // only `defined_functions` left that call uninferable and
                // turned a resolvable producer into a spurious `T0003`.
                module_env.def_rebound.insert(name.clone());
                module_env.defined_functions.insert(name.clone());
            }
            HirItem::TopLevelStmt(stmt) => {
                crate::bind_local_types_in_stmt(&mut module_env, &top_level_names, stmt);
            }
        }
    }
    module_env
}

/// The environment a producer in function `name`'s body is inferred in:
/// `module_env`'s child scope, the enclosing class (for `super()`), the
/// parameters, the flat whole-body bindings, and the D-245 demotion of every
/// conditionally bound name.
fn function_environment(
    module_env: &Environment,
    name: &str,
    params: &[(String, Ty)],
    body: &[HirStmt],
    names: &[&str],
) -> Environment {
    let mut env = module_env.child_for_function(names);
    // #433, mirroring `check_function_in`: extract the class name from a
    // mangled `<ClassName>.<method>` name so producer inference can
    // resolve `super()`. Without it `resolve_super_method_call` reaches
    // its `env.current_class().unwrap()` with `self` bound (the loop
    // below binds every parameter) and no class, and aborts the
    // compiler on `xs = []` / `xs.append(super().m())`. A top-level
    // function name contains no `.`, so this leaves `current_class`
    // `None` for those exactly as the checker does.
    env.current_class = name
        .split('.')
        .next()
        .filter(|prefix| *prefix != name)
        .map(String::from);
    for (param_name, param_ty) in params.iter() {
        env.bind(param_name.clone(), param_ty.clone());
    }
    crate::bind_local_types_in_body(&mut env, names, body);
    demote_conditional_bindings(&mut env, params, names, body);
    env
}

/// Demotes every binding `bind_local_types_in_body` recorded that the check
/// phase would treat as only *maybe* bound, so the producer source cannot
/// resolve a container from evidence the checker itself refuses to read.
///
/// `bind_local_types_in_body` is a flat forward binder: it records every
/// target it walks as [`BindingState::Definitely`], including one bound only
/// inside an `if` branch or a loop body. The checker does not -- a name
/// assigned on some paths but not all joins back as [`BindingState::Maybe`],
/// and reading it is `T0041` (see [`crate::env::BindingState`]'s own join
/// lattice). Without this demotion `if flag: v = "s"` / `xs = []` /
/// `xs.append(v)` resolved `list[str]` from `v` and the check phase reported
/// `T0034` against the resolved container, masking the `T0041` the same
/// program reports in its `xs = [v]` spelling. That is a *wrong* resolution
/// in exactly the sense D-245's invariant forbids: the pass manufactured a
/// resolution from a binding the checker is not entitled to read. It is the
/// same failure shape the `Ty::Infer` and `Ty::Optional` filters in
/// [`resolve`] already repair, reached through boundness rather than through
/// the resolved type.
///
/// The repair is `bind_maybe` rather than removal, and it is deliberately
/// asymmetric between the two inferred sources. `Environment::lookup` --
/// which is what `infer_expr_in` consults for a name read, and therefore what
/// [`find_producer`] goes through -- returns `None` for a `Maybe` binding, so
/// the producer's value simply fails to infer and the scan declines it.
/// Source 2 reads `binding_state` directly and is unaffected, which is
/// correct: it reads the *assignment target's* own recorded type, not a value
/// expression, and the target is being assigned at the site being rewritten.
///
/// The demotion is undone as [`find_producer`] descends: a name bound inside
/// an `if` branch or a loop body *is* readable at a producer inside that same
/// body, so each nested scope re-promotes the names its own construct binds
/// (see [`scoped_for_body`]). Only a producer that reads such a name from
/// *outside* its binding construct is declined, which is exactly the shape
/// the check phase reports `T0041` for.
///
/// The definite set is deliberately under-approximated: only an `Assign`, an
/// `AnnAssign` carrying a value, and a walrus in a test or expression
/// statement count as binding at a scope's own top level. An `if`/`else`
/// binding the same name in *both* arms really does join to `Definitely`
/// afterwards, and this demotes it anyway. That costs a missed resolution
/// (`T0003`), never a wrong element type, which is the direction this pass's
/// invariant requires; computing the true join would mean reimplementing the
/// checker's own statement walk ahead of it, in a pass whose design
/// justification is that it is infallible and pure.
fn demote_conditional_bindings(
    env: &mut Environment,
    params: &[(String, Ty)],
    names: &[&str],
    body: &[HirStmt],
) {
    let mut definite: Vec<&str> = params.iter().map(|(name, _)| name.as_str()).collect();
    collect_definite_top_level_names(body, &mut definite);
    let demoted: Vec<(String, Ty)> = names
        .iter()
        .filter(|name| !definite.contains(*name))
        .filter_map(|name| {
            env.binding_state(name)
                .map(|state| ((*name).to_string(), state.ty().clone()))
        })
        .collect();
    for (name, ty) in demoted {
        env.bind_maybe(name, ty);
    }
}

/// The names a *top-level* statement of `body` binds on every path reaching
/// the end of `body`. Nested bodies are deliberately not walked: a binding
/// inside one is exactly what [`demote_conditional_bindings`] demotes. A
/// `ForRange`/`ForList` target is likewise absent, because a loop body may
/// execute zero times and its target is `Maybe` afterwards -- inside that
/// body it is bound, and [`scoped_for_body`] is what restores it there.
fn collect_definite_top_level_names<'a>(body: &'a [HirStmt], names: &mut Vec<&'a str>) {
    for stmt in body {
        match stmt {
            HirStmt::Assign { target, .. } => names.push(target),
            // A value-less `AnnAssign` (`x: int`) declares without binding,
            // matching `Environment::declared`'s own "declared, not yet
            // assigned" state.
            HirStmt::AnnAssign { target, value, .. } if value.is_some() => names.push(target),
            // A walrus in an `if`/`while` test or a bare expression statement
            // is evaluated before the block is entered at all, so its target
            // is bound on every path -- the three placements
            // `violates_walrus_placement` permits.
            HirStmt::ExprStmt(expr)
            | HirStmt::If { test: expr, .. }
            | HirStmt::While { test: expr, .. } => {
                crate::collect_named_expr_names_in_expr(expr, names)
            }
            _ => {}
        }
    }
}

/// Every syntactic binding site in `body`, nested bodies included, as one
/// entry per site rather than one per name: a name listed twice is bound by
/// two different statements, and the flat whole-function environment records
/// only one of their types.
///
/// This counts sites, not reachable assignments -- the two arms of an `if`
/// are two sites even though only one runs -- which is what
/// [`scoped_for_body`] needs and is why no environment is consulted here.
fn collect_binding_sites<'a>(body: &'a [HirStmt], sites: &mut Vec<&'a str>) {
    for stmt in body {
        match stmt {
            HirStmt::Assign { target, .. } => sites.push(target),
            HirStmt::AnnAssign { target, value, .. } if value.is_some() => sites.push(target),
            // A loop target is rebound on every iteration, and a loop nested
            // in another construct's body is one more site for that name.
            HirStmt::ForRange { var, .. } | HirStmt::ForList { var, .. } => sites.push(var),
            HirStmt::ExprStmt(expr)
            | HirStmt::If { test: expr, .. }
            | HirStmt::While { test: expr, .. } => {
                crate::collect_named_expr_names_in_expr(expr, sites)
            }
            _ => {}
        }
        for nested in nested_bodies(stmt) {
            collect_binding_sites(nested, sites);
        }
    }
}

/// Whether `name` is bound by more than one of `sites`.
fn bound_by_several_sites(sites: &[&str], name: &str) -> bool {
    sites.iter().filter(|site| **site == name).count() > 1
}

/// The environment a producer scan of `body` -- one nested body of `stmt` --
/// should read, given the environment of the scope enclosing `stmt`.
///
/// [`demote_conditional_bindings`] demoted every binding a nested body
/// establishes, because none of them is readable *after* the construct
/// finishes. Inside the body they are readable, so this restores exactly
/// those: the names that body's own top-level statements bind, plus the loop
/// target of a `for`, which the loop itself binds on every iteration. The
/// restoration is per *body*, not per statement, so an `if`'s two arms never
/// see each other's bindings.
///
/// The restored *type*, however, is the flat binder's, not this body's, and
/// that is only trustworthy while the two cannot differ. `sites` is what
/// establishes they cannot: a name bound by more than one syntactic site
/// anywhere in the function is declined rather than promoted, because the
/// type the flat binder recorded for it may have been contributed by a
/// *different, mutually exclusive* site. For `if flag: v = True` /
/// `else: v = 1; xs = []; xs.append(v)` the flat pass retains `bool` from the
/// `if` arm, promoting it inside the `else` resolved `EmptyList(Bool)` and
/// reported `T0034`, while the `xs = [v]` spelling sees the branch-local
/// `int` and compiles -- a *wrong* resolution in exactly the sense D-245's
/// invariant forbids (#1021 bot review round 16).
///
/// Declining is the repair rather than reconstructing the body's own
/// environment, for the same reason [`demote_conditional_bindings`] gives:
/// threading a per-body binder ahead of the checker would reimplement the
/// statement-walk ordering the checker already owns, inside a pass whose
/// design justification is that it is infallible and pure. The test is
/// deliberately under-approximated in the safe direction -- two sites
/// assigning the *same* type are declined too, costing a `T0003` and never a
/// wrong element type.
///
/// `None` means nothing needed restoring and the caller can keep reading the
/// enclosing environment, which is the common case and avoids cloning an
/// `Environment` for every block statement in every function.
fn scoped_for_body(
    stmt: &HirStmt,
    body: &[HirStmt],
    env: &Environment,
    sites: &[&str],
) -> Option<Environment> {
    let mut restored: Vec<&str> = Vec::new();
    match stmt {
        HirStmt::ForRange { var, .. } | HirStmt::ForList { var, .. } => restored.push(var),
        _ => {}
    }
    collect_definite_top_level_names(body, &mut restored);
    let promotions: Vec<(String, Ty)> = restored
        .iter()
        .filter(|name| !bound_by_several_sites(sites, name))
        .filter_map(|name| match env.binding_state(name) {
            Some(BindingState::Maybe(ty)) => Some(((*name).to_string(), ty.clone())),
            _ => None,
        })
        .collect();
    if promotions.is_empty() {
        return None;
    }
    let mut scoped = env.clone();
    for (name, ty) in promotions {
        scoped.bind(name, ty);
    }
    Some(scoped)
}

fn body_has_empty_literal(body: &[HirStmt]) -> bool {
    any_stmt(body, &|stmt| match stmt {
        HirStmt::Assign { value, .. } => empty_literal(value).is_some(),
        HirStmt::AnnAssign { value, .. } => {
            value.as_ref().is_some_and(|v| empty_literal(v).is_some())
                || object_slot::is_annotated_object_display(stmt)
        }
        _ => false,
    })
}

/// Whether `found` holds for any statement of `body`, at any block depth
/// [`nested_bodies`] reaches. The one traversal both fast-path triggers
/// share: the local-binding one above and #1265's attribute-reset one
/// (`attr_slot::needs_class_phase`).
fn any_stmt(body: &[HirStmt], found: &dyn Fn(&HirStmt) -> bool) -> bool {
    body.iter()
        .any(|stmt| found(stmt) || nested_bodies(stmt).iter().any(|b| any_stmt(b, found)))
}

/// Every nested statement sequence of a block statement, for every block
/// statement this HIR has. The inventory is derived from `HirStmt`'s own
/// `Vec<HirStmt>` fields rather than from the cases a caller happened to
/// think of: a block form missing here is invisible to the whole pass, so an
/// annotated `xs: list[int] = []` inside it reports `T0003` even though the
/// annotation source needs no environment at all (#1021 bot review round).
/// `Try` and `TryStar` share one arm because their field shapes are
/// identical; a future block form must be added here and in
/// [`for_each_stmt_mut`]'s own match, which cannot share this borrow because
/// it needs `&mut`.
fn nested_bodies(stmt: &HirStmt) -> Vec<&[HirStmt]> {
    match stmt {
        HirStmt::If { body, orelse, .. } => vec![body, orelse],
        HirStmt::While { body, .. } => vec![body],
        HirStmt::ForRange { body, .. } => vec![body],
        HirStmt::ForList { body, .. } => vec![body],
        HirStmt::Match { cases, .. } => cases.iter().map(|case| case.body.as_slice()).collect(),
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
            // Source order, matching `rewrite_body`'s own traversal of the
            // same variant: `find_producer` recurses through this inventory
            // and documents that it returns the first *syntactic* producer,
            // which is only true while the two orders agree.
            let mut bodies: Vec<&[HirStmt]> = vec![body];
            bodies.extend(handlers.iter().map(|handler| handler.body.as_slice()));
            bodies.push(orelse);
            bodies.push(finalbody);
            bodies
        }
        _ => Vec::new(),
    }
}

fn rewrite_body(
    body: &mut [HirStmt],
    producers: &[HirStmt],
    env: &Environment,
    local_names: &[&str],
) {
    for_each_stmt_mut(body, &mut |stmt| match stmt {
        HirStmt::Assign { target, value } => {
            rewrite_value(value, target, None, producers, env, local_names);
        }
        HirStmt::AnnAssign {
            target,
            annotation,
            value: Some(value),
            ..
        } => {
            rewrite_value(value, target, Some(annotation), producers, env, local_names);
        }
        _ => {}
    });
}

/// Visits every statement of `body` and then every statement of each of its
/// nested block bodies, mutably. This is [`nested_bodies`]' `&mut` twin (it
/// cannot share that borrow), and the one traversal both rewrites share: the
/// local-binding one in [`rewrite_body`] and #1265's attribute-reset one in
/// `attr_slot`. `visit` only ever rewrites an expression inside the
/// statement, never its nested bodies, so recursing after the visit sees the
/// same statement list it would have seen before.
fn for_each_stmt_mut(body: &mut [HirStmt], visit: &mut dyn FnMut(&mut HirStmt)) {
    for stmt in body.iter_mut() {
        visit(stmt);
        match stmt {
            HirStmt::If { body, orelse, .. } => {
                for_each_stmt_mut(body, visit);
                for_each_stmt_mut(orelse, visit);
            }
            HirStmt::While { body, .. } => for_each_stmt_mut(body, visit),
            HirStmt::ForRange { body, .. } => for_each_stmt_mut(body, visit),
            HirStmt::ForList { body, .. } => for_each_stmt_mut(body, visit),
            HirStmt::Match { cases, .. } => {
                for case in cases.iter_mut() {
                    for_each_stmt_mut(&mut case.body, visit);
                }
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
                for_each_stmt_mut(body, visit);
                for handler in handlers.iter_mut() {
                    for_each_stmt_mut(&mut handler.body, visit);
                }
                for_each_stmt_mut(orelse, visit);
                for_each_stmt_mut(finalbody, visit);
            }
            _ => {}
        }
    }
}

fn rewrite_value(
    value: &mut HirExpr,
    target: &str,
    annotation: Option<&Ty>,
    producers: &[HirStmt],
    env: &Environment,
    local_names: &[&str],
) {
    // Part 2d of #1371: a list display bound to an object slot builds a
    // CPython `list` (`object_slot`), ahead of the native resolution.
    if object_slot::rewrite_object_slot(value, target, annotation, env) {
        return;
    }
    let Some(literal) = empty_literal(value) else {
        return;
    };
    let Some(resolution) = resolve(target, annotation, producers, env, local_names) else {
        return;
    };
    if resolution.matches(literal) {
        *value = resolution.into_expr();
    }
}

/// The three-source priority order. A resolved type is *not* validated for
/// *admissibility* here: an inferred `list[str]` is stored and then rejected
/// by `pycc_hir::check_container_ty` (`T0034`) in the check phase, exactly as
/// a written `xs: list[str] = [1]` annotation is -- D-228's rule that a
/// written annotation and an inferred literal share one gate.
///
/// It *is* validated for *concreteness*: a resolution containing `Ty::Infer`
/// is discarded, so the literal stays a raw `ListLiteral`/`DictLiteral` and
/// the check phase reports `T0003` against it. `Ty::Infer` is the private-
/// helper solver's placeholder, and this pass runs before that solver, so a
/// producer whose value is an unannotated helper parameter (`def _f(x): xs =
/// []; xs.append(x)`) infers `Ty::Infer` here. Freezing it into the node was
/// a *wrong* resolution, not a missed one: the solver never substitutes into
/// a rewritten node, so `check_container_ty` then reported a `T0034` naming
/// `list[<inferred>]` -- a type the source never mentions -- for a program
/// whose `xs = [x]` spelling the solver accepts. `Ty::Param` is deliberately
/// *not* discarded: both spellings already report the same `T0034` for it, so
/// there is no asymmetry to repair and discarding it would introduce one.
///
/// The two *inferred* sources -- the binding and the producer -- are validated
/// once more, for *flow independence*: a resolution containing `Ty::Optional`
/// is discarded. Both read the flat whole-function `Environment`, whose
/// narrowing overlay is empty, while the check phase resolves the same name
/// inside a narrowed branch. Narrowing here is exclusively `Optional`
/// narrowing (`crate::narrow::narrowing_target` recognizes only a
/// `name is None` / `name is not None` test against an `Ty::Optional`
/// binding), so an `Optional`-carrying inferred resolution is exactly the set
/// this pass can get wrong: for `if x is not None: xs = []; xs.append(x)` the
/// flat environment yields `Optional[int]` and `check_container_ty` reports a
/// `T0034` naming `list[int | None]`, while the `xs = [x]` spelling compiles.
/// That is a *wrong* resolution, not a missed one. The annotation source is
/// deliberately *not* filtered: `xs: list[int | None] = []` says so in
/// source, no narrowing is involved, and its `T0034` names the real D-105 gap
/// where `T0003` would be the worse diagnostic.
fn resolve(
    target: &str,
    annotation: Option<&Ty>,
    producers: &[HirStmt],
    env: &Environment,
    local_names: &[&str],
) -> Option<Resolution> {
    if let Some(from_annotation) = annotation.and_then(from_container_ty) {
        return concrete(from_annotation);
    }
    if let Some(from_binding) = env
        .binding_state(target)
        .map(BindingState::ty)
        .and_then(from_container_ty)
    {
        return inferred(from_binding);
    }
    find_producer(producers, target, env, local_names).and_then(inferred)
}

/// `Some(resolution)` when every type it carries is free of `Ty::Infer`,
/// `None` otherwise -- see [`resolve`] for why that placeholder is discarded
/// rather than stored. This is the gate the *annotation* source passes
/// through.
fn concrete(resolution: Resolution) -> Option<Resolution> {
    free_of(resolution, contains_infer)
}

/// The gate the two *inferred* sources pass through: [`concrete`], and then
/// additionally free of `Ty::Optional` -- see [`resolve`] for why a
/// narrowable type read from the flat environment is discarded rather than
/// stored.
fn inferred(resolution: Resolution) -> Option<Resolution> {
    free_of(concrete(resolution)?, contains_optional)
}

/// `Some(resolution)` when `rejected` holds for none of the types it carries.
/// Both halves of a dict resolution are checked, not just the value.
fn free_of(resolution: Resolution, rejected: fn(&Ty) -> bool) -> Option<Resolution> {
    let accepted = match &resolution {
        Resolution::List(element) => !rejected(element),
        Resolution::Dict(key, value) => !rejected(key) && !rejected(value),
    };
    accepted.then_some(resolution)
}

/// Whether `ty` is `Ty::Infer` or contains one anywhere inside it.
///
/// The recursion matters: a producer can yield a nested container whose
/// *element* is the placeholder (`xs.append(ys)` where `ys` is an
/// unannotated helper parameter's list), and a `list[list[<inferred>]]`
/// stored in the node is the same wrong resolution a bare
/// `list[<inferred>]` is.
pub(crate) fn contains_infer(ty: &Ty) -> bool {
    match ty {
        Ty::Infer => true,
        Ty::List(element) | Ty::Set(element) | Ty::FrozenSet(element) | Ty::Optional(element) => {
            contains_infer(element)
        }
        Ty::Dict(pair) => contains_infer(&pair.0) || contains_infer(&pair.1),
        Ty::Tuple(elements) => elements.iter().any(contains_infer),
        Ty::Int
        | Ty::Float
        | Ty::Bool
        | Ty::Str
        | Ty::None
        | Ty::Param(_)
        | Ty::Instance(_)
        | Ty::Protocol(_)
        | Ty::Object
        | Ty::MemoryView => false,
    }
}

/// Whether `ty` is `Ty::Optional` or contains one anywhere inside it.
///
/// The node itself answers `true` without recursing: it is the `Optional`
/// *wrapper* the check phase may have narrowed away, not anything inside it.
/// The recursion still matters for the nested shapes -- a producer can yield
/// `list[int | None]`, and freezing that into an element position is the same
/// wrong resolution a bare `int | None` is.
fn contains_optional(ty: &Ty) -> bool {
    match ty {
        Ty::Optional(_) => true,
        Ty::List(element) | Ty::Set(element) | Ty::FrozenSet(element) => contains_optional(element),
        Ty::Dict(pair) => contains_optional(&pair.0) || contains_optional(&pair.1),
        Ty::Tuple(elements) => elements.iter().any(contains_optional),
        Ty::Int
        | Ty::Float
        | Ty::Bool
        | Ty::Str
        | Ty::None
        | Ty::Infer
        | Ty::Param(_)
        | Ty::Instance(_)
        | Ty::Protocol(_)
        | Ty::Object
        | Ty::MemoryView => false,
    }
}

fn from_container_ty(ty: &Ty) -> Option<Resolution> {
    match ty {
        Ty::List(element) => Some(Resolution::List((**element).clone())),
        Ty::Dict(pair) => Some(Resolution::Dict(pair.0.clone(), pair.1.clone())),
        _ => None,
    }
}

/// `T0003` for an empty list literal no source of evidence could type.
pub(crate) fn unresolved_list(in_function_body: bool) -> Diagnostic {
    Diagnostic::error(
        "T0003",
        "an empty list literal has no inferable element type here".to_string(),
        Span::new(0, 0),
    )
    .with_help(help_for_position(
        in_function_body,
        "annotate the assignment target (`xs: list[int] = []`) or append a value to it",
        "move this binding into a function body, where `xs: list[int] = []` or a later \
         `xs.append(...)` can type it",
    ))
}

/// `T0003` for an empty dict literal no source of evidence could type.
pub(crate) fn unresolved_dict(in_function_body: bool) -> Diagnostic {
    Diagnostic::error(
        "T0003",
        "an empty dict literal has no inferable key/value types here".to_string(),
        Span::new(0, 0),
    )
    .with_help(help_for_position(
        in_function_body,
        "annotate the assignment target (`d: dict[str, int] = {}`) or assign an entry into it",
        "move this binding into a function body, where `d: dict[str, int] = {}` or a later \
         `d[k] = v` can type it",
    ))
}

/// Picks between the two remedies a `T0003` can honestly suggest.
///
/// Both of the in-function remedies are real: the annotation source is purely
/// syntactic and the producer scan runs over every block statement a function
/// body has. Neither works at module level, because D-245 item 8 puts
/// module-level statements outside this pass's scope entirely -- `VALUE: list[int] = []`
/// at module level reports the *same* `T0003` as the bare `VALUE = []` it is
/// offered as the fix for, so suggesting it there sends the reader in a
/// circle. The module-level wording names the one thing that does work, and
/// states the position as the reason rather than leaving it to be discovered.
fn help_for_position(in_function_body: bool, in_function: &str, at_module_level: &str) -> String {
    if in_function_body {
        in_function.to_string()
    } else {
        at_module_level.to_string()
    }
}

/// Names the binding in a `T0003` raised while checking `target`'s assigned
/// `value`. `T0003`'s span is the `Span::new(0, 0)` every container diagnostic
/// in this crate uses (`HirStmt::Assign` carries no span at all), so the
/// binding's name in the message is the only locator a user gets.
///
/// The substitution is gated on `value` *itself* being the empty literal that
/// failed (#1021 review round 1). A `T0003` also propagates out of a nested
/// element position -- `xs: list[int] = [[]]` fails on the inner `[]` while
/// `xs` is validly annotated -- and naming `xs` there points the user at the
/// wrong node. Such a diagnostic keeps `unresolved_list`/`unresolved_dict`'s
/// generic `" here"` wording, which is the only honest locator available when
/// the failing node is not the assigned value.
pub(crate) fn name_binding(
    mut diagnostic: Diagnostic,
    target: &str,
    value: &HirExpr,
) -> Diagnostic {
    if diagnostic.code == "T0003" && empty_literal(value).is_some() {
        diagnostic.message = diagnostic
            .message
            .replace(" here", &format!(" for `{target}`"));
        diagnostic.label = Some(diagnostic.message.clone());
    }
    diagnostic
}

#[cfg(test)]
mod tests;
