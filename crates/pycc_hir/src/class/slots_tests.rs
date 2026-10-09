//! Unit tests for `class/slots.rs` (#1368): one test per rule of the
//! issue plan's sections 4.1-4.4, each refusal pinned by its code and the
//! CPython text it quotes.

use super::{ClassSlotsRow, is_ascii_identifier, is_dunder, mangle};
use crate::pycc_parser_test_helper::parse;
use crate::{
    LoweredModule, ResolvedImport, ResolvedImports, ResolvedModule, lower_module,
    project_import_requests,
};
use pycc_diag::{Diagnostic, Span};

fn lower(source: &str) -> Result<LoweredModule, Diagnostic> {
    lower_module(&parse(source), &ResolvedImports::default(), None)
        .map_err(|mut diagnostics| diagnostics.remove(0))
}

fn rows(source: &str) -> Vec<ClassSlotsRow> {
    lower(source).expect("fixture must lower").class_slots
}

fn slots_of(source: &str, class: &str) -> Option<Vec<String>> {
    rows(source)
        .into_iter()
        .find(|(name, _)| name == class)
        .expect("the class has a row")
        .1
}

fn names(list: &[&str]) -> Option<Vec<String>> {
    Some(list.iter().map(|name| (*name).to_string()).collect())
}

fn error(source: &str) -> Diagnostic {
    lower(source).expect_err("fixture must be refused")
}

fn c0001(source: &str) -> String {
    let diagnostic = error(source);
    assert_eq!(diagnostic.code, "C0001", "source: {source:?}");
    diagnostic.message
}

fn t0044(source: &str) -> Diagnostic {
    let diagnostic = error(source);
    assert_eq!(diagnostic.code, "T0044", "source: {source:?}");
    diagnostic
}

/// `class C:` binding `__slots__ = <value>`, storing `self.a` in `__init__`.
fn slotted(value: &str) -> String {
    format!(
        "class C:\n    __slots__ = {value}\n\n    def __init__(self) -> None:\n        self.a = 1\n"
    )
}

// -- 4.1: the accepted spellings -------------------------------------------

#[test]
fn every_admitted_spelling_records_its_slot_list() {
    for (value, expected) in [
        ("'a'", names(&["a"])),
        ("('a',)", names(&["a"])),
        ("'a', 'b'", names(&["a", "b"])),
        ("['a', 'b']", names(&["a", "b"])),
        ("('a', 'a')", names(&["a", "a"])),
        ("('a', 'class')", names(&["a", "class"])),
    ] {
        assert_eq!(slots_of(&slotted(value), "C"), expected, "{value}");
    }
}

#[test]
fn an_empty_slot_list_is_recorded_as_slotted_and_empty() {
    for value in ["()", "[]"] {
        let source = format!("class C:\n    __slots__ = {value}\n");
        assert_eq!(slots_of(&source, "C"), Some(Vec::new()), "{value}");
    }
}

#[test]
fn a_class_without_slots_has_an_unslotted_row() {
    let source = "class C:\n    def __init__(self) -> None:\n        self.a = 1\n";
    assert_eq!(slots_of(source, "C"), None);
}

#[test]
fn the_annotated_binding_is_admitted_including_class_var() {
    let plain = "class C:\n    __slots__: tuple = ('a',)\n\n    def __init__(self) -> None:\n        \
                 self.a = 1\n";
    assert_eq!(slots_of(plain, "C"), names(&["a"]));
    let class_var = "from typing import ClassVar\n\n\nclass C:\n    __slots__: ClassVar[tuple] = \
                     ('a',)\n\n    def __init__(self) -> None:\n        self.a = 1\n";
    assert_eq!(slots_of(class_var, "C"), names(&["a"]));
}

#[test]
fn the_lark_shape_slots_alongside_value_less_declarations_is_admitted() {
    let source = "class ParseConf:\n    __slots__ = 'parse_table', 'start'\n\n    parse_table: \
                  int\n    start: int\n\n    def __init__(self, parse_table: int, start: int) -> \
                  None:\n        self.parse_table = parse_table\n        self.start = start\n";
    assert_eq!(
        slots_of(source, "ParseConf"),
        names(&["parse_table", "start"])
    );
}

#[test]
fn a_declared_slot_that_is_never_assigned_is_accepted() {
    assert_eq!(slots_of(&slotted("('a', 'b')"), "C"), names(&["a", "b"]));
}

#[test]
fn a_pep_695_generic_class_is_admitted_the_same_way() {
    let source = "class C[T]:\n    __slots__ = ('a',)\n\n    def __init__(self, a: T) -> None:\n        \
                  self.a = a\n";
    assert_eq!(slots_of(source, "C"), names(&["a"]));
}

// -- 4.1: the refused values -----------------------------------------------

#[test]
fn a_non_iterable_literal_quotes_cpythons_type_error() {
    for (value, type_name) in [
        ("1", "int"),
        ("1.5", "float"),
        ("1j", "complex"),
        ("True", "bool"),
        ("None", "NoneType"),
    ] {
        assert_eq!(
            c0001(&slotted(value)),
            format!(
                "`__slots__` must be a string or an iterable of strings -- CPython raises \
                 `TypeError: '{type_name}' object is not iterable` when the class is created"
            ),
            "{value}"
        );
    }
}

#[test]
fn a_non_literal_value_is_not_supported_yet() {
    for (value, kind) in [("x", "name"), ("{'a': 1}", "dict"), ("{'a'}", "set")] {
        let source = format!("x = ('a',)\n\n\n{}", slotted(value));
        let message = c0001(&source);
        assert!(
            message.starts_with(
                "a `__slots__` value that is not a string literal or a tuple or list of string \
                 literals is not supported yet"
            ),
            "{value}: {message}"
        );
        assert!(message.contains(kind), "{value}: {message}");
    }
}

#[test]
fn a_non_string_item_quotes_cpythons_type_error() {
    assert_eq!(
        c0001(&slotted("('a', 1)")),
        "every `__slots__` entry must be a string -- CPython raises `TypeError: __slots__ items \
         must be strings, not 'int'` when the class is created"
    );
}

#[test]
fn a_non_literal_item_is_not_supported_yet() {
    let message = c0001(&format!("x = 'a'\n\n\n{}", slotted("(x,)")));
    assert!(
        message
            .starts_with("a `__slots__` entry that is not a string literal is not supported yet"),
        "{message}"
    );
}

#[test]
fn a_non_identifier_item_quotes_cpythons_type_error() {
    for value in ["('a b',)", "('',)", "('1a',)"] {
        let message = c0001(&slotted(value));
        assert!(
            message.ends_with(
                "is not an identifier -- CPython raises `TypeError: __slots__ must be \
                 identifiers` when the class is created"
            ),
            "{value}: {message}"
        );
    }
}

#[test]
fn a_non_ascii_item_is_not_supported_yet() {
    assert_eq!(
        c0001(&slotted("('\u{e9}',)")),
        "the non-ASCII `__slots__` entry `\u{e9}` is not supported yet"
    );
}

#[test]
fn dict_and_weakref_entries_are_not_supported_yet() {
    for entry in ["__dict__", "__weakref__"] {
        assert_eq!(
            c0001(&slotted(&format!("('a', '{entry}')"))),
            format!(
                "a `{entry}` entry in `__slots__` is not supported yet -- pycc instances have \
                 neither a `__dict__` nor weak-reference support"
            )
        );
    }
}

#[test]
fn each_version_sensitive_name_is_not_supported_yet() {
    for entry in [
        "__firstlineno__",
        "__static_attributes__",
        "__annotations__",
    ] {
        assert_eq!(
            c0001(&slotted(&format!("('a', '{entry}')"))),
            format!(
                "a `__slots__` entry named `{entry}` is not supported yet -- whether it \
                 conflicts with the class namespace differs between CPython versions"
            )
        );
    }
}

#[test]
fn a_second_binding_is_not_supported_yet() {
    let source = "class C:\n    __slots__ = ('a',)\n    __slots__ = ('a',)\n\n    def \
                  __init__(self) -> None:\n        self.a = 1\n";
    let diagnostic = error(source);
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "a second `__slots__` binding in one class body is not supported yet"
    );
    let second = source.rfind("__slots__").expect("second binding") as u32;
    assert_eq!(diagnostic.span.map(|span| span.start), Some(second));
}

// -- 4.2: the class namespace ----------------------------------------------

fn conflict(slot: &str, mangled: &str) -> String {
    format!(
        "the `__slots__` entry `{slot}` of class `C` is also bound in the class body -- CPython \
         raises `ValueError: '{mangled}' in __slots__ conflicts with class variable` when the \
         class is created"
    )
}

#[test]
fn a_slot_named_like_a_class_body_binding_quotes_cpythons_value_error() {
    for (body, slot) in [
        ("    a = 1\n", "a"),
        ("    a: int = 1\n", "a"),
        ("    def a(self) -> int:\n        return 1\n", "a"),
        (
            "    @property\n    def a(self) -> int:\n        return 1\n",
            "a",
        ),
        (
            "    @staticmethod\n    def a() -> int:\n        return 1\n",
            "a",
        ),
        (
            "    def __init__(self) -> None:\n        pass\n",
            "__init__",
        ),
        ("    pass\n", "__module__"),
        ("    pass\n", "__slots__"),
    ] {
        let source = format!("class C:\n    __slots__ = ('{slot}',)\n{body}");
        assert_eq!(c0001(&source), conflict(slot, slot), "{body}");
    }
}

#[test]
fn a_class_body_private_name_is_compared_after_mangling() {
    // The body's `__a` is `_C__a`, which the explicitly spelled slot matches.
    let source = "class C:\n    __slots__ = ('_C__a',)\n    __a = 1\n";
    assert_eq!(c0001(source), conflict("_C__a", "_C__a"));
}

fn private(slot: &str, mangled: &str) -> String {
    format!(
        "the private `__slots__` entry `{slot}` is not supported yet -- CPython mangles it with \
         the declaring class's name to the slot `{mangled}`, and pycc does not mangle a private \
         attribute name on an instance, so `self.{slot}` would not reach it"
    )
}

#[test]
fn a_private_slot_entry_is_not_supported_yet() {
    // CPython stores `self.__x` as `_C__x`, and a module-level `C().__x` is
    // an `AttributeError` there; pycc keeps the name unmangled.
    let source = "class C:\n    __slots__ = ('a', '__x')\n\n    def __init__(self) -> None:\n        \
                  self.a = 1\n        self.__x = 2\n";
    let diagnostic = error(source);
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(diagnostic.message, private("__x", "_C__x"));
    assert_eq!(
        diagnostic.span.map(|span| span.start),
        source
            .find("__slots__")
            .and_then(|at| u32::try_from(at).ok())
    );
    assert_eq!(c0001(&slotted("('__x_',)")), private("__x_", "_C__x_"));
    assert_eq!(
        c0001("class __Ab:\n    __slots__ = ('__x',)\n"),
        private("__x", "_Ab__x")
    );
}

#[test]
fn a_private_slot_keeps_the_errors_cpython_raises_first() {
    // Both sides of the namespace check are mangled: the slot `__a` and the
    // body's `__a` are both `_C__a`, and CPython's `ValueError` quotes that.
    assert_eq!(
        c0001("class C:\n    __slots__ = ('__a',)\n    __a = 1\n"),
        conflict("__a", "_C__a")
    );
    // Every entry is validated before any is mangled, so a later non-string
    // entry is CPython's `TypeError`.
    assert_eq!(
        c0001(&slotted("('__x', 1)")),
        "every `__slots__` entry must be a string -- CPython raises `TypeError: __slots__ items \
         must be strings, not 'int'` when the class is created"
    );
}

#[test]
fn a_private_slot_of_a_class_that_mangles_nothing_is_admitted() {
    // A class named only with underscores mangles nothing, so its `__x` slot
    // is `__x` in CPython too.
    let source = "class __:\n    __slots__ = ('__x',)\n\n    def __init__(self) -> None:\n        \
                  self.__x = 1\n";
    assert_eq!(slots_of(source, "__"), names(&["__x"]));
}

#[test]
fn a_doc_slot_conflicts_only_with_a_docstring() {
    let with_docstring = "class C:\n    \"\"\"Doc.\"\"\"\n    __slots__ = ('__doc__',)\n";
    assert_eq!(c0001(with_docstring), conflict("__doc__", "__doc__"));
    let without = "class C:\n    __slots__ = ('__doc__',)\n";
    assert_eq!(c0001(without), dunder("__doc__"));
}

#[test]
fn a_value_less_annotation_and_implicit_names_do_not_conflict() {
    let source = "class C:\n    __slots__ = ('a',)\n    a: int\n\n    def __init__(self) -> \
                  None:\n        self.a = 1\n";
    assert_eq!(slots_of(source, "C"), names(&["a"]));
    // `__qualname__` is not in the namespace CPython checks, so it reaches
    // the dunder refusal rather than the conflict `ValueError`.
    let qualname = "class C:\n    __slots__ = ('__qualname__',)\n";
    assert_eq!(c0001(qualname), dunder("__qualname__"));
}

fn dunder(slot: &str) -> String {
    format!(
        "a `__slots__` entry named `{slot}` is not supported yet -- CPython gives many `__x__` \
         names a special meaning (a `__hash__` slot makes the class unhashable), and pycc does \
         not model which, so every dunder entry is refused"
    )
}

#[test]
fn a_dunder_slot_is_refused_at_the_binding() {
    for slot in ["__hash__", "__eq__", "__len__", "__str__"] {
        let source = format!(
            "class C:\n    __slots__ = ('a', '{slot}')\n\n    def __init__(self) -> None:\n        \
             self.a = 1\n"
        );
        let diagnostic = error(&source);
        assert_eq!(diagnostic.code, "C0001", "{slot}");
        assert_eq!(diagnostic.message, dunder(slot));
        let start = source.find("__slots__").expect("binding") as u32;
        let end = source.find(")\n").expect("binding end") as u32 + 1;
        assert_eq!(diagnostic.span, Some(Span::new(start, end)), "{slot}");
    }
}

#[test]
fn a_dunder_slot_bound_in_the_body_keeps_the_conflict_error() {
    let source = "class C:\n    __slots__ = ('__eq__',)\n\n    def __eq__(self, other: int) -> \
                  bool:\n        return True\n";
    assert_eq!(c0001(source), conflict("__eq__", "__eq__"));
}

#[test]
fn is_dunder_needs_a_non_empty_middle() {
    assert!(is_dunder("__a__"));
    assert!(!is_dunder("____"));
    assert!(!is_dunder("__a"));
    assert!(!is_dunder("a__"));
}

fn inherited(slot: &str, class: &str, ancestor: &str, mangled: &str) -> String {
    format!(
        "the `__slots__` entry `{slot}` of class `{class}` is not supported yet -- `{ancestor}` \
         binds `{mangled}` at class level, and CPython's member descriptor for the slot shadows \
         the inherited `{ancestor}.{mangled}`, so reading the unset slot raises `AttributeError` \
         where pycc would find the inherited binding"
    )
}

#[test]
fn a_name_an_ancestor_binds_at_class_level_is_refused_at_the_binding() {
    for (base, slot) in [
        (
            "    a = 1
",
            "a",
        ),
        (
            "    def a(self) -> int:
        return 1
",
            "a",
        ),
        (
            "    @staticmethod
    def a() -> int:
        return 1
",
            "a",
        ),
        (
            "    @classmethod
    def a(cls) -> int:
        return 1
",
            "a",
        ),
        (
            "    @property
    def a(self) -> int:
        return 1
",
            "a",
        ),
    ] {
        let source = format!(
            "class A:\n{base}\n\nclass B(A):\n    pass\n\n\nclass C(B):\n    __slots__ = \
             ('{slot}',)\n"
        );
        let diagnostic = error(&source);
        assert_eq!(diagnostic.code, "C0001", "{base}");
        assert_eq!(
            diagnostic.message,
            inherited(slot, "C", "A", slot),
            "{base}"
        );
        let start = source.find("__slots__").expect("binding") as u32;
        let end = source.rfind(")\n").expect("binding end") as u32 + 1;
        assert_eq!(diagnostic.span, Some(Span::new(start, end)), "{base}");
    }
}

#[test]
fn an_inherited_private_name_is_compared_after_mangling() {
    // `B.__p` is `_B__p`: a `C` slot spelled `_B__p` shadows it, while one
    // spelled `_C__p` does not.
    let base = "class B:\n    __p = 1\n\n\nclass C(B):\n    __slots__ = ";
    assert_eq!(
        c0001(&format!("{base}('_B__p',)\n")),
        inherited("_B__p", "C", "B", "_B__p")
    );
    assert_eq!(
        slots_of(&format!("{base}('_C__p',)\n"), "C"),
        names(&["_C__p"])
    );
}

#[test]
fn an_ancestor_instance_attribute_or_slot_does_not_conflict() {
    let instance = "class B:\n    def __init__(self) -> None:\n        self.a = 1\n\n\nclass \
                    C(B):\n    __slots__ = ('a',)\n";
    assert_eq!(slots_of(instance, "C"), names(&["a"]));
    let redeclared = "class B:\n    __slots__ = ('a',)\n\n\nclass C(B):\n    __slots__ = \
                      ('a',)\n";
    assert_eq!(slots_of(redeclared, "C"), names(&["a"]));
}

#[test]
fn a_builtin_exception_attribute_slot_is_refused() {
    // Each name is reported against the builtin that defines it: pycc's root
    // `Exception` stands in for CPython's `BaseException`.
    for (base, slot, definer) in [
        ("Exception", "args", "Exception"),
        ("ValueError", "with_traceback", "Exception"),
        ("OSError", "errno", "OSError"),
        ("FileNotFoundError", "filename", "OSError"),
        ("ImportError", "name", "ImportError"),
        ("ModuleNotFoundError", "path", "ImportError"),
        ("ExceptionGroup", "message", "BaseExceptionGroup"),
    ] {
        let source = format!("class C({base}):\n    __slots__ = ('{slot}',)\n");
        assert_eq!(
            c0001(&source),
            inherited(slot, "C", definer, slot),
            "{base}"
        );
    }
    // A name only another builtin defines does not conflict: CPython accepts
    // `class C(ValueError): __slots__ = ('errno',)`.
    for (base, slot) in [
        ("Exception", "code"),
        ("ValueError", "errno"),
        ("OSError", "name"),
        ("KeyError", "message"),
    ] {
        let admitted = format!("class C({base}):\n    __slots__ = ('{slot}',)\n");
        assert_eq!(slots_of(&admitted, "C"), names(&[slot]), "{base}");
    }
}

fn sibling(class: &str, declarer: &str, slot: &str, later: &str, mangled: &str) -> String {
    format!(
        "class `{class}` is not supported yet -- its base `{declarer}` declares the slot \
         `{slot}`, and `{later}`, later in `{class}`'s MRO, binds `{mangled}` at class level; \
         CPython's member descriptor for `{declarer}`'s slot comes first and shadows \
         `{later}.{mangled}`, so reading the unset slot raises `AttributeError` where pycc \
         would find `{later}`'s binding"
    )
}

#[test]
fn a_slotted_base_shadowing_a_later_sibling_binding_is_refused() {
    // `class C(B, A)`: `B`'s slot descriptor precedes `A` in `C`'s MRO, so
    // CPython's `C().a` reads the unset slot even though `C` binds no
    // `__slots__` itself.
    for binding in [
        "    a = 1\n",
        "    def a(self) -> int:\n        return 1\n",
        "    @property\n    def a(self) -> int:\n        return 1\n",
    ] {
        let source = format!(
            "class A:\n{binding}\n\nclass B:\n    __slots__ = ('a',)\n\n\nclass C(B, A):\n    \
             pass\n"
        );
        let diagnostic = error(&source);
        assert_eq!(diagnostic.code, "C0001", "{binding}");
        assert_eq!(
            diagnostic.message,
            sibling("C", "B", "a", "A", "a"),
            "{binding}"
        );
        let start = source.find("class C").expect("class C") as u32;
        assert_eq!(
            diagnostic.span.map(|span| span.start),
            Some(start),
            "{binding}"
        );
    }
    // The sibling's private name is compared after mangling by the sibling.
    // (With the sibling first, see
    // `an_earlier_binding_over_a_base_slot_is_refused`.)
    let mangled = "class A:\n    __p = 1\n\n\nclass B:\n    __slots__ = ('_A__p',)\n\n\nclass \
                   C(B, A):\n    pass\n";
    assert_eq!(c0001(mangled), sibling("C", "B", "_A__p", "A", "_A__p"));
}

fn shadowed(class: &str, declarer: &str, slot: &str, earlier: &str) -> String {
    format!(
        "class `{class}` is not supported yet -- its base `{declarer}` declares the slot \
         `{slot}`, and `{earlier}`, earlier in `{class}`'s MRO, binds `{slot}` at class level; \
         CPython finds `{earlier}.{slot}` before `{declarer}`'s member descriptor, so a store \
         to the slot does not reach it, where pycc would write the slot"
    )
}

#[test]
fn an_earlier_binding_over_a_base_slot_is_refused() {
    // `class C(A, B)` with every class slotted: CPython finds `A.a` before
    // `B`'s member descriptor, and `B.__init__`'s `self.a = 2` raises
    // `AttributeError` because `C` has no `__dict__`.
    let sibling_first = "class A:\n    __slots__ = ()\n    a = 1\n\n\nclass B:\n    __slots__ = \
                         ('a',)\n\n    def __init__(self) -> None:\n        self.a = 2\n\n\nclass \
                         C(A, B):\n    __slots__ = ()\n";
    assert_eq!(c0001(sibling_first), shadowed("C", "B", "a", "A"));
    // The class's own body is the earliest MRO entry.
    let own_body = "class B:\n    __slots__ = ('a',)\n\n\nclass C(B):\n    __slots__ = ()\n    \
                    a = 1\n";
    assert_eq!(c0001(own_body), shadowed("C", "B", "a", "C"));
    // With a `__dict__` the binding still wins: a read-only property raises
    // on the store, and a plain value leaves the store in the dictionary.
    for binding in [
        "    a = 1\n",
        "    @property\n    def a(self) -> int:\n        return 1\n",
    ] {
        let with_dict = format!(
            "class A:\n{binding}\n\nclass B:\n    __slots__ = ('a',)\n\n\nclass C(A, B):\n    \
             pass\n"
        );
        assert_eq!(c0001(&with_dict), shadowed("C", "B", "a", "A"), "{binding}");
    }
    let own_with_dict = "class B:\n    __slots__ = ('a',)\n\n\nclass C(B):\n    a = 1\n";
    assert_eq!(c0001(own_with_dict), shadowed("C", "B", "a", "C"));
}

// -- 4.3: an undeclared store ----------------------------------------------

fn no_slot(class: &str, attr: &str) -> String {
    format!(
        "class `{class}` has no slot for attribute `{attr}` -- every class in its MRO binds \
         `__slots__`, so its instances have no `__dict__`, and CPython raises \
         `AttributeError: '{class}' object has no attribute '{attr}' and no __dict__ for \
         setting new attributes` (3.13+ wording) at this store"
    )
}

#[test]
fn a_store_outside_the_slot_list_is_t0044_at_the_store() {
    let source = "class C:\n    __slots__ = ('a',)\n\n    def __init__(self) -> None:\n        \
                  self.a = 1\n        self.b = 2\n";
    let diagnostic = t0044(source);
    assert_eq!(diagnostic.message, no_slot("C", "b"));
    let start = source.find("self.b").expect("store") as u32;
    assert_eq!(diagnostic.span, Some(Span::new(start, start + 6)));
}

#[test]
fn an_annotated_store_outside_the_slot_list_is_t0044_at_the_store() {
    let source = "class C:\n    __slots__ = ()\n\n    def __init__(me) -> None:\n        me.b: \
                  list[int] = []\n";
    let diagnostic = t0044(source);
    assert_eq!(diagnostic.message, no_slot("C", "b"));
    let start = source.find("me.b").expect("store") as u32;
    assert_eq!(diagnostic.span, Some(Span::new(start, start + 4)));
}

#[test]
fn the_store_span_skips_earlier_init_statements() {
    let source = "class C:\n    __slots__ = ('a',)\n\n    def __init__(self) -> None:\n        \
                  print(1)\n        self.a = 1\n        self.b = 2\n";
    let diagnostic = t0044(source);
    assert_eq!(diagnostic.message, no_slot("C", "b"));
    let start = source.find("self.b").expect("store") as u32;
    assert_eq!(diagnostic.span, Some(Span::new(start, start + 6)));
}

#[test]
fn a_store_behind_an_abc_base_is_still_t0044() {
    let source = "from abc import ABC\n\n\nclass C(ABC):\n    __slots__ = ('a',)\n\n    def \
                  __init__(self, a: int) -> None:\n        self.b = a\n";
    assert_eq!(t0044(source).message, no_slot("C", "b"));
}

#[test]
fn a_slotted_subclass_of_a_slotted_base_may_store_the_base_slots() {
    let source = "class B:\n    __slots__ = ('a',)\n\n    def __init__(self) -> None:\n        \
                  self.a = 1\n\n\nclass C(B):\n    __slots__ = ('b',)\n\n    def __init__(self) \
                  -> None:\n        self.a = 1\n        self.b = 2\n";
    assert_eq!(slots_of(source, "C"), names(&["b"]));
}

fn private_store(class: &str, attr: &str, mangled: &str) -> String {
    format!(
        "the private instance attribute `{attr}` of class `{class}` is not supported yet (#1392) \
         -- `{class}` binds `__slots__`, CPython mangles the store with the class's name to \
         `{mangled}`, and pycc does not mangle a private attribute name on an instance, so \
         `self.{attr}` would not reach the storage CPython uses"
    )
}

#[test]
fn a_private_store_is_not_supported_yet_even_with_the_mangled_slot() {
    // CPython stores `self.__p` in the slot `_C__p`, and a module-level
    // `C().__p` raises `AttributeError`; pycc would lay the attribute out as
    // `__p` and read it back.
    let source = "class C:\n    __slots__ = ('a', '_C__p')\n\n    def __init__(self) -> None:\n        \
                  self.a = 1\n        self.__p = 1\n";
    let diagnostic = error(source);
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(diagnostic.message, private_store("C", "__p", "_C__p"));
    let start = source.find("self.__p").expect("store") as u32;
    assert_eq!(diagnostic.span, Some(Span::new(start, start + 8)));
}

#[test]
fn a_private_store_with_no_matching_slot_is_refused_before_t0044() {
    // `B`'s `_B__p` does not match `C`'s mangled `_C__p`, so CPython raises
    // `AttributeError` naming `_C__p`; the blanket private refusal comes
    // first (a conservative trade-off until #1392), so T0044 only sees names
    // spelled as CPython stores them.
    let source = "class B:\n    __slots__ = ('_B__p',)\n\n\nclass C(B):\n    __slots__ = ()\n\n    \
                  def __init__(self) -> None:\n        self.__p = 1\n";
    assert_eq!(c0001(source), private_store("C", "__p", "_C__p"));
}

#[test]
fn a_store_spelled_in_mangled_form_matches_the_mangled_slot() {
    let source = "class C:\n    __slots__ = ('_C__p',)\n\n    def __init__(self) -> None:\n        \
                  self._C__p = 1\n";
    assert_eq!(slots_of(source, "C"), names(&["_C__p"]));
}

#[test]
fn a_private_store_of_a_class_that_mangles_nothing_is_checked_as_written() {
    let source = "class __:\n    __slots__ = ()\n\n    def __init__(self) -> None:\n        \
                  self.__p = 1\n";
    assert_eq!(t0044(source).message, no_slot("__", "__p"));
}

#[test]
fn an_unslotted_ancestor_keeps_a_dict_so_nothing_is_checked() {
    let unslotted_base = "class B:\n    pass\n\n\nclass C(B):\n    __slots__ = ('a',)\n\n    def \
                          __init__(self) -> None:\n        self.b = 1\n";
    assert_eq!(slots_of(unslotted_base, "C"), names(&["a"]));
    let unslotted_sub = "class B:\n    __slots__ = ('a',)\n\n\nclass C(B):\n    def \
                         __init__(self) -> None:\n        self.b = 1\n";
    assert_eq!(slots_of(unslotted_sub, "C"), None);
}

#[test]
fn an_exception_class_keeps_a_dict_so_nothing_is_checked() {
    let source = "class E(Exception):\n    __slots__ = ('a',)\n\n    def __init__(self) -> \
                  None:\n        self.b = 1\n";
    assert_eq!(slots_of(source, "E"), names(&["a"]));
}

#[test]
fn an_unslotted_value_less_declaration_never_stored_stays_refused() {
    let message = c0001("class C:\n    x: int\n\n    def __init__(self) -> None:\n        pass\n");
    assert!(message.contains("is never assigned"), "{message}");
}

// -- 4.4: the layout ---------------------------------------------------------

fn layout_conflict(class: &str, first: &str) -> String {
    format!(
        "class `{class}` cannot be created: its bases have incompatible `__slots__` layouts -- \
         `{first}` declares slots and is not on the same inheritance chain as another slotted \
         or exception base, so CPython raises `TypeError: multiple bases have instance lay-out \
         conflict`"
    )
}

#[test]
fn two_independent_slotted_bases_conflict() {
    let shared = "class A:\n    __slots__ = ('x',)\n\n    def __init__(self) -> None:\n        \
                  self.x = 1\n\n\nclass B:\n    __slots__ = ('x',)\n\n    def __init__(self) -> \
                  None:\n        self.x = 1\n\n\nclass D(A, B):\n    pass\n";
    assert_eq!(c0001(shared), layout_conflict("D", "A"));
    let stateless = "class A:\n    __slots__ = ('x',)\n\n\nclass B:\n    __slots__ = ('y',)\n\n\n\
                     class D(A, B):\n    pass\n";
    assert_eq!(c0001(stateless), layout_conflict("D", "A"));
}

#[test]
fn a_slotted_base_reached_through_an_unslotted_subclass_still_conflicts() {
    let source = "class A:\n    __slots__ = ('x',)\n\n\nclass A2(A):\n    pass\n\n\nclass B:\n    \
                  __slots__ = ('y',)\n\n\nclass D(A2, B):\n    pass\n";
    assert_eq!(c0001(source), layout_conflict("D", "A"));
}

#[test]
fn a_diamond_with_an_empty_slotted_sibling_is_accepted() {
    let source = "class A:\n    __slots__ = ('x',)\n\n\nclass B(A):\n    __slots__ = ('y',)\n\n\n\
                  class C(A):\n    __slots__ = ()\n\n\nclass D(B, C):\n    __slots__ = ()\n";
    assert_eq!(slots_of(source, "D"), Some(Vec::new()));
}

#[test]
fn an_exception_base_beside_a_slotted_class_conflicts() {
    let source = "class A:\n    __slots__ = ('x',)\n\n\nclass D(Exception, A):\n    pass\n";
    assert_eq!(c0001(source), layout_conflict("D", "A"));
}

#[test]
fn a_slotted_exception_beside_a_foreign_exception_base_is_not_supported_yet() {
    for other in ["OSError", "KeyError"] {
        let source = format!(
            "class E(ValueError):\n    __slots__ = ('a',)\n\n\nclass F(E, {other}):\n    pass\n"
        );
        assert_eq!(
            c0001(&source),
            "class `F` combines the slotted exception class `E` with a builtin exception base \
             outside `E`'s own ancestry; this is not supported yet -- builtin exception classes \
             have different instance layouts, and pycc does not model which of them CPython can \
             combine with `__slots__`",
            "{other}"
        );
    }
}

#[test]
fn a_slotted_exception_subclass_on_its_own_chain_is_accepted() {
    let source = "class E(ValueError):\n    __slots__ = ('a',)\n\n\nclass F(E):\n    __slots__ = \
                  ('b',)\n";
    assert_eq!(slots_of(source, "F"), names(&["b"]));
}

// -- Unchanged routes --------------------------------------------------------

#[test]
fn a_value_less_slots_annotation_keeps_the_plain_refusal() {
    let message = c0001("class C:\n    __slots__: tuple\n");
    assert!(
        message.starts_with("this `__slots__` spelling is not supported yet"),
        "{message}"
    );
}

#[test]
fn a_dataclass_slots_binding_stays_refused() {
    let source = "from dataclasses import dataclass\n\n\n@dataclass\nclass C:\n    __slots__ = \
                  ('a',)\n    a: int\n";
    let diagnostic = error(source);
    assert_eq!(diagnostic.code, "C0001");
}

#[test]
fn enum_and_protocol_slots_stay_refused() {
    let enum_message =
        c0001("from enum import Enum\n\n\nclass C(Enum):\n    __slots__ = ()\n    A = 1\n");
    assert!(
        enum_message.starts_with("`__slots__` in an `Enum` body is not supported yet"),
        "{enum_message}"
    );
    let protocol =
        error("from typing import Protocol\n\n\nclass P(Protocol):\n    __slots__ = ()\n");
    assert_eq!(protocol.code, "C0001");
    assert!(
        !protocol.message.contains("__slots__` entry"),
        "{}",
        protocol.message
    );
}

// -- Helpers -----------------------------------------------------------------

#[test]
fn mangle_follows_cpythons_private_name_rule() {
    assert_eq!(mangle("__a", "C"), "_C__a");
    assert_eq!(mangle("__a", "__C"), "_C__a");
    assert_eq!(mangle("__a__", "C"), "__a__");
    assert_eq!(mangle("_a", "C"), "_a");
    assert_eq!(mangle("__a", "_"), "__a");
    assert_eq!(mangle("__a", "__"), "__a");
}

#[test]
fn is_ascii_identifier_matches_str_isidentifier_on_ascii() {
    assert!(is_ascii_identifier("_a1"));
    assert!(is_ascii_identifier("class"));
    assert!(!is_ascii_identifier(""));
    assert!(!is_ascii_identifier("1a"));
    assert!(!is_ascii_identifier("a-b"));
}

// -- Cross-module ------------------------------------------------------------

/// Lowers `importer` with every project import answered by `dependency`,
/// registered as `dep.py`, the way `src/modules.rs` does.
fn lower_with_dependency(dependency: &LoweredModule, importer: &str) -> Result<(), Diagnostic> {
    let parsed = parse(importer);
    let mut resolved = ResolvedImports::default();
    resolved.add_module(
        "dep.py".to_string(),
        &dependency.hir,
        &dependency.class_slots,
    );
    for request in project_import_requests(&parsed) {
        resolved.insert(
            request.span,
            ResolvedImport::Module(ResolvedModule {
                display_path: "dep.py".to_string(),
                hir: &dependency.hir,
                submodule_names: Vec::new(),
            }),
        );
    }
    lower_module(&parsed, &resolved, None)
        .map(drop)
        .map_err(|mut diagnostics| diagnostics.remove(0))
}

const IMPORTED_BASES: &str = "class A:\n    __slots__ = ('x',)\n\n\nclass B:\n    __slots__ = \
                              ('y',)\n";

#[test]
fn an_imported_slotted_base_checks_a_local_subclass_store() {
    let dependency = lower(IMPORTED_BASES).expect("dependency lowers");
    let diagnostic = lower_with_dependency(
        &dependency,
        "from dep import A\n\n\nclass C(A):\n    __slots__ = ()\n\n    def __init__(self) -> \
         None:\n        self.z = 1\n",
    )
    .expect_err("undeclared store");
    assert_eq!(diagnostic.code, "T0044");
    assert_eq!(diagnostic.message, no_slot("C", "z"));
}

#[test]
fn two_imported_slotted_bases_conflict() {
    let dependency = lower(IMPORTED_BASES).expect("dependency lowers");
    let diagnostic = lower_with_dependency(
        &dependency,
        "from dep import A, B\n\n\nclass D(A, B):\n    pass\n",
    )
    .expect_err("layout conflict");
    assert_eq!(diagnostic.message, layout_conflict("D", "A"));
}

#[test]
fn an_imported_slotted_base_admits_a_declared_store() {
    let dependency = lower(IMPORTED_BASES).expect("dependency lowers");
    lower_with_dependency(
        &dependency,
        "from dep import A\n\n\nclass C(A):\n    __slots__ = ()\n\n    def __init__(self) -> \
         None:\n        self.x = 1\n",
    )
    .expect("a store to the imported base's slot lowers");
}

#[test]
fn an_imported_base_class_variable_is_refused() {
    let dependency = lower("class A:\n    x = 1\n").expect("dependency lowers");
    let diagnostic = lower_with_dependency(
        &dependency,
        "from dep import A\n\n\nclass C(A):\n    __slots__ = ('x',)\n",
    )
    .expect_err("inherited class variable");
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(diagnostic.message, inherited("x", "C", "A", "x"));
}

#[test]
fn a_re_exported_slotted_base_carries_its_slots() {
    let defining = lower(IMPORTED_BASES).expect("dependency lowers");
    let reexport_source = parse("from defs import A\n");
    let mut reexport_resolved = ResolvedImports::default();
    reexport_resolved.add_module("defs.py".to_string(), &defining.hir, &defining.class_slots);
    for request in project_import_requests(&reexport_source) {
        reexport_resolved.insert(
            request.span,
            ResolvedImport::Module(ResolvedModule {
                display_path: "defs.py".to_string(),
                hir: &defining.hir,
                submodule_names: Vec::new(),
            }),
        );
    }
    let reexport =
        lower_module(&reexport_source, &reexport_resolved, None).expect("re-export lowers");
    // The re-exporting module authors no class, so its own table is empty:
    // the importer must reach `defs.py`'s rows through the recorded binding.
    assert!(reexport.class_slots.is_empty());

    let importer = parse(
        "from mid import A\n\n\nclass C(A):\n    __slots__ = ()\n\n    def __init__(self) -> \
         None:\n        self.z = 1\n",
    );
    let mut resolved = ResolvedImports::default();
    resolved.add_module("defs.py".to_string(), &defining.hir, &defining.class_slots);
    resolved.add_module("mid.py".to_string(), &reexport.hir, &reexport.class_slots);
    for request in project_import_requests(&importer) {
        resolved.insert(
            request.span,
            ResolvedImport::Module(ResolvedModule {
                display_path: "mid.py".to_string(),
                hir: &reexport.hir,
                submodule_names: Vec::new(),
            }),
        );
    }
    let diagnostic = lower_module(&importer, &resolved, None)
        .expect_err("undeclared store")
        .remove(0);
    assert_eq!(diagnostic.code, "T0044");
    assert_eq!(diagnostic.message, no_slot("C", "z"));
}

#[test]
fn a_slots_annotation_outside_the_admitted_spellings_is_refused() {
    let refused = "this `__slots__` annotation is not supported yet -- annotate it as `str`, \
                   `tuple`, `list`, `tuple[str, ...]` or `list[str]`, optionally inside \
                   `ClassVar[...]`";
    for ann in [
        "tuple",
        "tuple[str, ...]",
        "Tuple[str, ...]",
        "list",
        "list[str]",
        "List[str]",
        "str",
        "ClassVar[tuple[str, ...]]",
        // Part 1 of #889: a quoted spelling, at the top or nested, is
        // admitted exactly where the unquoted one is.
        "\"tuple[str, ...]\"",
        "list[\"str\"]",
        "tuple[\"str\", ...]",
        "ClassVar[\"list[str]\"]",
        "ClassVar['list[\"str\"]']",
    ] {
        let source = format!(
            "from typing import ClassVar, List, Tuple\n\n\nclass C:\n    __slots__: {ann} = ('a',)\n"
        );
        assert_eq!(slots_of(&source, "C"), names(&["a"]), "{ann}");
    }
    for ann in [
        "int",
        "Missing",
        "tuple[str]",
        "tuple[int, ...]",
        "tuple[str, int]",
        "list[int]",
        "ClassVar[int]",
        "Sequence[str]",
        "list[\"int\"]",
        "ClassVar[\"int\"]",
    ] {
        let source = format!("class C:\n    __slots__: {ann} = ('a',)\n");
        assert_eq!(c0001(&source), refused, "{ann}");
    }
    // CPython's class-creation errors come first: a layout conflict and an
    // undeclared store are reported over the annotation.
    let layout = "class A:\n    __slots__ = ('a',)\n\n\nclass B:\n    __slots__ = ('b',)\n\n\nclass \
                  C(A, B):\n    __slots__: int = ()\n";
    assert_ne!(c0001(layout), refused);
    let store = "class C:\n    __slots__: int = ()\n\n    def __init__(self) -> None:\n        \
                 self.x = 1\n";
    assert_eq!(t0044(store).message, no_slot("C", "x"));
}

#[test]
fn slots_over_unrelated_builtin_exception_bases_are_refused() {
    let refused = |class: &str| {
        format!(
            "class `{class}` uses `__slots__` over the unrelated builtin exception bases \
             `OSError` and `ImportError`; this is not supported yet -- builtin exception classes \
             have different instance layouts, and pycc does not model which of them CPython can \
             combine"
        )
    };
    // CPython: `TypeError: multiple bases have instance lay-out conflict`.
    let own = "class C(OSError, ImportError):\n    __slots__ = ()\n";
    assert_eq!(c0001(own), refused("C"));
    // A slotted user base brings `__slots__` in without an own binding.
    let inherited =
        "class E(OSError):\n    __slots__ = ()\n\n\nclass C(E, ImportError):\n    pass\n";
    assert_eq!(c0001(inherited), refused("C"));
    // Related builtins (one in the other's MRO) share a layout chain.
    lower("class C(OSError, Exception):\n    __slots__ = ()\n").expect("related builtins");
}

#[test]
fn a_slots_annotation_through_a_shadowed_name_is_refused() {
    let refused = "this `__slots__` annotation is not supported yet -- annotate it as `str`, \
                   `tuple`, `list`, `tuple[str, ...]` or `list[str]`, optionally inside \
                   `ClassVar[...]`";
    // A user class, a type alias or a class type parameter wins over the
    // builtin spelling, exactly as annotation resolution lets it.
    for source in [
        "class tuple:\n    pass\n\n\nclass C:\n    __slots__: tuple[str, ...] = ('a',)\n",
        "type List = int\n\n\nclass C:\n    __slots__: List[str] = ['a']\n",
        "class C[list]:\n    __slots__: list = ['a']\n",
        "class list:\n    __slots__: list[str] = ['a']\n",
    ] {
        assert_eq!(c0001(source), refused, "{source}");
    }
}
