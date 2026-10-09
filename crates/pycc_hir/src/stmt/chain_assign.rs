//! Chained assignment (`t1 = t2 = ... = tn = e`, #1213, Part 5 of #1018).
//!
//! `docs/TYPE_SYSTEM.md`'s "Chained assignment" section is the canonical
//! statement of the rule. In short: CPython evaluates `e` once, then assigns
//! it to each target left to right, evaluating each target's own
//! sub-expressions (an attribute base, a subscript base and key) as that
//! target is assigned. The chain is rewritten at the AST level into single
//! target `Stmt::Assign`s, each lowered through `lower_stmt`'s ordinary arm,
//! so every check that arm runs applies to each piece unchanged:
//!
//! - when `e` is [`is_unobservable`] (a bare name or a parameter-default
//!   literal), each target gets its own copy of `e` and no temporary exists;
//! - otherwise `e` is bound once to a synthesized temporary named by
//!   [`synthesize_chain_temp_name`], and each target is assigned from it.
//!
//! An empty `[]`/`{}` display is refused before either path.

use super::ExceptStarCtx;
use super::unpack::{lower_unpack, unpack_target_elements};
use crate::class::ClassAnnotationInfo;
use crate::expr::keyword_bind::SignatureTable;
use crate::expr::unobservable::is_unobservable;
use crate::{HirStmt, ImportBinding, Ty, unsupported};
use pycc_ast::{Expr, ExprContext, ExprName, Stmt, StmtAssign};
use pycc_diag::Diagnostic;

/// The prefix of every chained-assignment temporary. It begins with a digit,
/// so no Python source identifier can equal a name carrying it (the D-117
/// argument `expr.rs`'s `synthesize_comp_var_name` makes).
const CHAIN_TEMP_PREFIX: &str = "0chain_";

/// The temporary a chain whose statement starts at byte `offset` binds its
/// value to. The offset makes it unique within one module.
fn synthesize_chain_temp_name(offset: u32) -> String {
    format!("{CHAIN_TEMP_PREFIX}{offset}")
}

/// Lowers `stmt` into the statements it means: one for every statement
/// except a chained assignment (`a = b = e`, #1213), which
/// [`desugar_chain_assign`] expands into one single-target
/// assignment per piece, each lowered through [`lower_stmt`](super::lower_stmt), and a `del`
/// statement (#1244), which [`lower_delete`](super::del::lower_delete) expands into one
/// `HirStmt::Delete` per deleted name, one `HirStmt::DeleteSlice` per slice target
/// (Part 2c of #1371) and one `HirStmt::DeleteAttr` per attribute target (#1457). Every caller that lowers a statement list goes through
/// here; [`lower_stmt`](super::lower_stmt) itself never sees a multi-target `Stmt::Assign` or a
/// `Stmt::Delete`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_stmt_expanded(
    stmt: &Stmt,
    aliases: &[(String, Ty)],
    in_loop: bool,
    in_function: bool,
    in_finally: bool,
    except_star: ExceptStarCtx,
    class_name: Option<&str>,
    type_param: Option<&str>,
    class_defs: &[ClassAnnotationInfo],
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<Vec<HirStmt>, Diagnostic> {
    let lower = |piece: &Stmt| {
        super::lower_stmt(
            piece,
            aliases,
            in_loop,
            in_function,
            in_finally,
            except_star,
            class_name,
            type_param,
            class_defs,
            imports,
            signatures,
        )
    };
    // Part 1 of #891: a tuple-unpacking piece, alone or one piece of a
    // chain, lowers to several statements of its own.
    let expand = |piece: &Stmt| match piece {
        Stmt::Assign(assign) => match unpack_target_elements(&assign.targets[0]) {
            Some(elements) => lower_unpack(
                assign,
                elements,
                in_function,
                class_name,
                imports,
                signatures,
            ),
            None => Ok(vec![lower(piece)?]),
        },
        _ => Ok(vec![lower(piece)?]),
    };
    match stmt {
        Stmt::Assign(assign) if assign.targets.len() > 1 => {
            let mut lowered = Vec::new();
            for piece in desugar_chain_assign(assign)? {
                lowered.extend(expand(&piece)?);
            }
            Ok(lowered)
        }
        // #1244: `del a, b` deletes each name in turn, and `del ()` none.
        Stmt::Delete(del) => super::del::lower_delete(del, &|expr| {
            crate::expr::lower_expr(expr, in_function, class_name, imports, signatures)
        }),
        _ => expand(stmt),
    }
}

/// Rewrites the multi-target `assign` into the single-target statements it
/// means, in execution order, or refuses it with a `C0001`.
pub(super) fn desugar_chain_assign(assign: &StmtAssign) -> Result<Vec<Stmt>, Diagnostic> {
    let is_empty_display = match assign.value.as_ref() {
        Expr::List(list) => list.elts.is_empty(),
        Expr::Dict(dict) => dict.items.is_empty(),
        _ => false,
    };
    if is_empty_display {
        return Err(unsupported(
            "chained assignment of an empty `[]`/`{}` literal is not supported yet; inside a \
             function, annotate one name and assign it (`a: list[int] = []`, then `b = a`)",
            assign.range,
        ));
    }
    let single = |target: &Expr, value: Expr| {
        Stmt::Assign(StmtAssign {
            node_index: Default::default(),
            range: assign.range,
            targets: vec![target.clone()],
            value: Box::new(value),
        })
    };
    if is_unobservable(&assign.value) {
        return Ok(assign
            .targets
            .iter()
            .map(|target| single(target, assign.value.as_ref().clone()))
            .collect());
    }
    let temp = synthesize_chain_temp_name(u32::from(assign.range.start()));
    let temp_name = |ctx| {
        Expr::Name(ExprName {
            node_index: Default::default(),
            range: assign.range,
            id: temp.as_str().into(),
            ctx,
        })
    };
    let mut pieces = Vec::with_capacity(assign.targets.len() + 1);
    pieces.push(single(
        &temp_name(ExprContext::Store),
        assign.value.as_ref().clone(),
    ));
    pieces.extend(
        assign
            .targets
            .iter()
            .map(|target| single(target, temp_name(ExprContext::Load))),
    );
    Ok(pieces)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pycc_diag::Span;

    /// Parses `source` (one assignment statement) and returns its node.
    fn assign(source: &str) -> StmtAssign {
        let module = pycc_parser::parse(source).expect("test fixture must parse");
        module.body[0]
            .as_assign_stmt()
            .expect("an assignment")
            .clone()
    }

    /// Renders each piece as `target = value`: a name renders as itself,
    /// anything else by its kind name.
    fn shape(pieces: &[Stmt]) -> Vec<String> {
        fn render(expr: &Expr) -> String {
            match expr {
                Expr::Name(name) => name.id.to_string(),
                other => pycc_ast::expr_kind_name(other).to_string(),
            }
        }
        pieces
            .iter()
            .map(|piece| {
                let piece = piece
                    .as_assign_stmt()
                    .expect("every piece is an assignment");
                assert_eq!(piece.targets.len(), 1, "every piece has one target");
                format!("{} = {}", render(&piece.targets[0]), render(&piece.value))
            })
            .collect()
    }

    #[test]
    fn an_unobservable_value_is_copied_to_every_target_with_no_temporary() {
        let pieces = desugar_chain_assign(&assign("a = b = c = x\n")).unwrap();
        assert_eq!(shape(&pieces), ["a = x", "b = x", "c = x"]);
        let pieces = desugar_chain_assign(&assign("a = b = 0\n")).unwrap();
        assert_eq!(pieces.len(), 2);
        assert!(
            shape(&pieces)
                .iter()
                .all(|piece| !piece.contains(CHAIN_TEMP_PREFIX)),
            "{pieces:?}"
        );
    }

    #[test]
    fn any_other_value_is_bound_once_to_a_temporary_first() {
        let source = "\n\na = o.b = f()\n";
        let pieces = desugar_chain_assign(&assign(source)).unwrap();
        let shapes = shape(&pieces);
        assert_eq!(shapes.len(), 3);
        assert!(shapes[0].starts_with("0chain_2 = "), "{shapes:?}");
        assert_eq!(shapes[1], "a = 0chain_2");
        assert!(shapes[2].ends_with(" = 0chain_2"), "{shapes:?}");
        let first = pieces[0].as_assign_stmt().expect("an assignment");
        assert!(
            matches!(&first.targets[0], Expr::Name(temp) if temp.ctx == ExprContext::Store),
            "the first piece stores the temporary"
        );
        let second = pieces[1].as_assign_stmt().expect("an assignment");
        assert!(
            matches!(second.value.as_ref(), Expr::Name(read) if read.ctx == ExprContext::Load),
            "a later piece loads the temporary"
        );
    }

    #[test]
    fn an_empty_list_or_dict_display_is_refused_on_the_statement() {
        for source in ["a = b = []", "a = b = {}"] {
            let diagnostic = desugar_chain_assign(&assign(source)).expect_err(source);
            assert_eq!(diagnostic.code, "C0001");
            assert!(
                diagnostic.message.starts_with(
                    "chained assignment of an empty `[]`/`{}` literal is not supported yet"
                ),
                "{}",
                diagnostic.message
            );
            assert_eq!(diagnostic.span, Some(Span::new(0, source.len() as u32)));
        }
    }

    #[test]
    fn a_non_empty_display_goes_through_the_temporary_so_the_targets_alias() {
        for source in ["a = b = [1]", "a = b = {\"k\": 1}"] {
            let pieces = desugar_chain_assign(&assign(source)).unwrap();
            assert_eq!(shape(&pieces)[1..], ["a = 0chain_0", "b = 0chain_0"]);
        }
    }

    #[test]
    fn only_a_synthesized_name_is_recognized_as_one() {
        use crate::module::is_synthesized_name;
        assert!(is_synthesized_name(&synthesize_chain_temp_name(17)));
        assert!(is_synthesized_name("0comp_17_x"));
        assert!(!is_synthesized_name("chain_17"));
        assert!(!is_synthesized_name("_0comp"));
        assert!(!is_synthesized_name(""));
    }
}
