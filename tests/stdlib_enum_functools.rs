//! `enum.Enum`/`IntEnum`/`StrEnum` (printing, value lookup, `match`,
//! hashing, dict keys) and `functools.cache`/`lru_cache`
//! (`pycc_ast::desugar::functools`) against CPython 3.13: natively (strict
//! and `--non-strict`) and as an `--ext` module; an arbitrary decorator
//! runs in CPython under `--non-strict`.

#[path = "stdlib_support/mod.rs"]
mod support;

use support::{assert_ext_matches, assert_native_matches, check_fails, fixture, python, run_native};

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn enums_match_cpython() {
    assert_native_matches("stdlib_enum.py");
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn functools_caches_match_cpython() {
    assert_native_matches("stdlib_functools_cache.py");
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_ext_module_with_enums_and_a_cache_matches_its_source() {
    assert_ext_matches(
        "stdlib_enum_cache_ext.py",
        "import stdlib_enum_cache_ext as m\n\
         print(m.pick(1), m.pick(2), m.same(1, 1), m.same(1, 2))\n\
         print(m.level_text(5), m.fib(80))\n\
         try:\n    m.pick(7)\nexcept ValueError as e:\n    print('ValueError', e)\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_arbitrary_decorator_runs_in_cpython_under_non_strict() {
    let source = fixture("user_decorator_non_strict.py");
    let expected = support::outcome(
        &std::process::Command::new(python())
            .arg(&source)
            .output()
            .expect("the oracle runs"),
    );
    assert_eq!(
        run_native("stdlib_user_decorator", &source, &["--non-strict"]),
        expected
    );
}

#[test]
fn shapes_left_out_are_refused() {
    let rendered = check_fails("def deco(f):\n    return f\n\n@deco\ndef f(x: int) -> int:\n    return x\n");
    assert!(rendered.contains("function decorators are not supported yet"), "{rendered}");
    // An unannotated cached function keeps its decorator, which HIR refuses.
    let rendered = check_fails(
        "from functools import cache\n\n@cache\ndef f(x):\n    return x\n\nf(1)\n",
    );
    assert!(rendered.contains("function decorators are not supported yet"), "{rendered}");
    // `IntEnum` values are integers.
    let rendered = check_fails("from enum import IntEnum\n\nclass L(IntEnum):\n    A = \"a\"\n");
    assert!(rendered.contains("must be assigned an integer literal"), "{rendered}");
    // A dict key comparing by `__eq__`.
    let rendered = check_fails(
        "class K:\n    def __eq__(self, other: K) -> bool:\n        return True\n\n\
         d = {K(): 1}\n",
    );
    assert!(rendered.contains("a dict key of class `K`"), "{rendered}");
}
