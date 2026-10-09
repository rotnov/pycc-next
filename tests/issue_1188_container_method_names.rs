//! #1188: a user class may define a method named `append`, `pop`, `get` or
//! `add`.
//!
//! HIR lowering claims `x.append(v)`, `x.pop()`, `d.get(k, default)` and
//! `s.add(v)` from their spelling alone, because it runs before any type
//! exists. pycc used to protect that by refusing the four names as method
//! names outright. Now, in a module that can see a class defining one of
//! them, the call keeps both readings and the receiver's static type picks
//! one (`crates/pycc_hir/src/expr/receiver_dispatch.rs` states the rule;
//! `docs/TYPE_SYSTEM.md` carries it).
//!
//! The governing promise is the renamed-twin invariant from the #1188 plan:
//! for a program P that defines one of the four names as a method, let P' be
//! P with that method -- its `def` and every call to it -- renamed to a
//! neutral name. If pycc accepts P', it must accept P and print the same
//! thing; if pycc rejects P', P may be rejected with any diagnostic. So
//! every fixture here is written once, with a `%name%` placeholder at each
//! spelling of the user's method and a literal spelling at each container
//! call, and is run both ways. Every expected stdout below was taken from
//! CPython on the gate-on spelling.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::Command;

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

/// The placeholder spellings and what each becomes in P and in its twin.
const SPELLINGS: [(&str, &str, &str); 4] = [
    ("%get%", "get", "fetch"),
    ("%pop%", "pop", "take"),
    ("%append%", "append", "put"),
    ("%add%", "add", "push"),
];

fn render(source: &str, twin: bool) -> String {
    SPELLINGS
        .iter()
        .fold(source.to_string(), |text, (placeholder, name, renamed)| {
            text.replace(placeholder, if twin { renamed } else { name })
        })
}

struct Outcome {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Outcome {
    fn both(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

fn write_project(category: &str, files: &[(&str, &str)], twin: bool) -> ScratchDir {
    let dir = ScratchDir::new(category).expect("scratch");
    for (relative, source) in files {
        let path = dir.join(relative);
        std::fs::create_dir_all(path.parent().expect("a fixture path has a parent"))
            .expect("create the fixture's directory");
        std::fs::write(&path, render(source, twin)).expect("write the fixture");
    }
    dir
}

fn pycc_in(dir: &Path, subcommand: &str) -> Outcome {
    let output = pycc()
        .arg(subcommand)
        .arg("main.py")
        .current_dir(dir)
        .output()
        .expect("pycc should spawn");
    Outcome {
        code: output.status.code().expect("pycc must exit normally"),
        stdout: String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"),
        // The linker may warn about deployment targets on some hosts; that is
        // a property of the machine, not of the program.
        stderr: String::from_utf8_lossy(&output.stderr)
            .replace("\r\n", "\n")
            .lines()
            .filter(|line| !line.starts_with("ld: warning"))
            .map(|line| format!("{line}\n"))
            .collect(),
    }
}

fn run_both(category: &str, files: &[(&str, &str)], subcommand: &str) -> (Outcome, Outcome) {
    let program = write_project(&format!("{category}_p"), files, false);
    let twin = write_project(&format!("{category}_twin"), files, true);
    (pycc_in(&program, subcommand), pycc_in(&twin, subcommand))
}

/// P and its renamed twin both run cleanly and print `expected`, CPython's
/// output for P.
fn twins_print(category: &str, files: &[(&str, &str)], expected: &str) {
    let (program, twin) = run_both(category, files, "run");
    assert_eq!(
        program.code,
        0,
        "P of `{category}` must run: {}",
        program.both()
    );
    assert_eq!(
        twin.code,
        0,
        "the twin of `{category}` must run: {}",
        twin.both()
    );
    assert_eq!(
        program.stdout, expected,
        "P of `{category}` must print CPython's output"
    );
    assert_eq!(
        twin.stdout, expected,
        "the twin of `{category}` must print the same"
    );
}

fn single(source: &str) -> [(&'static str, &str); 1] {
    [("main.py", source)]
}

/// P and its twin are both refused by `subcommand`, with byte-identical
/// output that contains `needle`: the container reading's diagnostics are
/// unchanged inside a gate-on module.
fn twins_same_diagnostic(category: &str, source: &str, subcommand: &str, needle: &str) {
    let (program, twin) = run_both(category, &single(source), subcommand);
    assert_ne!(
        program.code, 0,
        "P of `{category}` must be refused by `{subcommand}`"
    );
    assert_eq!(
        program.both(),
        twin.both(),
        "P of `{category}` must report exactly what its twin reports"
    );
    assert!(
        program.both().contains(needle),
        "expected {needle:?} from `{category}`, got: {}",
        program.both()
    );
}

/// Whenever the twin is refused, P is refused too. Neither the code nor the
/// position is asserted: the invariant leaves both free.
fn twins_rejected(category: &str, source: &str) {
    let (program, twin) = run_both(category, &single(source), "check");
    assert_ne!(
        twin.code, 0,
        "the twin of `{category}` is expected to be refused"
    );
    assert_ne!(
        program.code, 0,
        "P of `{category}` must be refused like its twin"
    );
}

// -- Criterion 1: the method reading ------------------------------------------

#[test]
fn every_name_works_as_a_method_in_every_argument_shape() {
    // Each call has a shape the container reading would refuse: `get()` with
    // no arguments, `pop(1)`, `append()`, `add(a, b)`, a call on a call's
    // result, and calls through `self`, a parameter and a function local.
    twins_print(
        "1188_shapes",
        &single(
            "\
class Cache:
    def __init__(self, base: int) -> None:
        self.base = base

    def %get%(self) -> int:
        return self.base

    def %pop%(self, n: int) -> int:
        return self.base - n

    def %append%(self) -> int:
        return self.base * 2

    def %add%(self, a: int, b: int) -> int:
        return a + b + self.%get%()

    def total(self) -> int:
        return self.%get%() + self.%pop%(1) + self.%append%()


def make() -> Cache:
    return Cache(7)


def use(c: Cache) -> int:
    return c.%get%() + c.%add%(1, 2)


c = Cache(10)
print(c.%get%())
print(c.%pop%(1))
print(c.%append%())
print(c.%add%(1, 2))
print(c.total())
print(use(c))
print(make().%get%())


def inside() -> int:
    k = Cache(3)
    return k.%get%() + k.%pop%(2)


print(inside())
",
        ),
        "10\n9\n20\n13\n39\n23\n7\n4\n",
    );
}

#[test]
fn static_and_class_methods_and_a_shadowed_class_name() {
    // `h`'s parameter `S` shadows the class, so `S.get("a", 0)` is a dict
    // call there even though the class `S` has a static method `get`.
    twins_print(
        "1188_static",
        &single(
            "\
class S:
    @staticmethod
    def %get%() -> int:
        return 41

    @classmethod
    def %pop%(cls) -> int:
        return 42


print(S.%get%())
print(S.%pop%())


def h(S: dict[str, int]) -> int:
    return S.get(\"a\", 0)


print(h({\"a\": 5}))


def g() -> int:
    return S.%get%() + S.%pop%()


print(g())
",
        ),
        "41\n42\n5\n83\n",
    );
}

#[test]
fn a_protocol_typed_receiver_calls_the_method() {
    twins_print(
        "1188_protocol",
        &single(
            "\
from typing import Protocol


class P(Protocol):
    def %get%(self) -> int: ...


class C:
    def __init__(self, v: int) -> None:
        self.v = v

    def %get%(self) -> int:
        return self.v


def use(p: P) -> int:
    return p.%get%() + 1


print(use(C(4)))
",
        ),
        "5\n",
    );
}

#[test]
fn a_generic_class_instance_calls_the_method() {
    twins_print(
        "1188_generic",
        &single(
            "\
class Box[T]:
    def __init__(self, v: T) -> None:
        self.v = v

    def %get%(self) -> T:
        return self.v


b = Box[int](4)
print(b.%get%())
",
        ),
        "4\n",
    );
}

#[test]
fn a_protocol_specialized_method_and_a_container_call_share_a_module() {
    // `c.add(V())` is rewritten to its protocol specialization, which
    // replaces the whole receiver-dispatched node; `xs.append(...)` inside
    // the protocol function is left as a container call.
    twins_print(
        "1188_protocol_rewrite",
        &single(
            "\
from typing import Protocol


class P(Protocol):
    def val(self) -> int: ...


class V:
    def val(self) -> int:
        return 7


class C:
    def %add%(self, p: P) -> int:
        return p.val() + 1

    def %get%(self) -> int:
        return 2


def use(p: P) -> int:
    xs = [1]
    xs.append(p.val())
    return xs.pop()


c = C()
print(c.%add%(V()))
print(use(V()))
print(c.%get%())
",
        ),
        "8\n7\n2\n",
    );
}

#[test]
fn container_and_method_calls_around_a_generic_function() {
    twins_print(
        "1188_generic_fn",
        &single(
            "\
class C:
    def %get%(self) -> int:
        return 9


def ident[T](v: T) -> T:
    return v


def g() -> int:
    xs = [1]
    xs.append(ident(2))
    d = {\"a\": 3}
    return xs.pop() + d.get(\"a\", ident(0)) + C().%get%()


xs = [4]
xs.append(ident(5))
print(xs.pop(), C().%get%(), g())
",
        ),
        "5 9 14\n",
    );
}

/// A receiver-dispatched call inside a comprehension element (renamed with
/// the comprehension variable), inside a generic function's body (scanned
/// for recursive generic calls), and on an instance or a class-name receiver
/// in a module that monomorphization rewrites, keeps its reading in each
/// place.
#[test]
fn comprehensions_and_generic_bodies_hold_either_reading() {
    twins_print(
        "1188_compr_generic",
        &single(
            "\
class C:
    def %get%(self, n: int) -> int:
        return n + 1


class S:
    @staticmethod
    def %pop%() -> int:
        return 8


def twice[T](v: T) -> T:
    c = C()
    d = {\"a\": 1}
    print(c.%get%(1), d.get(\"a\", 0))
    return v


def g() -> int:
    c = C()
    return c.%get%(twice(3))


c = C()
d = {\"a\": 5}
ys = [c.%get%(x) for x in range(1, 3)]
zs = [d.get(\"b\", x) for x in range(7, 8)]
print(ys[0], ys[1], zs[0])
print(twice(4), g())
print(c.%get%(twice(5)), S.%pop%())
",
        ),
        "2 3 7\n2 1\n2 1\n4 4\n2 1\n6 8\n",
    );
}

#[test]
fn a_walrus_inside_either_reading_binds_its_target() {
    twins_print(
        "1188_walrus",
        &single(
            "\
class C:
    def %get%(self, n: int) -> int:
        return n * 2


c = C()
print(c.%get%(n := 5), n)
d = {\"a\": 1}
print(d.get(\"a\", z := 4), z)


def f() -> int:
    print(c.%get%(m := 6), m)
    e = {\"b\": 2}
    print(e.get(\"c\", j := 8), j)
    return m


print(f())
",
        ),
        "10 5\n1 4\n12 6\n8 8\n6\n",
    );
}

// -- Criterion 1: across modules ----------------------------------------------

const CACHE_MODULE: &str = "\
class Cache:
    def __init__(self, base: int) -> None:
        self.base = base

    def %get%(self, a: int, b: int) -> int:
        return self.base + a + b
";

#[test]
fn a_method_defined_two_imports_away() {
    twins_print(
        "1188_chain",
        &[
            ("dep.py", CACHE_MODULE),
            (
                "mid.py",
                "from dep import Cache\n\n\ndef probe(c: Cache) -> int:\n    return c.%get%(1, 2)\n",
            ),
            (
                "main.py",
                "from mid import probe\nfrom dep import Cache\n\nd = {\"a\": 1}\n\
                 print(probe(Cache(10)), d.get(\"a\", 0), d.get(\"z\", 9))\n",
            ),
        ],
        "13 1 9\n",
    );
}

#[test]
fn an_importer_that_never_names_the_class() {
    twins_print(
        "1188_indirect",
        &[
            ("dep.py", CACHE_MODULE),
            (
                "mid.py",
                "from dep import Cache\n\n\ndef make() -> Cache:\n    return Cache(10)\n",
            ),
            (
                "main.py",
                "\
from mid import make

c = make()
print(c.%get%(1, 2))
print(make().%get%(3, 4))


def local() -> int:
    k = make()
    return k.%get%(5, 6)


print(local())
",
            ),
        ],
        "13\n17\n21\n",
    );
}

#[test]
fn an_imported_instance_global() {
    twins_print(
        "1188_instance_global",
        &[
            ("dep.py", CACHE_MODULE),
            ("mid2.py", "from dep import Cache\n\ninst = Cache(2)\n"),
            (
                "main.py",
                "\
from mid2 import inst

print(inst.%get%(1, 2))


def f() -> int:
    return inst.%get%(3, 4)


print(f())
",
            ),
        ],
        "5\n9\n",
    );
}

#[test]
fn a_protocol_in_one_module_and_its_implementation_in_another() {
    twins_print(
        "1188_cross_protocol",
        &[
            (
                "dep.py",
                "\
from typing import Protocol


class P(Protocol):
    def %get%(self) -> int: ...


def use(p: P) -> int:
    return p.%get%()
",
            ),
            (
                "main.py",
                "\
from dep import use


class C:
    def %get%(self) -> int:
        return 5


print(use(C()))
",
            ),
        ],
        "5\n",
    );
}

#[test]
fn a_class_in_a_package_init() {
    twins_print(
        "1188_package",
        &[
            ("pkg/__init__.py", CACHE_MODULE),
            (
                "pkg/sub.py",
                "from pkg import Cache\n\n\ndef make() -> Cache:\n    return Cache(1)\n",
            ),
            (
                "main.py",
                "from pkg.sub import make\n\nprint(make().%get%(2, 3))\n",
            ),
        ],
        "6\n",
    );
}

/// The gate is a module's import closure, not every module loaded so far:
/// `b` never imports `dep`, so its container call keeps today's HIR
/// diagnostic and position whichever module `main` imports first.
#[test]
fn a_sibling_module_keeps_its_gate_off() {
    let dep = "class Cache:\n    def %pop%(self) -> int:\n        return 1\n";
    let b = "def _p(xs):\n    return xs.pop(0)\n\n\ndef q() -> int:\n    return 1\n";
    for (order, main) in [
        ("dep_first", "from dep import Cache\nfrom b import q\n"),
        ("b_first", "from b import q\nfrom dep import Cache\n"),
    ] {
        let dir = write_project(
            &format!("1188_sibling_{order}"),
            &[("dep.py", dep), ("b.py", b), ("main.py", main)],
            false,
        );
        let outcome = pycc_in(&dir, "check");
        assert_ne!(outcome.code, 0);
        assert!(
            outcome
                .both()
                .contains("error[C0001]: list.pop() takes no arguments, got 1\n --> b.py:2:12"),
            "{order}: {}",
            outcome.both()
        );
    }
}

// -- Criterion 2: the container reading ---------------------------------------

/// A class defining all four names, prepended to the container fixtures so
/// their module's gate is on for every name.
const ALL_FOUR: &str = "\
class K:
    def %get%(self) -> int:
        return 1

    def %pop%(self) -> int:
        return 2

    def %append%(self, v: int) -> None:
        pass

    def %add%(self, v: int) -> None:
        pass


";

fn with_all_four(body: &str) -> String {
    format!("{ALL_FOUR}{body}")
}

#[test]
fn container_calls_keep_working_beside_the_methods() {
    twins_print(
        "1188_containers",
        &single(&with_all_four(
            "\
xs = [1, 2]
xs.append(3)
print(xs.pop())
d = {\"a\": 1}
print(d.get(\"a\", 0), d.get(\"b\", 7))
s = {1}
s.add(2)
print(len(s))


def f() -> int:
    ys = []
    ys.append(5)
    zs: list[int] = [4]
    zs.append(6)
    return ys.pop() + zs.pop()


print(f())
k = K()
k.%append%(1)
k.%add%(1)
print(k.%get%() + k.%pop%())
",
        )),
        "3\n1 7\n2\n11\n3\n",
    );
}

#[test]
fn container_diagnostics_are_unchanged_through_check_and_run() {
    let cases = [
        (
            "append_arity",
            "xs = [1]\nxs.append()\n",
            "error[C0001]: list.append() takes exactly one argument, got 0\n --> main.py:16:1",
        ),
        (
            "get_arity",
            "d = {'a': 1}\nprint(d.get('a'))\n",
            "error[C0001]: `.get()` is only supported as `dict.get(key, default)` with exactly \
             two arguments so far, got 1\n --> main.py:16:7",
        ),
        (
            "non_name",
            "def f() -> list[int]:\n    return [1]\n\n\nf().append(1)\n",
            "error[C0001]: `.append()` is only supported on a name or an instance attribute \
             so far\n --> main.py:19:1",
        ),
        (
            "not_a_list",
            "x: int = 1\nx.append(1)\n",
            "error[T0033]: `int` does not support `.append()`",
        ),
        (
            "boundary_literal",
            "xs = [1]\nxs.append(99999999999999999999999)\n",
            "error[C0001]: integer literal does not fit in i64: 99999999999999999999999\n \
             --> main.py:16:11",
        ),
    ];
    for (name, body, needle) in cases {
        for subcommand in ["check", "run"] {
            twins_same_diagnostic(
                &format!("1188_diag_{name}_{subcommand}"),
                &with_all_four(body),
                subcommand,
                needle,
            );
        }
    }
}

#[test]
fn an_empty_list_still_infers_from_its_append() {
    twins_print(
        "1188_empty",
        &single(&with_all_four(
            "def f() -> int:\n    ys = []\n    ys.append(5)\n    return ys.pop()\n\n\nprint(f())\n",
        )),
        "5\n",
    );
}

#[test]
fn an_empty_list_whose_append_fails_to_infer_reports_as_before() {
    twins_same_diagnostic(
        "1188_empty_err",
        &with_all_four(
            "def f() -> int:\n    ys = []\n    ys.append(nope)\n    return 0\n\n\nprint(f())\n",
        ),
        "check",
        "T0003",
    );
}

/// Before #1188 this program was accepted, because its gate came only from
/// a protocol member, which the old refusal did not cover. It must keep its
/// output now that the gate is on.
#[test]
fn a_protocol_only_gate_keeps_the_dict_call() {
    twins_print(
        "1188_protocol_only",
        &single(
            "\
from typing import Protocol


class P(Protocol):
    def %get%(self) -> int: ...


d = {\"a\": 1}
print(d.get(\"a\", 0))
",
        ),
        "1\n",
    );
}

// -- Criterion 3, as the issue's own example actually reads -------------------

#[test]
fn a_method_named_like_a_container_method_uses_the_container_method() {
    // Inside `Bag.append`, `xs.append(v)` is the list's; `self.append(...)`
    // is the method's. `XS.append(v)` reaches a module-global list from
    // inside a method.
    twins_print(
        "1188_criterion3",
        &single(
            "\
XS: list[int] = [1]


class Bag:
    def %append%(self, xs: list[int], v: int) -> None:
        xs.append(v)

    def %get%(self, d: dict[str, int], k: str) -> int:
        return d.get(k, 0)

    def %add%(self, v: int) -> int:
        XS.append(v)
        return len(XS)

    def run(self) -> int:
        ys = [1]
        self.%append%(ys, 2)
        return len(ys) + self.%get%({\"a\": 3}, \"a\") + self.%get%({\"a\": 3}, \"b\")


b = Bag()
print(b.run())
print(b.%add%(5))
print(XS.pop())
",
        ),
        "5\n2\n5\n",
    );
}

// -- The review rounds' differential pairs -------------------------------------

/// A class with a static `get` and an instance `pop`, plus an annotated
/// consumer. The fixtures below put receivers of it into unannotated private
/// helpers, where the constraint solver has no class table to consult.
const HELPER_CLASS: &str = "\
class C:
    @staticmethod
    def %get%() -> int:
        return 3

    def %pop%(self, n: int) -> int:
        return n + 10


def mk() -> C:
    return C()


def use(x: C) -> int:
    return 1

";

fn with_helper_class(body: &str) -> String {
    format!("{HELPER_CLASS}{body}")
}

#[test]
fn receivers_inside_unannotated_private_helpers() {
    let cases = [
        (
            "class_name",
            "\ndef _h():\n    print(C.%get%())\n    return 0\n\n\n_h()\n",
            "3\n",
        ),
        (
            "opaque_local",
            "\ndef _h():\n    c = C()\n    print(c.%get%(), c.%pop%(1))\n    return 0\n\n\n_h()\n",
            "3 11\n",
        ),
        (
            "non_name",
            "\ndef _h():\n    print(mk().%get%(), mk().%pop%(2))\n    return 0\n\n\n_h()\n",
            "3 12\n",
        ),
    ];
    for (name, body, expected) in cases {
        twins_print(
            &format!("1188_helper_{name}"),
            &single(&with_helper_class(body)),
            expected,
        );
    }
}

#[test]
fn parameters_resolved_at_their_call_sites() {
    for (call, printed) in [("%get%()", "3"), ("%pop%(1)", "11")] {
        let cases = [
            (
                "after_use",
                format!(
                    "\ndef _h(c):\n    n = use(c)\n    print(c.{call})\n    return n\n\n\nprint(_h(C()))\n"
                ),
                format!("{printed}\n1\n"),
            ),
            (
                "before_use",
                format!(
                    "\ndef _g(c):\n    print(c.{call})\n    return use(c)\n\n\nprint(_g(C()))\n"
                ),
                format!("{printed}\n1\n"),
            ),
            (
                "across_functions",
                format!(
                    "\ndef _g(c):\n    print(c.{call})\n    return 0\n\n\n\
                     def _k(x: C):\n    return _g(x)\n\n\nprint(_k(C()))\n"
                ),
                format!("{printed}\n0\n"),
            ),
            (
                "alias",
                format!(
                    "\ndef _g(c):\n    d = c\n    print(d.{call})\n    return 0\n\n\n\
                     def _k(x: C):\n    return _g(x)\n\n\nprint(_k(C()))\n"
                ),
                format!("{printed}\n0\n"),
            ),
        ];
        for (name, body, expected) in cases {
            twins_print(
                &format!("1188_callsite_{name}"),
                &single(&with_helper_class(&body)),
                &expected,
            );
        }
    }
}

#[test]
fn annotated_class_bindings_beside_an_unannotated_helper() {
    for (call, printed) in [("%get%()", "3"), ("%pop%(1)", "11")] {
        let body = format!(
            "\ndef _h(n):\n    return n + 1\n\n\nc: C = C()\nprint(c.{call})\n\n\n\
             def f() -> int:\n    d: C = C()\n    print(d.{call})\n    print(c.{call})\n    \
             return _h(1)\n\n\nprint(f())\n"
        );
        twins_print(
            "1188_annotated",
            &single(&with_helper_class(&body)),
            &format!("{printed}\n{printed}\n{printed}\n2\n"),
        );
    }
}

#[test]
fn a_rejected_twin_stays_rejected() {
    twins_rejected(
        "1188_rejected_param",
        &with_all_four("def _h(c):\n    return c.%get%()\n"),
    );
    twins_rejected(
        "1188_rejected_pop_param",
        &with_all_four("def _p(xs):\n    return xs.pop(0)\n"),
    );
    twins_rejected(
        "1188_rejected_pop_local",
        &with_all_four("def _q():\n    ys = [1]\n    return ys.pop(0)\n"),
    );
}

/// The one case where the constraint solver cannot tell the readings apart:
/// a parameter no call site annotates. It must be a rejection, never code.
#[test]
fn an_unresolvable_receiver_is_refused_through_check_and_run() {
    for subcommand in ["check", "run"] {
        twins_same_diagnostic(
            &format!("1188_soundness_{subcommand}"),
            &with_all_four("def _h(c):\n    return c.%get%()\n"),
            subcommand,
            "error[T0021]: cannot infer type of parameter `c` in private helper `_h`; add an \
             annotation",
        );
    }
}

/// With no reachable method of the name, the gate is off and the container
/// diagnostic is exactly today's, at exactly today's position.
#[test]
fn a_gate_off_module_keeps_the_container_diagnostic() {
    let dir = write_project(
        "1188_gate_off",
        &single("def _p(xs):\n    return xs.pop(0)\n"),
        false,
    );
    let outcome = pycc_in(&dir, "check");
    assert_ne!(outcome.code, 0);
    assert!(
        outcome
            .both()
            .contains("error[C0001]: list.pop() takes no arguments, got 1\n --> main.py:2:12"),
        "{}",
        outcome.both()
    );
}

/// `b.get()` on a class that has no `get`, in a module where another class
/// does: the call is a method call on a class instance, so the diagnostic is
/// the ordinary unknown-method one.
#[test]
fn an_undefined_method_reports_the_unknown_method_diagnostic() {
    let dir = write_project(
        "1188_unknown_method",
        &single(&with_all_four(
            "class B:\n    pass\n\n\nb = B()\nprint(b.%get%())\n",
        )),
        false,
    );
    let outcome = pycc_in(&dir, "check");
    assert_ne!(outcome.code, 0);
    assert!(
        outcome
            .both()
            .contains("error[T0044]: class `B` has no method named `get`"),
        "{}",
        outcome.both()
    );
}

// -- Types and MIR agree on the reading ---------------------------------------

#[test]
fn one_name_is_an_instance_in_one_scope_and_a_list_in_another() {
    twins_print(
        "1188_shadow_scopes",
        &single(
            "\
class Box:
    def __init__(self) -> None:
        self.n = 0

    def %append%(self, v: int) -> int:
        self.n = self.n + v
        return self.n


x = Box()
print(x.%append%(3))


def f() -> int:
    x: list[int] = [1]
    x.append(2)
    return len(x)


print(f())
print(x.%append%(4))
",
        ),
        "3\n2\n7\n",
    );
}

/// A foreign CPython object takes the method reading (#1095): the foreign
/// import turns the receiver dispatch on by itself, so `gc.get(1, 2)` is
/// the foreign method call whether or not a user class defines `get`.
/// `tests/issue_1095_object_container_methods.rs` runs these names against
/// CPython.
#[test]
fn a_foreign_object_receiver_takes_the_method_reading() {
    let foreign = "import gc\nprint(gc.get(1, 2))\n";
    let gate_off = write_project("1188_foreign_off", &single(foreign), false);
    let gate_on = write_project(
        "1188_foreign_on",
        &single(&format!(
            "class K:\n    def %get%(self) -> int:\n        return 1\n\n\n{foreign}"
        )),
        false,
    );
    let off = pycc_in(&gate_off, "check");
    let on = pycc_in(&gate_on, "check");
    assert_eq!(off.code, 0, "{}", off.both());
    assert_eq!(on.code, 0, "{}", on.both());
}

// -- `--ext` -------------------------------------------------------------------

/// A class defining all four names, published through `--ext`.
const STORE: &str = "\
class Store:
    def __init__(self, n: int) -> None:
        self.n = n

    def get(self, k: int) -> int:
        return self.n + k

    def add(self, k: int) -> int:
        return self.get(k) + 1

    def append(self, k: int) -> int:
        return self.n * k

    def pop(self) -> int:
        return self.n - 1
";

/// `pycc build --ext` of a class defining all four names reports none of the
/// diagnostics the old refusal or a mis-dispatched call would produce.
///
/// `#[ignore]`d for the reason `tests/issue_1145_ext_instance_methods.rs`
/// gives: an `--ext` build needs a CPython 3.13+ with development headers,
/// which is a property of the machine. CI runs it on every Tier-1
/// `native-build-test` leg through `cargo test --workspace --
/// --include-ignored`. It asserts only the absence of those diagnostics, not
/// that the build succeeds, so an interpreter too old to build against
/// (`PYCC_PYTHON`) cannot fail it for a reason #1188 does not own.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_ext_build_of_the_four_methods_reports_no_container_diagnostic() {
    let dir = ScratchDir::new("ext_1188").expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(&src, STORE).expect("write the fixture source");
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("store"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    let reported = format!(
        "{}{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    // The last needle is the removed class-body refusal's wording, split so
    // the stale-reference sweep for that refusal does not match this test.
    let removed_refusal = concat!("collides with the ", "compiler");
    for forbidden in ["C0001", "T0033", removed_refusal] {
        assert!(
            !reported.contains(forbidden),
            "`--ext` must not report {forbidden:?}: {reported}"
        );
    }
}

/// The four methods are callable from CPython on an `--ext` artifact, and
/// each reaches the user's compiled body rather than a container fast path.
///
/// Mirrors `tests/issue_1145_ext_instance_methods.rs`'s
/// `an_instance_is_constructed_and_its_method_reaches_the_compiled_body`:
/// `#[ignore]`d for the same reason, run by every leg's
/// `cargo test --workspace -- --include-ignored` with a CPython 3.13+ as
/// `python3` (or `PYCC_PYTHON`), and asserting the build and the import
/// exactly as that file does.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_four_methods_are_callable_through_an_ext_build() {
    let dir = ScratchDir::new("ext_1188_call").expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(&src, STORE).expect("write the fixture source");
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("store"))
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
        .arg(
            "\
import store
s = store.Store(5)
assert type(s) is store.Store, type(s)
assert s.get(1) == 6, s.get(1)
assert s.add(1) == 7, s.add(1)
assert s.append(2) == 10, s.append(2)
assert s.pop() == 4, s.pop()
",
        )
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
