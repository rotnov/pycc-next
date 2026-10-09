//! Storage-slot binding collection: the pure MIR walks that decide which
//! names get a predeclared storage slot, and of which type, before any
//! LLVM IR is emitted -- one slot per name per function
//! ([`collect_stmt_bindings`]), the module-global set
//! ([`collect_module_bindings`]), and a walrus target's slot
//! ([`collect_expr_bindings`]). Moved out of `lib.rs` unchanged, except
//! that #1325 later added `Ty::Object` to the `MirStmt::Assign` arm's
//! storable-type allow-list; the walks have no inkwell dependency.

use super::*;

/// PEP 572 (#774): `collect_stmt_bindings`'s own counterpart for a
/// `MirExpr` rather than a `MirStmt` -- a walrus target's storage slot is
/// declared by an expression embedded in a statement (an `if`/`while` test,
/// or a bare expression statement), not by a `MirStmt::Assign` the way
/// every other binding this file predeclares is. Delegates the actual tree
/// walk to `MirExpr::collect_named_expr_bindings` (defined once in
/// `pycc_mir`, shared with that crate's own `pycc_mir::stmt::lower_stmt`
/// hoist-and-bind logic), then applies the exact same storable-type
/// allow-list `MirStmt::Assign`'s own arm below applies -- T0050
/// (`pycc_types::expr::is_walrus_value_ty_supported`) already restricts a
/// walrus's value to a subset of that allow-list (the non-reference-counted
/// scalars), so every `ty` reaching this filter is expected to pass it, but
/// applying the identical check here rather than assuming it keeps this
/// function correct on its own terms if that restriction ever changes.
pub(super) fn collect_expr_bindings(expr: &MirExpr, bindings: &mut BTreeMap<String, pycc_mir::Ty>) {
    let mut named_bindings = Vec::new();
    expr.collect_named_expr_bindings(&mut named_bindings);
    for (name, ty) in named_bindings {
        if matches!(
            ty,
            pycc_mir::Ty::Int
                | pycc_mir::Ty::Bool
                | pycc_mir::Ty::Float
                | pycc_mir::Ty::Str
                | pycc_mir::Ty::None
                | pycc_mir::Ty::List(_)
                | pycc_mir::Ty::Dict(_)
                | pycc_mir::Ty::Set(_)
                | pycc_mir::Ty::FrozenSet(_)
                | pycc_mir::Ty::Tuple(_)
                | pycc_mir::Ty::Instance(_)
                | pycc_mir::Ty::Optional(_)
        ) {
            bindings.entry(name).or_insert(ty);
        }
    }
}

pub(super) fn collect_stmt_bindings(stmt: &MirStmt, bindings: &mut BTreeMap<String, pycc_mir::Ty>) {
    match stmt {
        MirStmt::Assign { target, value } => {
            let ty = value.ty();
            // `Ty::List(_)` joined the allow-list at D-089 (Task 5 of
            // PR-10); `Ty::Dict(_)` joined it at PR-11 Task 5, `Ty::Set(_)`
            // at PR-11 Task 9, and `Ty::Tuple(_)` joins it here (PR-11b
            // Task 5) -- all for the identical reason: a tuple-typed
            // local's binding does need to be collected, since this task's
            // own codegen (`declare_module_globals`/
            // `storage_slot_at_entry`) depends on this slot already
            // existing, and `x = (1, 2)` is exactly the form D-116 ships.
            // Each is a real, deliberate inclusion, not just a louder panic
            // elsewhere. `Ty::None` is also storable via D-075's canonical
            // unit carrier; only `Ty::Infer` remains excluded.
            if matches!(
                ty,
                pycc_mir::Ty::Int
                    | pycc_mir::Ty::Bool
                    | pycc_mir::Ty::Float
                    | pycc_mir::Ty::Str
                    | pycc_mir::Ty::None
                    | pycc_mir::Ty::List(_)
                    | pycc_mir::Ty::Dict(_)
                    | pycc_mir::Ty::Set(_)
                    | pycc_mir::Ty::FrozenSet(_)
                    | pycc_mir::Ty::Tuple(_)
                    // D-154 (Part 1 of #375): `p = Point(1, 2)` needs its
                    // own predeclared storage slot exactly like every other
                    // heap-object-typed binding above.
                    | pycc_mir::Ty::Instance(_)
                    // `Optional[int]` (D-197, #763, Part 1 of #747): an
                    // `x: int | None = ...` binding is exactly the case
                    // `OptionalWrap`'s own doc comment describes -- its
                    // lowered `value.ty()` now correctly reports
                    // `Ty::Optional(_)` rather than the bare inner type,
                    // so this allow-list must recognize it or the slot
                    // this function exists to predeclare is silently
                    // skipped, surfacing here as a missing-slot panic
                    // rather than at the type-checking boundary.
                    | pycc_mir::Ty::Optional(_)
                    // Part 2a of #1142 (#1165): `a = ndarray(n)` binds
                    // artifact-owned buffer storage, and the slot this
                    // function predeclares is what the free-before-overwrite
                    // store path and the function's exit epilogue both
                    // operate on. Missing from this list, the binding is
                    // silently skipped and `emit_assign` panics on the
                    // absent slot -- the exact symptom the `Ty::Optional`
                    // comment above records.
                    | pycc_mir::Ty::MemoryView
                    // #1325: a module-level `x = product("ab")` binds a
                    // CPython object to a module global, which needs its
                    // predeclared slot exactly like every type above;
                    // missing from this list, `emit_assign` panics on the
                    // absent slot. Since Part 1 of #1333 (#1362) a
                    // function-local object binding reaches here too, through
                    // the same walk over the function body, and needs its
                    // predeclared slot for the identical reason.
                    | pycc_mir::Ty::Object
            ) {
                bindings.entry(target.clone()).or_insert(ty);
            }
        }
        // PEP 572 (#774): `test` also gets `collect_expr_bindings`'d, not
        // just `body`/`orelse` recursed into -- an `if` test condition is
        // one of the three placements a walrus is permitted in, and its
        // bound name needs a predeclared storage slot exactly like any
        // other local, or `emit_assign`'s own `locals.get(target).expect(..)`
        // panics the first time `emit_expr_unchecked`'s `MirExpr::NamedExpr`
        // arm tries to store into it.
        MirStmt::If {
            test, body, orelse, ..
        } => {
            collect_expr_bindings(test, bindings);
            for stmt in body {
                collect_stmt_bindings(stmt, bindings);
            }
            for stmt in orelse {
                collect_stmt_bindings(stmt, bindings);
            }
        }
        // PEP 572 (#774): mirrors `If`'s own `test` handling just above --
        // a `while` test condition is the other permitted placement.
        MirStmt::While { test, body } => {
            collect_expr_bindings(test, bindings);
            for stmt in body {
                collect_stmt_bindings(stmt, bindings);
            }
        }
        MirStmt::ForRange { var, body, .. } => {
            bindings.entry(var.clone()).or_insert(pycc_mir::Ty::Int);
            for stmt in body {
                collect_stmt_bindings(stmt, bindings);
            }
        }
        // `Ty::Int` for the same reason `ForRange` above hardcodes it, not
        // by analogy: a `for` target's type is the iterated element type,
        // and `pycc_types`' T0034 gate (D-105 scope cut 5) rejects every
        // `list[T]` but `list[int]` before codegen ever runs, so `list`'s
        // element type is `int` for every `ForList` that can reach this
        // crate. Deliberately not derived from `bindings[list]` instead:
        // that entry can be absent. Before D-228 (issue #918) the reason
        // given here was that `list` could never be a list-typed function
        // *parameter* at all; that is no longer true -- `def f(xs:
        // list[int])` now lowers and reaches codegen. The binding can still
        // be absent for the original second reason, which is the one that
        // actually matters: `list` can be a module-scope global iterated from
        // inside a function body, whose `local_bindings` is built from that
        // function body alone and so has no entry for it at all --
        // exactly what `a_module_level_list_binding_lives_in_a_global_slot`
        // (`tests/slice1_codegen_depth.rs`) exercises. A derived non-`int`
        // element type would allocate a slot `emit_stmt`'s own
        // `list[int]`-only `ForList` arm then stores an encoded `int` into. A
        // future PR widening codegen past `list[int]` owns both halves
        // together.
        MirStmt::ForList { var, body, .. } => {
            bindings.entry(var.clone()).or_insert(pycc_mir::Ty::Int);
            for stmt in body {
                collect_stmt_bindings(stmt, bindings);
            }
        }
        // PR 3c of #1082: a `for x in <object>:` target holds each item as
        // an opaque `PyObject *`, so its slot is `Ty::Object` -- a pointer
        // under `ty_to_basic_type`. Hardcoding `Ty::Int` the way `ForList`
        // does above would allocate an `i64` slot and then store a pointer
        // into it.
        // Inserted unconditionally rather than through `or_insert` the way
        // every arm above does, matching `pycc_mir`'s own `ForObject` arm,
        // which calls `bind` rather than `bind_variable` for the same
        // reason. `or_insert` would keep a narrower type a name was bound to
        // earlier in the same function (`x = 5`) and allocate an `i64` slot
        // this loop then stores a `PyObject *` into -- a silent
        // miscompilation, since the `local type drifted` guard below is a
        // `debug_assert` and vanishes in a release build. That shape is
        // refused in `pycc_types` (`T0023`), which is what actually makes it
        // unreachable; this arm and `pycc_mir`'s agree with that refusal
        // instead of contradicting it, so a future relaxation of the checker
        // cannot silently reintroduce the pun here.
        MirStmt::ForObject { var, body, .. } => {
            bindings.insert(var.clone(), pycc_mir::Ty::Object);
            for stmt in body {
                collect_stmt_bindings(stmt, bindings);
            }
        }
        // `d[k] = v` (PR-11 Task 4) reassigns an existing binding's
        // contents, not a name -- mirrors `pycc_types::collect_local_names`'s
        // own identical `HirStmt::DictSet` arm and its comment. Unlike
        // `ForDict` immediately below, this is real, permanent behavior, not
        // a temporary stub: no future codegen task ever needs `d[k] = v` to
        // introduce a new binding, since it structurally cannot.
        MirStmt::DictSet { .. } => {}
        // `b[i] = v` (Part 1 of #1142) writes through a buffer's storage --
        // same reasoning as `DictSet` immediately above: the statement
        // reassigns a binding's contents, never a name. Its base is always
        // an existing binding, under either provenance -- a wrapper-borrowed
        // parameter, or the artifact-owned storage Part 2a of #1142 (#1165)
        // added, whose own `MirStmt::Assign` gives it a slot through the arm
        // above before any store can name it.
        MirStmt::BufferSet { .. } => {}
        // `base.attr = value` (D-154, Part 1 of #375) reassigns an
        // existing instance's attribute slot, not a name -- same reasoning
        // as `DictSet` immediately above.
        MirStmt::AttrSet { .. } => {}
        // `del o[a:b]` (Part 2c of #1371) binds no name, and `pycc_hir`
        // refuses a walrus in its operands.
        MirStmt::ObjDelSlice { .. } => {}
        // `o.x = v` and `del o.x` (#1457) bind no name either: the store
        // writes into the object, and `pycc_hir` refuses a walrus in a `del`.
        MirStmt::ObjAttrSet { .. } | MirStmt::ObjDelAttr { .. } => {}
        // `MirStmt::ForDict`, produced when a `for k in d:` HIR loop's
        // base resolves to a dict-typed binding (mirrors `MirStmt::ForList`
        // above, which is produced for the list-typed case). `Ty::Str` for
        // the same reason `ForList`'s own comment gives for its `Ty::Int`
        // hardcode, not by analogy: a `for` target's type is the iterated
        // element type, and `pycc_mir`'s own `HirStmt::ForList` lowering
        // (see that crate's own `lower_stmt`) binds a dict-typed loop
        // variable to `kv.0` -- the dict's key type -- and `pycc_types`'
        // T0036 gate means that key type is always exactly `Ty::Str` for
        // every `Ty::Dict` value that ever reaches this crate (no other
        // key type is compiled). PR-11 Task 5's own codegen (`emit_stmt`'s
        // `MirStmt::ForDict` arm) binds the loop variable to a
        // `Scalar::Str` every iteration, so its slot must already exist
        // before that arm runs, exactly like `ForList`'s own `var` slot --
        // unlike Task 4's version of this arm, this is no longer a
        // deferred design decision. Recursing into `body` is unchanged: it
        // is what lets a nested, ordinary statement (e.g. `for k in d:\n
        // y = 1\n`) still get `y`'s own binding collected, exactly like
        // every other container arm above.
        MirStmt::ForDict { var, body, .. } => {
            bindings.entry(var.clone()).or_insert(pycc_mir::Ty::Str);
            for stmt in body {
                collect_stmt_bindings(stmt, bindings);
            }
        }
        // `MirStmt::ForSet`, produced when a `for x in s:` HIR loop's base
        // resolves to a set-typed binding (mirrors `MirStmt::ForDict`
        // immediately above). The target binds the set's element type,
        // which lowering carries as `var_ty`: `int`, or a user-class
        // instance since #1343. `emit_stmt`'s `MirStmt::ForSet` arm writes
        // that slot every iteration, so it must exist before the arm runs.
        // Recursing into `body` collects a nested statement's own bindings,
        // exactly like every other container arm above.
        MirStmt::ForSet {
            var, var_ty, body, ..
        } => {
            bindings
                .entry(var.clone())
                .or_insert_with(|| var_ty.clone());
            for stmt in body {
                collect_stmt_bindings(stmt, bindings);
            }
        }
        // `target = [elt for var in <source> [if cond]]` (PR-12 Task 5a,
        // D-117): a comprehension introduces *two* new bindings, not one --
        // `target` (the produced `list[T]` container) and `var` (the
        // synthesized loop variable, D-117). `var_ty` is carried explicitly
        // on the MIR node itself (Task 4), so unlike `ForList`'s own
        // `Ty::Int` hardcode above, no re-derivation is needed or attempted
        // here: `resolve_comp_source` (`pycc_mir`) already computed it once,
        // exactly mirroring `ForList`'s `Ty::Dict(kv) => kv.0` choice for a
        // `Dict` source. `target`'s own type is derived structurally from
        // `elt`, exactly like `MirExpr::ListLiteral`'s own `ty()` derivation
        // -- not re-read from `var_ty`, which is `var`'s type, not
        // `target`'s. Nothing recurses into `cond`/`elt`: neither can ever
        // contain a nested statement (both are plain `MirExpr` trees), so
        // there is no `body`-like recursion for this variant the way every
        // `For*` arm above has.
        MirStmt::ListCompAssign {
            target,
            var,
            var_ty,
            elt,
            ..
        } => {
            bindings
                .entry(var.clone())
                .or_insert_with(|| var_ty.clone());
            bindings
                .entry(target.clone())
                .or_insert(pycc_mir::Ty::List(Box::new(elt.ty())));
        }
        // `target = {key: value for var in <source> [if cond]}` (PR-12 Task
        // 5b, D-117): mirrors `ListCompAssign`'s own arm above exactly,
        // substituting `target`'s derived type (`Ty::Dict`, from `key`'s and
        // `value`'s own types, mirroring `MirExpr::DictLiteral`'s own `ty()`
        // derivation) for `Ty::List`. Same reasoning as `ListCompAssign`'s
        // own arm for everything else: `var_ty` is carried explicitly by
        // Task 4's own lowering, not re-derived; no recursion into
        // `cond`/`key`/`value`, none of which can ever contain a nested
        // statement.
        MirStmt::DictCompAssign {
            target,
            var,
            var_ty,
            key,
            value,
            ..
        } => {
            bindings
                .entry(var.clone())
                .or_insert_with(|| var_ty.clone());
            bindings
                .entry(target.clone())
                .or_insert(pycc_mir::Ty::Dict(Box::new((key.ty(), value.ty()))));
        }
        // `target = {elt for var in <source> [if cond]}` (PR-12 Task 5b,
        // D-117): mirrors `ListCompAssign`'s own arm above exactly,
        // substituting `Ty::Set` for `Ty::List`.
        MirStmt::SetCompAssign {
            target,
            var,
            var_ty,
            elt,
            ..
        } => {
            bindings
                .entry(var.clone())
                .or_insert_with(|| var_ty.clone());
            bindings
                .entry(target.clone())
                .or_insert(pycc_mir::Ty::Set(Box::new(elt.ty())));
        }
        // PEP 572 (#774): the third permitted walrus placement -- a bare
        // expression statement (`(n := 5)`, or a walrus nested inside a
        // larger expression statement like `f(n := 5)`). `Return`/`NoOp`/
        // `Unreachable` are excluded from this arm on purpose, not merely
        // grouped elsewhere: `pycc_hir::stmt::lower_stmt`'s own
        // `contains_named_expr` restriction rejects a walrus anywhere but
        // the three placements this file's `collect_stmt_bindings` now
        // handles (`ExprStmt` here, `If`/`While`'s own `test` above), so a
        // `MirStmt::Return` can never carry a `NamedExpr` to begin with.
        MirStmt::ExprStmt(expr) => collect_expr_bindings(expr, bindings),
        // Part 2 of #1175 (#1179) joins `Return` here for the same reason
        // and by the same argument: `MirStmt::ReturnBufferSlice` is lowered
        // from an `HirStmt::Return`, which `contains_named_expr` already
        // forbids a walrus in, and it binds no name of its own.
        MirStmt::Return(_)
        | MirStmt::ReturnBufferSlice { .. }
        | MirStmt::NoOp
        | MirStmt::Unreachable => {}
        MirStmt::Seq(stmts) => {
            for stmt in stmts {
                collect_stmt_bindings(stmt, bindings);
            }
        }
        // #382 (PR-22 Part 1): try/except/else/finally — recurse into all
        // nested bodies to collect any bindings introduced within them.
        // Part 3 of #382 (#542, PEP 654, D-202): `except*`/`TryStar` shares
        // this exact recursion -- a handler body's nested bindings are
        // collected identically regardless of whether the handler binds its
        // name to the named exception type (`Try`) or to `ExceptionGroup`
        // (`TryStar`), since that binding-type distinction is resolved by
        // `pycc_mir` lowering, not by this bindings-discovery pass.
        MirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }
        | MirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            for stmt in body {
                collect_stmt_bindings(stmt, bindings);
            }
            for handler in handlers {
                for stmt in &handler.body {
                    collect_stmt_bindings(stmt, bindings);
                }
            }
            for stmt in orelse {
                collect_stmt_bindings(stmt, bindings);
            }
            for stmt in finalbody {
                collect_stmt_bindings(stmt, bindings);
            }
        }
        // #382: raise/raise-from/reraise introduce no new bindings.
        MirStmt::Raise { .. }
        | MirStmt::RaiseFrom { .. }
        | MirStmt::ObjRaise { .. }
        | MirStmt::Reraise => {}
        // #1291, #1383: a foreign import or from-import nested in a
        // module-level block binds each name to a module global holding the
        // imported CPython object.
        MirStmt::ForeignImport { bindings: imports } => {
            for (local_name, _, _) in imports {
                bindings.insert(local_name.clone(), pycc_mir::Ty::Object);
            }
        }
    }
}

pub(super) fn collect_module_bindings(mir: &MirModule) -> BTreeMap<String, pycc_mir::Ty> {
    let mut bindings = BTreeMap::new();
    for item in &mir.items {
        match item {
            MirItem::TopLevelStmt(stmt) => collect_stmt_bindings(stmt, &mut bindings),
            // Part 1 of #1026: a foreign import binds its local name to an
            // opaque CPython object, which needs the same process-wide
            // storage every other module binding gets -- generated
            // functions can read a module global (D-041) regardless of
            // where the binding statement appears.
            MirItem::ForeignImport { local_name, .. } => {
                bindings.insert(local_name.clone(), pycc_mir::Ty::Object);
            }
            MirItem::Function { .. } => {}
        }
    }
    // #379 (PR-19): declare a module global for each enum member singleton.
    // Each member gets a synthetic global named `<Class>.<Member>.enum_member`
    // (the `.enum_member` suffix ensures no collision with real Python names,
    // which cannot contain `.`). The global's type is `Ty::Instance(class)`,
    // so `declare_module_globals` allocates it as an opaque pointer (null
    // until the module-init sequence stores the singleton into it). The init
    // sequence is emitted in `compile_to_object_with_observer` after the
    // top-level statement loop, mirroring how top-level `Assign` already
    // emits init code.
    for (class_name, class_def) in &mir.class_defs {
        for (member_name, _) in &class_def.enum_members {
            bindings.insert(
                format!("{class_name}.{member_name}.enum_member"),
                pycc_mir::Ty::Instance(Box::new(class_name.clone())),
            );
        }
    }
    bindings
}
