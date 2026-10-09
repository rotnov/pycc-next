//! End-to-end coverage for Part D of #1038 (#1066): exception *identity*
//! across the `ext` boundary.
//!
//! Everything here goes through the public `pycc` CLI and an installed
//! CPython, mirroring `tests/issue_1063_overflow_error.rs`'s `ext` harness.
//! It is the only place the synthesized classes are actually created: every
//! other test of this change asserts generated C *text*, which cannot show
//! that a host-side `except ValueError:` really matches a pycc class or that
//! the class object is the module attribute of the same name.
//!
//! `#[ignore]`d for the reason every `ext` test is: it asks an installed
//! CPython 3.13+ to import a built artifact, which is a property of the
//! machine. CI runs it on every Tier-1 `native-build-test` leg through that
//! job's `cargo test --workspace -- --include-ignored`. It is deliberately
//! not where line coverage comes from -- the coverage job runs `llvm-cov`
//! without `--include-ignored`, so this file holds assertions only.

use pycc_scratch::ScratchDir;
use std::process::Command;

/// The fixture. `class G(ExceptionGroup)` is first on purpose: it is
/// excluded from the synthesized table (a fake group class is worse than an
/// honest `Exception`) but `pycc_hir::program::finalize` still *consumes*
/// the first user tag for it, so every class below it carries a tag one
/// higher than its own table slot. That sparsity is exactly what a
/// cache indexed by `tag - FIRST_USER_EXCEPTION_TYPE_TAG` and sized by entry
/// count would get wrong -- silently, with no compile or link error -- and
/// the assertions on `boom()` below are what catch it.
///
/// `G` is declared but never raised: raising a *subclass* of a PEP 654 group
/// is not compiled today (the member list types as `list[ValueError]`, which
/// D-105 rejects), so `group_boom` raises `ExceptionGroup` itself, which is
/// the tag-23..=24 half of the same regression guard.
const FIXTURE: &str = "class G(ExceptionGroup):\n    pass\n\n\
     class MyError(Exception):\n    pass\n\n\
     class MyValueError(ValueError):\n    pass\n\n\
     class Deep(MyValueError):\n    pass\n\n\
     class Mixin:\n    pass\n\n\
     class Both(Mixin, MyError):\n    pass\n\n\
     def boom() -> int:\n    raise MyError(\"user defined boom\")\n\n\
     def value_boom() -> int:\n    raise MyValueError(\"value boom\")\n\n\
     def deep_boom() -> int:\n    raise Deep(\"deep boom\")\n\n\
     def both_boom() -> int:\n    raise Both(\"both boom\")\n\n\
     def group_boom() -> int:\n    try:\n        raise ValueError(\"inner\")\n    \
     except ValueError as e:\n        raise ExceptionGroup(\"grouped\", [e])\n    return 0\n\n\
     def ok() -> int:\n    return 7\n\n\
     x = 1\n";

const SCRIPT: &str = "import pycc_1066_mod as m\n\
     # The class is a module attribute, qualified with the module name.\n\
     try:\n\
     \x20   m.boom()\n\
     \x20   raise AssertionError('expected MyError')\n\
     except m.MyError as e:\n\
     \x20   assert type(e).__name__ == 'MyError', type(e).__name__\n\
     \x20   assert type(e).__module__ == 'pycc_1066_mod', type(e).__module__\n\
     \x20   assert e.args == ('user defined boom',), e.args\n\
     # A builtin subclass is caught by the builtin, which is the whole point:\n\
     # `except ValueError:` catches `MyValueError` natively too.\n\
     try:\n\
     \x20   m.value_boom()\n\
     \x20   raise AssertionError('expected ValueError')\n\
     except ValueError as e:\n\
     \x20   assert type(e) is m.MyValueError, type(e)\n\
     # A user base is carried as well, so the chain is the native one.\n\
     try:\n\
     \x20   m.deep_boom()\n\
     \x20   raise AssertionError('expected MyValueError')\n\
     except m.MyValueError as e:\n\
     \x20   assert type(e) is m.Deep, type(e)\n\
     \x20   assert m.Deep.__mro__[:3] == (m.Deep, m.MyValueError, ValueError), m.Deep.__mro__\n\
     # Two exception bases become a tuple base, so both match.\n\
     try:\n\
     \x20   m.both_boom()\n\
     \x20   raise AssertionError('expected MyError')\n\
     except m.MyError as e:\n\
     \x20   assert type(e) is m.Both, type(e)\n\
     # The class object is the same across raises, so `except type(prev):` works.\n\
     first = None\n\
     try:\n\
     \x20   m.boom()\n\
     except BaseException as e:\n\
     \x20   first = type(e)\n\
     try:\n\
     \x20   m.boom()\n\
     except first as e:\n\
     \x20   assert type(e) is m.MyError, type(e)\n\
     # C3's regression guards. A PEP 654 group still arrives as a plain\n\
     # `Exception` with its message, and a group-derived class is not\n\
     # synthesized at all -- no module attribute, so nothing fake exists.\n\
     try:\n\
     \x20   m.group_boom()\n\
     \x20   raise AssertionError('expected Exception')\n\
     except BaseException as e:\n\
     \x20   assert type(e) is Exception, type(e)\n\
     \x20   assert e.args == ('grouped',), e.args\n\
     assert not hasattr(m, 'G'), 'a group-derived class must not be synthesized'\n\
     # A non-exception base is dropped from the synthesized class, so\n\
     # `isinstance(e, Mixin)` does not hold host-side (docs/RUNTIME.md).\n\
     # `Mixin` itself is published as a plain class since #1450 (its\n\
     # implicit `object.__init__` makes it constructible), never as an\n\
     # exception class.\n\
     assert not issubclass(m.Mixin, BaseException), 'a non-exception class is not an exception class'\n\
     assert m.Mixin not in m.Both.__mro__, m.Both.__mro__\n\
     assert m.Both.__mro__[:2] == (m.Both, m.MyError), m.Both.__mro__\n\
     # A conforming call still returns normally, so registration left no\n\
     # pending state behind.\n\
     assert m.ok() == 7\n";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_built_ext_module_carries_user_exception_class_identity_to_the_host() {
    let dir = ScratchDir::new("1066_ext").expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(&src, FIXTURE).expect("write the fixture");
    let build = Command::new(std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc")))
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("pycc_1066_mod"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(SCRIPT)
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}
