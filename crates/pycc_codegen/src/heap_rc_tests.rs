//! Run-time checks of `heap_rc`'s ownership rules on hand-built MIR, for
//! shapes the type checker refuses today but MIR admits.

use crate::compile_to_object;
use crate::tests::{link_object_with_runtime, list_fixture_module};
use pycc_mir::{MirContainerReceiver, MirExpr, MirStmt, Ty};
use std::process::Command;

fn list_str() -> Ty {
    Ty::List(Box::new(Ty::Str))
}

fn fresh_str(tag: &str) -> MirExpr {
    MirExpr::BinOp {
        op: pycc_mir::BinOpKind::Add,
        left: Box::new(MirExpr::StringLiteral(tag.to_string())),
        right: Box::new(MirExpr::StringLiteral("!".to_string())),
        ty: Ty::Str,
    }
}

/// Builds `body` as `f()`, runs it (under valgrind's memcheck when it is
/// installed) and returns its stdout.
fn run_memcheck_clean(name: &str, body: Vec<MirStmt>) -> Vec<u8> {
    let mir = list_fixture_module(body);
    let dir = pycc_scratch::ScratchDir::new(name).expect("failed to create scratch dir");
    let obj_path = dir.join(format!("{name}.o"));
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join(name);
    link_object_with_runtime(&obj_path, &bin_path);
    let plain = Command::new(&bin_path).output().expect("binary should run");
    assert!(plain.status.success());
    if let Ok(checked) = Command::new("valgrind")
        .args(["-q", "--error-exitcode=99"])
        .arg(&bin_path)
        .output()
    {
        assert!(
            checked.status.success(),
            "memcheck errors:\n{}",
            String::from_utf8_lossy(&checked.stderr)
        );
    }
    plain.stdout
}

/// `xs.append((xs := [...]) and "d")`-shaped MIR: the walrus inside the
/// argument rebinds the receiver's own local, dropping the list `append`
/// is about to write into. The receiver keeps its own reference.
#[test]
fn a_walrus_rebinding_a_local_receiver_does_not_free_it_under_the_call() {
    let body = vec![
        MirStmt::Assign {
            target: "xs".to_string(),
            value: MirExpr::ListLiteral(vec![fresh_str("a")]),
        },
        MirStmt::ExprStmt(MirExpr::ListAppend {
            list: MirContainerReceiver::Name("xs".to_string()),
            value: Box::new(MirExpr::Sequence {
                discard: Box::new(MirExpr::NamedExpr {
                    name: "xs".to_string(),
                    value: Box::new(MirExpr::ListLiteral(vec![
                        fresh_str("b"),
                        fresh_str("c"),
                    ])),
                    ty: list_str(),
                }),
                value: Box::new(fresh_str("d")),
            }),
        }),
        MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "xs".to_string(),
                ty: list_str(),
            }],
            ty: Ty::None,
        }),
    ];
    let out = run_memcheck_clean("walrus_receiver", body);
    assert_eq!(out, b"['b!', 'c!']\n");
}
