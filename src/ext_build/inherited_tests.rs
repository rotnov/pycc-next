use super::*;

use crate::ext_build::collect_exports;

/// Parses, lowers and type-checks `source`, returning the resolved module
/// with its inherited-method copies.
fn resolved(source: &str) -> HirModule {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
    pycc_types::check_and_resolve(&hir).expect("test fixture must check")
}

const MODULE: &str = "class A:\n    def __init__(self) -> None:\n        self.x = 0\n        self.x = self.hook()\n    \
    def hook(self) -> int:\n        return 1\n    def m(self) -> int:\n        return 1\n    \
    def g(self) -> int:\n        return self.m()\n    @property\n    def p(self) -> int:\n        \
    return self.m()\nclass B(A):\n    def m(self) -> int:\n        return 2\n    \
    def hook(self) -> int:\n        return 7\nclass _P(A):\n    def m(self) -> int:\n        \
    return 3\n";

#[test]
fn a_copy_on_a_published_class_is_published_and_a_getter_copy_dropped() {
    let module = resolved(MODULE);
    assert_eq!(copy_export_verdict(&module, "A.g"), CopyVerdict::NotACopy);
    assert_eq!(copy_export_verdict(&module, "B.g"), CopyVerdict::Publish);
    assert_eq!(copy_export_verdict(&module, "B.p"), CopyVerdict::Drop);
    // A privately named receiver is never published.
    assert_eq!(copy_export_verdict(&module, "_P.g"), CopyVerdict::Drop);
}

#[test]
fn a_published_subclass_binds_its_copy_and_constructs_through_it() {
    let module = resolved(MODULE);
    let exports = collect_exports(&module).expect("the module exports");
    let inherited = exports
        .iter()
        .find(|e| e.name == "A.g")
        .expect("A.g is exported");
    assert_eq!(
        receiver_exact_export(&module, "B", inherited, &exports).name,
        "B.g"
    );
    // `A` itself runs its own body.
    assert_eq!(
        receiver_exact_export(&module, "A", inherited, &exports).name,
        "A.g"
    );
    assert_eq!(
        receiver_exact_init(&module, "B", "A.__init__"),
        "B.__init__"
    );
    assert_eq!(
        receiver_exact_init(&module, "A", "A.__init__"),
        "A.__init__"
    );
    // A class whose inherited constructor needed no copy keeps the origin.
    let plain = resolved(
        "class A:\n    def __init__(self) -> None:\n        self.x = 1\nclass B(A):\n    pass\n",
    );
    assert_eq!(receiver_exact_init(&plain, "B", "A.__init__"), "A.__init__");
}
