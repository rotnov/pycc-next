//! `dataclasses.dataclass` (`pycc_ast::desugar::dataclasses`) and the
//! instance protocol it compiles to -- `__str__`/`__repr__` in `print`,
//! `str()`, `repr()` and f-strings (`!r`/`!s`), the comparison dunders,
//! identity `==`/`is`, `hash()` of `str`/`float`/tuples -- against CPython
//! 3.13: natively (strict and `--non-strict`) and as an `--ext` module.

#[path = "stdlib_support/mod.rs"]
mod support;

use support::{assert_ext_matches, assert_native_matches, check_fails};

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn dataclasses_match_cpython() {
    assert_native_matches("stdlib_dataclasses.py");
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn instance_dunders_match_cpython() {
    assert_native_matches("instance_dunders.py");
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_ext_dataclass_matches_its_source() {
    assert_ext_matches(
        "stdlib_dataclasses_ext.py",
        "import stdlib_dataclasses_ext as m\n\
         p = m.make(3)\n\
         print(p, repr(p), str(p), m.show(p), [p])\n\
         q = m.Point(1)\n\
         print(q, q == m.Point(1), q < p, p > q, m.Point(1, 2, 'z'), m.Point(x=4, label='k'))\n\
         k = m.Key('a')\n\
         print(k, k == m.Key('a'), hash(k) == hash(m.Key('a')), len({k, m.Key('a')}))\n\
         try:\n    k.n = 3\nexcept AttributeError as e:\n    print(type(e).__name__, e)\n",
    );
}

#[test]
fn shapes_left_out_are_refused() {
    // An unknown option keeps the class decorated, which HIR refuses.
    let rendered = check_fails(
        "from dataclasses import dataclass\n\n@dataclass(slots=True)\nclass P:\n    x: int\n",
    );
    assert!(rendered.contains("C0001"), "{rendered}");
    // CPython's `non-default argument follows default argument`.
    let rendered = check_fails(
        "from dataclasses import dataclass\n\n@dataclass\nclass P:\n    x: int = 1\n    y: int\n",
    );
    assert!(rendered.contains("C0001"), "{rendered}");
    // An ordering comparison of a class without the dunder.
    let rendered = check_fails(
        "class P:\n    pass\n\ndef f(a: P, b: P) -> bool:\n    return a < b\n",
    );
    assert!(rendered.contains("cannot compare `P` and `P`"), "{rendered}");
    // `repr()` of an instance whose class defines only `__str__`.
    let rendered = check_fails(
        "class P:\n    def __str__(self) -> str:\n        return \"p\"\n\nx = repr(P())\n",
    );
    assert!(rendered.contains("C0001"), "{rendered}");
}
