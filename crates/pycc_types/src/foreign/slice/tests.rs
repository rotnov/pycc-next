//! Typing of a slice load on a CPython object (Part 2b of #1371).

use pycc_diag::Span;
use pycc_hir::ImportBinding;

fn check_foreign(source: &str) -> Result<(), Vec<pycc_diag::Diagnostic>> {
    let module = pycc_parser::parse(source).expect("test source must parse");
    let mut hir = pycc_hir::lower_checked(&module).expect("test source must lower");
    hir.imports.push(ImportBinding::Foreign {
        local_name: "numpy".to_string(),
        module_path: "numpy".to_string(),
        from: None,
        site: pycc_hir::ForeignImportSite::Item(0),
        span: Span::new(0, 0),
    });
    crate::check_all(&hir).map(|_| ())
}

fn assert_refused(source: &str, code: &str, phrase: &str) {
    let diagnostics = check_foreign(source).expect_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?} for {source}");
    assert_eq!(diagnostics[0].code, code, "{diagnostics:?} for {source}");
    assert!(
        diagnostics[0].message.contains(phrase),
        "{diagnostics:?} for {source}"
    );
}

/// Every bound shape -- each omitted, and each packable bound type -- is
/// admitted, and the result is an object: `len()` of it type-checks, and so
/// does an unannotated helper returning it, which only the constraint
/// solver's `Slice` arm types.
#[test]
fn an_object_slice_with_packable_bounds_is_an_object() {
    for source in [
        "s = numpy.pi[:]\n",
        "s = numpy.pi[1:]\n",
        "s = numpy.pi[:-2]\n",
        "s = numpy.pi[::2]\n",
        "s = numpy.pi[1:5:2]\n",
        "s = numpy.pi[True:'a':1.5]\n",
        "s = numpy.pi[numpy.e:]\n",
        "n = len(numpy.pi[1:])\n",
        "def f(size: int) -> int:\n    o = numpy.pi\n    s = o[-size:]\n    return len(s)\n",
        "def _h():\n    return numpy.pi[1:]\n\n\nn = len(_h())\n",
    ] {
        if let Err(diagnostics) = check_foreign(source) {
            panic!("must type-check: {source}\n{diagnostics:?}");
        }
    }
}

/// A bound with no packer is refused, wherever it stands, and the first
/// unpackable bound in source order is the one reported.
#[test]
fn an_object_slice_with_an_unpackable_bound_is_refused() {
    for (source, ty) in [
        ("xs = [1]\ns = numpy.pi[xs:]\n", "list[int]"),
        ("s = numpy.pi[:None]\n", "None"),
        ("xs = [1]\ns = numpy.pi[1::xs]\n", "list[int]"),
        ("s = numpy.pi[None:[1]]\n", "None"),
    ] {
        assert_refused(
            source,
            "I0404",
            &format!("slicing a CPython object with a `{ty}` bound"),
        );
    }
}

/// A non-object base keeps its native diagnostics.
#[test]
fn a_native_base_keeps_its_t0033() {
    assert_refused("x = 1\ns = x[1:]\n", "T0033", "does not support slicing");
}

/// Part 2c of #1371: `del o[a:b:c]` on an object is admitted with every
/// bound shape, in a module body and in a function body, including inside
/// an unannotated helper (the constraint solver's arm, which must also
/// walk the operands so a helper called from one is inferred) and a
/// generic function (the monomorphizer's rewrite), and with a generic
/// function call or generic class instantiation in a bound (the
/// monomorphizer's call rewrite and class-instantiation collector).
#[test]
fn an_object_slice_delete_is_admitted() {
    for source in [
        "del numpy.pi[:]\n",
        "del numpy.pi[1:]\n",
        "del numpy.pi[:-2]\n",
        "del numpy.pi[::2]\n",
        "del numpy.pi[True:'a':1.5]\n",
        "del numpy.pi[numpy.e:]\n",
        "o = numpy.pi\ndel o[1:], o[:1]\n",
        "def f(size: int) -> None:\n    o = numpy.pi\n    del o[-size:]\n",
        "def f(size: int) -> int:\n    o = numpy.pi\n    del o[-size:]\n    return size\n",
        "def _h():\n    del numpy.pi[1:]\n\n\n_h()\n",
        "def _f(x):\n    return x\n\n\ndel numpy.pi[_f(1):_f(2):_f(1)]\n",
        "def _f(x):\n    return x\n\n\ndel _f(numpy.pi)[1:]\n",
        "def _h(n):\n    del numpy.pi[n:]\n\n\n_h(1)\n",
        "def g[T](x: T) -> T:\n    return x\n\n\ndel numpy.pi[g(1):g(2):g(1)]\n",
        "def g[T](x: T) -> T:\n    return x\n\n\ndef f() -> None:\n    del numpy.pi[g(1):]\n",
        "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\n\
         class C:\n    def __init__(self) -> None:\n        self.x = 0\n    \
         def foo(self) -> int:\n        return 1\n\
         def proto_fn(p: P) -> int:\n    return p.foo()\n\
         def caller() -> None:\n    del numpy.pi[proto_fn(c):proto_fn(c):proto_fn(c)]\n\
         c = C()\ndel numpy.pi[proto_fn(c):]\ncaller()\n",
        "class Box[T]:\n    def __init__(self, x: T) -> None:\n        self.x = x\n\n    \
         def size(self) -> int:\n        return 1\n\n\n\
         del numpy.pi[Box(1).size():Box(2).size():Box(1).size()]\n",
        "def g[T](x: T) -> T:\n    del numpy.pi[1:]\n    return x\n\n\nn = g(1)\n",
    ] {
        if let Err(diagnostics) = check_foreign(source) {
            panic!("must type-check: {source}\n{diagnostics:?}");
        }
    }
}

/// A native base keeps the `C0001` HIR used to report, now located at the
/// slice target; an unpackable bound is the load's `I0404`.
#[test]
fn a_native_or_unpackable_slice_delete_is_refused() {
    let source = "xs = [1, 2]\ndel xs[0:1]\n";
    let diagnostics = check_foreign(source).expect_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, "C0001");
    assert!(
        diagnostics[0].message.contains("a `del` of a slice"),
        "{diagnostics:?}"
    );
    assert_eq!(diagnostics[0].span, Some(Span::new(16, 23)));
    assert_refused(
        "def f() -> None:\n    xs = [1]\n    del xs[1:]\n",
        "C0001",
        "a `del` of a slice",
    );
    assert_refused(
        "xs = [1]\ndel numpy.pi[xs:]\n",
        "I0404",
        "slicing a CPython object with a `list[int]` bound",
    );
    assert_refused(
        "del numpy.pi[:None]\n",
        "I0404",
        "slicing a CPython object with a `None` bound",
    );
}

/// An operand the constraint solver rejects inside an unannotated helper
/// surfaces that rejection: the solver's `DeleteSlice` arm propagates it.
#[test]
fn a_solver_error_in_a_slice_delete_operand_is_reported() {
    // `_identity` is inferred `int -> int` from its first call; the nested
    // `str` call conflicts with it, which only the solver reports.
    let source = "def _identity(value):\n    return value\n\n\n\
                  def _sink(value: int) -> int:\n    return value\n\n\n\
                  _identity(1)\n\n\n\
                  def _probe():\n    del numpy.pi[_sink(_identity(\"wrong\")):]\n";
    let diagnostics = check_foreign(source).expect_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, "T0021", "{diagnostics:?}");
}

/// A call of a protocol-parameter function in a slice `del` operand is
/// rewritten to its specialization: monomorphization drops the original
/// `proto_fn` item, so an unrewritten call would dangle.
#[test]
fn a_protocol_call_in_a_slice_delete_operand_is_specialized() {
    let source = "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\n\
                  class C:\n    def __init__(self) -> None:\n        self.x = 0\n    \
                  def foo(self) -> int:\n        return 1\n\
                  def proto_fn(p: P) -> int:\n    return p.foo()\n\
                  def caller() -> None:\n    del xs[proto_fn(c):proto_fn(c):proto_fn(c)]\n\
                  xs = [1, 2]\nc = C()\ndel xs[proto_fn(c):]\ncaller()\n";
    // The rewrite is type-blind and runs before the slice `del` base check,
    // so a native base exercises it without seeding a foreign import.
    let hir =
        pycc_hir::lower_checked(&pycc_parser::parse(source).expect("parses")).expect("lowers");
    let rewritten = format!(
        "{:?}",
        crate::monomorphize::monomorphize(&hir)
            .expect("monomorphizes")
            .items
    );
    assert!(
        !rewritten.contains("callee: \"proto_fn\""),
        "a dangling `proto_fn` call survived: {rewritten}"
    );
}
