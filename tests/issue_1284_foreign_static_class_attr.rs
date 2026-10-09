//! Part 1 of #1284 (#1345): a class attribute bound to
//! `staticmethod(<foreign callable>)` -- `exists = staticmethod(os.path.exists)`
//! -- is read and called through the class name or an instance, and every use
//! is rewritten to the recorded foreign reference, so the result is CPython's
//! own. Part 2 (#1346) admits any instance receiver, not only a plain name:
//! `make().exists(p)` evaluates `make()` first, then the arguments, then the
//! call.
//!
//! The `check` tests pin every refusal the admission and the use-site rules
//! report. The hosted tests compare an `--ext` build's import against the
//! host interpreter's own run of the same source; they are `#[ignore]`d and
//! run under `cargo test --workspace -- --include-ignored` with a CPython
//! 3.13+ that has development headers. The changed lines are covered by the
//! non-ignored tests here and by the unit tests in
//! `crates/pycc_hir/src/class/foreign_static_tests.rs`,
//! `crates/pycc_hir/src/class/attr_initializer.rs` and
//! `crates/pycc_mir/src/tests/foreign_static.rs`.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn host_python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

/// Writes `body` to `dir/<file>` and returns the path.
fn write(dir: &Path, file: &str, body: &str) -> PathBuf {
    let path = dir.join(file);
    std::fs::write(&path, body).expect("write the fixture source");
    path
}

fn check_with(dir: &Path, body: &str) -> Output {
    pycc()
        .arg("check")
        .arg(write(dir, "m.py", body))
        .output()
        .expect("pycc should spawn")
}

fn assert_checks(tag: &str, body: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    let output = check_with(&dir, body);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{body}{}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
}

/// `pycc check` of `body` fails with exactly one `code` diagnostic whose
/// text contains `needle`.
fn assert_one_error(tag: &str, body: &str, code: &str, needle: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    let output = check_with(&dir, body);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert_eq!(rendered.matches("error[").count(), 1, "{rendered}");
    assert!(rendered.contains(&format!("error[{code}]")), "{rendered}");
    assert!(rendered.contains(needle), "{rendered}");
}

const FS: &str = "import os\n\n\nclass FS:\n    exists = staticmethod(os.path.exists)\n\n\n";

fn with_fs(rest: &str) -> String {
    format!("{FS}{rest}")
}

/// The winning cases of the MRO rule: the class-name and instance calls on
/// the defining class, an inherited attribute (`D1`), `D3(FS, A)` whose
/// foreign attribute outranks `A`'s `@staticmethod`, `D7(FS, B)` whose
/// foreign attribute outranks `B`'s `@property`, a method-body
/// `self.exists(p)`, and a module-body use. Every subclass of `FS` resolves
/// `exists` to the same foreign attribute, so no instance call is refused.
const MRO_WINS: &str = "import os\n\n\n\
    class FS:\n    exists = staticmethod(os.path.exists)\n\n\n\
    class A:\n    @staticmethod\n    def exists(p: str) -> str:\n        return \"A.static\"\n\n\n\
    class B:\n    @property\n    def exists(self) -> int:\n        return 7\n\n\n\
    class D1(FS):\n    pass\n\n\n\
    class D3(FS, A):\n    pass\n\n\n\
    class D7(FS, B):\n    pass\n\n\n\
    class U(FS):\n    def m(self, p: str) -> None:\n        print(self.exists(p), FS.exists(p))\n\n\n\
    def probe(p: str) -> None:\n\
    \x20   fs = FS()\n\
    \x20   print(FS.exists(p), fs.exists(p))\n\
    \x20   d1 = D1()\n\
    \x20   print(D1.exists(p), d1.exists(p))\n\
    \x20   d3 = D3()\n\
    \x20   print(D3.exists(p), d3.exists(p))\n\
    \x20   d7 = D7()\n\
    \x20   print(D7.exists(p), d7.exists(p))\n\
    \x20   print(d7.exists.__name__)\n\
    \x20   u = U()\n\
    \x20   u.m(p)\n\n\n\
    probe(\"/\")\n\
    probe(\"/definitely/not/a/path/here\")\n\
    print(FS.exists(\"/\"))\n\
    print(FS.exists.__name__)\n";

/// The cases where the existing dispatch keeps the name: `D2`'s own method,
/// `D4(A, FS)` where `A`'s `@staticmethod` comes first, and `D5(S, FS)`
/// where `S`'s instance slot wins for an instance while the class name
/// still reaches the foreign attribute.
const MRO_OTHERS: &str = "import os\n\n\n\
    class FS:\n    exists = staticmethod(os.path.exists)\n\n\n\
    class A:\n    @staticmethod\n    def exists(p: str) -> str:\n        return \"A.static\"\n\n\n\
    class D2(FS):\n    def exists(self, p: str) -> str:\n        return \"D2method\"\n\n\n\
    class D4(A, FS):\n    pass\n\n\n\
    class S:\n    def __init__(self) -> None:\n        self.exists = 1\n\n\n\
    class D5(S, FS):\n    pass\n\n\n\
    def probe(p: str) -> None:\n\
    \x20   d2 = D2()\n\
    \x20   print(d2.exists(p))\n\
    \x20   d4 = D4()\n\
    \x20   print(D4.exists(p), d4.exists(p))\n\
    \x20   d5 = D5()\n\
    \x20   print(d5.exists, D5.exists(p))\n\n\n\
    probe(\"/\")\n";

/// The bare-name root, the self-named `mul = staticmethod(mul)` (its value
/// is evaluated before the target is bound), and a dotted root whose last
/// segment is `add` -- built as a method call directly, so #1188's
/// container dispatch never sees it.
const OPERATOR: &str = "import operator\nfrom operator import add, mul\n\n\n\
    class C:\n\
    \x20   plus = staticmethod(add)\n\
    \x20   mul = staticmethod(mul)\n\
    \x20   dotted = staticmethod(operator.add)\n\n\n\
    def probe(a: int, b: int) -> None:\n\
    \x20   c = C()\n\
    \x20   print(C.plus(a, b), c.plus(a, b), C.mul(a, b), c.mul(a, b))\n\
    \x20   print(C.dotted(a, b), c.dotted(a, b))\n\n\n\
    probe(2, 3)\n\
    print(C.dotted(\"a\", \"b\"), C.plus(1.5, 2.5))\n";

#[test]
fn check_accepts_both_root_shapes_and_every_supported_receiver() {
    assert_checks("fs_check_mro_wins", MRO_WINS);
    assert_checks("fs_check_mro_others", MRO_OTHERS);
    assert_checks("fs_check_operator", OPERATOR);
}

// -- admission refusals (C0001), one per shape ------------------------------

#[test]
fn a_staticmethod_call_of_the_wrong_arity_is_refused() {
    for (tag, value) in [
        ("fs_arity_zero", "staticmethod()"),
        ("fs_arity_two", "staticmethod(add, add)"),
        ("fs_arity_keyword", "staticmethod(f=add)"),
    ] {
        assert_one_error(
            tag,
            &format!("from operator import add\n\n\nclass C:\n    x = {value}\n"),
            "C0001",
            "which takes exactly one positional argument",
        );
    }
    assert_one_error(
        "fs_arity_starred",
        "from operator import add\nxs = [add]\n\n\nclass C:\n    x = staticmethod(*xs)\n",
        "C0001",
        "which takes exactly one positional argument",
    );
}

#[test]
fn a_root_that_is_not_a_foreign_import_is_refused() {
    let needle = "is not a foreign (CPython) import";
    for (tag, body) in [
        (
            "fs_root_function",
            "def f(a: int) -> int:\n    return a\n\n\nclass C:\n    x = staticmethod(f)\n",
        ),
        (
            "fs_root_std_module",
            "import math\n\n\nclass C:\n    x = staticmethod(math.floor)\n",
        ),
        (
            "fs_root_undefined",
            "class C:\n    x = staticmethod(nope)\n",
        ),
        (
            "fs_root_class",
            "class A:\n    pass\n\n\nclass C:\n    x = staticmethod(A)\n",
        ),
        (
            "fs_root_import_after",
            "class C:\n    x = staticmethod(add)\n\n\nfrom operator import add\n",
        ),
    ] {
        assert_one_error(tag, body, "C0001", needle);
    }
}

#[test]
fn a_conditional_import_root_is_refused() {
    assert_one_error(
        "fs_root_conditional",
        "try:\n    import os\nexcept ImportError:\n    pass\n\n\n\
         class C:\n    x = staticmethod(os.path.exists)\n",
        "C0001",
        "the import of `os` is conditional",
    );
}

#[test]
fn a_non_reference_argument_is_refused() {
    for (tag, value) in [
        ("fs_arg_call", "staticmethod(os.path.join(\"a\"))"),
        ("fs_arg_lambda", "staticmethod(lambda a: a)"),
    ] {
        assert_one_error(
            tag,
            &format!("import os\n\n\nclass C:\n    x = {value}\n"),
            "C0001",
            "whose argument is not a name or dotted attribute",
        );
    }
}

#[test]
fn a_rebound_staticmethod_is_refused() {
    let needle = "calls `staticmethod`, which is rebound";
    for (tag, prefix, body) in [
        ("fs_rebound_assign", "staticmethod = 1\n", ""),
        (
            "fs_rebound_def",
            "def staticmethod(x: int) -> int:\n    return x\n",
            "",
        ),
        ("fs_rebound_class_body", "", "    staticmethod = 1\n"),
    ] {
        assert_one_error(
            tag,
            &format!(
                "from operator import add\n{prefix}\n\nclass C:\n{body}    x = staticmethod(add)\n"
            ),
            "C0001",
            needle,
        );
    }
}

#[test]
fn a_root_bound_earlier_in_the_class_body_is_refused() {
    let needle = "refers to this class's own earlier binding";
    assert_one_error(
        "fs_earlier_assign",
        "import os\n\n\nclass C:\n    os = 1\n    x = staticmethod(os.path.exists)\n",
        "C0001",
        needle,
    );
    assert_one_error(
        "fs_earlier_def",
        "from operator import mul\n\n\nclass C:\n    def mul(self) -> int:\n        return 1\n\n\
         \x20   x = staticmethod(mul)\n",
        "C0001",
        needle,
    );
}

#[test]
fn a_name_pycc_cannot_model_is_refused() {
    for (tag, name, needle) in [
        ("fs_name_private", "__x", "is a class-private name"),
        ("fs_name_dunder", "__bool__", "is a special (dunder) name"),
        ("fs_name_get", "get", "is dispatched as a container method"),
        ("fs_name_add", "add", "is dispatched as a container method"),
    ] {
        assert_one_error(
            tag,
            &format!("from operator import add\n\n\nclass C:\n    {name} = staticmethod(add)\n"),
            "C0001",
            needle,
        );
    }
}

#[test]
fn every_other_non_literal_initializer_gets_its_own_refusal() {
    for (tag, value, needle) in [
        (
            "fs_other_classmethod",
            "classmethod(add)",
            "uses `classmethod(...)`, which is not supported yet (#1347)",
        ),
        (
            "fs_other_call",
            "os.getcwd()",
            "is initialized with a call, which is not supported yet (#1348)",
        ),
        (
            "fs_other_reference",
            "os.path.exists",
            "write `x = staticmethod(os.path.exists)`",
        ),
        (
            "fs_other_container",
            "[1, 2]",
            "is initialized with a container, which is not supported yet (#1348)",
        ),
        (
            "fs_other_comprehension",
            "[a for a in range(3)]",
            "is initialized with a container",
        ),
        (
            "fs_other_fallback",
            "-add",
            "must be initialized with a literal",
        ),
    ] {
        assert_one_error(
            tag,
            &format!("import os\nfrom operator import add\n\n\nclass C:\n    x = {value}\n"),
            "C0001",
            needle,
        );
    }
}

/// The annotated and `Final` spellings go through the same classifier and
/// get their own refusal; a non-scalar annotation still reports the
/// scalar-slot `C0001` first.
#[test]
fn the_annotated_spelling_is_refused_after_the_annotation_check() {
    for (tag, prefix, annotation) in [
        ("fs_annotated_int", "", "int"),
        (
            "fs_annotated_final",
            "from typing import Final\n",
            "Final[int]",
        ),
        (
            "fs_annotated_bare_final",
            "from typing import Final\n",
            "Final",
        ),
    ] {
        assert_one_error(
            tag,
            &format!(
                "{prefix}from operator import add\n\n\nclass C:\n    x: {annotation} = staticmethod(add)\n"
            ),
            "C0001",
            "is annotated -- only the un-annotated spelling",
        );
    }
    assert_one_error(
        "fs_annotated_list",
        "from operator import add\n\n\nclass C:\n    x: list[int] = staticmethod(add)\n",
        "C0001",
        "which is not a scalar slot type",
    );
}

/// `@dataclass` and `Enum` bodies never reach the classifier; their own
/// diagnostics are unchanged.
#[test]
fn dataclass_and_enum_bodies_keep_their_own_diagnostics() {
    assert_one_error(
        "fs_dataclass_body",
        "import os\nfrom dataclasses import dataclass\n\n\n@dataclass\n\
         class C:\n    exists = staticmethod(os.path.exists)\n",
        "C0001",
        "a `@dataclass` body statement must be a field declaration",
    );
    assert_one_error(
        "fs_enum_body",
        "import os\nfrom enum import Enum\n\n\nclass C(Enum):\n    exists = staticmethod(os.path.exists)\n",
        "C0001",
        "enum member `exists` must be assigned an integer or string literal",
    );
}

/// A collision with a method, own or inherited, keeps its `C0001` with the
/// foreign attribute's own reason.
#[test]
fn a_collision_names_the_foreign_attribute_reason() {
    let needle = "a `staticmethod(...)` class attribute is rewritten to its foreign callable";
    assert_one_error(
        "fs_collide_own",
        "import os\n\n\nclass C:\n    exists = staticmethod(os.path.exists)\n\n\
         \x20   def exists(self) -> int:\n        return 1\n",
        "C0001",
        needle,
    );
    assert_one_error(
        "fs_collide_inherited",
        "import os\n\n\nclass B:\n    def exists(self) -> int:\n        return 1\n\n\n\
         class C(B):\n    exists = staticmethod(os.path.exists)\n",
        "C0001",
        needle,
    );
}

// -- use-site refusals (T0044 / T0046) ---------------------------------------

#[test]
fn a_local_shadowing_the_root_is_refused() {
    let needle = "refers to `os`, a foreign import of the module that defines `FS`, which is \
                  shadowed by a local binding here";
    assert_one_error(
        "fs_shadow_local",
        &with_fs("def f(p: str) -> None:\n    os = 1\n    print(FS.exists(p))\n"),
        "T0044",
        needle,
    );
    assert_one_error(
        "fs_shadow_param",
        &with_fs("def f(os: int, p: str) -> None:\n    print(FS.exists(p))\n"),
        "T0044",
        needle,
    );
}

/// Part 2 of #1284 (#1346): a non-name instance receiver -- a constructor
/// call, a helper returning the class, one that raises, one holding a walrus
/// -- is evaluated before the foreign read or call, at module scope and in a
/// function body. The helpers returning `FS` are underscore-prefixed because
/// `--ext` refuses a public function returning a class instance (C0003).
const NON_NAME_RECEIVERS: &str = "import os\n\n\n\
    class FS:\n    exists = staticmethod(os.path.exists)\n\n\
    \x20   def __init__(self) -> None:\n        print(\"init\")\n\n\n\
    def _make() -> FS:\n    print(\"make\")\n    return FS()\n\n\n\
    def _boom() -> FS:\n    raise ValueError(\"boom\")\n\n\n\
    def _mk(n: int) -> FS:\n    print(\"mk\", n)\n    return FS()\n\n\n\
    def _side() -> str:\n    print(\"side\")\n    return \"/\"\n\n\n\
    def probe(p: str) -> None:\n\
    \x20   print(FS().exists(p))\n\
    \x20   print(_make().exists(_side()))\n\
    \x20   print(FS().exists.__name__.endswith(\"exists\"))\n\
    \x20   try:\n        print(_boom().exists(p))\n    except ValueError:\n        print(\"caught\")\n\
    \x20   print(_mk((n := 3)).exists(p))\n\
    \x20   print(n)\n\n\n\
    print(FS().exists(\"/\"))\n\
    print(_make().exists(_side()))\n\
    print(FS().exists.__name__.endswith(\"exists\"))\n\
    try:\n    print(_boom().exists(\"/\"))\nexcept ValueError:\n    print(\"caught\")\n\
    print(_mk((m := 3)).exists(\"/\"))\n\
    print(m)\n\
    probe(\"/definitely/not/a/path/here\")\n";

/// CPython 3.14.7's output for [`NON_NAME_RECEIVERS`].
const NON_NAME_RECEIVERS_OUT: &str = "init\nTrue\nmake\ninit\nside\nTrue\ninit\nTrue\ncaught\n\
    mk 3\ninit\nTrue\n3\n\
    init\nFalse\nmake\ninit\nside\nTrue\ninit\nTrue\ncaught\nmk 3\ninit\nFalse\n3\n";

#[test]
fn a_non_name_receiver_is_accepted() {
    assert_checks("fs_receiver_non_name", NON_NAME_RECEIVERS);
    assert_checks("fs_receiver_call", &with_fs("print(FS().exists(\"/\"))\n"));
    assert_checks("fs_receiver_read", &with_fs("print(FS().exists)\n"));
}

#[test]
fn a_super_receiver_is_refused() {
    let needle = "`super().exists` in class `D` reaches the `staticmethod(...)` class attribute \
                  `FS.exists`, which is not supported through `super()` yet (#1358)";
    assert_one_error(
        "fs_super_read",
        &with_fs("class D(FS):\n    def m(self) -> None:\n        print(super().exists)\n"),
        "T0044",
        needle,
    );
    assert_one_error(
        "fs_super_call",
        &with_fs(
            "class D(FS):\n    def m(self, p: str) -> None:\n        print(super().exists(p))\n",
        ),
        "T0044",
        needle,
    );
}

#[test]
fn a_subclass_that_resolves_the_name_differently_is_refused() {
    let needle = "through an instance could reach a subclass override in";
    assert_one_error(
        "fs_subclass_method",
        &with_fs(
            "class D2(FS):\n    def exists(self, p: str) -> str:\n        return \"D2method\"\n\n\n\
             fs = FS()\nprint(fs.exists(\"/\"))\n",
        ),
        "T0044",
        needle,
    );
    assert_one_error(
        "fs_subclass_slot",
        &with_fs(
            "class S:\n    def __init__(self) -> None:\n        self.exists = 1\n\n\n\
             class D5(S, FS):\n    pass\n\n\n\
             def f(fs: FS) -> None:\n    print(fs.exists(\"/\"))\n",
        ),
        "T0044",
        needle,
    );
}

/// A name only a subclass binds is an ordinary unknown attribute of the
/// receiver's declared class, not a subclass-override hazard: the refusal
/// must not suggest a `Base.typo(...)` call that does not exist.
#[test]
fn a_name_only_a_subclass_binds_keeps_the_unknown_attribute_error() {
    assert_one_error(
        "fs_subclass_only",
        &with_fs(
            "class Base:\n    pass\n\n\nclass Sub(Base):\n    typo = staticmethod(os.path.exists)\n\n\n\
             def g(b: Base) -> None:\n    print(b.typo(\"/\"))\n",
        ),
        "T0044",
        "class `Base` has no method named `typo`",
    );
}

/// A comprehension target named after the foreign root does not shadow it
/// at the use site: the loop variable is renamed during lowering, so the
/// rewritten chain still reads the module-level import, as CPython's
/// captured function would.
const COMPREHENSION_TARGET: &str = "import os\n\n\n\
    class FS:\n    exists = staticmethod(os.path.exists)\n\n\n\
    def f() -> None:\n    print(len([1 for os in range(3) if FS.exists(\"/\")]))\n\n\n\
    print(len([1 for os in range(2) if FS.exists(\"/definitely/not/a/path/here\")]))\n\
    f()\n";

#[test]
fn a_comprehension_target_named_after_the_root_is_accepted() {
    assert_checks("fs_comprehension_target", COMPREHENSION_TARGET);
}

/// With several diverging subclasses the refusal names the alphabetically
/// first, whatever order the class table iterates in.
#[test]
fn the_divergence_refusal_names_the_first_subclass_by_name() {
    let method = "    def exists(self, p: str) -> str:\n        return p\n\n\n";
    assert_one_error(
        "fs_subclass_two",
        &with_fs(&format!(
            "class Zed(FS):\n{method}class Alpha(FS):\n{method}class Mid(FS):\n{method}\
             fs = FS()\nprint(fs.exists(\"/\"))\n"
        )),
        "T0044",
        "could reach a subclass override in `Alpha`",
    );
}

/// `D(FS, B, S)`: the foreign attribute wins the class namespace, `S`'s
/// instance slot declines it, and `B`'s `@property` would be reached by the
/// property-first instance dispatch while CPython reads the slot. Both the
/// read and the call are refused.
#[test]
fn a_slot_behind_a_foreign_attribute_with_a_later_property_is_refused() {
    let classes = "class B:\n    @property\n    def exists(self) -> int:\n        return 7\n\n\n\
                   class S:\n    def __init__(self) -> None:\n        self.exists = 3\n\n\n\
                   class D(FS, B, S):\n    pass\n\n\nd = D()\n";
    let needle = "while an `@property` of the same name sits later in the MRO";
    assert_one_error(
        "fs_slot_property_read",
        &with_fs(&format!("{classes}print(d.exists)\n")),
        "T0044",
        needle,
    );
    assert_one_error(
        "fs_slot_property_call",
        &with_fs(&format!("{classes}print(d.exists(\"/\"))\n")),
        "T0044",
        needle,
    );
    // Without the property the slot shape keeps its existing handling.
    assert_checks(
        "fs_slot_no_property",
        &with_fs(
            "class S:\n    def __init__(self) -> None:\n        self.exists = 3\n\n\n\
             class D(FS, S):\n    pass\n\n\nd = D()\nprint(d.exists)\n",
        ),
    );
}

/// `cls.exists(p)` in a `@classmethod` takes the instance path, so a
/// subclass that overrides the name refuses it.
#[test]
fn a_cls_call_with_a_diverging_subclass_is_refused() {
    assert_one_error(
        "fs_cls_subclass",
        "import os\n\n\nclass FS:\n    exists = staticmethod(os.path.exists)\n\n\
         \x20   @classmethod\n    def probe(cls, p: str) -> None:\n        print(cls.exists(p))\n\n\n\
         class G(FS):\n    exists = staticmethod(os.path.isdir)\n",
        "T0044",
        "could reach a subclass override in `G`",
    );
}

/// Rule 7: an instance read whose positional winner is a method, with a
/// foreign attribute later in the MRO, is refused; the call in the same
/// shape reaches the method and is accepted.
#[test]
fn an_instance_read_of_a_method_shadowing_a_foreign_attribute_is_refused() {
    let classes = "class A:\n    def exists(self, p: str) -> str:\n        return p\n\n\n\
                   class D(A, FS):\n    pass\n\n\nd = D()\n";
    assert_one_error(
        "fs_rule7_read",
        &with_fs(&format!("{classes}print(d.exists)\n")),
        "T0044",
        "reaches a method that shadows a `staticmethod(...)` class attribute",
    );
    assert_checks(
        "fs_rule7_call",
        &with_fs(&format!("{classes}print(d.exists(\"/\"))\n")),
    );
}

#[test]
fn a_foreign_attribute_does_not_satisfy_a_protocol_attribute() {
    assert_one_error(
        "fs_protocol",
        &with_fs(
            "from typing import Protocol\n\n\nclass P(Protocol):\n    exists: int\n\n\n\
             def g(p: P) -> int:\n    return p.exists\n\n\ng(FS())\n",
        ),
        "T0046",
        "class `FS` does not conform to protocol `P`: missing attribute `exists`",
    );
}

#[test]
fn a_write_to_a_foreign_attribute_is_refused() {
    let needle = "it is a `staticmethod(...)` class attribute of class";
    assert_one_error(
        "fs_write_instance",
        &with_fs("fs = FS()\nfs.exists = 1\n"),
        "T0044",
        needle,
    );
    assert_one_error(
        "fs_write_subclass_self",
        &with_fs("class D(FS):\n    def __init__(self) -> None:\n        self.exists = 1\n"),
        "T0044",
        needle,
    );
}

// -- other hosts and call shapes: pinned as they are --------------------------

#[test]
fn other_call_shapes_keep_the_existing_rules() {
    assert_one_error(
        "fs_keyword_arg",
        &with_fs("print(FS.exists(path=\"/\"))\n"),
        "C0001",
        "keyword call arguments are not supported yet",
    );
    // The constraint solver has no term for a class-name read, so an
    // unannotated helper that *returns* the call cannot be inferred; one
    // that only calls it is accepted.
    assert_one_error(
        "fs_unannotated_return",
        &with_fs("def _h(p):\n    return FS.exists(p)\n\n\nprint(_h(\"/\"))\n"),
        "T0021",
        "cannot infer return type of private helper `_h`",
    );
    assert_checks(
        "fs_unannotated_call",
        &with_fs("def _h(p):\n    print(FS.exists(p))\n\n\n_h(\"/\")\n"),
    );
    // Binding the attribute or the call's result in a function body is
    // admitted since Part 1 of #1333: both are CPython objects, and a
    // function-local object binding is an ordinary binding.
    assert_checks(
        "fs_bind_in_function",
        &with_fs("def f() -> None:\n    e = FS.exists\n"),
    );
    assert_checks(
        "fs_bind_result_in_function",
        &with_fs("def f() -> None:\n    e = FS.exists(\"/\")\n    print(bool(e))\n"),
    );
    // A `Protocol`-derived host cannot be instantiated in pycc at all.
    assert_one_error(
        "fs_protocol_host",
        &with_fs(
            "from typing import Protocol\n\n\nclass P(Protocol):\n    def exists(self, p: str) -> str: ...\n\n\n\
             class D6(P, FS):\n    pass\n\n\nd6 = D6()\n",
        ),
        "C0001",
        "cannot instantiate protocol class `D6`",
    );
}

/// An exception subclass and a PEP 695 generic class both host the
/// attribute.
const OTHER_HOSTS: &str = "import os\n\n\n\
    class E(Exception):\n    exists = staticmethod(os.path.exists)\n\n\n\
    class G[T]:\n    exists = staticmethod(os.path.exists)\n\n\
    \x20   def __init__(self, v: T) -> None:\n        self.v = v\n\n\n\
    def probe(p: str) -> None:\n\
    \x20   g = G[int](1)\n\
    \x20   print(E.exists(p), G.exists(p), g.exists(p))\n\n\n\
    probe(\"/\")\n";

#[test]
fn check_accepts_an_exception_and_a_generic_host() {
    assert_checks("fs_other_hosts", OTHER_HOSTS);
}

// -- hosted --ext runs --------------------------------------------------------

/// Builds `dir/<source>` as the extension module `module`.
fn build_ext_from(dir: &Path, source: &str, module: &str) {
    let build = pycc()
        .arg("build")
        .arg(dir.join(source))
        .arg("-o")
        .arg(dir.join(module))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
}

fn python(dir: &Path, script: &str) -> Output {
    host_python()
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn")
}

fn assert_ok(run: &Output) {
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(run),
        stderr_of(run)
    );
}

/// Runs `target` and prints what it raised -- any exception, by type and
/// `str(e)` -- after whatever the module body printed.
fn raised_report(target: &str) -> String {
    format!(
        "import runpy\n\
         try:\n\
         \x20   {target}\n\
         except Exception as e:\n\
         \x20   print(type(e).__name__, e)\n\
         else:\n\
         \x20   print('no error')\n"
    )
}

/// Builds `body` (written as `m.py`), then returns the extension's import
/// report and CPython's own run of the same source.
fn compiled_and_oracle(tag: &str, module: &str, body: &str) -> (String, String) {
    let dir = ScratchDir::new(tag).expect("scratch");
    write(&dir, "m.py", body);
    build_ext_from(&dir, "m.py", module);
    let compiled = python(&dir, &raised_report(&format!("import {module}")));
    assert_ok(&compiled);
    let oracle = python(&dir, &raised_report("runpy.run_path('m.py')"));
    assert_ok(&oracle);
    (stdout_of(&compiled), stdout_of(&oracle))
}

fn assert_matches_cpython(tag: &str, module: &str, body: &str) -> String {
    let (compiled, oracle) = compiled_and_oracle(tag, module, body);
    assert_eq!(compiled, oracle);
    compiled
}

/// A runtime `isinstance` against a `@runtime_checkable` protocol answers
/// like CPython's `hasattr` check even though the static conformance check
/// refuses the attribute (T0046); a `cls.exists(p)` call and a
/// `cls.exists.__name__` read in a `@classmethod` run like CPython.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_protocol_isinstance_and_a_cls_call_run_like_cpython_in_the_host() {
    let body = "import os\nfrom typing import Protocol, runtime_checkable\n\n\n\
                @runtime_checkable\nclass P(Protocol):\n    exists: int\n\n\n\
                class FS:\n    exists = staticmethod(os.path.exists)\n\n\
                \x20   @classmethod\n    def probe(cls, p: str) -> None:\n\
                \x20       print(cls.exists(p))\n        print(bool(cls.exists.__name__))\n\n\n\
                def main() -> None:\n    f = FS()\n    print(isinstance(f, P))\n    FS.probe(\"/\")\n\n\n\
                main()\n";
    let out = assert_matches_cpython("fs_hosted_protocol_cls", "fs_protocol_cls_mod", body);
    assert_eq!(out, "True\nTrue\nTrue\nno error\n");
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_winning_mro_cases_run_like_cpython_in_the_host() {
    let out = assert_matches_cpython("fs_hosted_mro_wins", "fs_mro_wins_mod", MRO_WINS);
    // The function's `__name__` is `exists` on POSIX and `_path_exists` on
    // Windows; both runs agree, so only the booleans are pinned here.
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 15, "{out}");
    for (index, line) in lines.iter().enumerate() {
        let expected = match index {
            0..=3 | 5 => "True True",
            6..=9 | 11 => "False False",
            12 => "True",
            14 => "no error",
            _ => continue,
        };
        assert_eq!(*line, expected, "{out}");
    }
    for index in [4, 10, 13] {
        assert!(lines[index].ends_with("exists"), "{out}");
    }
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_existing_dispatch_cases_run_like_cpython_in_the_host() {
    let out = assert_matches_cpython("fs_hosted_mro_others", "fs_mro_others_mod", MRO_OTHERS);
    assert_eq!(out, "D2method\nA.static A.static\n1 True\nno error\n");
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn operator_roots_run_like_cpython_in_the_host() {
    let out = assert_matches_cpython("fs_hosted_operator", "fs_operator_mod", OPERATOR);
    assert_eq!(out, "5 5 6 6\n5 5\nab 4.0\nno error\n");
}

/// Only the exception host runs hosted: in a module with a PEP 695 generic
/// class, even a plain `os.path.exists(p)` in a function body passes
/// `pycc check` and fails `pycc build` with `T0021` (`os` is not defined),
/// independent of this attribute. That is the same check/build divergence
/// shape as item 3 of #989 (there `T0021` names the generic class itself);
/// the shared cause is inferred, not isolated.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_exception_host_runs_like_cpython_in_the_host() {
    let out = assert_matches_cpython(
        "fs_hosted_exception",
        "fs_exception_mod",
        "import os\n\n\nclass E(Exception):\n    exists = staticmethod(os.path.exists)\n\n\n\
         def probe(p: str) -> None:\n    print(E.exists(p))\n\n\n\
         probe(\"/\")\nprint(E.exists(\"/definitely/not/a/path/here\"))\n",
    );
    assert_eq!(out, "True\nFalse\nno error\n");
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_comprehension_target_named_after_the_root_runs_like_cpython_in_the_host() {
    let out = assert_matches_cpython(
        "fs_hosted_comprehension",
        "fs_comprehension_mod",
        COMPREHENSION_TARGET,
    );
    assert_eq!(out, "0\n3\nno error\n");
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_non_name_receiver_runs_like_cpython_in_the_host() {
    let out = assert_matches_cpython("fs_hosted_non_name", "fs_non_name_mod", NON_NAME_RECEIVERS);
    assert_eq!(out, format!("{NON_NAME_RECEIVERS_OUT}no error\n"));
}

/// `utils.py` defines `FS`; the built module imports it and uses it in a
/// function and in its own body.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_cross_module_use_runs_like_cpython_in_the_host() {
    let dir = ScratchDir::new("fs_hosted_cross").expect("scratch");
    write(&dir, "utils.py", FS);
    write(
        &dir,
        "main.py",
        "from utils import FS\n\n\ndef check(p: str) -> None:\n    print(FS.exists(p))\n\n\n\
         check(\"/\")\nprint(FS.exists(\"/definitely/not/a/path/here\"))\n",
    );
    build_ext_from(&dir, "main.py", "fs_cross_mod");
    let compiled = python(&dir, &raised_report("import fs_cross_mod"));
    assert_ok(&compiled);
    let oracle = python(&dir, &raised_report("runpy.run_path('main.py')"));
    assert_ok(&oracle);
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    assert_eq!(stdout_of(&compiled), "True\nFalse\nno error\n");
}

/// A documented divergence: CPython evaluates `os.path.nope` when the class
/// is created and fails the import; pycc re-reads the chain at each access,
/// so the `AttributeError` is raised by the first call, where a function
/// body's `except Exception` catches it.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_missing_attribute_raises_at_first_use_in_the_host() {
    let (compiled, oracle) = compiled_and_oracle(
        "fs_hosted_missing",
        "fs_missing_mod",
        "import os\n\n\nclass FS:\n    nope = staticmethod(os.path.nope)\n\n\n\
         def probe() -> None:\n    try:\n        print(FS.nope(\"/\"))\n    except Exception:\n\
         \x20       print(\"caught\")\n\n\nprint(\"imported\")\nprobe()\n",
    );
    assert_eq!(compiled, "imported\ncaught\nno error\n");
    assert!(oracle.starts_with("AttributeError "), "{oracle}");
    assert!(oracle.contains("has no attribute 'nope'"), "{oracle}");
}

/// Class attributes are not published on the extension type, and this one
/// is no exception.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_attribute_is_absent_on_the_host_type() {
    let dir = ScratchDir::new("fs_hosted_absent").expect("scratch");
    // A class is published only with an exportable method.
    write(
        &dir,
        "m.py",
        &format!("{FS}class Host(FS):\n    def ping(self) -> int:\n        return 1\n"),
    );
    build_ext_from(&dir, "m.py", "fs_absent_mod");
    let run = python(
        &dir,
        "import fs_absent_mod as m\nprint(m.Host().ping(), hasattr(m.Host, 'exists'))\n",
    );
    assert_ok(&run);
    assert_eq!(stdout_of(&run), "1 False\n");
}

/// A plain (embedded) build compiles its module with `ext` set, so the
/// class-name and instance calls -- including through a non-name receiver
/// (#1346) -- run there too, matching CPython's own output.
#[cfg(not(windows))]
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_embedded_build_runs_like_cpython() {
    for (tag, body) in [
        ("fs_embedded", MRO_WINS),
        ("fs_embedded_non_name", NON_NAME_RECEIVERS),
    ] {
        let dir = ScratchDir::new(tag).expect("scratch");
        let source = write(&dir, "m.py", body);
        let build = pycc()
            .arg("build")
            .arg(&source)
            .arg("-o")
            .arg(dir.join("app"))
            .output()
            .expect("pycc should spawn");
        assert!(build.status.success(), "{}", stderr_of(&build));
        let embedded = Command::new(dir.join("app"))
            .output()
            .expect("the embedded binary runs");
        assert_ok(&embedded);
        let oracle = host_python()
            .arg(&source)
            .output()
            .expect("python3 should spawn");
        assert_ok(&oracle);
        assert_eq!(stdout_of(&embedded), stdout_of(&oracle));
        if body == NON_NAME_RECEIVERS {
            assert_eq!(stdout_of(&embedded), NON_NAME_RECEIVERS_OUT);
        }
    }
}
