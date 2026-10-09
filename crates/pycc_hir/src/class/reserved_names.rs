//! Class-attribute names the interpreter reserves for itself, rejected in
//! every class body (#910, #975, #1459, #1465), plus the names CPython's `_EnumDict`
//! keeps out of an enum's member list (#979).
//!
//! Extracted from [`super::attrs`] under `AGENTS.md`'s "keep source files
//! decomposable" rule when #975 added the second name set below, because
//! `attrs.rs` is already past the ~1000-line bar.
//!
//! Five independent sets live here:
//!
//! * `__slots__` (#910), which Python reads as a declaration of the instance
//!   layout. Since #1368 a value-bound `__slots__` in a non-`@dataclass` body
//!   never reaches this module: `super::body` hands it to `super::slots`,
//!   which checks it against CPython's class-creation rules. What still
//!   arrives here is the value-less `__slots__: T` annotation (it binds
//!   nothing, so CPython keeps the instance `__dict__`) and the `@dataclass`
//!   route, neither of which is modelled. Its message is
//!   route-dependent ([`slots_message`]): an `Enum` body has no `__init__` and
//!   no instance layout at all, and CPython's `_EnumDict` turns the name into
//!   an ordinary class attribute there, so D-154's explanation would be false
//!   on that route. Since #980 the name has a *third* account, and since
//!   #984 a *fourth*; both deliberately live outside [`slots_message`],
//!   because on a method route the failure is not about the instance layout
//!   at all. `type.__new__` iterates `__slots__` while the `class` statement
//!   itself executes, so CPython never creates the class: the `@property`
//!   getter spelling carries [`PROPERTY_SLOTS_MESSAGE`] (#980) and every
//!   other `def` spelling carries [`method_slots_message`] (#984), whose only
//!   difference is the carrier type CPython names. Neither is a
//!   [`ClassBodyRoute`] variant -- see the scope notes below.
//! * The *instantiation and class-creation protocol* names (#975, D-236):
//!   `__init__`, `__new__` and `__init_subclass__`. The generating rule is
//!   "the names Python's object protocol calls implicitly rather than by
//!   name" -- `C()` runs `type.__call__` -> `__new__` -> `__init__`, and
//!   `class B(C)` runs `__init_subclass__` at class creation. This compiler
//!   resolves each protocol without ever consulting a class attribute of that
//!   name, so binding one of them silently changes behavior: for `__init__`
//!   and `__new__` that is a *measured* divergence -- CPython raises
//!   `TypeError` at the use site (`'int' object is not callable` for an `int`
//!   binding) while pycc compiled and ran the program, the D-198 false
//!   acceptance D-224 forbids. `__init_subclass__` is rejected
//!   **conservatively** instead, and D-236 records why: in an `Enum` body it
//!   can never diverge (an `Enum` with members cannot be subclassed, and
//!   `class C(Enum): __init_subclass__ = 1; B = 2` prints `2` under both
//!   engines), and in a plain body it diverges only once the class is actually
//!   subclassed -- whole-program information the class-body walk does not have
//!   when the guard fires. It is rejected on both routes for uniformity of the
//!   single rule, not because every binding was measured to fail.
//! * The `_EnumDict` **non-member** shapes (#979, D-238), which unlike the
//!   other two sets are a *shape* rather than an enumeration and are checked
//!   only on the [`ClassBodyRoute::Enum`] route: dunder-shaped `__x__`,
//!   name-mangled private `__x`, a name already spelled `_C__x` inside
//!   `class C`, and sunder-shaped `_x_`. The generating rule
//!   is CPython's `enum._EnumDict.__setitem__`, whose `_is_private`,
//!   `_is_sunder` and `_is_dunder` branches each keep a name out of the
//!   member list; all three were measured to diverge here, because
//!   `lower_enum_class` lowers every assignment as a member and has no
//!   non-member representation at all. `_x = 1` is not in the set and stays
//!   an ordinary member, agreeing with CPython, and so does `_C__x` in any
//!   class *not* called `C` -- one arm of this check is class-name-keyed
//!   because `_is_private` is. See [`enum_non_member_message`] for the
//!   measured table, the arm ordering, and the three families this predicate
//!   deliberately over-rejects.
//! * The *attribute-store protocol* names `__setattr__` and `__delattr__`
//!   (#1459), under D-236's generating rule: Python calls
//!   `type(obj).__setattr__` on every `obj.x = v` and `type(obj).__delattr__`
//!   on every `del obj.x`, and this compiler stores into (or refuses to
//!   delete from) a compiled instance without ever consulting a class member
//!   of either name. A `def` of either name compiled and ran with the method
//!   silently bypassed -- measured on `ddada6e4`: a `__setattr__` that only
//!   prints, under `c = C(1); c.n = 5`, printed nothing here and `intercept
//!   n` twice under CPython 3.13.9 -- and `__setattr__ = 1` compiled where
//!   CPython raises `TypeError: 'int' object is not callable` on the store.
//!   Unlike the instantiation-protocol names, the set is checked on *every*
//!   method binding form, not only the `@property` getter: a plain `def` is
//!   exactly the binding that diverges. It is checked on two routes:
//!   [`reject_reserved_method_name`] (every `def` spelling except a
//!   `@<name>.setter`) and [`reject_reserved_class_attr_name`] (after the
//!   other three sets, so an `Enum` body keeps #979's dunder-shape message).
//!   A `Protocol` body is deliberately not checked: its methods are never
//!   lowered into a class's method table, and any class listing a protocol
//!   among its bases is itself lowered as a protocol and cannot be
//!   instantiated, so no store can reach a protocol's `__setattr__`.
//! * The *attribute-read protocol* names `__getattribute__` and
//!   `__getattr__` (#1465), the read-side sibling of the set above under the
//!   same rule: Python calls `type(obj).__getattribute__` on every attribute
//!   read on an instance, and `type(obj).__getattr__` whenever that normal
//!   lookup raises `AttributeError`. This compiler reads a compiled field or
//!   property without consulting either name, and an `--ext` descriptor
//!   does the same from the host. Measured on `92f8971b`: a
//!   `__getattribute__` returning `42` left `print(c.n)` at `1` (CPython
//!   3.13.9: `42`); a `__getattr__` returning `42` behind a property whose
//!   getter raises `AttributeError` raised here where CPython printed `42`;
//!   and under `--ext` a host `c.zz` raised `AttributeError` and
//!   `hasattr(c, "zz")` was `False` where CPython 3.14.7 answered `42` and
//!   `True`. A `__getattr__` behind a read that never misses agrees with
//!   CPython only because the fallback is never reached, so refusing it
//!   regresses no working read; an explicit `c.__getattr__("x")` call is
//!   refused with it, as #1459 refuses an explicit `__setattr__` call. It is
//!   checked on the same two routes, at the same points, as the store set,
//!   and a `Protocol` body is exempt for the same reason. A module-level
//!   `def __getattr__` (PEP 562) is not a class body and never reaches this
//!   module; since #1467 an `--ext` build publishes the entry module's hook
//!   as a module attribute (`src/ext_build/module_hooks.rs`).
//!
//! Scope notes that are easy to get wrong. Each bullet was measured at
//! `28a1b194` unless it names a different commit, or names #984 -- this
//! change, whose own commit did not exist when the bullet was written:
//!
//! * The guard is reached from **five** class-body call sites, not three:
//!   the annotated and bare class-attribute paths and the `Enum` member loop
//!   (all three into [`reject_reserved_class_attr_name`]), `super::body`'s
//!   method loop (one call to [`reject_reserved_method_name`], serving the
//!   `@property` getter spelling and every non-getter spelling alike, since
//!   #984), and `super::protocol`'s own method walk (since #984, into
//!   [`reject_reserved_protocol_method_name`]). The last two are separate
//!   call sites because `super::lower_class` returns through
//!   `lower_protocol_class` *before* the method loop, so a `Protocol` body
//!   never reaches `super::body` at all.
//!
//!   #1266 adds a sixth, attribute-shaped call site:
//!   `super::declared_attrs::collect_declared_attrs`, which guards a plain
//!   body's value-less instance attribute declaration (`x: int`) with
//!   [`ClassBodyRoute::Plain`] before the walk ever starts.
//!
//!   None of the method call sites takes a [`ClassBodyRoute`]: a plain and a
//!   `@dataclass` body share the one method loop, and an `Enum` body rejects
//!   method definitions outright before it (`C0001`, "an enum class body must
//!   contain only member assignments"), so an enum arm there would be a dead
//!   match arm and would fail D-014's 100% region gate.
//! * A `@<name>.setter` named `__slots__` is **reachable** and is deliberately
//!   short-circuited before the `__slots__` check in
//!   [`reject_reserved_method_name`]. `super::classify_decorator` accepts
//!   `@__slots__.setter def __slots__(self, v)` as a
//!   [`MethodKind::PropertySetter`] without requiring a preceding getter --
//!   that requirement is enforced *later*, in `super::body`'s post-lowering
//!   accounting. Measured at `e77b4b13`, pycc already rejects that program
//!   with "a `@__slots__.setter` decorator requires a preceding `@property`
//!   getter", and CPython 3.13.9 raises `NameError: name '__slots__' is not
//!   defined` while evaluating the decorator expression -- not a `TypeError`
//!   about a non-iterable object. Emitting [`method_slots_message`] there
//!   would be false in every clause, which is the defect #980 and #984 exist
//!   to close.
//! * The set is **not** `ClassVar`-gated. `super::body`'s non-dataclass branch
//!   routes every `AnnAssign` to `lower_class_attr` regardless of the
//!   `ClassVar` wrapper, and #910's `Stmt::Assign` arm routes to
//!   `lower_unannotated_class_attr`, so `X: ClassVar[int] = 8`, `X: int = 8`
//!   and `X = 8` all reach the same hazard.
//! * The set deliberately does **not** include the other five names in
//!   `super::body`'s `DATACLASS_IMPLICIT_DUNDERS` (`__eq__`, `__repr__`,
//!   `__ne__`, `__str__`, `__format__`). Those are safe outside a dataclass
//!   *only because* every rewrite consulting them is `is_dataclass`-gated
//!   today (`pycc_mir/src/class.rs`'s early return, and `pycc_mir/src/expr.rs`'s
//!   `&& class_def.is_dataclass`). Ungating any of them for a plain class
//!   reintroduces the divergence and brings that name into this set.
//! * Conversely this set is checked in *every* class body, so it also closes
//!   the two names `DATACLASS_IMPLICIT_DUNDERS` omits (`__new__`,
//!   `__init_subclass__`) on the dataclass path, without disturbing the six
//!   messages that set already owns: `super::body` runs its own check first.
//! * The five checks inside [`reject_reserved_class_attr_name`] run in a
//!   **fixed order**: `__slots__`, then the instantiation-protocol names,
//!   then #979's `_EnumDict` shapes, then #1459's attribute-store protocol
//!   names, then #1465's attribute-read protocol names. The last two sit
//!   after the shape check for the same reason: on the `Enum` route it
//!   answers `__setattr__ = 1` and `__getattr__ = 1` first and keeps #979's
//!   message.
//!   `__slots__`, `__init__`, `__new__` and
//!   `__init_subclass__` are all dunder-shaped, so putting the shape check
//!   first would silently repoint every one of their pinned messages on the
//!   `Enum` route -- including the one asserted by
//!   `the_enum_slots_message_describes_the_enum_route`, which checks for the
//!   *absence* of the plain-class string and would still pass while the wrong
//!   message was emitted.
//! * #979's set is the only one that is route-*gated* rather than merely
//!   route-*worded*. `class C: __repr__ = 1` in a plain or `@dataclass` body
//!   is still accepted, because there the name is an ordinary class attribute
//!   under both engines; only an `Enum` body has a member list for it to fall
//!   out of.
//! * The `Enum`-body `__slots__` rejection is conservative too, but only in
//!   the narrow sense. CPython 3.13.9 *accepts* `class C(Enum): __slots__ =
//!   "x"` with `A = 1` (the `class` statement succeeds, `C.__slots__` is
//!   `'x'`, `list(C)` is `[C.A]`, and the member still has a `__dict__`), so
//!   the rejection does refuse a program CPython runs. It is
//!   not conservative in the weaker sense of "nothing would have gone wrong":
//!   without the guard pycc lowers the name as a member, so an all-`str` enum
//!   (`__slots__ = "x"` alongside `A = "y"`, which CPython leaves at one
//!   member) would have compiled to a two-member enum. `__slots__ = ()` is the
//!   shape where no divergence is reachable, because a non-literal member
//!   value is rejected anyway.
//! * `__init_subclass__`'s rejection is conservative on two different
//!   grounds. In an `Enum` body it can never diverge, because an `Enum` with
//!   members cannot be subclassed at all. In a plain class body it diverges
//!   only if the class is actually subclassed somewhere, which is
//!   whole-program information this class-body walk does not have. It is
//!   rejected in both for uniformity of the single rule, not because a
//!   divergence was measured at the declaration site.

use super::MethodKind;
use crate::unsupported;
use pycc_diag::Diagnostic;

/// Rejects a class-body binding of a name the interpreter gives its own
/// meaning, in every spelling and every class body.
///
/// Called first thing after name extraction from four of the six routes
/// into a class body: [`super::attrs::lower_class_attr`] (annotated),
/// [`super::attrs::lower_unannotated_class_attr`] (#910's bare assignment),
/// `super::declared_attrs::collect_declared_attrs` (a value-less instance
/// attribute declaration, #1266), and [`super::enum_class`]'s member loop. `route` distinguishes the last of
/// those, because only the `__slots__` explanation differs between them. The
/// remaining two call sites are method walks and do not come here:
/// [`super::body`]'s method loop calls [`reject_reserved_method_name`] and
/// [`super::protocol`]'s calls [`reject_reserved_protocol_method_name`],
/// because only part of this guard applies on a method route.
///
/// The five checks run in a fixed order that is load-bearing rather than
/// stylistic: `__slots__` first, the instantiation-protocol names second,
/// [`enum_non_member_message`] third (#979), then the attribute-store
/// protocol names (#1459) and the attribute-read protocol names (#1465) --
/// on the `Enum` route neither of the last two is ever reached for any of
/// its four names, because the dunder-shape arm answers all four first,
/// truthfully. Every name the first two own also
/// matches the third's `__`-prefix shape, so any other order would silently
/// repoint their pinned messages on the `Enum` route. The third also needs the
/// enclosing class's name, which [`ClassBodyRoute::Enum`] carries.
///
/// Because it runs during the class-body walk, it sits at the *head* of
/// D-235's pinned four-deep diagnostic precedence rather than reordering it.
pub(super) fn reject_reserved_class_attr_name(
    attr_name: &str,
    route: ClassBodyRoute<'_>,
    range: std::ops::Range<u32>,
) -> Result<(), Diagnostic> {
    if attr_name == "__slots__" {
        return Err(unsupported(slots_message(route), range));
    }
    if let Some(message) = instantiation_protocol_message(attr_name) {
        return Err(unsupported(message, range));
    }
    if let ClassBodyRoute::Enum { class_name } = route
        && let Some(message) = enum_non_member_message(attr_name, class_name)
    {
        return Err(unsupported(message, range));
    }
    if is_store_protocol_name(attr_name) {
        return Err(unsupported(store_protocol_attr_message(attr_name), range));
    }
    if is_read_protocol_name(attr_name) {
        return Err(unsupported(read_protocol_attr_message(attr_name), range));
    }
    Ok(())
}

/// Rejects a `@property` getter named after the instantiation or
/// class-creation protocol (#975, D-236; added in the #978 review round), or
/// named `__slots__` (#980).
///
/// The `@property` half of the class-body method route, and one of the two
/// call sites that do not go through [`reject_reserved_class_attr_name`].
/// Since #984 it is reached from [`reject_reserved_method_name`] rather than
/// directly from `super::body`; the dispatch is unchanged, only relocated.
/// `super::body`'s method loop routes `@property def __new__(self) -> int` to
/// `MethodKind::PropertyGetter`, so none of the four attribute routes sees
/// it; without this call `ensure_init` synthesizes a constructor from the
/// method table alone and pycc accepts `C()`, while CPython 3.13.9 raises
/// `TypeError: 'property' object is not callable` at that call.
///
/// `__slots__` is rejected here too, since
/// [#980](https://github.com/rotnov/pycc/issues/980), but under
/// [`PROPERTY_SLOTS_MESSAGE`] rather than [`slots_message`], which stays
/// deliberately unreachable from this route. The mechanism is a different one:
/// `type.__new__` *iterates* `__slots__` while the `class` statement itself
/// executes, so CPython never creates the class at all (measured on CPython
/// 3.13.9: `TypeError: 'property' object is not iterable` at class creation,
/// on a plain and on a `@dataclass` body alike), whereas D-154's "the instance
/// layout is fixed from `__init__`" describes a redundant *declaration* on a
/// class that is created. Emitting the attribute-route string here would be a
/// false account, which is what #980 was opened to avoid.
///
/// The `__slots__` branch takes no [`ClassBodyRoute`]. Only one class body
/// reaches this function: `super::body`'s method loop serves the plain and the
/// `@dataclass` route alike, and `super::enum_class` rejects a method
/// definition outright before any of this, so a route parameter would carry a
/// permanently dead arm. Since #984 a `Protocol` body reaches
/// [`reject_reserved_protocol_method_name`] instead, never this function --
/// `super::protocol` rejects every decorator on a protocol method, so no
/// getter can exist there.
///
/// The three *protocol* messages are shared verbatim with the attribute
/// routes and need no property-specific clause: each already says "binding
/// that name to a non-callable object", and a `property` object is exactly
/// that. `__slots__` is the one name whose message is not shared, because its
/// reason is not shared either.
///
/// Only the *getter* arm calls this, and a `@<name>.setter` is unreachable
/// for these names for two independent reasons. `super::classify_decorator`
/// requires a setter's own `def` name to equal the decorator's property name,
/// so `@value.setter def __new__` is already rejected there as a mismatch;
/// and the matching spelling `@__new__.setter def __new__` requires a
/// preceding `@property def __new__` getter, which is rejected here first.
///
/// `@staticmethod def __new__` and `@classmethod def __init_subclass__` are
/// deliberately *not* routed here: those bind a callable, which is what
/// CPython itself expects of the protocol, so they are not the divergence
/// this guard describes.
pub(super) fn reject_reserved_property_name(
    prop_name: &str,
    range: std::ops::Range<u32>,
) -> Result<(), Diagnostic> {
    if prop_name == "__slots__" {
        return Err(unsupported(PROPERTY_SLOTS_MESSAGE, range));
    }
    match instantiation_protocol_message(prop_name) {
        Some(message) => Err(unsupported(message, range)),
        None => Ok(()),
    }
}

/// Rejects a class-body `def` whose name the interpreter reserves for itself
/// ([#984](https://github.com/rotnov/pycc/issues/984)).
///
/// The single call from [`super::body`]'s method loop, replacing #980's
/// getter-only call: it dispatches the `@property` getter spelling to
/// [`reject_reserved_property_name`] unchanged, and adds `__slots__` on every
/// remaining `def` spelling. Measured at `e77b4b13` against CPython 3.13.9,
/// seven spellings diverged before this guard -- a plain `def`,
/// `@staticmethod`, `@classmethod`, `@abstractmethod`, `@override`, and a
/// plain `def` inside a `@dataclass` and inside a `Protocol` body -- each
/// raising `TypeError: '<carrier>' object is not iterable` at class creation
/// while pycc compiled and ran the program, the D-198 false acceptance D-224
/// forbids. The divergence is value-independent: none of those carriers is
/// ever iterable, whatever the method returns.
///
/// Two routing decisions carry the whole correctness argument.
///
/// The **instantiation-protocol** half of [`reject_reserved_property_name`]
/// stays gated to [`MethodKind::PropertyGetter`]. Widening it to every kind
/// would start rejecting a plain `def __new__`, which binds exactly the
/// callable CPython's protocol expects and is therefore outside D-236's rule;
/// it is tracked as [#981](https://github.com/rotnov/pycc/issues/981) and
/// pinned by `a_plain_new_method_is_left_to_issue_981`.
///
/// A [`MethodKind::PropertySetter`] is **short-circuited** rather than
/// checked. `@__slots__.setter def __slots__(self, v)` reaches here without a
/// preceding getter, because `super::classify_decorator` only requires the
/// setter's own `def` name to match the decorated property name; the
/// "requires a preceding `@property` getter" rejection lives *after* this
/// call. That program is already rejected today with that truthful message,
/// and CPython 3.13.9 raises `NameError: name '__slots__' is not defined`
/// while evaluating the decorator expression -- so [`method_slots_message`]'s
/// account ("a `function` object is not iterable", "this compiler accepts the
/// class body") would be false in every clause for it.
pub(super) fn reject_reserved_method_name(
    method_name: &str,
    kind: &MethodKind,
    range: std::ops::Range<u32>,
) -> Result<(), Diagnostic> {
    if matches!(kind, MethodKind::PropertySetter { .. }) {
        return Ok(());
    }
    // #1459: every other binding form of these two names, the `@property`
    // getter included, so this runs before the getter's own dispatch.
    if is_store_protocol_name(method_name) {
        return Err(unsupported(
            store_protocol_method_message(method_name),
            range,
        ));
    }
    // #1465: the read-side sibling, at the same point for the same reason.
    if is_read_protocol_name(method_name) {
        return Err(unsupported(
            read_protocol_method_message(method_name),
            range,
        ));
    }
    if let MethodKind::PropertyGetter { prop_name } = kind {
        return reject_reserved_property_name(prop_name, range);
    }
    if method_name == "__slots__" {
        return Err(unsupported(
            method_slots_message(method_slots_carrier(kind)),
            range,
        ));
    }
    Ok(())
}

/// Rejects a `Protocol`-body `def` named `__slots__` (#984).
///
/// The fifth class-body call site, and a genuinely separate one:
/// `super::lower_class` returns through `super::protocol::lower_protocol_class`
/// *before* [`super::body`]'s method loop, so a `Protocol` body never reaches
/// [`reject_reserved_method_name`] and a guard placed only there would leave
/// `class P(Protocol): def __slots__(self) -> int: ...` falsely accepted
/// (measured at `e77b4b13`: `pycc check` exit 0, CPython 3.13.9
/// `TypeError: 'function' object is not iterable`).
///
/// The carrier is always `function`. `super::protocol` rejects any decorator
/// on a protocol method outright and this call is placed *after* that
/// rejection, after the generic-type-parameter rejection, and after the
/// declaration-style-body rejection -- so the only input it newly rejects is
/// the one that is falsely accepted today, and every other protocol spelling
/// keeps the diagnostic it already had.
pub(super) fn reject_reserved_protocol_method_name(
    method_name: &str,
    range: std::ops::Range<u32>,
) -> Result<(), Diagnostic> {
    if method_name == "__slots__" {
        return Err(unsupported(method_slots_message("function"), range));
    }
    Ok(())
}

/// Whether `name` is one of the attribute-store protocol names (#1459):
/// `__setattr__` or `__delattr__`.
fn is_store_protocol_name(name: &str) -> bool {
    matches!(name, "__setattr__" | "__delattr__")
}

/// The `C0001` message for a class-body `def` of an attribute-store protocol
/// name (#1459), in any binding form -- a plain `def`, `@staticmethod`,
/// `@classmethod`, `@abstractmethod`/`@override`, or a `@property` getter.
///
/// It names no carrier type and makes no claim about *where* the compiler
/// stores: `super::body`'s method loop runs before
/// `super::exception_dunders`' class-level check, so this message is also the
/// one a user exception class gets, and an exception value has no slots.
fn store_protocol_method_message(name: &str) -> String {
    format!(
        "a `def {name}` in a class body is not supported yet -- Python calls `__setattr__` \
         implicitly on every attribute store on an instance (`obj.x = v`, including \
         `self.x = v` inside `__init__`) and `__delattr__` on every attribute deletion \
         (`del obj.x`), while this compiler never calls either on a store or deletion, so the \
         method would be silently bypassed rather than honored (#1459)"
    )
}

/// The `C0001` message for a class attribute named after an attribute-store
/// protocol name (#1459), on every attribute route: annotated, bare, and a
/// value-less declaration.
///
/// The `TypeError` claim is conditional ("binding it to a non-callable
/// object"), as [`instantiation_protocol_message`]'s is, because the guard
/// also sees a value-less `__setattr__: int`, which binds nothing in CPython,
/// and the admitted `staticmethod(<foreign import>)` initializer, which binds
/// a callable. The final clause is what holds for every one of them.
fn store_protocol_attr_message(name: &str) -> String {
    format!(
        "a class attribute named `{name}` is not supported yet -- Python calls whatever \
         `__setattr__` is bound to implicitly on every attribute store on an instance \
         (`obj.x = v`) and whatever `__delattr__` is bound to on every attribute deletion \
         (`del obj.x`), so binding it to a non-callable object makes CPython raise a \
         `TypeError` there, while this compiler never consults a class attribute of that name, \
         so the binding would be silently ignored rather than honored (#1459)"
    )
}

/// Whether `name` is one of the attribute-read protocol names (#1465):
/// `__getattribute__` or `__getattr__`.
fn is_read_protocol_name(name: &str) -> bool {
    matches!(name, "__getattribute__" | "__getattr__")
}

/// When CPython calls the attribute-read protocol name `name` (#1465):
/// `__getattribute__` on every read, `__getattr__` only when that normal
/// lookup raises `AttributeError`. The second must not claim "every read".
fn read_protocol_trigger(name: &str) -> &'static str {
    if name == "__getattribute__" {
        "on every attribute read on an instance (`obj.x`, method lookups and `self.x` \
         inside the class's own methods included)"
    } else {
        "whenever normal lookup of an attribute on an instance raises `AttributeError` (a \
         missing name, or a property getter that raises it)"
    }
}

/// The `C0001` message for a class-body `def` of an attribute-read protocol
/// name (#1465), in any binding form -- a plain `def`, `@staticmethod`,
/// `@classmethod`, `@abstractmethod`/`@override`, or a `@property` getter.
///
/// Like [`store_protocol_method_message`] it names no carrier and no slot,
/// because a user exception class reaches it too.
fn read_protocol_method_message(name: &str) -> String {
    format!(
        "a `def {name}` in a class body is not supported yet -- Python calls `{name}` \
         implicitly {}, while this compiler never calls it on a read, so the method would be \
         silently bypassed rather than honored (#1465)",
        read_protocol_trigger(name)
    )
}

/// The `C0001` message for a class attribute named after an attribute-read
/// protocol name (#1465), on every attribute route: annotated, bare, and a
/// value-less declaration. The `TypeError` claim is conditional for the
/// reason [`store_protocol_attr_message`] gives.
fn read_protocol_attr_message(name: &str) -> String {
    format!(
        "a class attribute named `{name}` is not supported yet -- Python calls whatever \
         `{name}` is bound to implicitly {}, so binding it to a non-callable object makes \
         CPython raise a `TypeError` there, while this compiler never consults a class \
         attribute of that name, so the binding would be silently ignored rather than honored \
         (#1465)",
        read_protocol_trigger(name)
    )
}

/// The type CPython names in `'<carrier>' object is not iterable` for a
/// class-body `def __slots__`, per binding form (#984).
///
/// Exactly two named arms plus a wildcard, all three reachable and each
/// pinned by a test, because a dead arm fails D-014's 100% region gate.
/// Measured at `e77b4b13`: CPython 3.13.9 names only these three carriers
/// across every diverging spelling.
fn method_slots_carrier(kind: &MethodKind) -> &'static str {
    match kind {
        MethodKind::StaticMethod => "staticmethod",
        MethodKind::ClassMethod => "classmethod",
        // Every other kind binds a plain function object: a bare `def`,
        // `@override`, `@abstractmethod`. `@property` and `@<name>.setter`
        // never reach here -- both are routed out in
        // `reject_reserved_method_name` above.
        _ => "function",
    }
}

/// The `C0001` message for a non-`@property` `def __slots__` in a class body
/// (#984) -- the *fourth* account this name carries.
///
/// A sibling of [`PROPERTY_SLOTS_MESSAGE`] rather than a replacement for it.
/// The two mechanisms are identical, but the getter's tail clause ("lowers the
/// getter as an ordinary property and creates the class anyway") is a
/// different clause rather than a substitution, so folding them into one
/// template would degenerate into a lookup table; and D-236's #980 amendment
/// note names `PROPERTY_SLOTS_MESSAGE` as a constant, so converting it into a
/// function would force an unrelated edit to an accepted decision for zero
/// behavioural gain.
///
/// Deliberately **interpolation-only**: the carrier is resolved once by
/// [`method_slots_carrier`] and never re-branched on here, so this function
/// cannot introduce a second, uncovered match arm.
///
/// Two wording constraints are load-bearing.
///
/// The tail says "accepts the class body", not "creates the class anyway" and
/// not "lowers the definition as an ordinary method". This message is also
/// emitted for a `Protocol` body, where pycc records a `ProtocolMember::Method`
/// and creates no class at all, so either narrower phrasing would be false
/// there.
///
/// It carries none of D-154's "instance layout is fixed at compile time from
/// its `__init__`" language, which is the plain attribute route's reason
/// ([`slots_message`]) and would be a false account here. Keeping the strings
/// disjoint is what lets the tests pin the distinction from both sides.
///
/// Like [`PROPERTY_SLOTS_MESSAGE`], and unlike
/// [`instantiation_protocol_message`]'s strings, it **names the bound type**.
/// D-236's #980 amendment permitted that when a decorator fixes the type
/// structurally; a plain `def __slots__` has no decorator, so #984's amendment
/// generalizes the rule to *fixed structurally by the binding form* (`def`,
/// `@staticmethod`, `@classmethod`, `@property`), never derived from a value.
fn method_slots_message(carrier: &'static str) -> String {
    format!(
        "a `def __slots__` in a class body is not supported yet -- `type.__new__` iterates \
         `__slots__` while the `class` statement itself executes, and a `{carrier}` object is \
         not iterable, so CPython never creates the class at all (measured on CPython 3.13.9: \
         `TypeError: '{carrier}' object is not iterable` at class creation), while this compiler \
         accepts the class body, so the program would compile and run here instead of failing"
    )
}

/// The `C0001` message for a `@property` getter named `__slots__` (#980).
///
/// The *third* account this name carries, and since #984 one of two that are
/// not a [`ClassBodyRoute`] arm of [`slots_message`] (the other being
/// [`method_slots_message`], the fourth account) -- a constant rather than a
/// match, because the single route that reaches it cannot branch.
///
/// It describes the mechanism CPython actually fails by: `type.__new__`
/// iterates `__slots__` while the `class` statement itself executes, and a
/// `property` object is not iterable, so the class is never created. It
/// carries none of D-154's "instance layout is fixed at compile time from its
/// `__init__`" language, which is the plain attribute route's reason and would
/// be a false account here; keeping the two strings disjoint is also what lets
/// the tests pin the distinction from both sides.
///
/// Unlike [`instantiation_protocol_message`]'s strings this one **names the
/// bound type**, `property`. The distinction is worth stating once: those omit
/// the type because the guard runs before value extraction and the type is
/// whatever an initializer happens to evaluate to, while here the carrier type
/// is fixed structurally by the `@property` decorator, whatever the getter
/// returns. #984 generalized the rule from the decorator to the *binding
/// form*: a message may name a type when the binding form (`def`,
/// `@staticmethod`, `@classmethod`, `@property`) fixes it structurally, never
/// when it would have to be derived from a value.
const PROPERTY_SLOTS_MESSAGE: &str = "a `@property` getter named `__slots__` is not supported yet -- `type.__new__` iterates \
     `__slots__` while the `class` statement itself executes, and a `property` object is not \
     iterable, so CPython never creates the class at all (measured on CPython 3.13.9: \
     `TypeError: 'property' object is not iterable` at class creation), while this compiler \
     lowers the getter as an ordinary property and creates the class anyway, so the program \
     would compile and run here instead of failing";

/// Which class-body route reached the guard.
///
/// It does two different jobs. It selects the `__slots__` message (see
/// [`slots_message`]), and -- since #979 -- it also decides whether the
/// [`enum_non_member_message`] check runs *at all*, because CPython's
/// `_EnumDict` non-member rule exists only in an `Enum` body. The
/// instantiation-protocol names use it for neither: they share one string
/// across all routes, because the reason they are reserved -- pycc resolves
/// each protocol without consulting a class attribute of that name -- is the
/// same everywhere.
///
/// It covers the four *attribute* routes only. The method routes do not pass
/// a `ClassBodyRoute` at all: `super::body`'s method loop is a single arm
/// serving both a plain and a `@dataclass` body, and a `Protocol` body has its
/// own walk, so they carry [`PROPERTY_SLOTS_MESSAGE`] and
/// [`method_slots_message`] instead of variants here.
///
/// `Plain` covers both the ordinary and the `@dataclass` class body: they
/// share `super::attrs`, and a dataclass instance layout is still fixed from
/// `__init__`, so D-154's explanation is accurate for both.
pub(super) enum ClassBodyRoute<'a> {
    /// [`super::attrs`]: an ordinary or `@dataclass` class body.
    Plain,
    /// [`super::enum_class`]'s member loop.
    ///
    /// It carries the enclosing class's name because one of
    /// [`enum_non_member_message`]'s arms is class-name-keyed: CPython's
    /// `enum._EnumDict._is_private` compares the raw key against the literal
    /// `_<ClassName>__` prefix, so whether `_C__x = 1` is a member depends on
    /// what the class is called.
    Enum { class_name: &'a str },
}

/// The `C0001` message for a class-body `__slots__` binding, per route (#910,
/// and the #975 review round that made the enum route reachable).
///
/// The routes need different text because they are rejected for different
/// reasons, and stating the wrong one is a false explanation rather than a
/// stylistic slip:
///
/// * A plain (or `@dataclass`) class has an instance layout. A value-bound
///   `__slots__` in a plain body is admitted by `super::slots` (#1368) and
///   never reaches here; the spellings that do -- a value-less annotation,
///   and any `__slots__` in a `@dataclass` body -- are not modelled.
/// * An enum has neither. `lower_enum_class` produces no `__init__` and no
///   instance layout at all; its members are a compile-time table. The name is
///   rejected there because CPython gives it a *third* meaning again -- an
///   ordinary class attribute -- that pycc does not model.
///
/// Two further `__slots__` accounts exist and are deliberately **not** arms
/// here: [`PROPERTY_SLOTS_MESSAGE`] for the `@property` getter route (#980),
/// and [`method_slots_message`] for every other `def` spelling and for a
/// `Protocol` body (#984). Both stay outside this match because their routes
/// have no [`ClassBodyRoute`] to dispatch on -- one arm of this enum could
/// never be produced for them, and a dead arm fails D-014's region gate.
fn slots_message(route: ClassBodyRoute<'_>) -> &'static str {
    match route {
        ClassBodyRoute::Plain => {
            "this `__slots__` spelling is not supported yet -- only a value-bound `__slots__` \
             (`__slots__ = ('a', 'b')`) in a class body that is not a `@dataclass` is; a \
             value-less `__slots__` annotation binds nothing in CPython (the class keeps its \
             `__dict__`), and `__slots__` in a `@dataclass` body is not modelled"
        }
        ClassBodyRoute::Enum { .. } => {
            "`__slots__` in an `Enum` body is not supported yet -- CPython's `_EnumDict` keeps \
             a dunder out of the member list, so there `__slots__` is an ordinary class \
             attribute and the members are unaffected (measured on CPython 3.13.9: `class \
             C(Enum): __slots__ = \"x\"` alongside `A = 1` leaves `C.__slots__ == \'x\'` and \
             `list(C) == [C.A]`), while this compiler has no enum-specific model for the name \
             and would otherwise lower it as a member -- an enum produced by `lower_enum_class` \
             has no `__init__` and no instance layout for `__slots__` to declare, only a \
             compile-time member table -- so the program is rejected rather than compiled to a \
             different member list"
        }
    }
}

/// The `C0001` message for a class attribute named after the instantiation or
/// class-creation protocol, or `None` if the name is not one of them (#975).
///
/// Each name gets its own sentence rather than one template with a
/// substituted noun: "the constructor" is wrong for `__new__` and for
/// `__init_subclass__`, and "instantiation" is wrong for `__init_subclass__`.
///
/// Only `__init__` and `__new__` name CPython's `TypeError`, because the same
/// string is emitted from the `Enum` member route, where CPython does *not*
/// raise for `__init_subclass__`.
///
/// Neither of them names the *type* of the bound object in that `TypeError`.
/// CPython's own text is `'<type>' object is not callable`, and `<type>` is
/// whatever the initializer evaluates to (`'int'`, `'str'`, `'bool'`,
/// `'float'`, ...). Deriving it here is deliberately not done: this guard
/// runs on the attribute *name* alone, before any value extraction, and must
/// stay cheap and value-independent so that all four class-body routes can
/// call it at the same early point. Naming one concrete type would make the
/// message wrong for every other binding, so the type is omitted instead.
/// [`PROPERTY_SLOTS_MESSAGE`] and [`method_slots_message`] do name a type, and
/// the difference is the rule: they may, because the *binding form* (`def`,
/// `@staticmethod`, `@classmethod`, `@property`) fixes the carrier type
/// structurally, while here the type would have to be derived from an
/// initializer this guard has not read. #984 generalized that rule from
/// "the decorator fixes it" to "the binding form fixes it", because a plain
/// `def __slots__` has no decorator at all.
///
/// Both messages name the `TypeError` *conditionally* in two axes. They say
/// "binding that name ... makes CPython raise" rather than "CPython raises here",
/// and they do not fix *when* it raises: the `C()` call site on the plain and
/// dataclass routes, but already at class creation on the `Enum` route, where
/// `enum.py`'s `__set_name__` invokes the now-non-callable member while the
/// `class` statement itself executes.
///
/// The first conditional exists because this guard runs before the
/// value-presence check, so it
/// also covers a value-less declaration (`__init__: int`, no `=`). CPython
/// creates no class `__dict__` entry at all for a bare annotation, so an
/// unconditional "CPython raises here" would be false for that input. The
/// name is still reserved, and the rejection is still correct; only the
/// claim about CPython has to be conditional.
fn instantiation_protocol_message(attr_name: &str) -> Option<&'static str> {
    match attr_name {
        "__init__" => Some(
            "a class attribute named `__init__` is not supported yet -- Python calls it \
             implicitly when the class is instantiated (`C()` runs `type.__call__` -> `__new__` \
             -> `__init__`), so binding that name to a non-callable object makes CPython raise a \
             `TypeError` when it reaches that call -- at the `C()` call site, or already at \
             class creation in an `Enum` body -- while this compiler resolves a class's \
             constructor from its methods alone and never consults a class attribute of that \
             name, so the binding would be silently ignored rather than honored",
        ),
        "__new__" => Some(
            "a class attribute named `__new__` is not supported yet -- Python calls it \
             implicitly when the class is instantiated (`C()` runs `type.__call__` -> `__new__` \
             before `__init__`), so binding that name to a non-callable object makes CPython \
             raise a `TypeError` when it reaches that call -- at the `C()` call site, or already \
             at class creation in an `Enum` body -- while this compiler does not model \
             `__new__` at all (construction is driven entirely by `__init__`), so the binding \
             would be silently ignored rather than honored",
        ),
        "__init_subclass__" => Some(
            "a class attribute named `__init_subclass__` is not supported yet -- Python calls \
             it implicitly when a subclass is created (`class B(C)`), but this compiler resolves \
             that hook by walking the MRO's function definitions and never consults a class \
             attribute of that name, so the binding would be silently ignored rather than \
             honored",
        ),
        _ => None,
    }
}

/// The `C0001` message for an `Enum`-body assignment CPython's
/// `enum._EnumDict.__setitem__` keeps out of the member list, or `None` if the
/// name is an ordinary member (#979, D-238).
///
/// Reached only from [`reject_reserved_class_attr_name`] and only for
/// [`ClassBodyRoute::Enum`]: a plain, `@dataclass` or `Protocol` body has no
/// `_EnumDict` and no member list, so `class C: __repr__ = 1` stays an
/// ordinary class attribute there and is accepted.
///
/// The predicate is deliberately four shapes rather than one, because
/// CPython's own rule is three sibling branches of one function --
/// `_is_private`, `_is_sunder` and `_is_dunder` -- and all three were measured
/// to diverge here at `edc454ba` against CPython 3.13.9 (`_is_private` in two
/// separate source spellings, hence four arms for three branches):
///
/// | enum body in `class C(Enum)` | CPython 3.13.9 | pycc before #979 |
/// |---|---|---|
/// | `__repr__ = 1; B = 2` | 1 member | 2 members |
/// | `__repr__ = "a"; B = "b"` | 1 | 2 |
/// | `_order_ = 'B'; B = 'b'` | 1 | 2 |
/// | `_foo_ = 1; B = 2` | `ValueError` at class creation | 2 |
/// | `__x = 1; B = 2` | 1 | 2 |
/// | `_C__x = 1; B = 2` | 1 | 2 |
/// | `_D__x = 1; B = 2` | 2 | 2 (agrees -- stays accepted) |
/// | `_x = 1; B = 2` | 2 | 2 (agrees -- stays accepted) |
///
/// The order of the arms matters. `__x__` matches the mangled-private prefix
/// too, so the dunder shape is tested first; the class-name-keyed arm is
/// tested before the sunder shape because `_C__x_` matches both and
/// `_is_private` is the branch CPython actually takes for it; the sunder shape
/// is tested last because it can only be reached by a name that does *not*
/// start with two underscores. The whole function runs after the `__slots__`
/// and instantiation-protocol checks for the same reason: those four names are
/// dunder-shaped, and testing this predicate first would replace their pinned
/// messages on the `Enum` route.
///
/// It matches on the *source* spelling, and that is why `_is_private` needs
/// two arms rather than one. pycc has no name-mangling pass, while CPython's
/// compiler mangles a class-body `__x` to `_C__x` *before* `_EnumDict` sees
/// it, and `_is_private` then compares that raw key against the literal
/// `_<ClassName>__` prefix. So the source names CPython keeps out under that
/// one branch are two disjoint sets: `__x` (mangled on the way in -- the
/// second arm's job, and class-name-independent) and a name already spelled
/// `_C__x` in the source of `class C` (never mangled, matched only because the
/// prefix happens to be literal -- the third arm's job, and
/// class-name-*keyed*, since the same spelling inside `class D(Enum)` is an
/// ordinary member).
///
/// **The predicate is a deliberate superset of CPython's non-member set.** It
/// never under-rejects -- every name CPython keeps out of the member list
/// matches one of the four shapes -- so no D-198 false acceptance survives in
/// this family. It over-rejects in four measured ways, all recorded in
/// D-238:
///
/// * Every sunder-shaped name, including the ones
///   `_EnumDict.__setitem__` allowlists rather than raising on (`_order_`,
///   `_generate_next_value_`, `_numeric_repr_`, `_missing_`, `_ignore_`,
///   `_iter_member_`, `_iter_member_by_value_`, `_iter_member_by_def_`,
///   `_add_alias_`, `_add_value_alias_`, and any `_repr_`-prefixed name).
///   None of them is a member under either engine, but CPython *runs* those
///   programs, and pycc models none of the behaviors they request. `_order_`
///   is not even purely conservative: without a guard pycc lowers two members
///   where CPython leaves one.
/// * Names matching only the `__`-prefix-and-suffix *shape*. CPython's
///   `_is_dunder` additionally requires `len > 4`, `name[2] != '_'` and
///   `name[-3] != '_'`, so a name failing any of those stays an ordinary
///   member there. Measured on CPython 3.13.9: `__`, `___`, `____` and
///   `___x___` are all members of their class and are rejected here.
/// * Names with *one* leading underscore and *two or more* trailing ones.
///   CPython's `_is_sunder` also requires `name[-2] != '_'`, so it does not
///   claim them, and neither does any sibling branch. Measured on CPython
///   3.13.9 in `class C(Enum)`: `_x__ = 1` and `_foo___ = 1` are ordinary
///   members (`list(C.__members__)` includes them), while the sunder arm here
///   rejects both. `_C__x__` is the same family reached from the other side --
///   the class-name-keyed arm skips it because it ends in `__`, and the sunder
///   arm then claims it.
/// * A `__x` assignment in a class whose *own* name begins with an underscore.
///   CPython's mangling strips the class name's leading underscores while
///   `_is_private` compares against the unstripped name, so the two disagree
///   there: measured on CPython 3.13.9, `class _C(Enum): __x = 1` beside
///   `B = 2` gives `list(_C.__members__) == ['_C__x', 'B']` -- two members,
///   the first under its mangled name. Modelling that would need a mangling
///   pass, since pycc would otherwise lower the member as `__x` and report
///   the wrong `.name`, so the second arm stays class-name-independent and
///   [`ENUM_PRIVATE_MESSAGE`] states both outcomes instead of claiming the
///   first.
///
/// This is the same posture the `Enum` arm of [`slots_message`] already
/// records, and it is forward-compatible in the direction that matters: a
/// `C0001` "not supported yet" may later be narrowed without breaking a
/// program that was accepted.
fn enum_non_member_message(attr_name: &str, class_name: &str) -> Option<&'static str> {
    if attr_name.starts_with("__") {
        if attr_name.ends_with("__") {
            return Some(ENUM_DUNDER_MESSAGE);
        }
        return Some(ENUM_PRIVATE_MESSAGE);
    }
    let mangled_prefix = format!("_{class_name}__");
    if attr_name.len() > mangled_prefix.len()
        && attr_name.starts_with(&mangled_prefix)
        && !attr_name.ends_with("__")
    {
        return Some(ENUM_PRIVATE_SPELLING_MESSAGE);
    }
    if attr_name.len() > 2 && attr_name.starts_with('_') && attr_name.ends_with('_') {
        return Some(ENUM_SUNDER_MESSAGE);
    }
    None
}

/// The `C0001` message for a dunder-shaped assignment in an `Enum` body
/// (#979).
///
/// Like the other messages in this module it names no value-derived type: the
/// guard runs on the attribute name alone, before any value extraction, so
/// the divergence is stated as a member-count and `__members__` difference
/// rather than as a `TypeError` about a concrete bound type (D-236's #984
/// amendment).
const ENUM_DUNDER_MESSAGE: &str = "a dunder-named assignment in an `Enum` body is not supported yet -- CPython's \
     `enum._EnumDict.__setitem__` keeps a dunder out of the member list (measured on CPython \
     3.13.9: `class C(Enum): __repr__ = 1` \
     alongside `B = 2` gives `list(C.__members__) == [\'B\']`, and `C.__repr__` is the plain \
     `int` `1`, so `C.__repr__.value` raises `AttributeError`; `__order__` is the one dunder \
     that is not even left as a class attribute -- `_EnumDict` rewrites the key to `_order_` \
     and `EnumType.__new__` then pops it out of the class dict entirely), while this compiler \
     has no \
     non-member class attribute on an enum at all -- `lower_enum_class` produces only a \
     compile-time member table -- and would otherwise lower the name as a member, giving two \
     members where CPython has one, so the program is rejected rather than compiled to a \
     different member list";

/// The `C0001` message for a name-mangled private assignment in an `Enum` body
/// (#979).
///
/// A separate string from [`ENUM_DUNDER_MESSAGE`] because the mechanism is a
/// different one and the name is not a dunder: CPython's *compiler* mangles it
/// before `_EnumDict` ever sees it.
///
/// This arm is deliberately class-name-*independent* even though the branch it
/// models is not, and the message says why rather than overclaiming. Mangling
/// strips the class name's own leading underscores while `_is_private`
/// compares against the unstripped name, so the two disagree exactly when the
/// class name begins with an underscore: in `class _C(Enum)`, `__x` mangles to
/// `_C__x`, `_is_private('_C', '_C__x')` tests the `__C__` prefix and says no,
/// and CPython ends up with `list(_C.__members__) == ['_C__x', 'B']` (measured
/// on 3.13.9). That is still not modellable here -- pycc has no mangling pass,
/// so it would lower the member under the source name `__x` and report
/// `.name` as `"__x"` where CPython reports `"_C__x"` -- so the rejection
/// stands and is recorded as the fourth over-rejected family in
/// [`enum_non_member_message`] and D-238.
const ENUM_PRIVATE_MESSAGE: &str = "a name-mangled private assignment in an `Enum` body is not supported yet -- a class-body \
     name with two leading underscores and at most one trailing underscore is rewritten by \
     CPython\'s compiler before `enum._EnumDict` ever sees it (`__x` in `class C` becomes \
     `_C__x`, the class name\'s own leading underscores being stripped first), while this \
     compiler performs no name mangling at all and has no non-member class attribute on an \
     enum, so neither of CPython\'s two outcomes is modellable here: when the mangled key still \
     carries the class\'s `_<ClassName>__` prefix, `_EnumDict.__setitem__` keeps it out of the \
     member list as an ordinary class attribute (measured on CPython 3.13.9: \
     `class C(Enum): __x = 1` alongside `B = 2` gives `list(C.__members__) == [\'B\']`), and \
     when it does not -- a class name that itself begins with an underscore -- CPython admits \
     it as a member under the *mangled* name (`class _C(Enum): __x = 1` gives \
     `list(_C.__members__) == [\'_C__x\', \'B\']`) where this compiler would lower it under the \
     source name `__x`, so the assignment is rejected rather than compiled either way";

/// The `C0001` message for an `Enum`-body assignment already spelled the way
/// CPython's compiler would have mangled a private name (#979).
///
/// The one class-name-keyed message in this module. `_C__x` inside
/// `class C(Enum)` is never mangled -- it has a single leading underscore --
/// but `enum._EnumDict.__setitem__` matches the *raw* key against the literal
/// `_<ClassName>__` prefix, so it lands in the same `_is_private` branch as a
/// mangled `__x` and is kept out of the member list. The identical spelling in
/// `class D(Enum)` is an ordinary member under both engines and stays
/// accepted, which is why this cannot be folded into
/// [`ENUM_PRIVATE_MESSAGE`]'s shape-only test.
const ENUM_PRIVATE_SPELLING_MESSAGE: &str = "an `Enum`-body assignment whose name already carries the mangled-private prefix of its own \
     class is not supported yet -- `enum._EnumDict.__setitem__` matches the raw key against the \
     literal `_<ClassName>__` prefix, so `_C__x` inside `class C(Enum)` is kept out of the \
     member list as an ordinary class attribute even though nothing mangled it (measured on \
     CPython 3.13.9: `class C(Enum): _C__x = 1` alongside `B = 2` gives \
     `list(C.__members__) == [\'B\']`, while the same `_C__x = 1` inside `class D(Enum)` is an \
     ordinary member and stays accepted here), and this compiler has no non-member class \
     attribute on an enum, so it would otherwise lower the name as a member, giving two members \
     where CPython has one";

/// The `C0001` message for a sunder-shaped assignment in an `Enum` body
/// (#979).
///
/// This is the one arm that rejects programs CPython runs without any member
/// divergence, and the message says so rather than claiming a measured
/// divergence for every input; `enum_non_member_message`'s own docs and D-238
/// carry the full allowlist.
const ENUM_SUNDER_MESSAGE: &str = "a sunder-named assignment in an `Enum` body is not supported yet -- CPython's \
     `enum._EnumDict.__setitem__` treats a `_sunder_` name as enum bookkeeping rather than a \
     member: an unrecognized one raises `ValueError: _sunder_ names, such as \'_foo_\', are \
     reserved for future Enum use` while the `class` statement itself executes (measured on \
     CPython 3.13.9), and the recognized ones (`_order_`, `_ignore_`, `_missing_`, \
     `_generate_next_value_` and their siblings) reorder, filter or generate the member list in \
     ways this compiler does not model -- `class C(Enum): _order_ = \'B\'` alongside `B = \'b\'` \
     leaves one member under CPython and would lower as two here -- so every sunder-shaped name \
     is rejected rather than lowered as a member";
