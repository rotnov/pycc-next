//! Unit tests for `class/exception_dunders.rs` (#1337, WI-6b).

use crate::class::tests::lower_ok;
use crate::lower_checked;

fn refusal(source: &str) -> String {
    let module = crate::pycc_parser_test_helper::parse(source);
    let diagnostic = lower_checked(&module).expect_err("the class should be refused");
    assert_eq!(diagnostic.code, "C0001", "{source}");
    diagnostic.message
}

const STR_MIXIN: &str =
    "class Mixin:\n    def __str__(self) -> str:\n        return \"custom\"\n\n";
const BOOL_MIXIN: &str = "class Mixin:\n    def __bool__(self) -> bool:\n        return False\n\n";

#[test]
fn a_mixin_str_before_the_builtin_base_is_refused() {
    let message = refusal(&format!(
        "{STR_MIXIN}class E(Mixin, ValueError):\n    pass\n"
    ));
    assert!(
        message.contains("exception class `E` has a user-defined `__str__` (defined by `Mixin`)"),
        "{message}"
    );
    assert!(message.contains("Part 3 of #541"), "{message}");
}

#[test]
fn a_builtin_base_before_the_mixin_masks_only_str_and_repr() {
    // `ValueError.__str__` wins, as CPython's `BaseException.__str__` does.
    lower_ok(&format!(
        "{STR_MIXIN}class E(ValueError, Mixin):\n    pass\n"
    ));
    let repr = "class Mixin:\n    def __repr__(self) -> str:\n        return \"r\"\n\n\
                class E(ValueError, Mixin):\n    pass\n";
    lower_ok(repr);
    // `__bool__` comes from `object`, last in CPython's MRO: the mixin wins.
    let message = refusal(&format!(
        "{BOOL_MIXIN}class E(ValueError, Mixin):\n    pass\n"
    ));
    assert!(
        message.contains("`__bool__` (defined by `Mixin`)"),
        "{message}"
    );
}

#[test]
fn a_dunder_on_the_exception_class_itself_is_refused() {
    let message =
        refusal("class E(ValueError):\n    def __bool__(self) -> bool:\n        return False\n");
    assert!(
        message.contains("`E` has a user-defined `__bool__`,"),
        "{message}"
    );
    // Through an intermediate user exception class, as a property too.
    let message = refusal(
        "class E(Exception):\n    @property\n    def __len__(self) -> int:\n        return 1\n\n\
         class F(E):\n    pass\n",
    );
    assert!(message.contains("exception class `E`"), "{message}");
}

#[test]
fn constructors_hooks_and_ordinary_members_are_accepted() {
    lower_ok(
        "class E(ValueError):\n    def __init__(self, m: str) -> None:\n        self.m = m\n    \
         def describe(self) -> str:\n        return self.m\n",
    );
    lower_ok("class E(KeyError):\n    pass\n\nclass F(E):\n    pass\n");
}

#[test]
fn a_class_that_is_not_an_exception_keeps_its_dunders() {
    lower_ok(&format!("{STR_MIXIN}class C(Mixin):\n    pass\n"));
    // A user class shadowing a builtin exception name is an ordinary class.
    lower_ok(
        "class ValueError:\n    pass\n\nclass E(ValueError):\n    def __str__(self) -> str:\n        \
         return \"e\"\n",
    );
}
