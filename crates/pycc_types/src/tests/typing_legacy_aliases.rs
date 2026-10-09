//! #1378 (Part 6 of #882): the `TypingFormMarker` symbols -- `typing.Dict`,
//! `List`, `Set`, `FrozenSet`, `Tuple`, `Any`, `Generic` -- used as a value
//! or called. Each is registered only so `from typing import ...` resolves;
//! none is a first-class value, and all of them get the one "typing
//! construct" message rather than a borrowed marker message.
//!
//! Every dispatch site is exercised: the value-position `match` and the
//! call-position `if`-chain in both the validation pass (`expr.rs`, reached
//! from module level or a fully annotated function) and the solver pass
//! (`constraints.rs`, reached from a private helper's body). The two
//! `if`-chains fall back to the generic "base class marker or decorator"
//! message, so a missing branch would compile and still be wrong.

use super::*;

const TYPING_FORM_NAMES: [&str; 7] = [
    "Dict",
    "List",
    "Set",
    "FrozenSet",
    "Tuple",
    "Any",
    "Generic",
];

fn assert_typing_form_error(source: &str, name: &str) {
    let err = check_source(source).unwrap_err();
    assert_eq!(err.code, "T0021", "{source:?}");
    assert_eq!(
        err.message,
        format!(
            "`typing.{name}` is a typing construct, not a first-class value — pycc does not support using it as a value"
        ),
        "{source:?}"
    );
}

#[test]
fn a_qualified_typing_form_used_as_a_value_is_t0021_in_the_validation_pass() {
    for name in TYPING_FORM_NAMES {
        assert_typing_form_error(
            &format!("import typing\nx = typing.{name}\nprint(x)\n"),
            name,
        );
    }
}

#[test]
fn a_qualified_typing_form_used_as_a_value_is_t0021_in_the_solver_pass() {
    for name in TYPING_FORM_NAMES {
        assert_typing_form_error(
            &format!(
                "import typing\ndef _helper() -> int:\n    x = typing.{name}\n    return 1\nprint(_helper())\n"
            ),
            name,
        );
    }
}

#[test]
fn a_qualified_typing_form_called_is_t0021_in_the_validation_pass() {
    for name in TYPING_FORM_NAMES {
        assert_typing_form_error(&format!("import typing\nx = typing.{name}()\n"), name);
        assert_typing_form_error(
            &format!(
                "import typing\ndef f() -> int:\n    x = typing.{name}()\n    return 1\nprint(f())\n"
            ),
            name,
        );
    }
}

#[test]
fn a_qualified_typing_form_called_is_t0021_in_the_solver_pass() {
    for name in TYPING_FORM_NAMES {
        assert_typing_form_error(
            &format!(
                "import typing\ndef _helper() -> int:\n    x = typing.{name}()\n    return 1\n_helper()\n"
            ),
            name,
        );
    }
}

#[test]
fn a_from_imported_typing_form_does_not_bind_a_value() {
    // Series-plan correction C-3 of #882: registering a symbol makes the
    // import resolve, but does not bind the bare name for value use -- the
    // same as `from typing import Final` today. The bare name is an
    // ordinary undefined name.
    let err = check_source("from typing import Dict\ny = Dict\nprint(y)\n").unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "name `Dict` is not defined");
}
