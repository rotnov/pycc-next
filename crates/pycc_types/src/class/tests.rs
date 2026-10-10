use crate::{check, check_and_resolve};
use pycc_hir::{BinOpKind, HirItem, HirModule, HirStmt, Ty};
use pycc_hir::{HirClassDef, HirExpr};

/// Builds a minimal `Point` class module: `__init__(self, x: int, y:
/// int)` sets both attributes from its own parameters; `bump(self) ->
/// None` reads and mutates `self.x`. `extra_items`/`extra_stmts` let
/// each test append its own instantiation/attribute/method-call
/// exercise without duplicating this fixture.
fn point_module(extra_items: Vec<HirItem>) -> HirModule {
    let self_ty = Ty::Instance(Box::new("Point".to_string()));
    let init = HirItem::Function {
        name: "Point.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("x".to_string(), Ty::Int),
            ("y".to_string(), Ty::Int),
        ],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "x".to_string(),
                value: HirExpr::Name("x".to_string()),
            },
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "y".to_string(),
                value: HirExpr::Name("y".to_string()),
            },
            HirStmt::Return(None),
        ],
    };
    let bump = HirItem::Function {
        name: "Point.bump".to_string(),
        params: vec![("self".to_string(), self_ty.clone())],
        return_ty: Ty::None,
        body: vec![
            HirStmt::AttrSet {
                base: HirExpr::Name("self".to_string()),
                attr: "x".to_string(),
                value: HirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(HirExpr::AttrGet {
                        base: Box::new(HirExpr::Name("self".to_string())),
                        attr: "x".to_string(),
                    }),
                    right: Box::new(HirExpr::IntLiteral(1)),
                },
            },
            HirStmt::Return(None),
        ],
    };
    let mut items = vec![init, bump];
    items.extend(extra_items);
    HirModule {
        seeded_builtin_exception_classes: false,
        items,
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![(
            "Point".to_string(),
            HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "Point".to_string(),
                bases: Vec::new(),
                mro: vec!["Point".to_string()],
                attrs: vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int)],
                methods: vec![
                    ("__init__".to_string(), "Point.__init__".to_string()),
                    ("bump".to_string(), "Point.bump".to_string()),
                ],
                type_param: None,
                properties: Vec::new(),
                static_methods: Vec::new(),
                class_methods: Vec::new(),
                is_enum: false,
                implicit_object_init: false,
                method_defaults: Vec::new(),
                enum_members: Vec::new(),
                is_dataclass: false,
                dataclass_fields: Vec::new(),
                is_protocol: false,
                runtime_checkable: false,
                protocol_members: Vec::new(),
                abstract_methods: Vec::new(),
                is_abstract: false,
            },
        )],
    }
}

fn top_level(stmt: HirStmt) -> HirItem {
    HirItem::TopLevelStmt(stmt)
}

// -- #436: @staticmethod / @classmethod type checking -------------------

/// Builds a module with a class `C` that has `__init__`, a static method
/// `create(x: int) -> int`, and a class method `greet(cls, x: int) ->
/// int`. `extra_items`/`extra_stmts` let each test append its own
/// call-site exercise.
fn static_class_module(extra_items: Vec<HirItem>) -> HirModule {
    let self_ty = Ty::Instance(Box::new("C".to_string()));
    let init = HirItem::Function {
        name: "C.__init__".to_string(),
        params: vec![("self".to_string(), self_ty.clone())],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    let static_fn = HirItem::Function {
        name: "C.create.static".to_string(),
        params: vec![("x".to_string(), Ty::Int)],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let class_fn = HirItem::Function {
        name: "C.greet.classmethod".to_string(),
        params: vec![
            ("cls".to_string(), self_ty.clone()),
            ("x".to_string(), Ty::Int),
        ],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let mut items = vec![init, static_fn, class_fn];
    items.extend(extra_items);
    HirModule {
        seeded_builtin_exception_classes: false,
        items,
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![(
            "C".to_string(),
            HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "C".to_string(),
                bases: Vec::new(),
                mro: vec!["C".to_string()],
                attrs: Vec::new(),
                methods: vec![("__init__".to_string(), "C.__init__".to_string())],
                type_param: None,
                properties: Vec::new(),
                static_methods: vec![("create".to_string(), "C.create.static".to_string())],
                class_methods: vec![("greet".to_string(), "C.greet.classmethod".to_string())],
                is_enum: false,
                implicit_object_init: false,
                method_defaults: Vec::new(),
                enum_members: Vec::new(),
                is_dataclass: false,
                dataclass_fields: Vec::new(),
                is_protocol: false,
                runtime_checkable: false,
                protocol_members: Vec::new(),
                abstract_methods: Vec::new(),
                is_abstract: false,
            },
        )],
    }
}

// -- PEP 544 (#380): Protocol conformance checking --------------------

/// Parses and lowers source code, then type-checks it with
/// `check_and_resolve`. Returns the resolved HIR on success or the
/// diagnostic on failure.
fn check_source(source: &str) -> Result<pycc_hir::HirModule, pycc_diag::Diagnostic> {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
    check_and_resolve(&hir)
}

// Topic submodules split out of this file.
mod instances;
mod solver_paths;
mod properties;
mod inheritance;
mod static_methods;
mod isinstance_and_eq;
mod protocols;
