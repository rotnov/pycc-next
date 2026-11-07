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
    // The guards read `builtins` through a hidden import, first, and its
    // types once, into globals.
    assert!(matches!(&module.body[0], Stmt::Import(i) if i.names[0].name.as_str() == "builtins"));
    assert!(matches!(&module.body[1], Stmt::Assign(a)
        if matches!(&a.targets[0], Expr::Name(n) if n.id.as_str() == "__pycc_specialize_int")));
    let Stmt::FunctionDef(original) = &module.body[4] else { unreachable!() };
    assert!(matches!(original.body[0], Stmt::If(_)), "the guard comes first");
    let Stmt::FunctionDef(clone) = &module.body[5] else { unreachable!() };
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
    let Stmt::ClassDef(class) = &module.body[4] else { unreachable!() };
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
    // Dunder, decorated, generator, variadic and keyword-only defs, and one
    // binding `type` itself.
    for source in [
        "def __f(s):\n    for c in s:\n        print(s[0])\n",
        "def f(s):\n    for c in s:\n        type = s[0]\n",
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
fn a_private_function_is_specialized_and_a_local_type_skips_only_its_own() {
    let source = "def _f(s):\n    for c in s:\n        print(s[0])\n\n\n\
def g(s):\n    length, type = s[0], s[1]\n    for c in s:\n        print(c)\n";
    assert_eq!(guesses(source), vec![("_f".to_string(), vec![("s".to_string(), "str")])]);
}

#[test]
fn the_integer_attribute_a_method_reads_first_is_a_snapshot() {
    let source = concat!(
        "class R:\n",
        "    def m(self, bs):\n",
        "        if self.depth == 8:\n",
        "            return 0\n",
        "        out = bytearray()\n",
        "        for o in bs:\n",
        "            out.append(o >> self.depth)\n",
        "        return out\n",
    );
    assert_eq!(
        guesses(source),
        vec![(
            "R.m".to_string(),
            vec![("bs".to_string(), "bytearray"), ("self.depth".to_string(), "int")]
        )]
    );
    let (module, _) = plan_of(source);
    let Stmt::ClassDef(class) = &module.body[4] else { unreachable!() };
    let Stmt::FunctionDef(clone) = &class.body[1] else { unreachable!() };
    let last = clone.parameters.args.last().expect("the snapshot parameter");
    assert_eq!(last.parameter.name.as_str(), "_pycc_attr_depth");
    // A method call before a read (it may change the attribute), a store,
    // or an attribute read later than first, takes no snapshot.
    for body in [
        "        self.reset()\n        if self.depth == 8:\n            return 0\n",
        "        if self.depth == 8:\n            self.depth = 1\n",
        "        if bs == 8:\n            return self.depth\n",
        "        if self.depth == 8:\n            print(bs)\n        k = self.depth + 1\n",
    ] {
        let source = format!(
            "class R:\n    def m(self, bs):\n{body}        for o in bs:\n            print(o - 1)\n"
        );
        let planned = guesses(&source);
        assert!(
            planned.iter().all(|(_, params)| params.iter().all(|(p, _)| !p.contains('.'))),
            "{source}: {planned:?}"
        );
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
    let Stmt::FunctionDef(clone) = &module.body[5] else { unreachable!() };
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

#[test]
fn a_parameter_every_call_passes_a_bytearray_is_one() {
    // `raw[i]` and `raw[i:j]` alone guess `str`; the one call passes a
    // local bound only to `bytearray(...)` (here in a nested function, and
    // routed through the fallback builtins as the planner may route it).
    let method = "class R:\n    def d(self, raw):\n        out = []\n        for i in range(len(raw)):\n            out.append(raw[i:i + 1])\n        return raw[0], out\n";
    for make in ["bytearray(it)", "__pycc_fallback_builtins.bytearray(it)"] {
        let source = format!(
            "{method}    def read(self, it):\n        def rows():\n            bs = {make}\n            return self.d(bs)\n        return rows()\n"
        );
        assert_eq!(
            guesses(&source),
            vec![("R.d".to_string(), vec![("raw".to_string(), "bytearray")])],
            "{make}"
        );
    }
    let str_guess = || vec![("R.d".to_string(), vec![("raw".to_string(), "str")])];
    // A call passing anything else, or none at all, keeps the body's guess.
    let other = format!("{method}    def read(self, it):\n        bs = bytearray(it)\n        self.d(bs)\n        return self.d(it)\n");
    assert_eq!(guesses(&other), str_guess());
    let keyword = format!("{method}    def read(self, it):\n        return self.d(raw=bytearray(it))\n");
    assert_eq!(guesses(&keyword), str_guess());
    let rebound = format!("{method}    def read(self, it):\n        bs = bytearray(it)\n        bs = it\n        return self.d(bs)\n");
    assert_eq!(guesses(&rebound), str_guess());
    let starred = format!("{method}    def read(self, it):\n        return self.d(*it)\n");
    assert_eq!(guesses(&starred), str_guess());
    assert_eq!(guesses(method), str_guess());
    // A text use outweighs the calls.
    let text = "class R:\n    def d(self, raw):\n        for i in range(len(raw)):\n            raw.upper()\n    def read(self, it):\n        return self.d(bytearray(it))\n";
    assert_eq!(guesses(text), str_guess());
    // A function's calls are its own: a method of the same name is not one.
    let function = "def d(raw):\n    t = 0\n    for i in range(len(raw)):\n        t += len(raw[i:])\n    return t\nclass C:\n    def m(self, it):\n        return self.d(it)\ndef g(it):\n    return d(bytearray(it))\n";
    assert_eq!(guesses(function), vec![("d".to_string(), vec![("raw".to_string(), "bytearray")])]);
}

/// What [`specialize_observed`] does to `source` given `observed` (by
/// function name: its report name and its parameters' observed types).
/// One function's observed types in a test: its name, report name and
/// parameters' types.
type Seen<'a> = (&'a str, &'a str, &'a [(&'a str, &'static str)]);

fn observed_plan(source: &str, observed: &[Seen<'_>]) -> (ModModule, Vec<Specialized>) {
    let mut module = parse(source);
    let lookup = |name: &str| {
        observed.iter().find(|(n, _, _)| *n == name).map(|(_, report, types)| {
            (report.to_string(), types.iter().map(|(p, t)| (p.to_string(), *t)).collect::<Vec<_>>())
        })
    };
    let (planned, _) = specialize_observed(&mut module, &lookup, &|_| false, &|_| false);
    (module, planned)
}

/// A selected function's lift is specialized for the types the profile
/// observed: its dunder-shaped name and its `object` annotations do not
/// keep it out, since the selection names it. An observed type no native
/// body takes, or a parameter not observed, keeps its annotation.
#[test]
fn a_flagged_lift_is_specialized_for_its_observed_types() {
    let source = "\
def __pycc_lift0_count__(self: object, s: object, c: object, d: object) -> object:
    n = 0
    for i in range(len(s)):
        if s[i] == c:
            n += 1
    return n
";
    let (module, planned) = observed_plan(
        source,
        &[("__pycc_lift0_count__", "C.count", &[("s", "str"), ("c", "str"), ("d", "dict")])],
    );
    assert_eq!(planned.len(), 1);
    assert_eq!(planned[0].qualname, "C.count");
    assert_eq!(planned[0].params, [("s".to_string(), "str"), ("c".to_string(), "str")]);
    assert!(def_names(&module.body).contains(&format!("{NATIVE_PREFIX}__pycc_lift0_count__")));
    // A function the selection does not name is never specialized here,
    // whatever its name.
    let (_, none) = observed_plan(source, &[]);
    assert!(none.is_empty());
}

/// As for a guess: a function that does not loop, an observed type no
/// native body takes, or a module that binds a guard's type name is left
/// alone.
#[test]
fn an_observed_function_without_a_loop_or_a_native_type_is_left_alone() {
    let flat = "def f(s: object) -> object:\n    return s[0]\n";
    assert!(observed_plan(flat, &[("f", "f", &[("s", "str")])]).1.is_empty());
    let looped = "def f(s: object) -> object:\n    for c in s:\n        pass\n    return s\n";
    assert!(observed_plan(looped, &[("f", "f", &[("s", "dict")])]).1.is_empty());
    assert_eq!(observed_plan(looped, &[("f", "f", &[("s", "str")])]).1.len(), 1);
    let bound = format!("str = bytes\n{looped}");
    assert!(observed_plan(&bound, &[("f", "f", &[("s", "str")])]).1.is_empty());
}

/// Every way a body may bind a parameter again keeps the parameter out of
/// an observed-type specialization; reading it does not.
#[test]
fn a_parameter_bound_again_anywhere_in_the_body_is_rebound() {
    let rebinding = [
        "n = 1",
        "n += 1",
        "n: int = 1",
        "for n in xs: pass",
        "with f() as n: pass",
        "(n := 1)",
        "del n",
        "[0 for n in xs]",
        "try:\n    pass\nexcept E as n:\n    pass",
        "match xs:\n    case [*n]: pass",
        "match xs:\n    case {**n}: pass",
        "match xs:\n    case n: pass",
        "def g():\n    nonlocal n\n    n = 1",
        "def g():\n    global n",
        "def g(n): pass",
        "lambda n: n",
        "[(n := 1) for _ in xs]",
        "from m import n",
        "async def g():\n    async for n in xs: pass",
        "class C:\n    n = 1",
        "import n",
    ];
    for body in rebinding {
        assert!(crate::namespace::bound_anywhere(&parse(body).body).contains("n"), "{body}");
    }
    let reading = "m = n + 1\nfor i in range(n): print(n, i)\ndef g():\n    return n\nlambda: n\n";
    assert!(!crate::namespace::bound_anywhere(&parse(reading).body).contains("n"));
}

/// The loads of `_pycc_object_` names in `body`.
fn objects_read(body: &[Stmt]) -> Vec<String> {
    struct Names(Vec<String>);
    impl<'a> Visitor<'a> for Names {
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Name(n) = expr
                && n.id.as_str().starts_with("_pycc_object_")
            {
                self.0.push(n.id.to_string());
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut names = Names(Vec::new());
    for stmt in body {
        names.visit_stmt(stmt);
    }
    names.0
}

/// A use of a specialized parameter reads the original object unless it
/// provably keeps the value inside builtin code: arithmetic and
/// comparisons of native operands, items and indexes of native sequences,
/// tests, loops over native sequences, unshadowed `len`/`ord`/`range`, and
/// a native `str`'s methods.
#[test]
fn a_specialized_parameter_reads_its_original_object_unless_provably_consumed() {
    let params = [
        ("x".to_string(), "float"),
        ("i".to_string(), "int"),
        ("s".to_string(), "str"),
        ("b".to_string(), "bytearray"),
    ];
    let none = std::collections::BTreeSet::new();
    let escaping = [
        "return x",
        "y = x",
        "ys.append(x)",
        "f(x)",
        "[x]",
        "(x, 1)",
        "{x: 1}",
        "x is y",
        "y is not x",
        "x in ys",
        "d[x]",
        "ys[i]",
        "x or y",
        "min(x, y)",
        "yield x",
        "lambda: x",
        "return s",
        "f(k=s)",
        "s is t",
        "hash(x)",
        "hash(s)",
        "y = x + z",
        "y = z * i",
        "y = x == z",
        "t += x",
        "len(s, k=1)",
        "y = t.find(s)",
        "y = x.real",
        "y = '{}'.format(x)",
        "y = s.format(i)",
        "y = s.split(sep=s)",
        "y = s if z else s",
        "return +x",
        "y = +i",
        "return s + ''",
        "return s * 1",
        "y = s[:]",
        "y = s[i:]",
        "y = s.strip()",
        "y = s.upper()",
        "y = s.split()",
        "y = 'sep'.join(s)",
        "y = t.replace(s, s)",
        "for c in s[:]: ys.append(s[:])",
        "y = s.ljust(i) == t",
    ];
    for source in escaping {
        let mut body = parse(source).body.to_vec();
        let used = escapes::keep_objects(&mut body, &params, &Default::default(), &none);
        assert!(!used.is_empty(), "{source}");
        assert!(!objects_read(&body).is_empty(), "{source}");
    }
    let consumed = [
        "y = x + 1 < 2.5",
        "y = x + 1",
        "y = x * i - 2.5",
        "y = len(s) + i",
        "return i // 2",
        "y = -x",
        "y = ~i",
        "y = not s",
        "if x: pass",
        "if +x: pass",
        "while x < 1: pass",
        "y = x == i",
        "y = 1.0 if x else 2.0",
        "for c in s: pass",
        "for c in s[i:]: pass",
        "[c for c in s if x]",
        "y = s[i:] == s",
        "y = s[0]",
        "y = s[i] == s[i + 1]",
        "y = 'a' in s",
        "y = z in s",
        "y = len(s)",
        "y = len(s[1:]) + 1",
        "y = ord(s[0])",
        "for k in range(i): pass",
        "y = s.find(t)",
        "y = s.find(s[1:])",
        "y = s.startswith(s[:i])",
        "if s.strip(): pass",
        "assert x, 'msg'",
        "return b",
        "f(b)",
        "y = b[i]",
        "y = s[z.w]",
    ];
    for source in consumed {
        let mut body = parse(source).body.to_vec();
        let used = escapes::keep_objects(&mut body, &params, &Default::default(), &none);
        assert!(used.is_empty(), "{source}");
    }
    // A builtin a module binding, a formal parameter or the body binds
    // is not taken to be one.
    let shadowed = std::collections::BTreeSet::from(["len"]);
    let mut body = parse("y = len(s)").body.to_vec();
    assert_eq!(escapes::keep_objects(&mut body, &params, &Default::default(), &shadowed), ["s"]);

}

/// A specialization that reads a parameter's object takes it as a last
/// parameter, which the guard passes; a parameter bound again is left out,
/// and says why.
#[test]
fn an_observed_specialization_takes_the_objects_it_reads_and_skips_rebound_parameters() {
    let source = "\
def f(x: object, n: object, xs: object) -> object:
    for item in xs:
        n = item
    t = 0.0
    for _ in range(3):
        t += x
    return x
";
    let mut module = parse(source);
    let lookup = |name: &str| {
        (name == "f").then(|| ("f".to_string(), vec![("x".to_string(), "float"), ("n".to_string(), "int")]))
    };
    let (planned, excluded) = specialize_observed(&mut module, &lookup, &|_| false, &|_| false);
    assert_eq!(planned.len(), 1);
    assert_eq!(planned[0].params, [("x".to_string(), "float")]);
    assert_eq!(
        excluded,
        [("f".to_string(), "parameter `n` (observed `int`) may be bound again in the body".to_string())]
    );
    let native = module
        .body
        .iter()
        .find_map(|stmt| match stmt {
            Stmt::FunctionDef(def) if def.name.as_str() == format!("{NATIVE_PREFIX}f") => Some(def),
            _ => None,
        })
        .expect("the native copy");
    let names: Vec<&str> = native.parameters.args.iter().map(|a| a.parameter.name.as_str()).collect();
    assert_eq!(names, ["x", "n", "xs", "_pycc_object_x"]);
    // `t += x` may reach `t`'s own `__iadd__`, `return x` lets it out.
    assert_eq!(objects_read(&native.body), ["_pycc_object_x", "_pycc_object_x"]);
    let Stmt::FunctionDef(original) = &module.body.iter().find(|s| matches!(s, Stmt::FunctionDef(d) if d.name.as_str() == "f")).unwrap() else {
        unreachable!()
    };
    let Stmt::If(guard) = &original.body[0] else { panic!("the guard") };
    let Stmt::Return(ret) = &guard.body[0] else { panic!("the guard's call") };
    let Some(Expr::Call(call)) = ret.value.as_deref() else { panic!("a call") };
    let args: Vec<&str> = call
        .arguments
        .args
        .iter()
        .map(|a| match a {
            Expr::Name(n) => n.id.as_str(),
            _ => "?",
        })
        .collect();
    assert_eq!(args, ["x", "n", "xs", "x"]);
}

/// A builtin's name bound in the body by a binding form no target spells
/// (`case len:`, `except ... as len`) or by a formal parameter is not the
/// builtin: the specialized parameter passed to it reads its object.
#[test]
fn a_builtin_shadowed_by_any_binding_form_is_not_taken_for_the_builtin() {
    let bodies = [
        "    match v:\n        case len:\n            return len(x)\n",
        "    try:\n        raise v\n    except E as len:\n        return len(x)\n",
        "    import len\n    return len(x)\n",
        "    def len(c):\n        return c\n    return len(x)\n",
        "    [(len := v) for _ in range(1)]\n    return len(x)\n",
    ];
    for body in bodies {
        let source = format!("def f(x: object, v: object) -> object:\n    for _ in range(2):\n        pass\n{body}");
        let mut module = parse(&source);
        let lookup =
            |name: &str| (name == "f").then(|| ("f".to_string(), vec![("x".to_string(), "str")]));
        let (planned, _) = specialize_observed(&mut module, &lookup, &|_| false, &|_| false);
        assert_eq!(planned.len(), 1, "{body}");
        let native = module
            .body
            .iter()
            .find_map(|stmt| match stmt {
                Stmt::FunctionDef(def) if def.name.as_str() == format!("{NATIVE_PREFIX}f") => Some(def),
                _ => None,
            })
            .expect("the native copy");
        assert_eq!(objects_read(&native.body), ["_pycc_object_x"], "{body}");
    }
    // A parameter named `len` shadows it as well.
    let source = "def f(x: object, len: object) -> object:\n    for _ in range(2):\n        pass\n    return len(x)\n";
    let mut module = parse(source);
    let lookup = |name: &str| (name == "f").then(|| ("f".to_string(), vec![("x".to_string(), "str")]));
    specialize_observed(&mut module, &lookup, &|_| false, &|_| false);
    let Some(Stmt::FunctionDef(native)) = module.body.iter().find(|s| matches!(s, Stmt::FunctionDef(d) if d.name.as_str().starts_with(NATIVE_PREFIX))) else {
        panic!("the native copy")
    };
    assert_eq!(objects_read(&native.body), ["_pycc_object_x"]);
    // The builtin itself keeps the native value.
    let source = "def f(x: object) -> object:\n    for _ in range(2):\n        pass\n    return len(x)\n";
    let mut module = parse(source);
    specialize_observed(&mut module, &lookup, &|_| false, &|_| false);
    let Some(Stmt::FunctionDef(native)) = module.body.iter().find(|s| matches!(s, Stmt::FunctionDef(d) if d.name.as_str().starts_with(NATIVE_PREFIX))) else {
        panic!("the native copy")
    };
    assert!(objects_read(&native.body).is_empty());
}

/// A guessed parameter the body binds again stays native only when every
/// binding gives it a value of its type in the function's own scope and no
/// use, of it or of a value derived from it that may be it, leaves.
#[test]
fn a_rebound_guess_stays_native_only_if_its_value_never_leaves() {
    let params = |source: &str| -> Vec<(String, &'static str)> {
        guesses(source).into_iter().flat_map(|(_, params)| params).collect()
    };
    let x_only = vec![("x".to_string(), "int")];
    // The rebound value leaves: `text` is not specialized.
    assert_eq!(params("def f(text, x):\n    for i in range(x):\n        pass\n    text = text[x:]\n    return text\n"), x_only);
    // A loop target, or a value of another type, may not fit the slot.
    assert_eq!(params("def f(text, x):\n    for i in range(len(text) + x):\n        pass\n    for text in [x]:\n        pass\n    return 0\n"), x_only);
    assert_eq!(params("def f(text, x):\n    for i in range(len(text) + x):\n        pass\n    text = len(text)\n    return 0\n"), x_only);
    // Every use consumed: both stay native.
    let both = vec![("text".to_string(), "str"), ("x".to_string(), "int")];
    assert_eq!(params("def f(text, x):\n    n = 0\n    while len(text) > x % 7:\n        text = text[1:]\n        n += 1\n    return n\n"), both);
    assert_eq!(params("def f(text, x):\n    for i in range(x):\n        x += 1\n        if x > 9:\n            break\n    return len(text[x:])\n"), both);
    // A method's object parameters come after its attribute snapshots.
    let source = "class C:\n    def __init__(self):\n        self.base = 0\n    def keep(self, s):\n        n = self.base\n        for i in range(len(s)):\n            n += 1\n        return s\n";
    let (module, planned) = plan_of(source);
    assert_eq!(planned.len(), 1);
    let Stmt::ClassDef(class) = &module.body[module.body.len() - 1] else { panic!("the class") };
    let native = class
        .body
        .iter()
        .find_map(|stmt| match stmt {
            Stmt::FunctionDef(def) if def.name.as_str().starts_with(NATIVE_PREFIX) => Some(def),
            _ => None,
        })
        .expect("the native copy");
    let names: Vec<&str> = native.parameters.args.iter().map(|a| a.parameter.name.as_str()).collect();
    assert_eq!(names.last(), Some(&"_pycc_object_s"), "{names:?}");
    assert_eq!(objects_read(&native.body), ["_pycc_object_s"]);
}
