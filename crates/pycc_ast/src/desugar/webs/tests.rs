use super::*;

/// Every name node of `source` after splitting, in source order.
fn names_after(source: &str) -> Vec<String> {
    let mut module = ruff_python_parser::parse_module(source)
        .unwrap()
        .into_syntax();
    let mut body: Vec<Stmt> = std::mem::take(&mut module.body).into_iter().collect();
    split_webs(&mut body);
    struct Collect(Vec<String>);
    impl<'a> Visitor<'a> for Collect {
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Name(name) = expr {
                self.0.push(name.id.to_string());
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
fn a_straight_line_rebinding_splits_and_the_last_binding_keeps_the_name() {
    assert_eq!(
        names_after("n = input()\nn = int(n)\nprint(n)\n"),
        ["input", "n_1", "int", "n_1", "n", "print", "n"]
    );
}

#[test]
fn two_loops_over_one_name_split() {
    assert_eq!(
        names_after("for i in s:\n    print(i)\nfor i in r:\n    print(i)\n"),
        ["s", "i_1", "print", "i_1", "r", "i", "print", "i"]
    );
}

#[test]
fn a_loop_carried_binding_does_not_split() {
    let names = names_after("x = 0\nfor i in r:\n    print(x)\n    x = 's'\n");
    assert!(names.iter().all(|n| n != "x_1"), "{names:?}");
}

#[test]
fn a_global_a_function_reads_never_splits() {
    let names = names_after("n = 1\nn = 'a'\ndef f():\n    return n\n");
    assert!(names.iter().all(|n| n != "n_1"), "{names:?}");
}

#[test]
fn a_finally_after_a_break_joins_its_binding_with_the_one_before() {
    let names = names_after(
        "for i in r:\n    try:\n        v = 1\n        break\n    finally:\n        v = 's'\nprint(v)\n",
    );
    assert!(names.iter().all(|n| n != "v_1"), "{names:?}");
}

#[test]
fn an_exception_edge_joins_a_handler_read_with_the_try_body_binding() {
    let names = names_after("x = 's'\ntry:\n    x = 1\n    f()\nexcept E:\n    print(x)\n");
    assert!(names.iter().all(|n| n != "x_1"), "{names:?}");
}

#[test]
fn an_augmented_assignment_stays_in_its_web() {
    assert_eq!(
        names_after("s = ''\ns += 'a'\ns = 5\nprint(s)\n"),
        ["s_1", "s_1", "s", "print", "s"]
    );
}

/// Issue #6: an unannotated parameter is the first definition of its web;
/// a rebinding no read of the parameter's own value reaches is its own
/// variable.
#[test]
fn an_unannotated_parameter_rebinding_splits() {
    assert_eq!(
        names_after("def f(a):\n    a = str(a)\n    return a\n"),
        ["str", "a", "a_1", "a_1"]
    );
}

#[test]
fn a_conditional_parameter_rebinding_does_not_split() {
    let names = names_after("def f(a):\n    if g():\n        a = str(a)\n    return a\n");
    assert!(names.iter().all(|n| n != "a_1"), "{names:?}");
}

#[test]
fn an_annotated_parameter_is_never_split() {
    assert_eq!(
        names_after("def f(a: int):\n    a = str(a)\n    return a\n"),
        ["int", "str", "a", "a", "a"]
    );
}

/// A guard normalizing a parameter's shape gets join copies, so the
/// rebinding and the value after the `if` have variables of their own.
#[test]
fn an_isinstance_guard_rebinding_a_parameter_gets_join_copies() {
    assert_eq!(
        names_after(
            "def f(b):\n    if not isinstance(b[0], list):\n        b = [[x] for x in b]\n    return b\n"
        ),
        ["isinstance", "b", "list", "b", "x", "x", "b_1", "b_1", "b_2", "b", "b_2", "b_2"]
    );
}

#[test]
fn a_bare_name_guard_or_a_non_builtin_class_gets_no_copies() {
    for source in [
        "def f(b):\n    if not isinstance(b, list):\n        b = [b]\n    return b\n",
        "def f(b):\n    if not isinstance(b[0], C):\n        b = [b]\n    return b\n",
        "def f(b):\n    if not isinstance(b[0], list):\n        b = [b]\n    else:\n        pass\n    return b\n",
        "list = tuple\ndef f(b):\n    if not isinstance(b[0], list):\n        b = [b]\n    return b\n",
    ] {
        let names = names_after(source);
        assert!(names.iter().all(|n| n != "b_1" && n != "b_2"), "{source}: {names:?}");
    }
}

#[test]
fn an_annotated_name_is_never_split() {
    let names = names_after("x: int = 1\nprint(x)\nx = 's'\nprint(x)\n");
    assert!(names.iter().all(|n| n != "x_1"), "{names:?}");
}

#[test]
fn a_maybe_unbound_read_keeps_the_name_for_its_web() {
    assert_eq!(
        names_after("def f(c):\n    if c:\n        w = 1\n    print(w)\n    w = 's'\n    print(w)\n"),
        ["c", "w", "print", "w", "w_1", "print", "w_1"]
    );
}

#[test]
fn a_scope_calling_locals_is_left_alone() {
    let names = names_after("n = 1\nprint(n)\nn = 's'\nprint(locals())\n");
    assert!(names.iter().all(|n| n != "n_1"), "{names:?}");
}

#[test]
fn a_fresh_name_skips_names_already_taken() {
    assert_eq!(
        names_after("n_1 = 0\nn = 1\nprint(n)\nn = 's'\nprint(n, n_1)\n"),
        ["n_1", "n_2", "print", "n_2", "n", "print", "n", "n_1"]
    );
}

#[test]
fn a_comprehension_target_is_its_own_scope() {
    // The comprehension's `x` is not the module's `x`.
    assert_eq!(
        names_after("x = 1\nprint(x)\nys = [x for x in r]\nx = 's'\nprint(x)\n"),
        ["x_1", "print", "x_1", "r", "x", "x", "ys", "x", "print", "x"]
    );
}

#[test]
fn a_lambda_capturing_a_name_keeps_it_whole() {
    let names = names_after("k = 1\nf = lambda: k\nk = 's'\n");
    assert!(names.iter().all(|n| n != "k_1"), "{names:?}");
}
