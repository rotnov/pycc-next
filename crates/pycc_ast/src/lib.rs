/// Ruff's own generic AST visitor, re-exported through this facade so
/// downstream crates can walk every node of a module without hand-rolling
/// (and having to keep exhaustive) a `Stmt`/`Expr` match of their own. The
/// trait's default methods recurse through every child node, so an
/// implementor that overrides a single `visit_*` method still sees the
/// whole tree, and an upstream AST addition is covered automatically.
pub use ruff_python_ast::visitor;

mod string_annotation;
pub use string_annotation::{
    normalize_string_annotations, parse_string_annotation, unquote_nested_string_annotations,
};

pub use ruff_python_ast::{
    Alias, Arguments, BoolOp, CmpOp, Comprehension, ConversionFlag, Decorator, ElifElseClause,
    ExceptHandler, ExceptHandlerExceptHandler, Expr, ExprAttribute, ExprBinOp, ExprBooleanLiteral,
    ExprCall, ExprCompare, ExprContext, ExprDictComp, ExprFString, ExprListComp, ExprName,
    ExprNamed, ExprNumberLiteral, ExprSetComp, ExprSlice, ExprStringLiteral, ExprSubscript,
    ExprTuple, ExprUnaryOp, Identifier, Int, InterpolatedElement, InterpolatedStringElement,
    InterpolatedStringLiteralElement, MatchCase, ModModule, Number, Operator, ParameterWithDefault,
    Parameters, Pattern, PatternArguments, PatternKeyword, PatternMatchAs, PatternMatchClass,
    PatternMatchMapping, PatternMatchOr, PatternMatchSequence, PatternMatchSingleton,
    PatternMatchStar, PatternMatchValue, Singleton, Stmt, StmtAnnAssign, StmtAssert, StmtAssign,
    StmtAugAssign, StmtClassDef, StmtDelete, StmtExpr, StmtFor, StmtFunctionDef, StmtIf,
    StmtImportFrom, StmtMatch, StmtPass, StmtRaise, StmtReturn, StmtTry, StmtTypeAlias, StmtWhile,
    StringLiteral, StringLiteralFlags, StringLiteralValue, TypeParam, TypeParams, UnaryOp,
};

/// Returns the byte range of a statement without exposing the upstream
/// text-size crate through pycc's AST facade.
pub fn stmt_range(stmt: &Stmt) -> std::ops::Range<u32> {
    let range = match stmt {
        Stmt::FunctionDef(ruff_python_ast::StmtFunctionDef { range, .. })
        | Stmt::ClassDef(ruff_python_ast::StmtClassDef { range, .. })
        | Stmt::Return(ruff_python_ast::StmtReturn { range, .. })
        | Stmt::Delete(ruff_python_ast::StmtDelete { range, .. })
        | Stmt::TypeAlias(ruff_python_ast::StmtTypeAlias { range, .. })
        | Stmt::Assign(ruff_python_ast::StmtAssign { range, .. })
        | Stmt::AugAssign(ruff_python_ast::StmtAugAssign { range, .. })
        | Stmt::AnnAssign(ruff_python_ast::StmtAnnAssign { range, .. })
        | Stmt::For(ruff_python_ast::StmtFor { range, .. })
        | Stmt::While(ruff_python_ast::StmtWhile { range, .. })
        | Stmt::If(ruff_python_ast::StmtIf { range, .. })
        | Stmt::With(ruff_python_ast::StmtWith { range, .. })
        | Stmt::Match(ruff_python_ast::StmtMatch { range, .. })
        | Stmt::Raise(ruff_python_ast::StmtRaise { range, .. })
        | Stmt::Try(ruff_python_ast::StmtTry { range, .. })
        | Stmt::Assert(ruff_python_ast::StmtAssert { range, .. })
        | Stmt::Import(ruff_python_ast::StmtImport { range, .. })
        | Stmt::ImportFrom(ruff_python_ast::StmtImportFrom { range, .. })
        | Stmt::Global(ruff_python_ast::StmtGlobal { range, .. })
        | Stmt::Nonlocal(ruff_python_ast::StmtNonlocal { range, .. })
        | Stmt::Expr(ruff_python_ast::StmtExpr { range, .. })
        | Stmt::Pass(ruff_python_ast::StmtPass { range, .. })
        | Stmt::Break(ruff_python_ast::StmtBreak { range, .. })
        | Stmt::Continue(ruff_python_ast::StmtContinue { range, .. })
        | Stmt::IpyEscapeCommand(ruff_python_ast::StmtIpyEscapeCommand { range, .. }) => range,
    };
    (*range).into()
}

/// Returns the byte range of an expression without exposing the upstream
/// text-size crate through pycc's AST facade.
pub fn expr_range(expr: &Expr) -> std::ops::Range<u32> {
    let range = match expr {
        Expr::BoolOp(ruff_python_ast::ExprBoolOp { range, .. })
        | Expr::Named(ruff_python_ast::ExprNamed { range, .. })
        | Expr::BinOp(ruff_python_ast::ExprBinOp { range, .. })
        | Expr::UnaryOp(ruff_python_ast::ExprUnaryOp { range, .. })
        | Expr::Lambda(ruff_python_ast::ExprLambda { range, .. })
        | Expr::If(ruff_python_ast::ExprIf { range, .. })
        | Expr::Dict(ruff_python_ast::ExprDict { range, .. })
        | Expr::Set(ruff_python_ast::ExprSet { range, .. })
        | Expr::ListComp(ruff_python_ast::ExprListComp { range, .. })
        | Expr::SetComp(ruff_python_ast::ExprSetComp { range, .. })
        | Expr::DictComp(ruff_python_ast::ExprDictComp { range, .. })
        | Expr::Generator(ruff_python_ast::ExprGenerator { range, .. })
        | Expr::Await(ruff_python_ast::ExprAwait { range, .. })
        | Expr::Yield(ruff_python_ast::ExprYield { range, .. })
        | Expr::YieldFrom(ruff_python_ast::ExprYieldFrom { range, .. })
        | Expr::Compare(ruff_python_ast::ExprCompare { range, .. })
        | Expr::Call(ruff_python_ast::ExprCall { range, .. })
        | Expr::FString(ruff_python_ast::ExprFString { range, .. })
        | Expr::TString(ruff_python_ast::ExprTString { range, .. })
        | Expr::StringLiteral(ruff_python_ast::ExprStringLiteral { range, .. })
        | Expr::BytesLiteral(ruff_python_ast::ExprBytesLiteral { range, .. })
        | Expr::NumberLiteral(ruff_python_ast::ExprNumberLiteral { range, .. })
        | Expr::BooleanLiteral(ruff_python_ast::ExprBooleanLiteral { range, .. })
        | Expr::NoneLiteral(ruff_python_ast::ExprNoneLiteral { range, .. })
        | Expr::EllipsisLiteral(ruff_python_ast::ExprEllipsisLiteral { range, .. })
        | Expr::Attribute(ruff_python_ast::ExprAttribute { range, .. })
        | Expr::Subscript(ruff_python_ast::ExprSubscript { range, .. })
        | Expr::Starred(ruff_python_ast::ExprStarred { range, .. })
        | Expr::Name(ruff_python_ast::ExprName { range, .. })
        | Expr::List(ruff_python_ast::ExprList { range, .. })
        | Expr::Tuple(ruff_python_ast::ExprTuple { range, .. })
        | Expr::Slice(ruff_python_ast::ExprSlice { range, .. })
        | Expr::IpyEscapeCommand(ruff_python_ast::ExprIpyEscapeCommand { range, .. }) => range,
    };
    (*range).into()
}

/// Names a statement's syntactic kind in Python terms, with its own
/// article, for a user-facing diagnostic (issue #890) -- ``a `with`
/// statement``, `an assignment statement`, never the Rust `Debug` form of
/// the node. The match is exhaustive on purpose: a `ruff_python_ast`
/// upgrade that adds a variant fails to compile here rather than silently
/// printing something generic. Every phrase is distinct within the table,
/// and none begins with ``type annotation ` `` or ``class ` ``, the two
/// prefixes `pycc_hir`'s D-219 cascade detection classifies a `C0001`
/// message by.
pub fn stmt_kind_name(stmt: &Stmt) -> &'static str {
    match stmt {
        Stmt::FunctionDef(_) => "a function definition (`def ...`)",
        Stmt::ClassDef(_) => "a class definition (`class ...`)",
        Stmt::Return(_) => "a `return` statement",
        Stmt::Delete(_) => "a `del` statement",
        Stmt::TypeAlias(_) => "a `type` alias statement",
        Stmt::Assign(_) => "an assignment statement",
        Stmt::AugAssign(_) => "an augmented assignment (`x += 1`)",
        Stmt::AnnAssign(_) => "an annotated assignment (`x: int = 1`)",
        Stmt::For(_) => "a `for` loop",
        Stmt::While(_) => "a `while` loop",
        Stmt::If(_) => "an `if` statement",
        Stmt::With(_) => "a `with` statement",
        Stmt::Match(_) => "a `match` statement",
        Stmt::Raise(_) => "a `raise` statement",
        Stmt::Try(_) => "a `try` statement",
        Stmt::Assert(_) => "an `assert` statement",
        Stmt::Import(_) => "an `import` statement",
        Stmt::ImportFrom(_) => "a `from ... import ...` statement",
        Stmt::Global(_) => "a `global` declaration",
        Stmt::Nonlocal(_) => "a `nonlocal` declaration",
        Stmt::Expr(_) => "an expression statement",
        Stmt::Pass(_) => "a `pass` statement",
        Stmt::Break(_) => "a `break` statement",
        Stmt::Continue(_) => "a `continue` statement",
        Stmt::IpyEscapeCommand(_) => "an IPython escape command (`%magic` / `!shell`)",
    }
}

/// Names an expression's syntactic kind in Python terms, with its own
/// article, for a user-facing diagnostic (issue #890) -- `a tuple`, ``an
/// attribute expression (`obj.attr`)``, never the Rust `Debug` form of the
/// node. Exhaustive for the same reason as `stmt_kind_name`, and under the
/// same two constraints on its phrases.
pub fn expr_kind_name(expr: &Expr) -> &'static str {
    match expr {
        Expr::BoolOp(_) => "an `and`/`or` boolean expression",
        Expr::Named(_) => "an assignment expression (`x := ...`)",
        Expr::BinOp(_) => "a binary operator expression",
        Expr::UnaryOp(_) => "a unary operator expression",
        Expr::Lambda(_) => "a `lambda`",
        Expr::If(_) => "a conditional expression (`x if c else y`)",
        Expr::Dict(_) => "a dict display (`{...}`)",
        Expr::Set(_) => "a set display (`{...}`)",
        Expr::ListComp(_) => "a list comprehension",
        Expr::SetComp(_) => "a set comprehension",
        Expr::DictComp(_) => "a dict comprehension",
        Expr::Generator(_) => "a generator expression",
        Expr::Await(_) => "an `await` expression",
        Expr::Yield(_) => "a `yield` expression",
        Expr::YieldFrom(_) => "a `yield from` expression",
        Expr::Compare(_) => "a comparison expression",
        Expr::Call(_) => "a call expression",
        Expr::FString(_) => "an f-string",
        Expr::TString(_) => "a t-string (template string literal)",
        Expr::StringLiteral(_) => "a string literal",
        Expr::BytesLiteral(_) => "a bytes literal",
        Expr::NumberLiteral(_) => "a number literal",
        Expr::BooleanLiteral(_) => "a `True`/`False` literal",
        Expr::NoneLiteral(_) => "a `None` literal",
        Expr::EllipsisLiteral(_) => "an `...` ellipsis literal",
        Expr::Attribute(_) => "an attribute expression (`obj.attr`)",
        Expr::Subscript(_) => "a subscript expression (`obj[key]`)",
        Expr::Starred(_) => "a starred expression (`*x`)",
        Expr::Name(_) => "a bare name",
        Expr::List(_) => "a list display (`[...]`)",
        Expr::Tuple(_) => "a tuple",
        Expr::Slice(_) => "a slice (`a:b`)",
        Expr::IpyEscapeCommand(_) => "an IPython escape command (`%magic` / `!shell`)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn re_exports_are_the_same_type_as_upstream() {
        fn assert_same_type<T>(_: T) {}
        let m = ModModule {
            node_index: Default::default(),
            range: Default::default(),
            body: Default::default(),
        };
        assert_same_type::<ruff_python_ast::ModModule>(m);
    }

    #[test]
    fn range_facade_preserves_statement_and_expression_byte_offsets() {
        let module = parse_test_source("if True:\n    x = [1]\n");
        assert_eq!(stmt_range(&module.body[0]), 0..20);

        let if_stmt = module.body[0].as_if_stmt().unwrap();
        let assignment = if_stmt.body[0].as_assign_stmt().unwrap();
        assert_eq!(expr_range(&assignment.value), 17..20);
    }

    #[test]
    fn range_facade_covers_every_upstream_statement_and_expression_variant() {
        let statement_sources = [
            "def function():\n    pass\n",
            "class Class:\n    pass\n",
            "return 1\n",
            "del value\n",
            "type Alias = int\n",
            "value = 1\n",
            "value += 1\n",
            "value: int = 1\n",
            "for value in values:\n    pass\n",
            "while value:\n    pass\n",
            "if value:\n    pass\n",
            "with value:\n    pass\n",
            "match value:\n    case _:\n        pass\n",
            "raise value\n",
            "try:\n    pass\nexcept:\n    pass\n",
            "assert value\n",
            "import module\n",
            "from module import value\n",
            "global value\n",
            "nonlocal value\n",
            "value\n",
            "pass\n",
            "break\n",
            "continue\n",
        ];
        for source in statement_sources {
            let module = parse_test_source(source);
            let range = stmt_range(&module.body[0]);
            assert!(range.start <= range.end, "invalid range for {source:?}");
            assert!(
                range.end as usize <= source.len(),
                "range escaped {source:?}"
            );
        }

        let expression_sources = [
            "left and right",
            "(target := value)",
            "left + right",
            "-value",
            "lambda: value",
            "left if test else right",
            "{}",
            "{value}",
            "[value for value in values]",
            "{value for value in values}",
            "{key: value for key in values}",
            "(value for value in values)",
            "await value",
            "left < right",
            "function()",
            "f\"{value}\"",
            "t\"{value}\"",
            "\"value\"",
            "b\"value\"",
            "1",
            "True",
            "None",
            "...",
            "value.attribute",
            "value[0]",
            "value",
            "[value]",
            "(value,)",
        ];
        for source in expression_sources {
            let parsed = ruff_python_parser::parse_expression(source).unwrap();
            let range = expr_range(parsed.expr());
            assert!(range.start <= range.end, "invalid range for {source:?}");
            assert!(
                range.end as usize <= source.len(),
                "range escaped {source:?}"
            );
        }

        for source in [
            "def generator():\n    yield value\n",
            "def generator():\n    yield from value\n",
        ] {
            let module = parse_test_source(source);
            let function = module.body[0].as_function_def_stmt().unwrap();
            let expression = function.body[0].as_expr_stmt().unwrap();
            let range = expr_range(&expression.value);
            assert!(range.start <= range.end, "invalid range for {source:?}");
        }

        let starred = ruff_python_parser::parse_expression("[*value]").unwrap();
        let starred = starred.expr().as_list_expr().unwrap().elts[0]
            .as_starred_expr()
            .unwrap();
        assert_eq!(expr_range(&Expr::Starred(starred.clone())), 1..7);

        let sliced = ruff_python_parser::parse_expression("value[1:2]").unwrap();
        let slice = &sliced.expr().as_subscript_expr().unwrap().slice;
        assert_eq!(expr_range(slice), 6..9);

        let ipython = ruff_python_parser::parse(
            "!pwd\nvalue = !!pwd\n",
            ruff_python_parser::ParseOptions::from(ruff_python_parser::Mode::Ipython),
        )
        .unwrap()
        .try_into_module()
        .unwrap()
        .into_syntax();
        assert_eq!(stmt_range(&ipython.body[0]), 0..4);
        let assignment = ipython.body[1].as_assign_stmt().unwrap();
        assert_eq!(expr_range(&assignment.value), 13..18);
    }

    #[test]
    fn kind_names_cover_every_upstream_variant() {
        // Mirrors `range_facade_covers_every_upstream_statement_and_expression_variant`
        // above source-for-source: that set is exactly the 25 `Stmt` and 33
        // `Expr` variants of the pinned `ruff_python_ast`, so every arm of
        // both kind-name tables is hit and its phrase asserted verbatim
        // (the phrases land byte-exact in `tests/diagnostics` fixtures).
        let statement_cases = [
            (
                "def function():\n    pass\n",
                "a function definition (`def ...`)",
            ),
            (
                "class Class:\n    pass\n",
                "a class definition (`class ...`)",
            ),
            ("return 1\n", "a `return` statement"),
            ("del value\n", "a `del` statement"),
            ("type Alias = int\n", "a `type` alias statement"),
            ("value = 1\n", "an assignment statement"),
            ("value += 1\n", "an augmented assignment (`x += 1`)"),
            ("value: int = 1\n", "an annotated assignment (`x: int = 1`)"),
            ("for value in values:\n    pass\n", "a `for` loop"),
            ("while value:\n    pass\n", "a `while` loop"),
            ("if value:\n    pass\n", "an `if` statement"),
            ("with value:\n    pass\n", "a `with` statement"),
            (
                "match value:\n    case _:\n        pass\n",
                "a `match` statement",
            ),
            ("raise value\n", "a `raise` statement"),
            ("try:\n    pass\nexcept:\n    pass\n", "a `try` statement"),
            ("assert value\n", "an `assert` statement"),
            ("import module\n", "an `import` statement"),
            (
                "from module import value\n",
                "a `from ... import ...` statement",
            ),
            ("global value\n", "a `global` declaration"),
            ("nonlocal value\n", "a `nonlocal` declaration"),
            ("value\n", "an expression statement"),
            ("pass\n", "a `pass` statement"),
            ("break\n", "a `break` statement"),
            ("continue\n", "a `continue` statement"),
        ];
        for (source, expected) in statement_cases {
            let module = parse_test_source(source);
            assert_eq!(stmt_kind_name(&module.body[0]), expected, "{source:?}");
        }

        let expression_cases = [
            ("left and right", "an `and`/`or` boolean expression"),
            ("(target := value)", "an assignment expression (`x := ...`)"),
            ("left + right", "a binary operator expression"),
            ("-value", "a unary operator expression"),
            ("lambda: value", "a `lambda`"),
            (
                "left if test else right",
                "a conditional expression (`x if c else y`)",
            ),
            ("{}", "a dict display (`{...}`)"),
            ("{value}", "a set display (`{...}`)"),
            ("[value for value in values]", "a list comprehension"),
            ("{value for value in values}", "a set comprehension"),
            ("{key: value for key in values}", "a dict comprehension"),
            ("(value for value in values)", "a generator expression"),
            ("await value", "an `await` expression"),
            ("left < right", "a comparison expression"),
            ("function()", "a call expression"),
            ("f\"{value}\"", "an f-string"),
            ("t\"{value}\"", "a t-string (template string literal)"),
            ("\"value\"", "a string literal"),
            ("b\"value\"", "a bytes literal"),
            ("1", "a number literal"),
            ("True", "a `True`/`False` literal"),
            ("None", "a `None` literal"),
            ("...", "an `...` ellipsis literal"),
            ("value.attribute", "an attribute expression (`obj.attr`)"),
            ("value[0]", "a subscript expression (`obj[key]`)"),
            ("value", "a bare name"),
            ("[value]", "a list display (`[...]`)"),
            ("(value,)", "a tuple"),
        ];
        for (source, expected) in expression_cases {
            let parsed = ruff_python_parser::parse_expression(source).unwrap();
            assert_eq!(expr_kind_name(parsed.expr()), expected, "{source:?}");
        }

        for (source, expected) in [
            (
                "def generator():\n    yield value\n",
                "a `yield` expression",
            ),
            (
                "def generator():\n    yield from value\n",
                "a `yield from` expression",
            ),
        ] {
            let module = parse_test_source(source);
            let function = module.body[0].as_function_def_stmt().unwrap();
            let expression = function.body[0].as_expr_stmt().unwrap();
            assert_eq!(expr_kind_name(&expression.value), expected, "{source:?}");
        }

        let starred = ruff_python_parser::parse_expression("[*value]").unwrap();
        let starred = starred.expr().as_list_expr().unwrap().elts[0]
            .as_starred_expr()
            .unwrap();
        assert_eq!(
            expr_kind_name(&Expr::Starred(starred.clone())),
            "a starred expression (`*x`)"
        );

        let sliced = ruff_python_parser::parse_expression("value[1:2]").unwrap();
        let slice = &sliced.expr().as_subscript_expr().unwrap().slice;
        assert_eq!(expr_kind_name(slice), "a slice (`a:b`)");

        let ipython = ruff_python_parser::parse(
            "!pwd\nvalue = !!pwd\n",
            ruff_python_parser::ParseOptions::from(ruff_python_parser::Mode::Ipython),
        )
        .unwrap()
        .try_into_module()
        .unwrap()
        .into_syntax();
        assert_eq!(
            stmt_kind_name(&ipython.body[0]),
            "an IPython escape command (`%magic` / `!shell`)"
        );
        let assignment = ipython.body[1].as_assign_stmt().unwrap();
        assert_eq!(
            expr_kind_name(&assignment.value),
            "an IPython escape command (`%magic` / `!shell`)"
        );
    }

    #[test]
    fn re_exported_grammar_types_resolve_and_have_the_expected_shape() {
        // Every assertion below extracts a value via a re-exported type's
        // `.as_xxx()` accessor (generated by ruff's own `is_macro::Is`
        // derive) plus `.unwrap()`, not a hand-rolled `let PATTERN = ...
        // else { panic!(...) }` -- per TESTING.md's own documented
        // coverage guidance, `.unwrap()`'s panic branch lives in
        // libcore/libstd, outside this crate's instrumented regions, so it
        // never reads as an uncovered region the way a hand-written
        // panic arm would (confirmed: an earlier draft using `let-else`
        // panics left 11 regions uncovered, one per never-taken panic arm).
        // The point of this test is still to fail to *compile* if a
        // re-export is missing or misnamed.
        let module = parse_test_source("x = 1\ny = x + 2\n");
        let first = module.body[0].as_assign_stmt().unwrap();
        assert_eq!(first.targets.len(), 1);
        let second = module.body[1].as_assign_stmt().unwrap();
        let bin_op = second.value.as_bin_op_expr().unwrap();
        assert_eq!(bin_op.op, Operator::Add);

        let module =
            parse_test_source("if y == 3:\n    pass\nelif True:\n    pass\nelse:\n    pass\n");
        let if_stmt = module.body[0].as_if_stmt().unwrap();
        let compare = if_stmt.test.as_compare_expr().unwrap();
        assert_eq!(compare.ops[0], CmpOp::Eq);
        assert_eq!(if_stmt.elif_else_clauses.len(), 2);
        assert!(if_stmt.elif_else_clauses[0].test.is_some());
        assert!(if_stmt.elif_else_clauses[1].test.is_none());
        let elif_test = if_stmt.elif_else_clauses[0].test.as_ref().unwrap();
        let elif_bool = elif_test.as_boolean_literal_expr().unwrap();
        assert!(elif_bool.value);

        let module = parse_test_source("while y < 10:\n    pass\n");
        assert!(module.body[0].as_while_stmt().is_some());

        let module = parse_test_source("for i in range(3):\n    pass\n");
        let for_stmt = module.body[0].as_for_stmt().unwrap();
        assert!(!for_stmt.is_async);
        let target = for_stmt.target.as_name_expr().unwrap();
        assert_eq!(target.ctx, ExprContext::Store);

        let module = parse_test_source("z = -y\n");
        let assign = module.body[0].as_assign_stmt().unwrap();
        let unary = assign.value.as_unary_op_expr().unwrap();
        assert_eq!(unary.op, UnaryOp::USub);

        let module = parse_test_source("s = \"hi\"\n");
        let assign = module.body[0].as_assign_stmt().unwrap();
        let string_lit = assign.value.as_string_literal_expr().unwrap();
        assert_eq!(string_lit.value.to_str(), "hi");

        let module = parse_test_source("f = f\"{y}\"\n");
        let assign = module.body[0].as_assign_stmt().unwrap();
        let fstring = assign.value.as_f_string_expr().unwrap();
        let elements: Vec<_> = fstring.value.elements().collect();
        assert_eq!(elements.len(), 1);
        assert!(elements[0].as_interpolation().is_some());

        assert_eq!(Number::Float(1.5), Number::Float(1.5));
    }

    fn parse_test_source(source: &str) -> ModModule {
        ruff_python_parser::parse_module(source)
            .unwrap()
            .into_syntax()
    }
}
