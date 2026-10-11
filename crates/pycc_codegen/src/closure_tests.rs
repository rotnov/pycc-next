//! Calling and building function values (`closure`): the callee object
//! outlives a call whose arguments may rebind its storage, and each
//! capture count of one function gets its own call and drop functions.

use super::*;
use pycc_mir::{BinOpKind, MirExpr, MirItem, MirModule, MirStmt, Ty};

fn callable(params: Vec<Ty>, ret: Ty) -> Ty {
    Ty::Callable(Box::new((params, ret)))
}

fn function(name: &str, params: Vec<(&str, Ty)>, return_ty: Ty, body: Vec<MirStmt>) -> MirItem {
    MirItem::Function {
        name: name.to_string(),
        params: params
            .into_iter()
            .map(|(name, ty)| (name.to_string(), ty))
            .collect(),
        return_ty,
        body,
    }
}

fn name(name: &str, ty: Ty) -> MirExpr {
    MirExpr::Name {
        name: name.to_string(),
        ty,
    }
}

/// `def add(a: int, x: int) -> int: return a + x` and `def side() -> int:
/// return 2`, plus `items`; the LLVM text of the whole module.
fn module_ir(label: &str, items: Vec<MirItem>) -> String {
    let mut all = vec![
        function(
            "add",
            vec![("a", Ty::Int), ("x", Ty::Int)],
            Ty::Int,
            vec![MirStmt::Return(Some(MirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(name("a", Ty::Int)),
                right: Box::new(name("x", Ty::Int)),
                ty: Ty::Int,
            }))],
        ),
        function(
            "side",
            Vec::new(),
            Ty::Int,
            vec![MirStmt::Return(Some(MirExpr::IntLiteral(2)))],
        ),
    ];
    all.extend(items);
    let dir = pycc_scratch::ScratchDir::new(label).expect("failed to create scratch dir");
    let mut ir = String::new();
    let mut observer = |module: &inkwell::module::Module<'_>, _: Option<&'static str>| {
        if ir.is_empty() {
            ir = crate::llvm_string_to_owned(module.print_to_string());
        }
    };
    compile_to_object_with_observer(
        &MirModule {
            items: all,
            ..Default::default()
        },
        &dir.join(format!("{label}.o")),
        &CompileOptions::default(),
        Some(&mut observer),
    )
    .expect("codegen should succeed");
    assert!(!ir.is_empty(), "the observer saw no module");
    ir
}

/// The body of function `symbol` in `ir`.
fn body<'a>(ir: &'a str, symbol: &str) -> &'a str {
    let start = ir
        .find(&format!("@{symbol}("))
        .unwrap_or_else(|| panic!("no {symbol} in {ir}"));
    let rest = &ir[start..];
    &rest[..rest.find("\n}\n").expect("a function body ends")]
}

/// `h(side())`: the call may rebind the storage `h` was read from, so the
/// object is retained before the argument runs and released after the
/// call (and on the argument's exception edge), and the call function is
/// read before the argument runs. `h(1)` runs no code between the read
/// and the call, so it borrows the object.
#[test]
fn a_borrowed_callee_is_retained_across_arguments_that_run_code() {
    let h = callable(vec![Ty::Int], Ty::Int);
    let call = |arg: MirExpr| {
        vec![MirStmt::Return(Some(MirExpr::CallClosure {
            callee: Box::new(name("h", h.clone())),
            args: vec![arg],
            ty: Ty::Int,
        }))]
    };
    let ir = module_ir(
        "closure_call_retains",
        vec![
            function(
                "rebinding",
                vec![("h", h.clone())],
                Ty::Int,
                call(MirExpr::Call {
                    callee: "side".to_string(),
                    args: Vec::new(),
                    ty: Ty::Int,
                }),
            ),
            function(
                "inert",
                vec![("h", h.clone())],
                Ty::Int,
                call(MirExpr::IntLiteral(1)),
            ),
        ],
    );
    let rebinding = body(&ir, "pyfn_rebinding");
    let incref = rebinding
        .find("@pycc_rt_closure_incref")
        .unwrap_or_else(|| panic!("the callee is not retained: {rebinding}"));
    let load = rebinding
        .find("%closure_call = load")
        .unwrap_or_else(|| panic!("no call-function load: {rebinding}"));
    let argument = rebinding
        .find("@pyfn_side(")
        .unwrap_or_else(|| panic!("no argument call: {rebinding}"));
    let indirect = rebinding
        .find("%call_closure = call")
        .unwrap_or_else(|| panic!("no indirect call: {rebinding}"));
    assert!(incref < argument && load < argument, "{rebinding}");
    let inert = body(&ir, "pyfn_inert");
    assert!(!inert.contains("@pycc_rt_closure_incref"), "{inert}");
    // Both release the parameter slot `h` on return; the retaining call
    // also releases its own reference after the call and when the
    // argument raises.
    let releases = |text: &str| text.matches("@pycc_rt_closure_decref").count();
    let after_call = &rebinding[indirect..];
    assert_eq!(
        releases(after_call),
        releases(inert) + 1,
        "the retained callee is not released after the call: {rebinding}"
    );
    assert!(
        releases(rebinding) >= releases(inert) + 2,
        "the retained callee is not released on the exception edge: {rebinding}"
    );
}

/// Two values of `add`, one capturing nothing and one capturing `a`, get
/// separate call and drop functions: the second's call takes one argument
/// and its drop releases one word.
#[test]
fn each_capture_count_gets_its_own_call_and_drop_functions() {
    let make = |captures: Vec<MirExpr>, params: Vec<Ty>| {
        MirStmt::ExprStmt(MirExpr::MakeClosure {
            function: "add".to_string(),
            captures,
            ty: callable(params, Ty::Int),
        })
    };
    let ir = module_ir(
        "closure_thunk_per_count",
        vec![function(
            "two",
            Vec::new(),
            Ty::None,
            vec![
                make(Vec::new(), vec![Ty::Int, Ty::Int]),
                make(vec![MirExpr::IntLiteral(1)], vec![Ty::Int]),
                MirStmt::Return(None),
            ],
        )],
    );
    let none = body(&ir, "pycc_closure_call.add.0");
    let one = body(&ir, "pycc_closure_call.add.1");
    assert_eq!(
        none.lines().next().unwrap().matches("i64 %").count(),
        2,
        "{none}"
    );
    assert_eq!(
        one.lines().next().unwrap().matches("i64 %").count(),
        1,
        "{one}"
    );
    assert!(ir.contains("@pycc_closure_drop.add.0("), "{ir}");
    assert!(ir.contains("@pycc_closure_drop.add.1("), "{ir}");
}
