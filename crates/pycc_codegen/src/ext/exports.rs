//! The shape a public export presents to the generated C wrapper (#1050):
//! the C-legal spelling of a pycc name, the `pycc_ext_thunk_<name>` symbol,
//! which names are exportable, and the parameter and result slots of an
//! export thunk. The driver (`src/ext_build.rs`) renders C against exactly
//! these, and `crate::ext_thunk` emits the LLVM side.

use super::EXT_THUNK_PREFIX;
use pycc_mir::Ty;

/// The C-legal spelling of a possibly-dotted pycc name.
///
/// A method reaches MIR under a dotted name (`Grid.scale.static`), and two
/// of the places that name is used are *C identifiers*: the
/// `extern void *fnptr_<name>;` declaration `pycc::ext_build`'s
/// `wrapper_for` emits, and the `pycc_ext_wrap_<name>` /
/// `pycc_ext_thunk_<name>` symbols. `extern void *fnptr_Grid.scale.static;`
/// is not accepted by any C compiler, so the dotted spelling has to be
/// encoded -- and `src/ext_build.rs`'s rebind-dedup comment records that a
/// *duplicate* export name makes clang reject the generated `.inc`
/// outright, so the encoding has to be injective as well as legal.
///
/// The encoding: a name with no `.` is returned unchanged; otherwise the
/// result is `"0m"` followed by, for each `.`-separated segment in order,
/// the segment's byte length in decimal, then `"_"`, then the segment. So
/// `f` stays `f` and `Grid.scale.static` becomes `0m4_Grid5_scale6_static`.
///
/// **It is injective, and that is argued rather than fixture-tested.**
/// Every dot-free name reaching this function is either a Python identifier
/// or carries the compiler-generated `0gen_` prefix
/// (`pycc_types::monomorphize`), and neither can begin with `0m` -- the
/// premise is stated this way rather than as "Python identifiers cannot
/// begin with a digit", because a dot-free name like `0gen_make__T_int` is
/// a real identity-branch input that *does* begin with a digit. So the
/// identity branch's outputs never collide with a `0m...` output. Within
/// the `0m` branch the encoding is length-prefixed and therefore uniquely
/// decodable, so two distinct dotted names cannot mangle alike. `.` -> `_`
/// and `.` -> `__` both fail this argument, the second one silently: a
/// module-level `def Grid__scale__static` would collide with
/// `Grid.scale.static`.
///
/// Every use site prefixes the result (`fnptr_`, `fnname_`,
/// `pycc_ext_thunk_`, `pycc_ext_wrap_`), so the leading digit never starts
/// a C identifier. Because the dot-free case is the identity, every symbol
/// the compiler emitted before methods became exportable is byte-identical.
///
/// This is the one canonical implementation: `pycc::ext_build` calls it
/// rather than reimplementing it, exactly as it already calls
/// [`ext_thunk_symbol`].
#[must_use]
pub fn mangle_ext_name(name: &str) -> String {
    if !name.contains('.') {
        return name.to_string();
    }
    let mut out = String::from("0m");
    for segment in name.split('.') {
        out.push_str(&segment.len().to_string());
        out.push('_');
        out.push_str(segment);
    }
    out
}

/// The external symbol `name`'s scalar-only `ext` export thunk is emitted
/// under.
///
/// `name` is mangled through [`mangle_ext_name`] first, so a method's thunk
/// is a legal C identifier the generated `extern` declaration can name. The
/// mangling is the identity for a dot-free name, so every module-level
/// function's thunk symbol is unchanged.
#[must_use]
pub fn ext_thunk_symbol(name: &str) -> String {
    format!("{EXT_THUNK_PREFIX}{}", mangle_ext_name(name))
}

/// Whether `name` is a name D-244 rule 1 can export at all, disregarding
/// its signature.
///
/// The tests are exactly `pycc::ext_build::collect_exports`' own *lexical*
/// verdict, and this is their one canonical home so the two sides of the
/// seam cannot drift: a wrapper generated for a name codegen declined to
/// emit a thunk for links cleanly and crashes on the first call.
/// `src/ext_build_tests/exports.rs` carries a test pinning the two equal
/// over a shared table of names, because the drift is otherwise silent --
/// `wrapper_for` picks the thunk `extern` or the `fnptr_` `extern` from
/// [`ext_thunk_required`], so a disagreement emits the wrong C declaration
/// for a `tuple`-carrying method.
///
/// Two name classes are answered `true` beyond that lexical verdict: the
/// PEP 562 module hooks `__getattr__` and `__dir__` (#1467), which the
/// driver admits through `is_module_hook`, and `<Class>.<slot dunder>`
/// (#1427), which it admits through `is_slot_dunder_method`, neither through
/// `classify_export_name`. So the parity test compares this function with
/// `classify_export_name(name).is_some() || is_module_hook(name) ||
/// is_slot_dunder_method(name)` -- the last for #1427's comparison and
/// `__hash__` slots, admitted for any non-empty class segment.
///
/// The public-name test is D-038's predicate, spelled out rather than
/// delegated to `pycc_hir::is_public_name` because this crate deliberately
/// does not depend on `pycc_hir` -- it sees only `pycc_mir`'s re-export of
/// `Ty`. The body there is `!name.starts_with('_')` and nothing else; if it
/// ever grows a case, this copy must grow with it.
///
/// **Dotted names are no longer refused wholesale.** A method reaches MIR
/// under `<Class>.<method>` and the suffixed spellings
/// `<Class>.<method>.static`, `<Class>.<method>.classmethod` and
/// `<Class>.<property>.setter` (`pycc_hir::class`'s mangling). This admits
/// the `.static` and `.classmethod` spellings and -- since #1145 -- the
/// bare `<Class>.<method>` one, with every segment public.
/// `<Class>.<property>.setter` stays refused: a `@property` is attribute
/// syntax on the host side, never a method table entry.
///
/// The bare spelling covers the three `MethodKind`s `Regular`,
/// `PropertyGetter` and `AbstractMethod` at once, and nothing here can tell
/// them apart. Admitting it is deliberate rather than an approximation:
/// widening this mirror is what makes it a **superset** of the driver's
/// admitted set again, and a superset is the safe direction. The driver
/// narrows the bare spelling back down with filters that read
/// `HirModule::class_defs`; leaving this side refusing it would instead
/// make `ext_thunk_required` answer `false` for a `tuple`-carrying instance
/// method, so the wrapper would emit the `fnptr_` cast form -- measured to
/// fault (SIGBUS) on aarch64-apple-darwin for an out-pointer signature.
///
/// **This verdict is purely lexical, and must stay so.** The function
/// receives a bare `&str` and this crate cannot see `pycc_hir`, so a
/// verdict that consulted `HirModule::class_defs` would have no
/// mirror-comparable form here and the parity test would stop being
/// well-formed. The driver layers its exception-class exclusion *on top of*
/// this verdict rather than inside it, which makes this mirror a
/// **superset** of the driver's admitted set: at worst a thunk is emitted
/// for a name no wrapper calls, which is dead code -- never the link error
/// the drift above would be.
///
/// A monomorphized generic specialization carries the `0gen_` prefix and
/// has no `fnptr_` global to dispatch through, so it stays refused; that
/// test is applied to the whole name *before* the split and takes
/// precedence, as cheap defense in depth. It is not a live hazard: the real
/// specialization shapes put the substitution suffix last
/// (`0gen_<Class>.<method>__<P>_<C>`), so a `0gen_` name's last segment is
/// never `static`.
///
/// A receiver-exact copy of an inherited body (#1337, D-254) needs no case
/// of its own: a primary copy is spelled exactly as the receiver's own
/// definition would be (`C.m`, `C.k.classmethod`) and is judged as one, and
/// a `super()`-target copy (`C.m.0super_D`, plus a kind suffix for a setter
/// or classmethod) carries a third segment that is
/// neither `static` nor `classmethod`, so it is refused here and by the
/// driver alike -- it is only ever called from another copy, never hosted.
#[must_use]
pub fn is_ext_exportable_name(name: &str) -> bool {
    // `0gen_` specializations and the `0m` qualified globals of another
    // project module (v0.3 accept 1) are compiler spellings, never exports.
    if name.starts_with('0') {
        return false;
    }
    // The compiled entry of a published signature
    // (`pycc_ast::ext_signatures`), which the driver exports.
    if name.starts_with("_pycc_sig_") {
        return true;
    }
    // A method body the non-strict fallback lifted out of a class CPython
    // creates (`src/fallback/lift.rs`, `LIFT_PREFIX`), which the driver
    // exports for its Python forwarder.
    if name.starts_with("__pycc_lift") && !name.contains('.') {
        return true;
    }
    // #1467: the two PEP 562 module hooks, which the driver publishes from
    // the entry module although D-038's predicate refuses every dunder. The
    // driver admits only the entry module's own `def`, which this lexical
    // mirror cannot see, so a helper module's hook gets a dead thunk -- the
    // safe superset direction described above.
    if name == "__getattr__" || name == "__dir__" {
        return true;
    }
    if is_slot_dunder_method_name(name) {
        return true;
    }
    let mut segments = name.split('.');
    // `str::split` always yields at least one segment, so the fallback is
    // unreachable rather than a second refusal path; an empty first segment
    // is refused on the next line either way, which is what the driver's
    // mirror does with its own empty-segment guard.
    let first = segments.next().unwrap_or("");
    // A private module-level function, which the driver publishes in
    // non-strict mode only: admitted here in every mode, the superset
    // direction. A compiler name (`_pycc_...`) never is.
    if !name.contains('.')
        && first.starts_with('_')
        && !first.starts_with("__")
        && !first.starts_with("_pycc_")
    {
        return true;
    }
    // A private class's methods (`_Node.render`), which the driver
    // publishes in non-strict mode only: admitted in every mode, the
    // superset direction.
    let private_class = first.starts_with('_') && !first.starts_with("__") && !first.starts_with("_pycc_");
    if (first.starts_with('_') && !private_class) || first.is_empty() {
        return false;
    }
    let Some(second) = segments.next() else {
        // A dot-free name: a module-level function, admitted by D-038's
        // predicate alone.
        return true;
    };
    // A private instance method (`<Class>._m`, not `__m`), which the
    // driver publishes in non-strict mode only: admitted here in every
    // mode, the superset direction.
    let private = second.starts_with('_') && !second.starts_with("__");
    if (second.starts_with('_') && !private) || second.is_empty() {
        return false;
    }
    if private {
        return segments.next().is_none();
    }
    match segments.next() {
        // `<Class>.<method>` -- `Regular`, `PropertyGetter` or
        // `AbstractMethod`, indistinguishable here and all admitted as the
        // superset the driver narrows (#1145).
        None => true,
        Some(kind) => {
            // The only four-segment name is a `super()`-target copy of a
            // setter or classmethod (`C.k.0super_D.classmethod`, D-254),
            // which is never hosted: a class nested in a class or a
            // function is refused by `pycc_hir` (`stmt.rs`, `class.rs`), so
            // no `A.B.method` name exists. Refusing every fourth segment is
            // the fail-closed reading.
            segments.next().is_none() && (kind == "static" || kind == "classmethod")
        }
    }
}

/// The special methods the `--ext` driver installs as a type slot rather
/// than a `PyMethodDef` row (#1427): the six rich comparisons and
/// `__hash__`. Mirrors `src/ext_build/richcompare.rs`'s `SLOT_DUNDERS`;
/// the parity test in `src/ext_build_tests/exports.rs` keeps the two equal.
const SLOT_DUNDERS: [&str; 9] = [
    "__lt__", "__le__", "__eq__", "__ne__", "__gt__", "__ge__", "__hash__", "__repr__", "__str__",
];

/// #1427: whether `name` is `<Class>.<slot dunder>` -- a comparison or
/// `__hash__` method the driver wraps for a `tp_richcompare`/`tp_hash`
/// slot. The class segment may be private: an unpublished class whose
/// instance crosses gets a carrier type carrying the same slots, so its
/// wrapper needs the thunk exactly as a published one's does. A `0gen_`
/// name is refused by the caller before this runs.
fn is_slot_dunder_method_name(name: &str) -> bool {
    let mut segments = name.split('.');
    let class = segments.next().unwrap_or("");
    let Some(method) = segments.next() else {
        return false;
    };
    !class.is_empty() && segments.next().is_none() && SLOT_DUNDERS.contains(&method)
}

/// The boundary slots a value of type `ty` occupies when it crosses the
/// `ext` seam: a `tuple`'s elements, in order, or the type itself.
///
/// D-116 fixes a tuple's arity and restricts its elements to `int`, `bool`
/// and `float` (`pycc_hir`'s `check_tuple_element_ty` rejects anything else
/// with `T0039`, and `tuple[int, ...]`/`tuple[()]` with `T0053`), so the
/// returned slice is always non-empty and never itself contains a tuple.
///
/// A *boxed* tuple (`pycc_hir::tuple_is_boxed`: an element other than
/// `int`/`bool`/`float`, such as `tuple[str, int]`) is not an aggregate at
/// all but one pointer to its runtime list, so it occupies one slot of its
/// own like a `list`. No export carries one -- the driver refuses it with
/// `C0003` -- but codegen emits thunks for every exportable *name*, which
/// includes a non-strict function whose return is typed from its body and
/// that the driver leaves unpublished ([`ext_thunk_required`]).
#[must_use]
pub fn ext_boundary_slots(ty: &Ty) -> &[Ty] {
    match ty {
        Ty::Tuple(elems) if is_flat_tuple(ty) => elems.as_slice(),
        _ => std::slice::from_ref(ty),
    }
}

/// Whether `ty` is a tuple that crosses the seam flattened into its
/// elements: an unboxed (struct) tuple. A boxed tuple is a single pointer
/// ([`ext_boundary_slots`]).
#[must_use]
pub fn is_flat_tuple(ty: &Ty) -> bool {
    matches!(ty, Ty::Tuple(elems) if !pycc_hir::tuple_is_boxed(elems))
}

/// The thunk's own parameter list: every declared parameter flattened in
/// place to the scalars it occupies.
#[must_use]
pub fn ext_thunk_param_tys(param_tys: &[Ty]) -> Vec<Ty> {
    param_tys
        .iter()
        .flat_map(|ty| ext_boundary_slots(ty).iter().cloned())
        .collect()
}

/// The element types a `tuple` return is handed back through, one trailing
/// out-pointer each, appended after every flattened parameter. Empty for
/// every other return type, which stays a real return value.
///
/// Returning a tuple *by value* is not an option: pycc's own aggregate
/// calling convention is not the platform C struct ABI. Measured on
/// aarch64-apple-darwin, a pycc function returning `tuple[int, int, int,
/// int, int]` hands the five words back in `x0`-`x4` where clang's C ABI
/// would pass a hidden `sret` pointer -- so a C declaration of the compiled
/// function would disagree with it silently. The thunk exists precisely to
/// keep every aggregate on the LLVM side of the seam.
///
/// Part 2 of #1175 (#1179) adds the second producer of out-pointers, and it
/// is **not** a function of the declared return type: an export declared
/// `-> memoryview` carries three trailing `i64` out-slots only when its own
/// body returns a sub-range of a buffer parameter. `has_slice`, `start` and
/// `stop`, in that order. Keying this on `Ty::MemoryView` alone would flip
/// every existing buffer-returning export off the direct `fnptr_<name>`
/// cast path and onto the thunk path, changing generated C for exports this
/// change does not touch; hence the explicit per-body argument.
///
/// The third slot is load-bearing. `stop = -1` is not "absent", it is the
/// legal `b[0:-1]`, and `i64::MIN`/`i64::MAX` are legal and clamp
/// correctly, so no pair of `i64`s is available as an in-band
/// whole-view sentinel.
#[must_use]
pub fn ext_thunk_out_tys(return_ty: &Ty, returns_buffer_slice: bool) -> Vec<Ty> {
    match return_ty {
        Ty::Tuple(elems) if is_flat_tuple(return_ty) => (**elems).clone(),
        _ if returns_buffer_slice => vec![Ty::Int, Ty::Int, Ty::Int],
        _ => Vec::new(),
    }
}

/// Whether `body` contains the admitted buffer sub-range `return`
/// (Part 2 of #1175, #1179).
///
/// The codegen-side half of a fact the driver computes independently from
/// HIR (`src/ext_build.rs`'s `ExtExport::returns_buffer_slice`). `ExtExport`
/// is a driver-only type that never reaches this crate, and the fact governs
/// both the compiled function's own LLVM signature and the generated C call
/// form, so each side must answer it from the IR it has. Because
/// `MirStmt::ReturnBufferSlice` exists precisely so the admitted shape has a
/// node of its own, this walk is an exact presence test rather than a second
/// copy of the admission rule -- the same mirror relationship
/// [`is_ext_exportable_name`] already has with the driver's own export
/// predicate, and pinned by the cross-crate parity test.
///
/// Exhaustive over every nested-body statement form, modelled on
/// `pycc_mir`'s own `set_frame_function`, so a `return b[i:j]` inside an
/// `if`, a loop or a `try` is found -- branching provenance is admitted and
/// must be.
#[must_use]
pub fn body_returns_buffer_slice(body: &[pycc_mir::MirStmt]) -> bool {
    use pycc_mir::MirStmt;
    body.iter().any(|stmt| match stmt {
        MirStmt::ReturnBufferSlice { .. } => true,
        MirStmt::If { body, orelse, .. } => {
            body_returns_buffer_slice(body) || body_returns_buffer_slice(orelse)
        }
        MirStmt::While { body, .. }
        | MirStmt::ForRange { body, .. }
        | MirStmt::ForList { body, .. }
        | MirStmt::ForObject { body, .. }
        | MirStmt::ForDict { body, .. }
        | MirStmt::ForSet { body, .. }
        | MirStmt::ForGen { body, .. } => body_returns_buffer_slice(body),
        MirStmt::Seq(stmts) => body_returns_buffer_slice(stmts),
        MirStmt::AssignMulti(multi) => body_returns_buffer_slice(&multi.fallback),
        MirStmt::LoopElse { loop_stmt, orelse } => {
            body_returns_buffer_slice(std::slice::from_ref(loop_stmt.as_ref()))
                || body_returns_buffer_slice(orelse)
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
            body_returns_buffer_slice(body)
                || handlers
                    .iter()
                    .any(|handler| body_returns_buffer_slice(&handler.body))
                || body_returns_buffer_slice(orelse)
                || body_returns_buffer_slice(finalbody)
        }
        MirStmt::ExprStmt(_)
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
        | MirStmt::SetOut { .. }
        | MirStmt::AttrSet { .. }
        | MirStmt::ObjDelSlice { .. }
        | MirStmt::ObjSetSlice { .. }
        | MirStmt::ObjAttrSet { .. }
        | MirStmt::ObjDelAttr { .. }
        | MirStmt::ObjSetItem { .. }
        | MirStmt::ObjDelItem { .. }
        | MirStmt::ObjRaise { .. }
        | MirStmt::Raise { .. }
        | MirStmt::RaiseFrom { .. }
        | MirStmt::ForeignImport { .. }
        | MirStmt::Break
        | MirStmt::Continue
        | MirStmt::Pos(_)
        | MirStmt::Yield(_)
        | MirStmt::GenReturn(_)
        | MirStmt::Reraise => false,
    })
}

/// Every function *name* whose LLVM signature must carry the three
/// buffer-sub-range out-pointers (Part 2 of #1175, #1179).
///
/// The fact is a property of the **name**, not of one `def`: every
/// definition of a name shares one `fnptr_<name>` slot and one
/// `UserFunction::fn_type`, so a redefinition cannot give two definitions
/// two different arities without making some indirect call ill-typed. This
/// therefore unions over every definition -- if *any* `def f` returns a
/// sub-range, every `def f` is widened, and the driver's own half of the
/// fact unions the same way at its dedup site (`ExtExport` in
/// `src/ext_build.rs`).
///
/// "Union" rather than "the last definition wins" is what keeps a
/// definition that slices from ever being the un-widened one: its
/// `MirStmt::ReturnBufferSlice` would then store through parameters that
/// do not exist. The cost is a widened definition whose own body only ever
/// does a bare `return b`, and that is exactly the case
/// `MirStmt::Return`'s own `has_slice = 0` store exists to describe -- the
/// two halves are load-bearing for each other.
#[must_use]
pub fn buffer_slice_out_names(items: &[pycc_mir::MirItem]) -> std::collections::BTreeSet<String> {
    items
        .iter()
        .filter_map(|item| match item {
            pycc_mir::MirItem::Function { name, body, .. } if body_returns_buffer_slice(body) => {
                Some(name.clone())
            }
            _ => None,
        })
        .collect()
}

/// Whether a function needs a scalar-only export thunk emitted for it.
///
/// Only a signature that actually carries an aggregate does: a scalar-only
/// export's generated wrapper still reaches the compiled function through
/// the `fnptr_<name>` global directly, and emitting a thunk it would never
/// call would be dead weight in every artifact. A boxed tuple is a single
/// pointer, not an aggregate ([`is_flat_tuple`]), so it needs no thunk.
///
/// Part 2 of #1175 (#1179) adds one non-signature reason: an export whose
/// body returns a sub-range of a buffer parameter needs the three trailing
/// out-pointers [`ext_thunk_out_tys`] describes, so it can no longer be a
/// pure function of the declared signature. Every other `-> memoryview`
/// export keeps the direct cast path byte-for-byte, which is what the
/// `returns_buffer_slice` argument buys -- it is passed in rather than
/// inferred so the driver and codegen halves of the fact stay two
/// deliberately mirrored computations rather than three.
#[must_use]
pub fn ext_thunk_required(
    name: &str,
    param_tys: &[Ty],
    return_ty: &Ty,
    returns_buffer_slice: bool,
) -> bool {
    is_ext_exportable_name(name)
        && (param_tys.iter().any(is_flat_tuple) || is_flat_tuple(return_ty) || returns_buffer_slice)
}
