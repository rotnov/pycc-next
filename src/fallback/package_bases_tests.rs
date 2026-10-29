use super::*;

/// The names subclasses in the other modules define, transitively, by the
/// class of the entry they derive from, through a re-export.
#[test]
fn the_names_other_modules_subclasses_define() {
    let dir = pycc_scratch::ScratchDir::new("package_overrides").expect("scratch");
    let package = dir.join("pk");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("__init__.py"), "from .base import Base\n").unwrap();
    std::fs::write(
        package.join("base.py"),
        "class Base:\n    def a(self) -> int:\n        return 1\nclass Other:\n    pass\n",
    )
    .unwrap();
    std::fs::write(
        package.join("sub.py"),
        "from . import Base\nclass Sub(Base):\n    x = 1\n    y: int = 2\n    \
         def a(self) -> int:\n        return 2\n",
    )
    .unwrap();
    std::fs::write(
        package.join("deeper.py"),
        "from .sub import Sub\nclass Deep(Sub):\n    def b(self) -> int:\n        return 3\n",
    )
    .unwrap();
    let base = package.join("base.py");
    assert_eq!(
        overridden_in_package(&base),
        BTreeMap::from([(
            "Base".to_string(),
            BTreeSet::from([
                "a".to_string(),
                "b".to_string(),
                "x".to_string(),
                "y".to_string()
            ])
        )])
    );
    assert_eq!(subclassed_in_package(&base)["Base"], "pk.sub.Sub");
    assert!(overridden_in_package(&package.join("deeper.py")).is_empty());
    assert!(overridden_in_package(&dir.join("nowhere.py")).is_empty());
}
