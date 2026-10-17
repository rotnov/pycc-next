use super::*;

/// A compact rendering of the expressions these tests look at.
fn show(expr: &Expr) -> String {
    match expr {
        Expr::Name(name) => name.id.to_string(),
        Expr::NumberLiteral(n) => match &n.value {
            Number::Int(i) => i.to_string(),
            _ => "?".to_string(),
        },
        Expr::UnaryOp(u) if u.op == UnaryOp::USub => format!("-{}", show(&u.operand)),
        Expr::Tuple(t) => format!(
            "({})",
            t.elts.iter().map(show).collect::<Vec<_>>().join(", ")
        ),
        Expr::List(l) => format!(
            "[{}]",
            l.elts.iter().map(show).collect::<Vec<_>>().join(", ")
        ),
        Expr::Subscript(s) => format!("{}[{}]", show(&s.value), show(&s.slice)),
        Expr::Call(c) => format!(
            "{}({})",
            show(&c.func),
            c.arguments
                .args
                .iter()
                .map(show)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expr::ListComp(c) => format!(
            "[{} for {} in {}]",
            show(&c.elt),
            show(&c.generators[0].target),
            show(&c.generators[0].iter)
        ),
        _ => "?".to_string(),
    }
}

/// Every assignment value and loop iterable of `source` after the pass,
/// nested functions included, as `target = value` / `for x in iter`.
fn rewritten(source: &str) -> Vec<String> {
    let mut module = ruff_python_parser::parse_module(source)
        .unwrap()
        .into_syntax();
    let mut body: Vec<Stmt> = std::mem::take(&mut module.body).into_iter().collect();
    range_values(&mut body);
    struct Collect(Vec<String>);
    impl<'a> Visitor<'a> for Collect {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::Assign(a) => {
                    self.0
                        .push(format!("{} = {}", show(&a.targets[0]), show(&a.value)))
                }
                Stmt::For(f) => {
                    self.0
                        .push(format!("for {} in {}", show(&f.target), show(&f.iter)))
                }
                _ => {}
            }
            visitor::walk_stmt(self, stmt);
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::ListComp(c) = expr {
                self.0.push(show(expr));
                for g in &c.generators {
                    self.visit_expr(&g.iter);
                }
                return;
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut collect = Collect(Vec::new());
    for stmt in &body {
        collect.visit_stmt(stmt);
    }
    collect.0
}

#[test]
fn a_range_list_becomes_triples_and_its_elements_are_iterated() {
    let out = rewritten(
        "def f(a, k):\n    r = list(map(range, a))\n    r[k] = range(1, 3)\n    for i in r[0]:\n        print(i, len(r))\n",
    );
    assert!(
        out.contains(&"r = [(0, _pycc_range_0, 1) for _pycc_range_0 in a]".to_string()),
        "{out:?}"
    );
    assert!(out.contains(&"r[k] = (1, 3, 1)".to_string()), "{out:?}");
    assert!(
        out.contains(&"for i in range(r[0][0], r[0][1], r[0][2])".to_string()),
        "{out:?}"
    );
}

#[test]
fn a_range_name_is_iterated_through_its_triple() {
    let out = rewritten("def f(n):\n    d = range(n, 0, -2)\n    return [v for v in d]\n");
    assert!(out.contains(&"d = (n, 0, -2)".to_string()), "{out:?}");
    assert!(
        out.contains(&"[v for v in range(d[0], d[1], d[2])]".to_string()),
        "{out:?}"
    );
}

#[test]
fn any_other_use_leaves_the_name_alone() {
    for source in [
        // Printed.
        "def f(n):\n    d = range(n)\n    print(d)\n",
        // Read in a nested function.
        "def f(n):\n    d = range(n)\n    def g():\n        return [v for v in d]\n    return g\n",
        // A zero (or unknown) step raises when the range is made.
        "def f(n):\n    d = range(0, n, 0)\n    for v in d:\n        pass\n",
        "def f(n, s):\n    d = range(0, n, s)\n    for v in d:\n        pass\n",
        // A parameter is not ours.
        "def f(d):\n    for v in d:\n        pass\n",
        // An element read other than by iteration.
        "def f(n):\n    d = [range(n)]\n    x = d[0]\n",
        // Rebound to something else.
        "def f(n):\n    d = range(n)\n    d = 3\n    for v in d:\n        pass\n",
        // `range` is not the builtin.
        "range = list\ndef f(n):\n    d = range(n)\n    for v in d:\n        pass\n",
    ] {
        let out = rewritten(source);
        assert!(
            out.iter().all(|line| !line.contains("range(d")
                && !line.starts_with("d = (")
                && !line.contains("[(")),
            "{source}\n=> {out:?}"
        );
    }
}
