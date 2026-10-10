use super::*;
use crate::pycc_parser_test_helper::parse;

/// Every node `locate_item` places in `source`'s items, as the placed
/// source text, with `Debug` of the HIR node.
fn placed(source: &str) -> Vec<(String, String)> {
    let mut module = parse(source);
    pycc_ast::desugar(&mut module);
    let hir = crate::lower_all(&module).expect("fixture lowers");
    let origins = crate::last_item_origins();
    let spans = item_spans(&module, source, &hir.items, &origins);
    let mut out = Vec::new();
    for (item, item_spans) in hir.items.iter().zip(&spans) {
        let Some(item_spans) = item_spans else {
            continue;
        };
        locate_item(item, item_spans, &mut |node, span| {
            let text = source[span.start as usize..span.end as usize].to_string();
            let node = match node {
                Located::Stmt(stmt) => format!("{stmt:?}"),
                Located::Expr(expr) => format!("{expr:?}"),
            };
            out.push((text, node));
        });
    }
    out
}

fn text_of(placed: &[(String, String)], node: &str) -> Vec<String> {
    placed
        .iter()
        .filter(|(_, debug)| debug == node)
        .map(|(text, _)| text.clone())
        .collect()
}

#[test]
fn reads_of_one_name_are_placed_in_order() {
    let placed = placed("def f(x: int) -> int:\n    y = x + 1\n    return y * x\n");
    let reads: Vec<String> = text_of(&placed, "Name(\"x\")");
    assert_eq!(reads, ["x", "x"]);
    assert_eq!(text_of(&placed, "Name(\"y\")"), ["y"]);
    assert!(placed.iter().any(|(text, _)| text == "return y * x"));
    assert!(placed.iter().any(|(text, _)| text == "y * x"));
}

#[test]
fn a_compound_statement_is_placed_at_its_header_line() {
    let placed = placed(
        "def f(c: bool) -> int:\n    if c:\n        return 1\n    elif not c:\n        return 2\n    return 0\n",
    );
    let ifs: Vec<&String> = placed
        .iter()
        .filter(|(_, debug)| debug.starts_with("If"))
        .map(|(text, _)| text)
        .collect();
    assert_eq!(ifs, ["if c", "elif not c"]);
}

#[test]
fn rewritten_statements_keep_their_source() {
    // `+=` reads its target, `assert` is an `if`, and an unpacking binds a
    // temporary first: each lowered node still finds its source.
    let placed = placed(
        "def f(t: tuple[int, int]) -> int:\n    s = 0\n    s += 2\n    assert s > 1, \"small\"\n    a, b = t\n    return a\n",
    );
    assert!(
        placed
            .iter()
            .any(|(text, debug)| text == "s" && debug == "Name(\"s\")")
    );
    assert!(placed.iter().any(|(text, _)| text == "s > 1"));
    assert!(placed.iter().any(|(text, _)| text == "\"small\""));
    assert!(text_of(&placed, "Name(\"t\")").contains(&"t".to_string()));
}

#[test]
fn an_ambiguous_key_is_left_unplaced() {
    // Two AST reads of `x` but one HIR read: which one it is is unknown.
    let hir = [(Some(NodeKey::new("n:x")), 1)];
    let ast = [
        (Some(NodeKey::new("n:x")), 'a'),
        (Some(NodeKey::new("n:x")), 'b'),
    ];
    assert!(pair(&hir, &ast).is_empty());
    // One AST candidate places every HIR node with its key.
    let hir = [
        (Some(NodeKey::new("n:x")), 1),
        (Some(NodeKey::new("n:x")), 2),
    ];
    assert_eq!(pair(&hir, &ast[..1]), [(1, 'a'), (2, 'a')]);
}

#[test]
fn a_method_is_located_in_its_class() {
    let placed = placed(
        "class C:\n    def m(self, x: int) -> int:\n        return x + 1\n\n    def n(self) -> int:\n        return 2\n",
    );
    assert!(placed.iter().any(|(text, _)| text == "return x + 1"));
    assert!(placed.iter().any(|(text, _)| text == "return 2"));
}
