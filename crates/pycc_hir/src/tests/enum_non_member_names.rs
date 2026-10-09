//! Assignments CPython's `enum._EnumDict` keeps out of an enum's member list
//! ([#979](https://github.com/rotnov/pycc/issues/979), [D-238]).
//!
//! [D-238]: ../../../../docs/decisions/D-238-reject-enum-body-assignments-cpython-keeps-out-of-the.md
//!
//! Its own child module rather than more of `reserved_dunder_class_attrs.rs`
//! for the reason that file gives for itself (AGENTS.md "Keep source files
//! decomposable"): a different name set, a different generating rule, and a
//! route gate the D-236 set does not have. The one D-236 test this change
//! touches -- the inverted `__repr__` pin -- stays in that file, where the
//! history it corrects lives.
//!
//! The defect class is D-198 false acceptance. `enum._EnumDict.__setitem__`
//! has three sibling branches that keep a name out of the member list
//! (`_is_private`, `_is_sunder`, `_is_dunder`); `lower_enum_class` lowered
//! every assignment as a member and has no non-member representation at all,
//! so all three diverged. Measured at `edc454ba` against CPython 3.13.9:
//! `__repr__ = 1; B = 2` and `__x = 1; B = 2` each gave two members here and
//! one there, `_order_ = 'B'; B = 'b'` likewise, `_C__x = 1; B = 2` inside
//! `class C(Enum)` likewise, and `_foo_ = 1; B = 2` compiled here while
//! CPython raised `ValueError` at class creation.
//!
//! Four properties are pinned, each of which a narrower fix would have
//! missed:
//!
//! 1. **Four shapes, not one.** The issue proposed a dunder-only predicate,
//!    which leaves `__x`, `_C__x` and both sunder shapes accepted.
//! 2. **`_is_private` is class-name-keyed.** It matches the *raw* key against
//!    the literal `_<ClassName>__` prefix, so `_C__x` is a non-member inside
//!    `class C(Enum)` and an ordinary member inside `class D(Enum)`. A
//!    shape-only `_*__*` test would over-reject the second.
//! 3. **`_x = 1` is not in the set.** CPython lowers it as an ordinary member
//!    and so does pycc; the guard must not swallow it, nor `_foo = 1`.
//! 4. **The guard is `Enum`-route-only.** A plain or `@dataclass` body has no
//!    member list, so the same names stay ordinary class attributes there.
//!
//! Every name is exercised on its own class body, never combined: the enum
//! member loop returns on the first error, so a body listing several would
//! only ever prove the first one fires.

use super::*;

/// The four rejected shapes and the distinctive fragment of each one's
/// message.
///
/// Every entry is written for a class named `C`, because the fourth is
/// class-name-keyed.
///
/// The messages are deliberately distinct rather than one template: the
/// mechanisms differ (a dunder is skipped by `_EnumDict`, a private name is
/// mangled by the *compiler* before `_EnumDict` sees it, an already-mangled
/// spelling is matched literally against the class's own prefix, a sunder is
/// enum bookkeeping that may raise `ValueError`), so a shared string would be
/// a false account for three of the four.
const NON_MEMBER: [(&str, &str); 4] = [
    (
        "__repr__",
        "a dunder-named assignment in an `Enum` body is not supported yet",
    ),
    (
        "__x",
        "a name-mangled private assignment in an `Enum` body is not supported yet",
    ),
    (
        "_C__x",
        "an `Enum`-body assignment whose name already carries the mangled-private prefix of \
         its own class is not supported yet",
    ),
    (
        "_foo_",
        "a sunder-named assignment in an `Enum` body is not supported yet",
    ),
];

/// Each shape is rejected with its own message, on an `int`-valued enum.
#[test]
fn each_enum_non_member_shape_is_rejected_with_its_own_message() {
    for (name, needle) in NON_MEMBER {
        assert_capability_error_message(
            &format!("from enum import Enum\n\n\nclass C(Enum):\n    {name} = 1\n    B = 2\n"),
            needle,
        );
    }
}

/// The value kind is irrelevant: the guard runs on the name alone, before any
/// value extraction, so a `str`-valued enum reports the same message.
///
/// This is the shape the *accidental* rejections miss. Before #979 two of the
/// nine measured shapes were already rejected, but only because the value was
/// non-literal or mismatched the enum's `int`/`str` kind -- `_order_ = 'B'`
/// beside `B = 2` was a kind mismatch, while `_order_ = 'B'` beside `B = 'b'`
/// compiled to two members where CPython has one.
#[test]
fn a_str_valued_enum_reports_the_same_messages() {
    for (name, needle) in NON_MEMBER {
        assert_capability_error_message(
            &format!(
                "from enum import Enum\n\n\nclass C(Enum):\n    {name} = \"a\"\n    B = \"b\"\n"
            ),
            needle,
        );
    }
}

/// `_order_` specifically: a sunder CPython *recognizes* rather than raising
/// on, and the one whose rejection is not purely conservative.
///
/// CPython 3.13.9 accepts `class C(Enum): _order_ = 'B'` with `B = 'b'` and
/// leaves one member; pycc lowered two. Rejecting it is therefore a fix, not
/// only conservatism -- unlike the other recognized sunders, whose programs
/// CPython simply runs.
#[test]
fn a_recognized_sunder_is_rejected_too() {
    assert_capability_error_message(
        "from enum import Enum\n\n\nclass C(Enum):\n    _order_ = \"B\"\n    B = \"b\"\n",
        "a sunder-named assignment in an `Enum` body is not supported yet",
    );
}

/// A single leading underscore is not in the set. CPython lowers `_x` as an
/// ordinary member (`list(C.__members__) == ['_x', 'B']`) and so does pycc, so
/// the guard must fall through.
///
/// This case also keeps the enum member loop's value-extraction path covered
/// on an accepting input, which the two inverted #975 pins no longer do.
#[test]
fn a_single_underscore_member_stays_accepted() {
    let module = pycc_parser_test_helper::parse(
        "from enum import Enum\n\n\nclass C(Enum):\n    _x = 1\n    B = 2\n",
    );
    lower_checked(&module).expect("`_x` must still lower as an enum member");
}

/// A leading underscore with no trailing one and more than two characters:
/// `_foo`. CPython lowers it as an ordinary member.
///
/// It is the only shape in this file that reaches the sunder arm's trailing
/// `ends_with('_')` test and falls through it -- `_x` is too short for the
/// length test, and `__x` is taken by the private arm first -- so without it
/// that operand is never exercised in the falling-through direction.
#[test]
fn a_longer_single_underscore_member_stays_accepted() {
    let module = pycc_parser_test_helper::parse(
        "from enum import Enum\n\n\nclass C(Enum):\n    _foo = 1\n    B = 2\n",
    );
    lower_checked(&module).expect("`_foo` must still lower as an enum member");
}

/// The guard is route-gated, not global: in a plain class body every one of
/// the four shapes is still an ordinary class attribute under both engines
/// and stays accepted.
///
/// `dunders_outside_the_instantiation_protocol_stay_accepted_in_a_plain_class`
/// in the sibling module pins the dunder half of this for D-236's own set;
/// this pins the three shapes #979 adds, which that test does not cover.
#[test]
fn the_non_member_shapes_stay_accepted_in_a_plain_class() {
    for name in ["__repr__", "__x", "_C__x", "_foo_", "_order_"] {
        let module =
            pycc_parser_test_helper::parse(&format!("class C:\n    {name} = 8\n\n\nc = C()\n"));
        lower_checked(&module).unwrap_or_else(|e| {
            panic!("`{name}` must still lower in a plain class body: {e:?}");
        });
    }
}

/// D-236's names are dunder-shaped too, so the new check must run *after*
/// theirs. This pins that ordering from the other side: on the `Enum` route
/// each of the four still reports its own D-236 message, not #979's.
///
/// Without the ordering the failure would be silent --
/// `the_enum_slots_message_describes_the_enum_route` asserts the *absence* of
/// the plain-class string, so it would still pass while the wrong message was
/// emitted.
#[test]
fn the_d236_names_keep_their_own_messages_on_the_enum_route() {
    for (name, needle) in [
        (
            "__slots__",
            "`__slots__` in an `Enum` body is not supported yet",
        ),
        (
            "__init__",
            "a class attribute named `__init__` is not supported yet",
        ),
        (
            "__new__",
            "a class attribute named `__new__` is not supported yet",
        ),
        (
            "__init_subclass__",
            "a class attribute named `__init_subclass__` is not supported yet",
        ),
    ] {
        assert_capability_error_message(
            &format!("from enum import Enum\n\n\nclass C(Enum):\n    {name} = 1\n    B = 2\n"),
            needle,
        );
    }
}

/// The class-name-keyed arm really is keyed: the same `_C__x = 1` spelling
/// inside `class D(Enum)` stays an ordinary member.
///
/// CPython agrees -- `_is_private('D', '_C__x')` compares against the literal
/// `_D__` prefix and does not match, so `list(D.__members__) == ['_C__x', 'B']`
/// (measured on CPython 3.13.9). A shape-only `_*__*` predicate would have
/// rejected this program, turning the fix into a fresh over-rejection.
#[test]
fn the_mangled_spelling_of_another_class_stays_accepted() {
    let module = pycc_parser_test_helper::parse(
        "from enum import Enum\n\n\nclass D(Enum):\n    _C__x = 1\n    B = 2\n",
    );
    lower_checked(&module).expect("`_C__x` must still lower as a member of `class D`");
}

/// A name carrying the class's own mangled prefix *and* a trailing `__` is
/// outside `_is_private` (CPython requires the name not to end in two
/// underscores) and is claimed by the sunder arm instead.
///
/// It pins the class-name-keyed arm's `!ends_with("__")` operand in the
/// falling-through direction, and records the over-rejection honestly:
/// `_C__x__ = 1` is an ordinary member on CPython 3.13.9
/// (`list(C.__members__) == ['_C__x__', 'B']`), and the sunder message is the
/// one pycc reports.
#[test]
fn a_mangled_prefix_with_a_trailing_dunder_falls_through_to_the_sunder_arm() {
    assert_capability_error_message(
        "from enum import Enum\n\n\nclass C(Enum):\n    _C__x__ = 1\n    B = 2\n",
        "a sunder-named assignment in an `Enum` body is not supported yet",
    );
}

/// The two shape-only over-rejections are pinned behaviorally, not only in
/// prose.
///
/// `reserved_names.rs` and D-238 record both as *measured* over-rejections:
/// `__`, `___`, `____` and `___x___` fail `_is_dunder`'s `len > 4`,
/// `name[2] != '_'` and `name[-3] != '_'` conditions, and `_x__` / `_foo___`
/// fail `_is_sunder`'s `name[-2] != '_'`, so CPython 3.13.9 lowers all six as
/// ordinary members while pycc rejects them. D-238's Alternatives keeps
/// narrowing either arm toward CPython's exact shape as a compatible
/// follow-up; without this test that narrowing would flip six documented
/// rejections to acceptances with nothing failing, because every branch
/// involved is already reached by other inputs and D-014's gate measures
/// reachability rather than outcome.
#[test]
fn the_shape_only_over_rejections_are_pinned() {
    for name in ["__", "___", "____", "___x___"] {
        assert_capability_error_message(
            &format!("from enum import Enum\n\n\nclass C(Enum):\n    {name} = 1\n    B = 2\n"),
            "a dunder-named assignment in an `Enum` body is not supported yet",
        );
    }
    for name in ["_x__", "_foo___"] {
        assert_capability_error_message(
            &format!("from enum import Enum\n\n\nclass C(Enum):\n    {name} = 1\n    B = 2\n"),
            "a sunder-named assignment in an `Enum` body is not supported yet",
        );
    }
}

/// A `__x` assignment in a class whose own name begins with an underscore is
/// rejected too, and the message is written not to overclaim it.
///
/// CPython's compiler strips the class name's leading underscores when
/// mangling while `_is_private` compares against the unstripped name, so they
/// disagree exactly here: measured on CPython 3.13.9,
/// `class _C(Enum): __x = 1` beside `B = 2` gives
/// `list(_C.__members__) == ['_C__x', 'B']` -- two members, the first under
/// its *mangled* name. pycc has no mangling pass and would lower it under the
/// source name `__x`, reporting the wrong `.name`, so the rejection stands as
/// the fourth documented over-rejection rather than as a claim that CPython
/// keeps the name out of the member list.
#[test]
fn a_private_name_in_an_underscored_class_is_still_rejected() {
    for class_name in ["_C", "__C", "___"] {
        assert_capability_error_message(
            &format!(
                "from enum import Enum\n\n\nclass {class_name}(Enum):\n    __x = 1\n    B = 2\n"
            ),
            "a name-mangled private assignment in an `Enum` body is not supported yet",
        );
    }
}
