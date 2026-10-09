use super::*;

pub(crate) fn env() -> MarkerEnv {
    MarkerEnv::new([
        "cpython".into(),
        "3.14.7".into(),
        "posix".into(),
        "arm64".into(),
        "CPython".into(),
        "25.0.0".into(),
        "Darwin".into(),
        "Darwin Kernel Version 25.0.0".into(),
        "3.14.7".into(),
        "3.14".into(),
        "darwin".into(),
    ])
}

fn eval(text: &str, extra: &str) -> Result<bool, String> {
    parse_marker(text)?.evaluate(&env(), extra, text)
}

fn holds(text: &str) -> bool {
    eval(text, "").unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn refused(text: &str) -> String {
    eval(text, "").expect_err(text)
}

#[test]
fn version_comparisons_use_release_segments() {
    assert!(holds("python_version >= \"3.8\""));
    assert!(!holds("python_version < \"3.10\""));
    assert!(holds("python_version < \"3.15\""));
    assert!(holds("python_version > '3.9'"));
    assert!(holds("python_version <= '3.14'"));
    assert!(holds("python_version == '3.14.0'"));
    assert!(holds("python_full_version != '3.14.6'"));
    assert!(holds("\"3.8\" <= python_version"));
    assert!(!holds("\"3.20\" <= python_version"));
}

#[test]
fn compatible_release_pins_all_but_the_last_segment() {
    assert!(holds("python_full_version ~= '3.14.2'"));
    assert!(!holds("python_full_version ~= '3.14.8'"));
    assert!(!holds("python_full_version ~= '3.13.0'"));
    assert!(holds("python_version ~= '3.10'"));
    assert!(!holds("python_version ~= '3.14.1'"));
    // A shorter left-hand side pads with zeros: 3.14 ~= 3.14.0.
    assert!(holds("python_version ~= '3.14.0'"));
    assert!(refused("python_version ~= '3'").contains("two release segments"));
}

#[test]
fn string_comparisons_and_containment() {
    assert!(!holds("sys_platform == \"win32\""));
    assert!(holds("sys_platform != 'win32'"));
    assert!(holds("\"dar\" in sys_platform"));
    assert!(!holds("\"linux\" in sys_platform"));
    assert!(holds("\"linux\" not in sys_platform"));
    assert!(holds("platform_machine == 'arm64' and os_name == 'posix'"));
    assert!(holds("'a' == 'a'"));
    assert!(holds("platform_release >= '24'"));
}

#[test]
fn and_binds_tighter_than_or_and_parentheses_group() {
    assert!(holds(
        "os_name == 'nt' and os_name == 'nt' or os_name == 'posix'"
    ));
    assert!(!holds(
        "os_name == 'nt' and (os_name == 'nt' or os_name == 'posix')"
    ));
    assert!(holds(
        "os_name == 'posix' or os_name == 'nt' and os_name == 'nt'"
    ));
    assert!(holds("(os_name == 'posix')"));
}

#[test]
fn extra_compares_normalized_and_only_by_equality() {
    assert!(eval("extra == \"x\"", "x").unwrap());
    assert!(!eval("extra == \"x\"", "").unwrap());
    assert!(!eval("extra == \"x\"", "y").unwrap());
    assert!(eval("extra == \"Foo_Bar\"", "foo-bar").unwrap());
    assert!(eval("\"foo.bar\" == extra", "foo-bar").unwrap());
    assert!(eval("extra != \"x\"", "").unwrap());
    assert!(eval("extra >= 'x'", "x").unwrap_err().contains("`extra`"));
}

#[test]
fn unsupported_markers_are_refused() {
    assert!(
        parse_marker("python_implementation == 'x'")
            .unwrap_err()
            .contains("unknown marker variable")
    );
    assert!(
        parse_marker("python_version === 'x'")
            .unwrap_err()
            .contains("unsupported operator")
    );
    assert!(
        parse_marker("sys_platform === 'darwin'")
            .unwrap_err()
            .contains("unsupported operator")
    );
    assert!(
        parse_marker("os_name == 'posix")
            .unwrap_err()
            .contains("unterminated string")
    );
    assert!(
        parse_marker("os_name == 'posix' os_name")
            .unwrap_err()
            .contains("trailing tokens")
    );
    assert!(
        parse_marker("(os_name == 'posix'")
            .unwrap_err()
            .contains("unclosed")
    );
    assert!(
        parse_marker("os_name not 'posix'")
            .unwrap_err()
            .contains("`not` without `in`")
    );
    assert!(
        parse_marker("os_name")
            .unwrap_err()
            .contains("missing marker operator")
    );
    assert!(
        parse_marker("os_name ==")
            .unwrap_err()
            .contains("missing marker value")
    );
    assert!(
        parse_marker("os_name == 'a' ; x")
            .unwrap_err()
            .contains("unexpected character")
    );
    assert!(refused("python_version >= 'three'").contains("plain release"));
    assert!(refused("python_version >= '3.8.0rc1'").contains("plain release"));
    assert!(refused("python_version == '3.8.*'").contains("plain release"));
    assert!(refused("'3.8.*' != python_full_version").contains("plain release"));
    assert!(refused("os_name < 'posix'").contains("plain release"));
    assert!(refused("python_version >= '3..8'").contains("plain release"));
}

#[test]
fn a_parse_error_in_a_branch_that_would_not_run_still_refuses() {
    assert!(parse_marker("os_name == 'posix' or bogus == 'x'").is_err());
}

#[test]
fn names_normalize_per_pep_503() {
    assert_eq!(normalize_name("Foo__Bar.baz-Qux"), "foo-bar-baz-qux");
    assert_eq!(normalize_name("tinypkg"), "tinypkg");
}
