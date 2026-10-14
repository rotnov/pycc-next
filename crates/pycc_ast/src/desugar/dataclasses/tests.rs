//! Unit tests for the `@dataclass` rewrite: which classes are written out
//! and which methods they get. The generated code's behavior is pinned end
//! to end against CPython by `tests/stdlib_dataclasses.rs`.

use super::*;

fn expand(source: &str) -> Vec<Stmt> {
    let module = ruff_python_parser::parse_module(source).expect("fixture parses");
    let mut body: Vec<Stmt> = module.into_syntax().body.into_iter().collect();
    expand_dataclasses(&mut body);
    body
}

fn class<'a>(body: &'a [Stmt], name: &str) -> &'a StmtClassDef {
    body.iter()
        .find_map(|s| match s {
            Stmt::ClassDef(c) if c.name.as_str() == name => Some(c),
            _ => None,
        })
        .expect("class present")
}

/// The method names of `class`, in order (a property and its setter twice).
fn methods_of(class: &StmtClassDef) -> Vec<String> {
    class
        .body
        .iter()
        .filter_map(|s| match s {
            Stmt::FunctionDef(def) => Some(def.name.to_string()),
            _ => None,
        })
        .collect()
}

/// `__init__`'s parameter names and whether each has a default.
fn init_params(class: &StmtClassDef) -> Vec<(String, bool)> {
    class
        .body
        .iter()
        .find_map(|s| match s {
            Stmt::FunctionDef(def) if def.name.as_str() == "__init__" => Some(def),
            _ => None,
        })
        .expect("__init__")
        .parameters
        .args
        .iter()
        .map(|p| (p.parameter.name.to_string(), p.default.is_some()))
        .collect()
}

#[test]
fn a_plain_dataclass_gets_init_eq_and_repr() {
    let body = expand(
        "from dataclasses import dataclass, field\n\n@dataclass\nclass P:\n    x: int\n    \
         y: int = 2\n    z: list[int] = field(default_factory=list)\n",
    );
    // `field` is no longer imported; `dataclass` stays.
    let Stmt::ImportFrom(import) = &body[0] else {
        panic!("import kept")
    };
    assert_eq!(import.names.len(), 1);
    let p = class(&body, "P");
    assert!(p.decorator_list.is_empty());
    assert_eq!(methods_of(p), ["__init__", "__eq__", "__repr__"]);
    assert_eq!(
        init_params(p),
        [
            ("self".to_string(), false),
            ("x".to_string(), false),
            ("y".to_string(), true),
            ("z".to_string(), true)
        ]
    );
}

#[test]
fn options_add_ordering_hash_and_frozen_properties() {
    let body = expand(
        "@dataclass(order=True, frozen=True)\nclass P:\n    x: int\n",
    );
    assert!(matches!(&body[0], Stmt::ClassDef(c) if c.name.as_str() == FROZEN_ERROR));
    let p = class(&body, "P");
    assert_eq!(
        methods_of(p),
        ["__init__", "x", "x", "__eq__", "__lt__", "__le__", "__gt__", "__ge__", "__hash__", "__repr__"]
    );
}

#[test]
fn user_methods_are_kept_and_not_generated() {
    let body = expand(
        "@dataclass()\nclass P:\n    \"Doc.\"\n    x: int\n    def __repr__(self) -> str:\n        \
         return \"p\"\n    def __post_init__(self) -> None:\n        pass\n",
    );
    let p = class(&body, "P");
    assert_eq!(methods_of(p), ["__repr__", "__post_init__", "__init__", "__eq__"]);
}

#[test]
fn a_subclass_inherits_fields_and_a_plain_subclass_gets_its_own_dunders() {
    let body = expand(
        "@dataclass\nclass A:\n    x: int\n\n@dataclass\nclass B(A):\n    y: str = \"\"\n\n\
         class C(B):\n    pass\n",
    );
    assert_eq!(
        init_params(class(&body, "B")),
        [
            ("self".to_string(), false),
            ("x".to_string(), false),
            ("y".to_string(), true)
        ]
    );
    assert_eq!(methods_of(class(&body, "C")), ["__eq__", "__repr__"]);
}

#[test]
fn shapes_it_cannot_write_out_stay_decorated() {
    for source in [
        "@dataclass(slots=True)\nclass P:\n    x: int\n",
        "@dataclass\nclass P:\n    x: int = 1\n    y: int\n",
        "@dataclass\nclass P:\n    x: InitVar[int]\n",
        "@dataclass\nclass P:\n    x: list[int] = []\n",
        "@dataclass\nclass P:\n    x: int = field(kw_only=True)\n",
        "@dataclass\nclass P:\n    x: int = field(init=False)\n",
        "@dataclass(order=True)\nclass P:\n    x: int\n    def __lt__(self, o: P) -> bool:\n        return True\n",
        "@dataclass\n@other\nclass P:\n    x: int\n",
    ] {
        let body = expand(source);
        let p = class(&body, "P");
        assert!(!p.decorator_list.is_empty(), "{source}");
    }
}

#[test]
fn frozen_instance_error_is_renamed_where_imported() {
    let body = expand(
        "from dataclasses import FrozenInstanceError\n\ntry:\n    pass\nexcept FrozenInstanceError:\n    pass\n",
    );
    assert!(matches!(&body[0], Stmt::ClassDef(c) if c.name.as_str() == FROZEN_ERROR));
    let Stmt::Try(try_stmt) = &body[1] else {
        panic!("try kept")
    };
    let ruff_python_ast::ExceptHandler::ExceptHandler(handler) = &try_stmt.handlers[0];
    assert!(matches!(handler.type_.as_deref(), Some(Expr::Name(n)) if n.id.as_str() == FROZEN_ERROR));
}
