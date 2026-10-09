use super::*;

fn int(value: i64) -> Option<HirExpr> {
    Some(HirExpr::IntLiteral(value))
}

#[test]
fn the_required_count_is_the_index_of_the_first_default() {
    assert_eq!(required_count(&[]), 0);
    assert_eq!(required_count(&[None, None]), 2);
    assert_eq!(required_count(&[None, int(1), int(2)]), 1);
    assert_eq!(required_count(&[int(1)]), 0);
}

#[test]
fn a_defaulted_argument_is_read_from_its_local_and_any_other_from_the_host() {
    let defaults = [None, int(1)];
    assert_eq!(arg_expr(&defaults, 0, "args[0]".to_string()), "args[0]");
    assert_eq!(arg_expr(&defaults, 1, "args[1]".to_string()), "v1");
    // An export without defaults has an empty vector.
    assert_eq!(arg_expr(&[], 0, "args[0]".to_string()), "args[0]");
}

#[test]
fn none_and_the_two_bools_are_the_interpreter_singletons() {
    for (default, name) in [
        (HirExpr::NoneLiteral, "Py_None"),
        (HirExpr::BoolLiteral(true), "Py_True"),
        (HirExpr::BoolLiteral(false), "Py_False"),
    ] {
        let object = default_object("p", 0, &default);
        assert_eq!(object.expr, name);
        assert!(object.definition.is_none());
    }
}

#[test]
fn an_int_float_or_str_default_is_one_cached_object_created_on_first_use() {
    let object = default_object("wrap_f", 2, &HirExpr::IntLiteral(-7));
    assert_eq!(object.expr, "pycc_ext_default_wrap_f_2()");
    assert_eq!(
        object.definition.as_deref(),
        Some(
            "static PyObject *pycc_ext_default_wrap_f_2(void)\n{\n    \
             static PyObject *value = NULL;\n    if (value == NULL) {\n        \
             value = PyLong_FromLongLong(-7LL);\n    }\n    return value;\n}\n"
        )
    );
    let float = default_object("p", 0, &HirExpr::FloatLiteral(0.1));
    assert!(
        float
            .definition
            .as_deref()
            .is_some_and(|text| text.contains("PyFloat_FromDouble(1e-1)")),
        "{:?}",
        float.definition
    );
    let text = default_object("p", 0, &HirExpr::StringLiteral("ab".to_string()));
    assert!(
        text.definition
            .as_deref()
            .is_some_and(|text| text.contains("PyUnicode_FromStringAndSize(\"ab\", 2)")),
        "{:?}",
        text.definition
    );
}

#[test]
fn the_most_negative_int_is_spelled_as_an_expression() {
    assert_eq!(c_long_long(i64::MIN), "(-9223372036854775807LL - 1)");
    assert_eq!(c_long_long(i64::MAX), "9223372036854775807LL");
    assert_eq!(c_long_long(0), "0LL");
}

#[test]
fn a_float_reads_back_exactly_and_an_infinity_is_huge_val() {
    assert_eq!(c_double(-1.5), "-1.5e0");
    assert_eq!(c_double(0.1).parse::<f64>(), Ok(0.1));
    assert_eq!(c_double(f64::INFINITY), "HUGE_VAL");
    assert_eq!(c_double(f64::NEG_INFINITY), "-HUGE_VAL");
}

#[test]
fn every_str_byte_outside_letters_digits_and_space_is_a_fixed_width_escape() {
    // A quote, a backslash, a newline, an embedded NUL followed by a digit
    // (which a variable-width escape would swallow), and a two-byte UTF-8
    // character; the length counts bytes.
    let text = "a\"\\\n\u{0}7 \u{e9}";
    assert_eq!(
        c_unicode(text),
        "PyUnicode_FromStringAndSize(\"a\\042\\134\\012\\0007 \\303\\251\", 9)"
    );
}

#[test]
fn the_range_check_names_both_bounds_and_binds_each_defaulted_argument() {
    let defaults = [None, HirExpr::NoneLiteral.into(), int(3)];
    let text = range_arity_check(
        "S.feed",
        "wrap_S_feed",
        &defaults,
        &ArgSource {
            count: "nargs",
            item: &|index| format!("args[{index}]"),
            fail: "        return NULL;\n",
        },
    );
    assert_eq!(
        text,
        "    if (nargs < 1 || nargs > 3) {\n        PyErr_Format(PyExc_TypeError, \
         \"S.feed() takes from 1 to 3 arguments (%zd given)\", nargs);\n        \
         return NULL;\n    }\n\
         \x20   PyObject *v1 = nargs > 1 ? args[1] : Py_None;\n\
         \x20   PyObject *v2 = nargs > 2 ? args[2] : pycc_ext_default_wrap_S_feed_2();\n\
         \x20   if (v2 == NULL) {\n        return NULL;\n    }\n"
    );
    assert_eq!(
        default_object_helpers("wrap_S_feed", &defaults),
        default_object("wrap_S_feed", 2, &HirExpr::IntLiteral(3))
            .definition
            .expect("an int default is cached")
    );
    assert_eq!(default_object_helpers("wrap_S_feed", &[None]), "");
}

#[test]
#[should_panic(expected = "a non-literal default reached the boundary")]
fn a_non_literal_default_is_an_internal_error() {
    default_object("p", 0, &HirExpr::Name("x".to_string()));
}
