use super::*;
use crate::lock::marker::tests::env;

#[test]
fn a_bare_name_normalizes() {
    let req = parse_requirement("Tiny_Dep").unwrap();
    assert_eq!(req.name, "tiny-dep");
    assert!(req.extras.is_empty());
    assert!(req.marker.is_none());
}

#[test]
fn extras_specifiers_and_markers_parse() {
    let req = parse_requirement("b [X_y, z] (>=1.0,<2) ; sys_platform == 'darwin'").unwrap();
    assert_eq!(req.name, "b");
    assert_eq!(req.extras.iter().collect::<Vec<_>>(), ["x-y", "z"]);
    let (marker, text) = req.marker.unwrap();
    assert_eq!(text, "sys_platform == 'darwin'");
    assert!(marker.evaluate(&env(), "", &text).unwrap());
    let req = parse_requirement("c>=2;extra=='test'").unwrap();
    assert_eq!(req.name, "c");
    assert_eq!(req.marker.unwrap().1, "extra=='test'");
}

#[test]
fn a_url_requirement_keeps_its_marker() {
    let req = parse_requirement("d @ https://example.invalid/d.whl ; os_name == 'nt'").unwrap();
    assert_eq!(req.name, "d");
    assert_eq!(req.marker.unwrap().1, "os_name == 'nt'");
    let req = parse_requirement("d@https://example.invalid/d;x.whl").unwrap();
    assert!(req.marker.is_none());
    assert!(
        parse_requirement("d @ https://example.invalid/d.whl junk")
            .unwrap_err()
            .contains("after a URL")
    );
}

#[test]
fn malformed_requirements_are_refused() {
    assert!(
        parse_requirement("")
            .unwrap_err()
            .contains("distribution name")
    );
    assert!(
        parse_requirement("-x")
            .unwrap_err()
            .contains("distribution name")
    );
    assert!(
        parse_requirement("x-")
            .unwrap_err()
            .contains("distribution name")
    );
    assert!(parse_requirement("x [a").unwrap_err().contains("unclosed"));
    assert!(
        parse_requirement("x [a,,b]")
            .unwrap_err()
            .contains("malformed extra")
    );
    assert!(
        parse_requirement("x [a b]")
            .unwrap_err()
            .contains("malformed extra")
    );
    assert!(
        parse_requirement("x ; bogus == '1'")
            .unwrap_err()
            .contains("unknown marker")
    );
}
