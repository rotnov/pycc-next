//! Reading a function item's mangled name: which class a method body is
//! anchored in, and the plain name a traceback frame shows. Extracted from
//! `lib.rs` (#1337) when inherited-method copies added a spelling both have
//! to understand.

use std::collections::HashMap;

use pycc_hir::{HirClassDef, SUPER_TARGET_MARKER, inherited_copy_origin};

/// The class a method item's body is anchored in -- the class whose MRO a
/// `super()` inside it continues after (#433) -- or `None` for a top-level
/// function. For an inherited-method copy (#1337, D-254) the name's prefix is
/// the copy's *receiver*, but the body was written in its origin class, so
/// the anchor is the origin; the receiver is the type of `self`.
pub(super) fn item_anchor_class<'a>(
    name: &'a str,
    classes: &'a HashMap<String, HirClassDef>,
) -> Option<&'a str> {
    if let Some(copy) = inherited_copy_origin(name, &|n| classes.get(n)) {
        let (origin, _) = classes
            .get_key_value(copy.origin_class.as_str())
            .expect("a copy's origin class is registered");
        return Some(origin.as_str());
    }
    name.split('.')
        .next()
        .filter(|prefix| *prefix != name)
        .map(|prefix| resolve_method_owner_class(prefix, classes))
}

/// #953: Recovers the class a mangled method name belongs to.
///
/// The first dotted component of `<ClassName>.<method>` is normally the
/// class itself, and PEP 695's own generic-class methods
/// (`0gen_C__T_int.method`) keep that property because the *class* is what
/// carries the `0gen_` prefix there. A protocol-parameter method
/// specialization mangles the whole method name instead
/// (`0gen_C.take__P_C`, `pycc_types::monomorphize`), so its first
/// component is `0gen_C` -- not a registered class. Resolving by lookup
/// rather than by naive prefix keeps both conventions working:
/// `classes`'s own key wins when it exists, a `0gen_`-stripped remainder
/// is tried next, and the raw prefix is returned unchanged otherwise so
/// this function can never change behavior for a name that resolves
/// today.
pub(super) fn resolve_method_owner_class<'a>(
    prefix: &'a str,
    classes: &HashMap<String, HirClassDef>,
) -> &'a str {
    if classes.contains_key(prefix) {
        return prefix;
    }
    match prefix
        .strip_prefix("0gen_")
        .filter(|stripped| classes.contains_key(*stripped))
    {
        Some(stripped) => stripped,
        None => prefix,
    }
}

/// Recovers the plain Python source name of a function from
/// `pycc_hir`'s internal mangled identifier, for traceback rendering
/// (#707). `HirItem::Function::name` is `"<module>"` for the module's own
/// top-level statements, a bare identifier for a top-level `def`, or
/// `"<ClassName>.<method_name>"` for a method -- with a further
/// `.classmethod`/`.static`/`.setter` suffix appended for a classmethod,
/// staticmethod, or property setter (see `pycc_hir::class`'s
/// `mangled_method_name`, around line 2439). None of that mangling is
/// meaningful to a Python programmer reading a traceback: CPython's own
/// traceback frames print a method's plain `co_name` (e.g. `create`), never
/// the qualified `Class.method` form and never an implementation-internal
/// suffix. This strips both layers -- the trailing mangling suffix, then
/// the `<ClassName>.` prefix -- so `pycc_rt_exception_set_frame` receives
/// the same name CPython would show, while a top-level function name or
/// `"<module>"` (which contain no `.` after suffix stripping, since
/// identifiers cannot contain `.`) pass through unchanged.
pub(super) fn source_frame_name(mangled: &str) -> String {
    // #1337 (D-254): a `super()`-target inherited-method copy is named
    // `<C>.<m>.0super_<T>[.<kind>]`; its frame is the copied body's `m`,
    // as CPython's would be.
    let marker = format!(".{SUPER_TARGET_MARKER}");
    let mangled = match mangled.split_once(marker.as_str()) {
        Some((copied, _origin)) => copied,
        None => mangled,
    };
    let without_suffix = mangled
        .strip_suffix(".classmethod")
        .or_else(|| mangled.strip_suffix(".static"))
        .or_else(|| mangled.strip_suffix(".setter"))
        .unwrap_or(mangled);
    // #953: a protocol-parameter method specialization is named
    // `0gen_<Class>.<method>__<P>_<C>`; stripping the leading `0gen_`
    // keeps the class-qualified split below meaningful instead of
    // rendering the whole mangled string as the frame name. The
    // substitution suffix itself stays (the frame reads `take__P_C`):
    // trimming it by string surgery is not safe for a dunder method,
    // whose own name already contains `__`.
    let without_suffix = without_suffix
        .strip_prefix("0gen_")
        .unwrap_or(without_suffix);
    match without_suffix.split_once('.') {
        Some((_class_name, method_name)) => method_name.to_string(),
        None => without_suffix.to_string(),
    }
}
