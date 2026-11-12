use super::*;

fn k(q: &str) -> FunctionKey {
    ("m".into(), q.into(), 1)
}

fn set(names: &[&str]) -> BTreeSet<FunctionKey> {
    names.iter().map(|q| k(q)).collect()
}

#[test]
fn pruning_tries_whole_units_least_self_time_first() {
    // Most self time first.
    let order = [k("hot"), k("a"), k("x"), k("b"), k("cold")];
    let units = [set(&["a", "b"])];
    let removable = set(&["hot", "a", "x", "b", "cold"]);
    assert_eq!(
        removals(&order, &units, &removable),
        [set(&["cold"]), set(&["x"]), set(&["a", "b"]), set(&["hot"])]
    );
    // The carried set's members are not removable; a unit keeps only its
    // removable members.
    let removable = set(&["hot", "b", "x"]);
    assert_eq!(removals(&order, &units, &removable), [set(&["b"]), set(&["x"]), set(&["hot"])]);
}

#[test]
fn the_static_regions_come_from_each_modules_source() {
    let dir = pycc_scratch::ScratchDir::new("units_test").unwrap();
    let pkg = dir.join("pkg");
    std::fs::create_dir_all(pkg.join("sub")).unwrap();
    std::fs::write(pkg.join("__init__.py"), "").unwrap();
    std::fs::write(pkg.join("sub/__init__.py"), "def f():\n    return g()\n\ndef g():\n    return 1\n").unwrap();
    std::fs::write(pkg.join("m.py"), "def a(x):\n    return a(x - 1) if x else b()\n\ndef b():\n    return 0\n").unwrap();
    let key = |m: &str, q: &str, l: u32| (m.to_string(), q.to_string(), l);
    let candidates: BTreeSet<FunctionKey> = [
        key("pkg.m", "a", 1),
        key("pkg.m", "b", 4),
        key("pkg.sub", "f", 1),
        key("pkg.sub", "g", 4),
        key("pkg.gone", "h", 1),
    ]
    .into();
    let mut regions = static_regions(&pkg, &candidates);
    regions.sort();
    assert_eq!(
        regions,
        [
            [key("pkg.m", "a", 1), key("pkg.m", "b", 4)].into_iter().collect::<BTreeSet<_>>(),
            [key("pkg.sub", "f", 1), key("pkg.sub", "g", 4)].into_iter().collect(),
        ]
    );
    // A module alone, and a selection that leaves `b` out: `a` still calls
    // itself.
    assert_eq!(
        static_regions(&pkg.join("m.py"), &[key("pkg.m", "a", 1)].into()),
        [[key("pkg.m", "a", 1)].into_iter().collect::<BTreeSet<_>>()]
    );
    assert!(static_regions(&pkg, &[key("pkg.m", "b", 4)].into()).is_empty());
}
