pub use pycc_hir::{
    EnumMemberValue, FromImport, HirClassDef, InheritedCopy, foreign_binds_root,
    inherited_copy_origin,
};
mod binop;
use binop::binop_result_ty;
mod boolop;
mod compare_chain;
mod if_exp;
pub use compare_chain::{MirCompareKind, MirCompareLink};
mod class;
#[cfg(test)]
use class::eval_isinstance_protocol;
use class::{class_def_of, mro_attrs};
mod dict_add;
mod exception;
mod exception_isinstance;
mod generator;
pub use exception::{MirExceptHandler, MirExceptionValue};
use exception::{handler_type_tags, lower_raise};
mod expr;
use expr::lower_expr;
mod matching;
#[cfg(test)]
use matching::nest_match_alternatives;
use matching::try_lower_enum_member_attr;
mod item_names;
#[cfg(test)]
use item_names::resolve_method_owner_class;
use item_names::{item_anchor_class, source_frame_name};
mod obj_arith;
mod obj_compare;
mod object_box;
mod object_narrow;
pub use obj_compare::{ObjBuiltinClass, ObjIsInstanceClass};

/// The prefix of a native specialization's name: `pycc_ast::NATIVE_PREFIX`,
/// which this crate does not depend on (the driver checks they agree).
pub const NATIVE_PREFIX: &str = "_pycc_native_";
mod receiver_exact;
mod set_ops;
pub use set_ops::{SetElementOps, SetEqOp, SetHashOp};
mod comp_source;
mod stmt;
mod unpack_display;
pub mod multi_return;
mod optional_str;
mod str_scan;
mod visit_mut;
pub(crate) use comp_source::{resolve_comp_source, str_to_list, tuple_to_list};
#[cfg(debug_assertions)]
mod verify_receiver;
mod visit;
pub use visit::comp_source_exprs;
// MIR node definitions split out of the crate root (mechanical move).
mod mir_expr;
mod mir_expr_payloads;
mod mir_expr_queries;
mod mir_stmt;
pub use mir_expr::*;
pub use mir_expr_payloads::*;
pub use mir_stmt::*;
use pycc_hir::{ForeignImportSite, HirItem, HirModule, HirStmt, ImportBinding};
#[cfg(test)]
use pycc_hir::CompIter;
use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use stmt::lower_stmt;

// Re-exported (not just `use`d) because `pycc_codegen` doesn't depend on
// `pycc_hir` directly (see its Cargo.toml) -- `Ty`, `BinOpKind`, and
// `CmpOpKind` all reach this crate's public API through `MirExpr`'s fields
// (`Name`/`Call` carry a `Ty`; `BinOp` carries a `BinOpKind`; `Compare`
// carries a `CmpOpKind`), so each must be nameable as
// `pycc_mir::{Ty, BinOpKind, CmpOpKind}` from any downstream crate, exactly
// like `pycc_types` already re-exports `Ty` (`pycc_types::Ty`, its own line
// 4) for the same reason.
pub use pycc_hir::{BinOpKind, BoolOpKind, CmpOpKind, EXCEPTION_GROUP_TYPE_TAG, Ty, UnaryOpKind};

/// Monotonic counter for synthesized match-subject temporaries. Each
/// `match` statement gets a unique `__match_subj_N` name, avoiding
/// collisions between multiple matches at the same scope level (which
/// would otherwise share the same `scopes.len()`-derived name and
/// cause a first-assignment-wins type drift in `bind_variable`).
static MATCH_SUBJECT_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// The receiver of `MirExpr::ListAppend`/`ListPop`/`DictGetOrDefault`
/// (mirrors `pycc_hir::ContainerReceiver`, #1263).
#[derive(Debug, Clone, PartialEq)]
pub enum MirContainerReceiver {
    /// A bare variable name, read through the codegen's name-read helpers
    /// exactly as before #1263.
    Name(String),
    /// A lowered attribute read (`MirExpr::AttrGet`, or a property getter
    /// call), whose value is the container pointer. Codegen evaluates it
    /// before the node's own arguments, matching CPython's order.
    Attr(Box<MirExpr>),
}

impl MirContainerReceiver {
    /// The attribute read of an [`MirContainerReceiver::Attr`] receiver;
    /// `None` for a bare name, which has no sub-expression to walk.
    #[must_use]
    pub fn attr_expr(&self) -> Option<&MirExpr> {
        match self {
            MirContainerReceiver::Name(_) => None,
            MirContainerReceiver::Attr(receiver) => Some(receiver),
        }
    }
}

/// [`MirStmt::AssignMulti`]'s payload: `t0, ..., tn = original(args)` where
/// `original` returns a tuple of `n + 1` values, through `twin`, its
/// multi-value twin (`crate::multi_return`), which returns result 0 and
/// stores the others through trailing out-pointers. Each temporary in
/// `temps` (with the twin's type for that result) receives a reference it
/// owns. `fallback` binds the same temporaries by calling `original` and
/// unpacking its tuple; codegen emits it instead of the twin call when
/// `original`'s name is not bound to its one definition, so the call
/// always reaches what CPython would call.
#[derive(Debug, Clone, PartialEq)]
pub struct AssignMulti {
    pub original: String,
    pub twin: String,
    pub args: Vec<MirExpr>,
    pub temps: Vec<(String, Ty)>,
    pub fallback: Vec<MirStmt>,
}

/// See [`MirExpr::RtCall`].
#[derive(Debug, Clone, PartialEq)]
pub struct RtCallExpr {
    pub sig: pycc_hir::builtins::BuiltinSig,
    pub args: Vec<MirExpr>,
}

/// [`MirExpr::Comprehension`]'s payload (#1254, D-250). The fields mirror
/// `MirStmt::ListCompAssign` and its siblings minus `target`: `var` is the
/// D-117 synthesized loop-variable name, scoped to this node -- codegen
/// gives it a slot of its own, and no enclosing-scope binding exists for
/// it -- and `var_ty` its resolved type.
#[derive(Debug, Clone, PartialEq)]
pub struct MirComprehension {
    pub var: String,
    pub var_ty: Ty,
    pub source: CompSource,
    pub cond: Option<MirExpr>,
    pub elt: MirCompElt,
}

/// How an object is read as an index ([`MirExpr::ObjectIndex`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexUse {
    /// A `range()` argument.
    Range,
    /// A list index.
    List,
    /// A `str` index.
    Str,
    /// A tuple index.
    Tuple,
    /// A slice bound (`xs[o:]`), clamped as CPython clamps one.
    Slice,
}

impl IndexUse {
    /// `pycc_ext_obj_coerce`'s kind code for this use.
    #[must_use]
    pub fn coerce_kind(self) -> u64 {
        match self {
            IndexUse::Range => 4,
            IndexUse::List => 5,
            IndexUse::Str => 6,
            IndexUse::Tuple => 7,
            IndexUse::Slice => 8,
        }
    }
}

/// [`MirExpr::ObjectIndex`] around `index` when it is a CPython object, the
/// run-time `int` check of an `int | float` (`pycc_rt_num_index`), and
/// `index` itself otherwise.
#[must_use]
pub fn index_operand(index: MirExpr, use_: IndexUse) -> MirExpr {
    match index.ty() {
        Ty::Object => MirExpr::ObjectIndex(Box::new(index), use_),
        Ty::Num => MirExpr::RtCall(Box::new(RtCallExpr {
            sig: pycc_hir::builtins::num_index_sig(),
            args: vec![index, MirExpr::IntLiteral(use_.coerce_kind() as i64)],
        })),
        _ => index,
    }
}

/// The CPython container a [`MirExpr::ObjDisplay`] builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjDisplayKind {
    Tuple,
    Set,
    Dict,
    /// A set display of three or more constants, built in the order
    /// CPython's folded frozenset constant gives it
    /// (`pycc_rt_set_constant_display`'s module documentation).
    ConstantSet,
}

impl ObjDisplayKind {
    /// The selector `pycc_ext_obj_build_display` switches on.
    pub fn shim_code(self) -> u64 {
        match self {
            Self::Tuple => 0,
            Self::Set => 1,
            Self::Dict => 2,
            Self::ConstantSet => 3,
        }
    }
}

/// A lowered comprehension's element expressions, by kind (#1254).
#[derive(Debug, Clone, PartialEq)]
pub enum MirCompElt {
    List(MirExpr),
    /// The element and, as for [`MirExpr::SetLiteral`], its
    /// [`SetElementOps`]: `None` for an `int` element, the instance class's
    /// hash and equality for a `set[C]` (#1344, Part 2 of #1336).
    Set(MirExpr, Option<Box<SetElementOps>>),
    Dict {
        key: MirExpr,
        value: MirExpr,
    },
}

impl MirComprehension {
    /// Whether a list, set or dict comprehension over a native source
    /// collects CPython objects (non-strict mode, where `pycc_types` admits
    /// it): no native container holds an `object` element (nor a native
    /// dict an `object` key or value), so it produces a fresh CPython
    /// `list`, `set` or `dict`, as one over a CPython source does.
    pub fn collects_objects(&self) -> bool {
        match &self.elt {
            MirCompElt::List(elt) | MirCompElt::Set(elt, None) => elt.ty() == Ty::Object,
            MirCompElt::Dict { key, value } => key.ty() == Ty::Object || value.ty() == Ty::Object,
            _ => false,
        }
    }

    /// Whether a list comprehension over a CPython object collects into a
    /// native list (`pycc_hir::object_comp_collects_natively`): its element
    /// is a native container or tuple, which has no CPython form.
    pub fn collects_natively(&self) -> bool {
        matches!(self.source, CompSource::Object(_))
            && matches!(&self.elt, MirCompElt::List(elt)
                if pycc_hir::object_comp_collects_natively(&elt.ty()))
    }

    /// The produced container type, derived from the element types exactly
    /// as the statement form's binding of `target` is. A comprehension over
    /// a CPython object ([`CompSource::Object`], Part 1 of #1255) produces a
    /// CPython `list` or `set`, which is the opaque [`Ty::Object`] whatever
    /// its elements are.
    pub fn ty(&self) -> Ty {
        if let CompSource::Object(_) = self.source {
            return match &self.elt {
                MirCompElt::List(elt) if self.collects_natively() => Ty::List(Box::new(elt.ty())),
                _ => Ty::Object,
            };
        }
        if self.collects_objects() {
            return Ty::Object;
        }
        match &self.elt {
            MirCompElt::List(elt) => Ty::List(Box::new(elt.ty())),
            MirCompElt::Set(elt, _) => Ty::Set(Box::new(elt.ty())),
            MirCompElt::Dict { key, value } => Ty::Dict(Box::new((key.ty(), value.ty()))),
        }
    }
}

/// [`MirExpr::InstanceOrder`]'s payload.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceOrderExpr {
    pub kind: OrderKind,
    /// The list ordered (a list display for `min(a, b, ...)`), or with
    /// `keyed` its `(key, element)` pairs (`pycc_ast::desugar::keys`).
    pub list: MirExpr,
    /// `xs` of a keyed `xs.sort(key=...)`, evaluated before `list`.
    pub target: Option<MirExpr>,
    /// The `reverse=` flag (a `bool`) of a sort.
    pub reverse: Option<MirExpr>,
    pub keyed: bool,
    /// The mangled comparison method: `__lt__` (a sort, `min`) or `__gt__`
    /// (`max`), or the other one with `swap` (`a < b` as `b.__gt__(a)`).
    pub callee: String,
    pub swap: bool,
    /// The element type (of the elements yielded; the keys are instances).
    pub elem: Ty,
}

impl InstanceOrderExpr {
    /// The sub-expressions, in evaluation order.
    pub fn operands(&self) -> Vec<&MirExpr> {
        let mut out: Vec<&MirExpr> = self.target.iter().collect();
        out.push(&self.list);
        out.extend(self.reverse.iter());
        out
    }
}

/// What an [`InstanceOrderExpr`] computes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderKind {
    Sorted,
    Sort,
    Min,
    Max,
}

impl OrderKind {
    /// The mode `pycc_rt_order_begin` takes.
    pub fn mode(self) -> i64 {
        match self {
            OrderKind::Sorted => 0,
            OrderKind::Sort => 1,
            OrderKind::Min => 2,
            OrderKind::Max => 3,
        }
    }
}

/// How a [`MirExpr::InstanceHash`] hashes its instance (#1335).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceHashVia {
    /// `object.__hash__`: the operand is the instance, hashed by address.
    Identity,
    /// A user `__hash__`: the operand is the call to it, whose `int` or
    /// `bool` result becomes the hash as CPython's `slot_tp_hash` does.
    Method,
}

/// The payload of [`MirExpr::ObjKeywordCall`] (Part 8 of #1371).
///
/// `call` is the positional half of the call, exactly the node the same
/// call without its keywords lowers to: a [`MirExpr::ObjMethodCall`] or a
/// [`MirExpr::ObjCall`]. `names` and `values` are the keyword arguments in
/// source order, one name per value; `pycc_parser` already refuses a
/// repeated name (`L0001`), so the names are distinct. Codegen evaluates the
/// callee (and, for a method call, looks the method up), then the
/// positional arguments, then the keyword values, which is CPython's order,
/// and marshals the values after the positional arguments in one vectorcall
/// argument array with the names as its `kwnames` tuple. The values follow
/// the positional argument rule (`pycc_types`' `check_object_call_args`).
#[derive(Debug, Clone, PartialEq)]
pub struct ObjKeywordCall {
    pub call: MirExpr,
    pub names: Vec<String>,
    pub values: Vec<MirExpr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MirFStringPart {
    Literal(String),
    Interpolation(Box<MirExpr>),
}

#[derive(Default)]
pub struct MirModule {
    pub items: Vec<MirItem>,
    /// #379 (PR-19): the module's class definitions, carried through to
    /// codegen so it can emit per-enum-member singleton init sequences.
    /// Codegen needs the `enum_members` table (member names and value
    /// types) to allocate and initialize each member's singleton instance
    /// at module-init time. Non-enum class defs are also carried, though
    /// codegen only reads `enum_members` from them today.
    pub class_defs: Vec<(String, pycc_hir::HirClassDef)>,
}

pub fn build(hir: &HirModule) -> MirModule {
    let mut scopes: Vec<HashMap<String, Ty>> = vec![HashMap::new()];
    // D-154 (Part 1 of #375): a plain `HashMap` clone of `hir.class_defs`,
    // built once and threaded read-only through every lowering call below
    // (`lower_item`/`lower_stmt`/`lower_expr`/`resolve_comp_source`) --
    // mirrors `scopes`' own "compute once in `build`, thread through every
    // recursive call" shape, but never mutated the way `scopes` is (a
    // class's declared shape does not change while lowering a module).
    let classes: HashMap<String, HirClassDef> = hir.class_defs.iter().cloned().collect();
    // Part 2 of #1026 (#1081): a foreign import's local name is a *value*
    // binding of type `object` for the rest of the module, so `lower_expr`'s
    // `HirExpr::Name` arm can resolve it through `lookup` like any other.
    // Part 1 needed no such binding -- the check phase refused every
    // expression-position read of a foreign name, so lowering never saw one
    // and `lookup`'s "pycc_types::check should have rejected this" panic was
    // correct. Part 2 admits `numpy.pi`, which makes the read reachable, and
    // without this bind the first admitted program would hit that panic.
    //
    // Bound eagerly, before any statement lowers, rather than at the import's
    // own position. That is sound only because the check phase refuses every
    // read this bind could answer wrongly, and PR 2a of #1081 had to *restore*
    // both halves of that after its review found the eager bind admitting
    // programs Part 1 refused: `pycc_types::module` no longer pre-seeds the
    // foreign name ahead of its own position, so a module-body read above the
    // `import` is `T0021`. A function body may read the name since #1316,
    // and a helper called above the `import` is admitted, as D-041 cannot
    // prove call order: the read goes through the global's initialized
    // flag, and `pycc_codegen`'s unbound-global branch raises the
    // `NameError` CPython raises there instead of trapping (`llvm.trap`,
    // rc 133). Part 1's shadowing rule (`C0001`, `docs/TYPE_SYSTEM.md`)
    // then guarantees no other top-level statement ever rebinds the name,
    // except the one admitted rebinding, an `except ImportError` fallback
    // of the same `try` (#1485), which stores a boxed `None` or another
    // foreign object into the same `object` slot -- so the eager bind
    // answers every admitted read with the same type a positional one
    // would. A function body reads it through the same
    // outward `lookup` walk, matching the constraint solver's own
    // per-function `foreign_objects` copy.
    // A function-local import (v0.8 accept 2) binds a local instead, at
    // its own `MirStmt::ForeignImport`.
    for import in hir.imports.iter().filter(|import| import.binds_module_name()) {
        if let ImportBinding::Foreign { local_name, .. } = import {
            bind(&mut scopes, local_name.clone(), Ty::Object);
        }
    }
    // First pass: register every function's mangled `$fn:name` signature
    // before lowering any item body -- mirrors `pycc_types::check`'s own
    // two-pass fix (D-038/D-039) so a forward reference, a sibling call, or
    // a recursive self-call all resolve to the right return type regardless
    // of where the callee's `def` appears in the module.
    for item in &hir.items {
        if let HirItem::Function {
            name,
            params,
            return_ty,
            ..
        } = item
        {
            bind(&mut scopes, format!("$fn:{name}"), return_ty.clone());
            // The function's value type, for a use of it as a value
            // (`MirExpr::MakeClosure`).
            bind(
                &mut scopes,
                format!("$callable:{name}"),
                Ty::Callable(Box::new((
                    params.iter().map(|(_, ty)| ty.clone()).collect(),
                    return_ty.clone(),
                ))),
            );
        }
    }
    // Lower module statements first, in source order, so the module scope is
    // complete before any function body is lowered. This mirrors
    // `pycc_types::check_with_signatures_all`'s D-041 three-pass contract:
    // top-level forward reads stay invalid because these statements are still
    // visited sequentially, while a function may read a global assigned after
    // its `def` because function bodies are evaluated only when called.
    let mut lowered: Vec<Option<MirItem>> = hir.items.iter().map(|_| None).collect();
    for (index, item) in hir.items.iter().enumerate() {
        if let HirItem::TopLevelStmt(stmt) = item {
            lowered[index] = Some(lower_item(item, &mut scopes, &classes));
            // Issue #769 (Part 2 of #747): the early-return continuation
            // shape has no meaning at true module scope (`return` cannot
            // syntactically appear outside a function, so
            // `apply_post_if_narrowing` never actually narrows anything
            // here), but this call keeps every sequential-statement-list
            // walk in this crate routed through the same narrowing-aware
            // shape uniformly, mirroring
            // `pycc_types::check_with_environment_all`'s own identical no-op
            // call at its module top level.
            object_narrow::apply_post_if_narrowing(stmt, &mut scopes, &classes);
        }
    }
    for (index, item) in hir.items.iter().enumerate() {
        if matches!(item, HirItem::Function { .. }) {
            lowered[index] = Some(lower_item(item, &mut scopes, &classes));
        }
    }
    let mut items: Vec<MirItem> = lowered
        .into_iter()
        .map(|item| item.expect("every HIR item is either a function or a top-level statement"))
        .collect();
    splice_foreign_imports(&mut items, &hir.imports);
    let mut module = MirModule {
        items,
        class_defs: hir.class_defs.clone(),
    };
    str_scan::rewrite(&mut module);
    unpack_display::rewrite(&mut module);
    optional_str::rewrite(&mut module);
    multi_return::rewrite(&mut module);
    // #1337 (D-254): every user-method call runs the body its static
    // receiver class resolves to; re-derived independently of the routing
    // sites and checked on every debug (so every test) build.
    #[cfg(debug_assertions)]
    verify_receiver::verify(&module, &classes);
    module
}

/// Part 1 of #1026: inserts a [`MirItem::ForeignImport`] into `items` for
/// every foreign binding, each at the position it occupied among the module
/// statements (its `ForeignImportSite::Item` index, rebased onto the linked
/// program by `pycc_hir::program::link`). A `ForeignImportSite::Block`
/// import is skipped: it runs as the `MirStmt::ForeignImport` statement its
/// block lowers to (#1291).
///
/// Insertion is by ascending index with a running offset, so two imports
/// recorded at the same or at increasing positions both land in source
/// order: the `n`-th insertion shifts every later recorded index by `n`.
/// `HirModule::imports` is already in source order -- `module::lower_module`
/// appends each statement's bindings as it walks the body, and `link`
/// concatenates modules in dependency order -- so the indices arrive sorted
/// and `sort` is unnecessary; the offset alone is what keeps them correct.
///
/// An index past the end of `items` cannot occur: it is the item count at
/// the moment the import lowered, and every stage between that moment and
/// this one either appends items or, when it changes the list the positions
/// refer to, recomputes them against the list it produces --
/// `pycc_types::monomorphize`, which drops each original generic and each
/// protocol-parameter function, and `pycc_types::enum_lower`'s
/// `unroll_enum_loops`, which expands one top-level enum loop into one item
/// per member. `insert` would panic rather than misplace
/// the call if that invariant were ever broken, which is the failure this
/// splice wants -- PR 1c of #1080 review finding 1 is exactly that panic,
/// observed before `monomorphize` did the recomputation.
fn splice_foreign_imports(items: &mut Vec<MirItem>, imports: &[ImportBinding]) {
    let mut inserted = 0usize;
    for binding in imports {
        let ImportBinding::Foreign {
            local_name,
            module_path,
            from,
            site: ForeignImportSite::Item(item_index),
            ..
        } = binding
        else {
            continue;
        };
        items.insert(
            item_index + inserted,
            MirItem::ForeignImport {
                local_name: local_name.clone(),
                module_path: module_path.clone(),
                from: from.clone(),
            },
        );
        inserted += 1;
    }
}

fn lower_item(
    item: &HirItem,
    scopes: &mut Vec<HashMap<String, Ty>>,
    classes: &HashMap<String, HirClassDef>,
) -> MirItem {
    match item {
        HirItem::Function {
            name,
            params,
            return_ty,
            body,
        } => {
            // #433: extract the class name from a mangled
            // `<ClassName>.<method>` name so `lower_expr`'s `Super` arm
            // can resolve the next class in the MRO. A top-level function
            // name contains no `.`, so `current_class` is `None` for those.
            let current_class: Option<&str> = item_anchor_class(name, classes);
            let mut frame: HashMap<String, Ty> = params.iter().cloned().collect();
            if matches!(return_ty, Ty::Generator(_)) {
                frame.insert(generator::GENERATOR_KEY.to_string(), return_ty.clone());
            }
            if matches!(return_ty, Ty::Tuple(_)) {
                frame.insert(stmt::RETURN_KEY.to_string(), return_ty.clone());
            }
            scopes.push(frame);
            let mut body = lower_stmt_sequence(body, scopes, classes, current_class);
            scopes.pop();
            let frame_name = source_frame_name(name);
            set_frame_function(&mut body, &frame_name);
            MirItem::Function {
                name: name.clone(),
                params: params.clone(),
                return_ty: return_ty.clone(),
                body,
            }
        }
        HirItem::TopLevelStmt(stmt) => {
            let mut stmt = lower_stmt(stmt, scopes, classes, None);
            set_frame_function(std::slice::from_mut(&mut stmt), "<module>");
            MirItem::TopLevelStmt(stmt)
        }
    }
}

/// Fills every [`MirStmt::Raise`]/[`MirStmt::RaiseFrom`] reachable from
/// `body` with `frame_name` -- the enclosing function's source name, or
/// `"<module>"` for a top-level statement (#707). Run once per item by
/// [`lower_item`] after lowering completes, rather than threading
/// `frame_name` as a parameter through every statement-lowering function's
/// own recursive descent (see [`MirStmt::Raise::frame_function`]'s doc
/// comment for why): `frame_name` never varies within one item, so a single
/// post-order walk over the already-built tree is strictly cheaper and
/// cannot desynchronize from `lower_stmt`'s own call graph the way a
/// parallel threaded parameter could.
///
/// Recurses into every `MirStmt` variant that nests further statements --
/// `If`, `While`, every `For*` loop, `Try`/`TryStar` (body, each handler's
/// body, `orelse`, `finalbody`), and `Seq` -- so a `raise` anywhere inside a
/// loop body, a handler, or a `match`'s lowered `if` chain (`Seq`/`If`) is
/// reached, not only a top-level statement in `body` itself.
fn set_frame_function(body: &mut [MirStmt], frame_name: &str) {
    for stmt in body {
        match stmt {
            MirStmt::Raise { frame_function, .. } => {
                frame_function.clear();
                frame_function.push_str(frame_name);
            }
            MirStmt::RaiseFrom { frame_function, .. } => {
                frame_function.clear();
                frame_function.push_str(frame_name);
            }
            MirStmt::If { body, orelse, .. } => {
                set_frame_function(body, frame_name);
                set_frame_function(orelse, frame_name);
            }
            MirStmt::While { body, .. }
            | MirStmt::ForRange { body, .. }
            | MirStmt::ForList { body, .. }
            | MirStmt::ForObject { body, .. }
            | MirStmt::ForDict { body, .. }
            | MirStmt::ForSet { body, .. }
            | MirStmt::ForGen { body, .. } => set_frame_function(body, frame_name),
            MirStmt::Seq(stmts) => set_frame_function(stmts, frame_name),
            MirStmt::AssignMulti(multi) => set_frame_function(&mut multi.fallback, frame_name),
            MirStmt::LoopElse { loop_stmt, orelse } => {
                set_frame_function(std::slice::from_mut(loop_stmt.as_mut()), frame_name);
                set_frame_function(orelse, frame_name);
            }
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
                set_frame_function(body, frame_name);
                for handler in handlers {
                    set_frame_function(&mut handler.body, frame_name);
                }
                set_frame_function(orelse, frame_name);
                set_frame_function(finalbody, frame_name);
            }
            MirStmt::ExprStmt(_)
            | MirStmt::Yield(_)
            | MirStmt::GenReturn(_)
            | MirStmt::Assign { .. }
            | MirStmt::NoOp
            | MirStmt::ReleaseName(_)
            | MirStmt::Unreachable
            | MirStmt::DictSet { .. }
            | MirStmt::BufferSet { .. }
            | MirStmt::ListSet { .. }
            | MirStmt::ListCompAssign { .. }
            | MirStmt::DictCompAssign { .. }
            | MirStmt::SetCompAssign { .. }
            | MirStmt::Return(_)
            | MirStmt::ReturnBufferSlice { .. }
            | MirStmt::SetOut { .. }
            | MirStmt::AttrSet { .. }
            | MirStmt::ObjDelSlice { .. }
            | MirStmt::ObjSetSlice { .. }
            | MirStmt::ObjAttrSet { .. }
            | MirStmt::ObjDelAttr { .. }
            | MirStmt::ObjSetItem { .. }
            | MirStmt::ObjDelItem { .. }
            | MirStmt::ObjRaise { .. }
            | MirStmt::ForeignImport { .. }
            | MirStmt::Break
            | MirStmt::Continue
            | MirStmt::Reraise => {}
        }
    }
}

fn bind(scopes: &mut [HashMap<String, Ty>], name: String, ty: Ty) {
    scopes
        .last_mut()
        .expect("at least one scope is always present")
        .insert(name, ty);
}

fn bind_variable(scopes: &mut [HashMap<String, Ty>], name: String, ty: Ty) {
    scopes
        .last_mut()
        .expect("at least one scope is always present")
        .entry(name)
        .or_insert(ty);
}

fn lookup(scopes: &[HashMap<String, Ty>], name: &str) -> Ty {
    scopes
        .iter()
        .rev()
        .find_map(|scope| scope.get(name).cloned())
        .unwrap_or_else(|| panic!("pycc_mir: internal error: `{name}` has no recorded type -- pycc_types::check should have rejected this HIR before it reached pycc_mir"))
}

/// Issue #769 (Part 2 of #747): the sentinel key `narrowed_scope_key`
/// pushes into the top `scopes` frame to record "`name` is currently
/// narrowed to its `Optional`'s inner type" -- mirroring `lower_expr`'s
/// existing `format!("$fn:{callee}")` sentinel-key pattern (used to record
/// "this name is a user-defined function", also inside the same `scopes`
/// stack rather than through a separate threaded parameter). `$` never
/// appears in a real Python identifier, so a sentinel key can never
/// collide with an actual variable name.
fn narrowed_scope_key(name: &str) -> String {
    format!("$narrowed:{name}")
}

/// Issue #769 (Part 2 of #747): records that `name` is narrowed to `inner`
/// for the remainder of the current top `scopes` frame -- called by
/// `lower_stmt`'s `If` arm before lowering the narrowed branch, and popped
/// with [`kill_narrowing`] immediately after.
fn push_narrowing(scopes: &mut [HashMap<String, Ty>], name: &str, inner: Ty) {
    scopes
        .last_mut()
        .expect("at least one scope is always present")
        .insert(narrowed_scope_key(name), inner);
}

/// Issue #769 (Part 2 of #747): clears `name`'s narrowing sentinel from the
/// top `scopes` frame -- called both to pop a narrowed region
/// ([`push_narrowing`]'s counterpart) and, independently, whenever `name`
/// is reassigned inside a narrowed region (`lower_stmt`'s `Assign`/
/// `AnnAssign` arms) so a stale `MirExpr::OptionalUnwrap` is never emitted
/// for a read that follows the reassignment.
///
/// D-068 re-review of #780 (seventh round): only clears the sentinel in
/// `scopes.last()`, while [`narrowed_ty`] searches the whole stack
/// innermost-to-outward. This is sound today only because `scopes` ever
/// gains a second frame in exactly one place (`lower_item`'s `Function`
/// arm) and no narrowing sentinel is ever live across that push --
/// functions are lowered in a separate pass after all module-level
/// statements, and this HIR has no nested-function construct. If a future
/// change adds another `scopes.push` while an outer-frame sentinel is
/// still live, that sentinel would become unkillable through this
/// function (which only touches the top frame) while remaining visible to
/// [`narrowed_ty`]'s full-stack search -- silently reintroducing the same
/// stale-narrowing class every earlier round of this review fixed, through
/// a different mechanism. Any new `scopes.push` site must account for this
/// before it can carry a live narrowing sentinel across it.
fn kill_narrowing(scopes: &mut [HashMap<String, Ty>], name: &str) {
    scopes
        .last_mut()
        .expect("at least one scope is always present")
        .remove(&narrowed_scope_key(name));
}

/// Issue #769 follow-up (D-068 re-review of #780, third round): the MIR
/// counterpart of `pycc_types::narrow::apply_kill_prescan` -- drops every
/// name `body` reassigns anywhere within it (per `pycc_hir::killed_names`)
/// from `scopes`' top-frame narrowing sentinels, for `body`'s entire
/// extent rather than only from the kill's own source position onward.
///
/// Required for MIR to stay consistent with the checker's identical fix:
/// without this, `pycc_types::check` would reject a re-entrant read (a
/// loop body, or an `except` handler reached partway through the `try`
/// body it guards) that the checker's own `narrow::apply_kill_prescan`
/// drops the narrowing for, but MIR lowering -- reached only when
/// `pycc_types::check` already accepted the same HIR for some *other*,
/// still-narrowable read in the same body -- would still emit an
/// unconditional `MirExpr::OptionalUnwrap` for a read this checker fix
/// now no longer allows to be narrowed. Called from the same two site
/// classes as the checker's: `stmt::lower_loop_body` (every
/// `While`/`ForRange`/`ForList` lowering path, all funneled through that
/// one shared helper) and `stmt::try_stmt::lower_try` (each handler body,
/// prescanned against the *try body's* kill set before lowering, and the
/// `finally` body against every path's).
fn apply_kill_prescan(scopes: &mut [HashMap<String, Ty>], body: &[HirStmt]) {
    for name in pycc_hir::killed_names(body) {
        kill_narrowing(scopes, &name);
    }
}

/// Issue #769 (Part 2 of #747): `Some(inner)` when `name` is currently
/// narrowed to `inner` (an `Optional`'s inner type), `None` otherwise.
/// Consulted only by `lower_expr`'s `HirExpr::Name` arm, exactly mirroring
/// `pycc_types::env::Environment::narrowed_ty`'s own "reads consult the
/// overlay, assignment targets never do" split -- `lower_stmt`'s own
/// `Assign`/`AnnAssign` arms bind `target`'s *real* type via `bind`/
/// `bind_variable` directly and never call this.
pub(crate) fn narrowed_ty(scopes: &[HashMap<String, Ty>], name: &str) -> Option<Ty> {
    scopes
        .iter()
        .rev()
        .find_map(|scope| scope.get(&narrowed_scope_key(name)).cloned())
}

/// Issue #769 (Part 2 of #747): lowers a sequential list of statements --
/// a function body, or any `if`/`while`/`for` body -- applying the
/// early-return continuation narrowing ([`object_narrow::apply_post_if_narrowing`]) after
/// each one. The narrowing-aware replacement for the raw `stmts.iter()
/// .map(|s| lower_stmt(s, scopes, classes, current_class)).collect()`
/// pattern every sequential body lowering in this crate used before this
/// issue (mirroring `pycc_types::narrow::check_stmt_sequence`'s identical
/// role at the checker layer).
///
/// Callers processing the *top-level* sequence of a function or module body
/// call this directly, since narrowing established there is meant to persist
/// for the rest of that same sequence. A *nested* body (an `if`/`while`/`for`
/// arm, a `try`/`except` handler, a `match` arm, ...) must instead go through
/// [`lower_scoped_body`], which isolates whatever this function accumulates
/// in `scopes`' top frame so it cannot leak past the nested body's own close
/// -- see that function's doc comment for the soundness rationale.
fn lower_stmt_sequence(
    stmts: &[HirStmt],
    scopes: &mut Vec<HashMap<String, Ty>>,
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> Vec<MirStmt> {
    let mut out = Vec::with_capacity(stmts.len());
    for stmt in stmts {
        out.push(lower_stmt(stmt, scopes, classes, current_class));
        object_narrow::apply_post_if_narrowing(stmt, scopes, classes);
    }
    out
}

/// Issue #769 (Part 2 of #747): captures the narrowing-sentinel subset
/// (every `$narrowed:{name}` key, see [`narrowed_scope_key`]) of `scopes`'
/// top frame, for later restoration via [`restore_narrowing`].
fn narrowing_snapshot(scopes: &[HashMap<String, Ty>]) -> HashMap<String, Ty> {
    scopes
        .last()
        .expect("at least one scope is always present")
        .iter()
        .filter(|(key, _)| key.starts_with("$narrowed:"))
        .map(|(key, ty)| (key.clone(), ty.clone()))
        .collect()
}

/// Issue #769 (Part 2 of #747): replaces `scopes`' top frame's entire
/// narrowing-sentinel subset with `snapshot` -- the counterpart of
/// [`narrowing_snapshot`], undoing every `push_narrowing`/`kill_narrowing`
/// mutation made since the snapshot was taken, whether from a direct
/// `push_narrowing` call or indirectly from a nested
/// [`object_narrow::apply_post_if_narrowing`] hit deeper in the body being restored from.
fn restore_narrowing(scopes: &mut [HashMap<String, Ty>], snapshot: HashMap<String, Ty>) {
    let top = scopes
        .last_mut()
        .expect("at least one scope is always present");
    top.retain(|key, _| !key.starts_with("$narrowed:"));
    top.extend(snapshot);
}

/// #1476: the end state of the branch an `if` guard narrows `name` in, with
/// the guard's own narrowing replaced by `name`'s pre-`if` entry in
/// `before` (or none) -- unless `branch` rebinds `name`, whose kill already
/// decided its end state. The MIR-side twin of
/// `pycc_types::narrow::end_branch_narrowing`.
fn end_branch_narrowing(
    before: &HashMap<String, Ty>,
    end: &mut HashMap<String, Ty>,
    name: &str,
    branch: &[HirStmt],
) {
    if pycc_hir::killed_names(branch)
        .iter()
        .any(|killed| killed == name)
    {
        return;
    }
    let key = narrowed_scope_key(name);
    match before.get(&key) {
        Some(outer) => end.insert(key, outer.clone()),
        None => end.remove(&key),
    };
}

/// Blocker fix (D-068 review of #780): the MIR-side counterpart of
/// `pycc_types::narrow::join_narrowed`. A name stays narrowed to `ty` after
/// two or more possible-path narrowing snapshots are reconciled only if
/// *every* supplied snapshot narrows it to that exact `ty`; a snapshot that
/// killed it (or never had it) drops it from the intersection entirely.
/// Operates on [`narrowing_snapshot`]-shaped maps (`$narrowed:{name}` ->
/// inner type), so the result can be handed directly to
/// [`restore_narrowing`]. See that Rust doc's checker-side twin for the
/// full soundness rationale -- this is the identical algorithm, kept as an
/// independent copy rather than a shared crate dependency because
/// `pycc_mir` does not (and per the existing `pycc_hir::optional_none_test`
/// precedent, should not) depend on `pycc_types`.
///
/// Every call site has at least one snapshot by construction (an `if`'s
/// body/orelse end-state, a loop's pre-body/post-body end-state, or a
/// `try`'s non-empty list of fall-through end states), so
/// `first` is taken separately from `rest` rather than a possibly-empty
/// slice -- that keeps the join total without an unreachable empty-input
/// branch.
fn join_narrowed(
    first: &HashMap<String, Ty>,
    rest: &[&HashMap<String, Ty>],
) -> HashMap<String, Ty> {
    let mut joined: HashMap<String, Ty> = first.clone();
    for snapshot in rest {
        joined.retain(|key, ty| snapshot.get(key) == Some(ty));
    }
    joined
}

/// Issue #769 (Part 2 of #747): lowers a *nested* statement body (an
/// `if`/`while`/`for` arm, a `try`/`except`/`finally` handler, a `match`
/// arm, ...) with its own narrowing state isolated from the enclosing
/// sequence, mirroring `pycc_types::narrow`'s own per-branch `Environment`
/// clone-and-discard design (see `narrow.rs`'s module doc comment) one layer
/// down.
///
/// `scopes`' single function-level frame (pushed once at function entry,
/// never per-block -- see this module's existing scope-stack convention) is
/// shared, mutable state threaded through every nested body, unlike the
/// checker's own per-branch `Environment` *clones*. Without this wrapper, a
/// narrowing fact established by [`object_narrow::apply_post_if_narrowing`] deep inside one
/// branch (e.g. a nested early-return guard) would persist in that shared
/// frame past the branch's own close and incorrectly narrow reads in a
/// sibling branch or in code after the enclosing construct entirely.
/// [`narrowing_snapshot`]/[`restore_narrowing`] recreate the clone-and-discard
/// effect cheaply, without cloning the rest of `scopes`.
///
/// `narrow` optionally pushes one additional in-branch narrowing fact (the
/// enclosing `if`'s own recognized `is`/`is not None` test, exactly the
/// role `push_narrowing`/`kill_narrowing` played before this wrapper existed)
/// before lowering `stmts` -- `restore_narrowing` undoes this push too, since
/// the snapshot is taken before it is applied.
///
/// Blocker fix (D-068 review of #780): also returns the narrowing-sentinel
/// subset as it stood *immediately after* lowering `stmts`, before the
/// isolating restore -- this branch's "ending" narrowed state. Isolation
/// during lowering (so a nested body's own narrowing never leaks into a
/// sibling while both are being lowered) and reporting that branch's own
/// outcome to the caller are two different concerns: unconditionally
/// discarding the ending state (the pre-fix behavior) silently reverted a
/// same-branch `kill_narrowing` the moment this nested body closed, even
/// when nothing else contradicted it. A caller that needs the
/// join-reconciled effect of one or more sibling branches (`HirStmt::If`'s
/// body+orelse pair, a loop's single body joined against "the loop ran zero
/// times") combines the returned end-states with [`join_narrowed`] and
/// applies the result back onto `scopes` via a second, deliberate
/// `restore_narrowing` call of its own -- see `stmt.rs`'s `If`/`While`/
/// `ForRange`/`ForList` arms, and `stmt::try_stmt`, which since #1476
/// continues `else` from the body's end state and joins every
/// fall-through path's end state after the `try`. A caller that does not
/// need that (a `match` case) simply ignores the second return value,
/// preserving the prior "restore to entry state" behavior exactly.
fn lower_scoped_body(
    stmts: &[HirStmt],
    scopes: &mut Vec<HashMap<String, Ty>>,
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
    narrow: Option<(&str, Ty)>,
) -> (Vec<MirStmt>, HashMap<String, Ty>) {
    let snapshot = narrowing_snapshot(scopes);
    if let Some((name, inner)) = narrow {
        push_narrowing(scopes, name, inner);
    }
    let out = lower_stmt_sequence(stmts, scopes, classes, current_class);
    let end_state = narrowing_snapshot(scopes);
    restore_narrowing(scopes, snapshot);
    (out, end_state)
}

/// #436/#432: Looks up a class in the `classes` table by its MRO entry
/// name, panicking with a consistent internal-error message if the class
/// is not registered. Centralizing this lookup ensures the defensive
/// panic is covered by a single test rather than duplicated across every
/// MRO walk (where later walks are unreachable after the first walk
/// panics on a ghost class).
fn mro_class_def<'a>(
    mro_class: &str,
    classes: &'a HashMap<String, HirClassDef>,
) -> &'a HirClassDef {
    classes.get(mro_class).unwrap_or_else(|| {
        panic!(
            "pycc_mir: internal error: class `{mro_class}` in MRO has no registered \
             HirClassDef -- pycc_types::check should have rejected this HIR before \
             it reached pycc_mir"
        )
    })
}

#[cfg(test)]
mod tests;
