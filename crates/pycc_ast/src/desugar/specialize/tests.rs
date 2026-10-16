use super::*;

fn parse(source: &str) -> ModModule {
    ruff_python_parser::parse_module(source)
        .expect("valid Python")
        .into_syntax()
}

fn plan_of(source: &str) -> (ModModule, Vec<Specialized>) {
    let mut module = parse(source);
    let planned = specialize_untyped(&mut module, &|_| false, &|_| false);
    (module, planned)
}

fn guesses(source: &str) -> Vec<(String, Vec<(String, &'static str)>)> {
    plan_of(source)
        .1
        .into_iter()
        .map(|s| (s.qualname, s.params))
        .collect()
}

fn def_names(body: &[Stmt]) -> Vec<String> {
    body.iter()
        .filter_map(|stmt| match stmt {
            Stmt::FunctionDef(def) => Some(def.name.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn an_indexed_parameter_is_text_and_a_bound_an_int() {
    let source = "def f(s, n, d):\n    t = 0\n    for i in range(n):\n        t += len(s[i:])\n    return t > d\n";
    assert_eq!(
        guesses(source),
        vec![(
            "f".to_string(),
            vec![("s".to_string(), "str"), ("n".to_string(), "int")]
        )]
    );
    let (module, _) = plan_of(source);
    assert_eq!(def_names(&module.body), ["f", "_pycc_native_f"]);
    // The guards read `builtins` through a hidden import, first.
    assert!(matches!(&module.body[0], Stmt::Import(i) if i.names[0].name.as_str() == "builtins"));
    let Stmt::FunctionDef(original) = &module.body[1] else { unreachable!() };
    assert!(matches!(original.body[0], Stmt::If(_)), "the guard comes first");
    let Stmt::FunctionDef(clone) = &module.body[2] else { unreachable!() };
    let annotation = |i: usize| match clone.parameters.args[i].parameter.annotation.as_deref() {
        Some(Expr::Name(n)) => n.id.to_string(),
        other => panic!("{other:?}"),
    };
    assert_eq!((annotation(0), annotation(1), annotation(2)), ("str".into(), "int".into(), "object".into()));
    assert!(clone.returns.is_some());
    // The copy keeps the original's ranges, for tracebacks.
    assert_eq!(clone.range, original.range);
}

#[test]
fn a_method_guard_calls_through_the_receiver_after_the_docstring() {
    let source = "class C:\n    def m(self, text, k=0):\n        \"\"\"Doc.\"\"\"\n        while k < len(text):\n            k += 1\n        return text.find('x')\n";
    let (module, planned) = plan_of(source);
    assert_eq!(planned.len(), 1);
    assert_eq!(planned[0].qualname, "C.m");
    let Stmt::ClassDef(class) = &module.body[1] else { unreachable!() };
    assert_eq!(def_names(&class.body), ["m", "_pycc_native_m"]);
    let Stmt::FunctionDef(m) = &class.body[0] else { unreachable!() };
    assert!(matches!(m.body[0], Stmt::Expr(_)), "the docstring stays first");
    assert!(matches!(m.body[1], Stmt::If(_)));
    let Stmt::FunctionDef(clone) = &class.body[1] else { unreachable!() };
    assert!(clone.parameters.args.iter().all(|a| a.default.is_none()));
    // The literal default's type annotates the copy, as it types the original.
    assert!(matches!(clone.parameters.args[2].parameter.annotation.as_deref(),
        Some(Expr::Name(n)) if n.id.as_str() == "int"));
}

#[test]
fn containers_and_loopless_or_unguessable_functions_are_left_alone() {
    // A list-shaped parameter (item store, container method).
    assert!(guesses("def f(xs):\n    for i in range(len(xs)):\n        xs[i] = 0\n").is_empty());
    assert!(guesses("def f(xs):\n    for x in xs:\n        xs.append(x)\n").is_empty());
    // A mapping read by a string key, an item of an item.
    assert!(guesses("def f(d):\n    for i in range(3):\n        print(d['k'])\n").is_empty());
    assert!(guesses("def f(g):\n    for i in range(3):\n        print(g[i][0])\n").is_empty());
    // No loop: a conversion would not pay for itself.
    assert!(guesses("def f(s):\n    return s[1:]\n").is_empty());
    // Nothing says what `x` is.
    assert!(guesses("def f(x):\n    for i in range(3):\n        print(x)\n").is_empty());
    // A float operand is not an int.
    assert!(guesses("def f(x):\n    while x < 1:\n        x = x * 2.0\n").is_empty());
    // Private, decorated, generator, variadic and keyword-only defs.
    for source in [
        "def _f(s):\n    for c in s:\n        print(s[0])\n",
        "@d\ndef f(s):\n    for c in s:\n        print(s[0])\n",
        "def f(s):\n    for c in s:\n        yield s[0]\n",
        "def f(s, *a):\n    for c in s:\n        print(s[0])\n",
        "def f(s, *, k):\n    for c in s:\n        print(s[0])\n",
        "def f(s, k=g()):\n    for c in s:\n        print(s[0])\n",
    ] {
        assert!(guesses(source).is_empty(), "{source}");
    }
}

#[test]
fn a_module_that_rebinds_a_name_the_guard_spells_is_left_alone() {
    for binding in ["type = 1\n", "def str(): pass\n", "from m import int\n"] {
        let source = format!("{binding}def f(s):\n    for c in s:\n        print(s[0])\n");
        assert!(guesses(&source).is_empty(), "{source}");
    }
}

#[test]
fn a_skipped_specialization_is_not_planned() {
    let mut module = parse("def f(s):\n    for c in s:\n        print(s[0])\n");
    assert!(specialize_untyped(&mut module, &|q| q == "f", &|_| false).is_empty());
    assert_eq!(def_names(&module.body), ["f"]);
}

#[test]
fn text_evidence_wins_over_sequence_counter_evidence_only_when_consistent() {
    // `startswith` is text-only, and a comparison with a string literal too.
    assert_eq!(
        guesses("def f(a, b):\n    while a != 'x':\n        a = a[1:]\n    return b.startswith(a)\n"),
        vec![("f".to_string(), vec![("a".to_string(), "str"), ("b".to_string(), "str")])]
    );
}

#[test]
fn a_parameter_whose_items_are_numbers_is_a_bytearray() {
    let paeth = "def f(fu, scanline, previous, result):\n    ai = -fu\n    for i in range(len(result)):\n        x = scanline[i]\n        if ai < 0:\n            a = c = 0\n        else:\n            a = result[ai]\n            c = previous[ai]\n        b = previous[i]\n        p = a + b - c\n        pa = abs(p - a)\n        result[i] = (x + pa) & 0xff\n        ai += 1\n";
    assert_eq!(
        guesses(paeth),
        vec![(
            "f".to_string(),
            vec![
                ("fu".to_string(), "int"),
                ("scanline".to_string(), "bytearray"),
                ("previous".to_string(), "bytearray"),
                ("result".to_string(), "bytearray"),
            ]
        )]
    );
    // The copy spells the byte buffer annotation; the guard tests the
    // builtin type.
    let (module, _) = plan_of(paeth);
    let Stmt::FunctionDef(clone) = &module.body[2] else { unreachable!() };
    assert!(matches!(clone.parameters.args[1].parameter.annotation.as_deref(),
        Some(Expr::Name(n)) if n.id.as_str() == BYTEARRAY_ANNOTATION));
    // An item stored a number, with no item read, is one too; a list
    // stored a literal, or whose items are only concatenated, is not.
    assert_eq!(
        guesses("def f(s, out):\n    for i in range(len(s)):\n        out[i] = s[i] ^ 1\n"),
        vec![("f".to_string(), vec![("s".to_string(), "bytearray"), ("out".to_string(), "bytearray")])]
    );
    assert_eq!(
        guesses("def f(s, t):\n    r = ''\n    for i in range(len(s)):\n        r = r + s[i] + t[i]\n    return r\n"),
        vec![("f".to_string(), vec![("s".to_string(), "str"), ("t".to_string(), "str")])]
    );
    // A parameter the body binds again is not (its binding would be
    // checked to be a `bytearray`).
    let rebound = guesses("def f(s, k):\n    for i in range(k):\n        x = s[i] ^ 1\n        s = bytes(s)\n    return x\n");
    assert!(!rebound.iter().any(|(_, ps)| ps.iter().any(|(_, t)| *t == "bytearray")), "{rebound:?}");
    // An item local rebound to something else is not an item.
    assert_eq!(
        guesses("def f(s):\n    for i in range(len(s)):\n        x = s[i]\n        x = str(x)\n        print(x - 1)\n"),
        vec![("f".to_string(), vec![("s".to_string(), "str")])]
    );
}
