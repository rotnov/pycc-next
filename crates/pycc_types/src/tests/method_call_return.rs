//! #1420: an unannotated private method whose return is a method call on a
//! concrete user-class instance infers the resolved method's return type,
//! and an inherited copy whose inferred return drifts from its origin's is
//! refused (`T0022`) rather than miscompiled.

use super::*;

fn return_of(source: &str, function: &str) -> Ty {
    let hir = check_source(source).unwrap_or_else(|err| {
        panic!(
            "{source}\nexpected to type-check, got {}: {}",
            err.code, err.message
        )
    });
    hir.items
        .iter()
        .find_map(|item| match item {
            HirItem::Function {
                name, return_ty, ..
            } if name == function => Some(return_ty.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no item `{function}`"))
}

fn refused(source: &str, code: &str, message: &str) {
    let err = check_source(source).expect_err(source);
    assert_eq!(err.code, code, "{source}: {}", err.message);
    assert!(
        err.message.contains(message),
        "{source}\nexpected a message containing {message:?}, got {:?}",
        err.message
    );
}

const CLASS: &str = "class C:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n    \
    def value(self) -> int:\n        return self.n\n\n    def label(self) -> str:\n        \
    return \"c\"\n\n    def clone(self) -> \"C\":\n        return C(self.n)\n";

#[test]
fn a_self_method_call_return_is_inferred() {
    let source = format!("{CLASS}\n    def _v(self):\n        return self.value()\n");
    assert_eq!(return_of(&source, "C._v"), Ty::Int);
}

#[test]
fn a_dunder_returning_a_method_call_infers_the_class_instance() {
    let source = format!("{CLASS}\n    def __copy__(self):\n        return self.clone()\n");
    assert_eq!(
        return_of(&source, "C.__copy__"),
        Ty::Instance(Box::new("C".to_string()))
    );
}

#[test]
fn a_method_call_on_a_parameter_or_local_instance_is_inferred() {
    let source = format!(
        "{CLASS}\n\ndef _of(c: C):\n    return c.label()\n\ndef _made(p: C):\n    c = p\n    return c.value()\n"
    );
    assert_eq!(return_of(&source, "_of"), Ty::Str);
    assert_eq!(return_of(&source, "_made"), Ty::Int);
}

#[test]
fn an_inherited_method_resolves_through_the_mro() {
    let source = format!(
        "{CLASS}\n\nclass D(C):\n    def _l(self):\n        return self.label()\n\n\
         def _d(d: D):\n    return d.value()\n"
    );
    assert_eq!(return_of(&source, "D._l"), Ty::Str);
    assert_eq!(return_of(&source, "_d"), Ty::Int);
}

#[test]
fn a_chain_of_inferred_helpers_resolves_in_any_source_order() {
    // `_a` is declared before the helper it calls, which is itself inferred.
    let source = format!(
        "{CLASS}\n    def _a(self):\n        return self._b()\n\n    def _b(self):\n        return self.value() + 1\n"
    );
    assert_eq!(return_of(&source, "C._a"), Ty::Int);
    assert_eq!(return_of(&source, "C._b"), Ty::Int);
}

#[test]
fn self_recursion_is_refused_without_hanging() {
    let source = format!("{CLASS}\n    def _f(self):\n        return self._f()\n");
    refused(
        &source,
        "T0021",
        "cannot infer return type of private helper `C._f`",
    );
}

#[test]
fn recursion_with_a_base_case_resolves_from_the_base_case() {
    // The recursive call adds nothing of its own; the base-case return
    // constrains the helper.
    let source = format!(
        "{CLASS}\n    def _f(self, n: int):\n        if n == 0:\n            return 1\n        \
         return self._f(n - 1)\n"
    );
    assert_eq!(return_of(&source, "C._f"), Ty::Int);
}

#[test]
fn mutual_recursion_is_refused_without_hanging() {
    let source = format!(
        "{CLASS}\n    def _a(self):\n        return self._b()\n\n    def _b(self):\n        return self._a()\n"
    );
    refused(
        &source,
        "T0021",
        "cannot infer return type of private helper `C._",
    );
}

#[test]
fn an_unknown_method_answers_nothing_and_keeps_the_prior_refusal() {
    let source = format!("{CLASS}\n    def _m(self):\n        return self.missing()\n");
    refused(
        &source,
        "T0021",
        "cannot infer return type of private helper `C._m`",
    );
}

#[test]
fn a_method_call_on_a_builtin_receiver_is_not_answered_here() {
    // `str.upper()` is typed by the check phase, never by the user-class
    // lookup; the helper still needs an annotation as before.
    refused(
        "def _u(s: str):\n    return s.upper()\n",
        "T0021",
        "cannot infer return type of private helper `_u`",
    );
}

#[test]
fn a_generic_method_return_is_not_answered() {
    refused(
        "class Box[T]:\n    \
         def __init__(self, item: T) -> None:\n        self.item = item\n\n    \
         def fetch(self) -> T:\n        return self.item\n\n    \
         def _g(self):\n        return self.fetch()\n",
        "T0021",
        "cannot infer return type of private helper `Box._g`",
    );
}

#[test]
fn an_inherited_copy_whose_inferred_return_drifts_is_refused() {
    refused(
        "class Base:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n    \
         def val(self) -> int:\n        return self.n\n\n    def _twice(self):\n        \
         return self.val()\n\n\nclass Derived(Base):\n    def val(self) -> str:\n        \
         return \"d\"\n\n\ny: int = Derived(2)._twice()\n",
        "T0022",
        "expected `int`, found `str` (inherited `Base._twice` compiled for subclass `Derived`)",
    );
}

#[test]
fn an_inherited_copy_narrowing_to_the_receiver_subclass_is_admitted() {
    let source = "class Base:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n    \
         def _mk(self):\n        return type(self)(self.n + 1)\n\n    \
         def tag(self) -> str:\n        return \"base\"\n\n\nclass Derived(Base):\n    \
         def tag(self) -> str:\n        return \"derived\"\n\n\nprint(Derived(2)._mk().tag())\n";
    assert_eq!(
        return_of(source, "Base._mk"),
        Ty::Instance(Box::new("Base".to_string()))
    );
}

#[test]
fn an_agreeing_inherited_copy_is_admitted() {
    let source = "class Base:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n    \
         def val(self) -> int:\n        return self.n\n\n    def _twice(self):\n        \
         return self.val() * 2\n\n\nclass Derived(Base):\n    def val(self) -> int:\n        \
         return self.n + 1\n\n\nprint(Derived(2)._twice())\n";
    assert_eq!(return_of(source, "Base._twice"), Ty::Int);
}

#[test]
fn an_inherited_copy_returning_bool_under_an_int_origin_is_refused() {
    refused(
        "class Base:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n    \
         def val(self) -> int:\n        return self.n\n\n    def _get(self):\n        \
         return self.val()\n\n\nclass Derived(Base):\n    def val(self) -> bool:\n        \
         return True\n\n\nprint(Derived(2)._get() + 1)\n",
        "T0022",
        "expected `int`, found `bool` (inherited `Base._get` compiled for subclass `Derived`)",
    );
}
