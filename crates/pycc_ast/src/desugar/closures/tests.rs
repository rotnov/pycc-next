use super::*;

fn sites(source: &str) -> HashMap<String, usize> {
    let module = ruff_python_parser::parse_module(source)
        .unwrap()
        .into_syntax();
    let body: Vec<Stmt> = module.body.into_iter().collect();
    binding_sites(&body)
}

/// Every binding form rebinds `k` after its one assignment, so a function
/// value capturing `k` cannot bind it early (`values::stable_captures`).
#[test]
fn every_binding_form_is_a_binding_site() {
    for form in [
        "k = 2",
        "k += 1",
        "k: int = 2",
        "for k in xs:\n    pass",
        "with m as k:\n    pass",
        "del k",
        "print(k := 2)",
        "print([(k := x) for x in xs])",
        "try:\n    pass\nexcept E as k:\n    pass",
        "match v:\n    case k:\n        pass",
        "match v:\n    case [x, *k]:\n        pass",
        "match v:\n    case {**k}:\n        pass",
        "match v:\n    case P(x=k):\n        pass",
        "match v:\n    case 1 as k:\n        pass",
        "match v:\n    case [k] | (k, _):\n        pass",
        "type k = int",
        "import k",
        "import os as k",
        "from m import k",
        "global k",
        "nonlocal k",
        "def k() -> None:\n    pass",
        "class k:\n    pass",
    ] {
        let source = format!("k = 1\n{form}\n");
        let count = sites(&source).get("k").copied().unwrap_or(0);
        assert!(count >= 2, "{form}: {count}");
    }
}

/// A comprehension's own target and a nested function's assignment bind
/// in their own scopes; a lone assignment is one site.
#[test]
fn inner_scopes_do_not_bind_in_the_function() {
    let found =
        sites("k = 1\nprint([k for k in xs])\ndef f() -> None:\n    k = 2\nh = lambda k: k\n");
    assert_eq!(found.get("k"), Some(&1));
    assert!(sites("from m import *\n").contains_key("*"));
}
