//! #1316: the bridge-table watermark every generated wrapper takes.
//!
//! A compiled function body now bridges a failed foreign operation into a
//! pycc exception, keeping the CPython original in the shim's per-thread
//! bridge table. Every frame that can return to CPython takes
//! `pycc_ext_bridge_mark()` before compiled code runs and calls
//! `pycc_ext_bridge_release_to(mark)` on both exits, so the table holds at
//! most one top-level host call's entries. These tests pin the order on the
//! failing exit -- look the escaping entry up first, release after --, that
//! every exit counts the frame out only around its pack (Part 1 of #1499),
//! and that the helpers the generated text names sit in the shim above the
//! `#include` that splices the generated text in.

use super::*;

/// Part 1 of #1499: every exit a frame can take once it has marked returns
/// through the counting-out helper, so the frame stays a live activation
/// until its return value is packed and every cleanup before that `return`
/// -- the watermark release, a `PyBuffer_Release`, both of which can run a
/// finalizer that switches the GIL -- has run. Checked per marked function
/// in `inc`: from its mark to the end of its body, every line that returns
/// counts out.
pub(super) fn assert_every_exit_after_the_mark_counts_out(inc: &str) {
    let mark = "    Py_ssize_t bridge_mark = pycc_ext_bridge_mark();\n";
    let mut marks = 0;
    for (at, _) in inc.match_indices(mark) {
        marks += 1;
        let body = &inc[at..];
        let body = &body[..body.find("\n}\n").expect("the function's end")];
        let exits: Vec<&str> = body
            .lines()
            .filter(|line| line.contains("return") || line.contains("Py_RETURN"))
            .collect();
        assert!(!exits.is_empty(), "{body}");
        for exit in exits {
            assert!(
                exit.trim_start()
                    .starts_with("return pycc_ext_activation_exit"),
                "an exit after the mark that does not count the frame out: {exit}\n{body}"
            );
        }
    }
    assert!(marks > 0, "{inc}");
}

fn plain_export(name: &str) -> ExtExport {
    ExtExport {
        defaults: Vec::new(),
        name: name.to_string(),
        class: None,
        method: None,
        returns_buffer_slice: false,
        receiver: ExtReceiver::None,
        params: vec![Ty::Int],
        param_writable: vec![false],
        return_ty: Ty::Int,
        keyword_names: None,
    }
}

#[test]
fn a_wrapper_marks_before_the_call_and_releases_after_the_lookup_on_both_exits() {
    let exports = [plain_export("f")];
    let inc = generate_exports_inc("m", &exports, &[], &[], &[], &[]);
    let mark = inc
        .find("    Py_ssize_t bridge_mark = pycc_ext_bridge_mark();\n")
        .expect("the wrapper takes a mark");
    let call = inc
        .find("(a0);\n")
        .expect("the wrapper calls the compiled function");
    assert!(mark < call, "{inc}");
    assert!(
        inc.contains(
            "    if (pycc_rt_ext_pending_type() >= 0) {\n        pycc_ext_raise_pending();\n        \
             pycc_ext_bridge_release_to(bridge_mark);\n        return pycc_ext_activation_exit(NULL);\n    }\n    \
             pycc_ext_bridge_release_to(bridge_mark);\n    return pycc_ext_activation_exit(pycc_ext_pack_int(\"f\", result));\n"
        ),
        "{inc}"
    );
    assert_eq!(inc.matches("pycc_ext_bridge_mark()").count(), 1, "{inc}");
    assert_eq!(
        inc.matches("pycc_ext_bridge_release_to(bridge_mark);")
            .count(),
        2,
        "{inc}"
    );
    assert_every_exit_after_the_mark_counts_out(&inc);
}

/// Every return shape `wrapper_for` emits -- a scalar pack, `None`, and a
/// tuple's three exits -- counts the wrapper out only around its pack.
#[test]
fn every_wrapper_return_shape_counts_out_after_its_pack() {
    for return_ty in [
        Ty::Int,
        Ty::Float,
        Ty::Bool,
        Ty::Str,
        Ty::None,
        Ty::Tuple(Box::new(vec![Ty::Int, Ty::Float])),
    ] {
        let mut export = plain_export("f");
        export.return_ty = return_ty;
        let inc = generate_exports_inc("m", &[export], &[], &[], &[], &[]);
        assert_every_exit_after_the_mark_counts_out(&inc);
    }
}

#[test]
fn the_watermark_helpers_are_defined_above_the_generated_include() {
    let include = SHIM_C
        .find("#include \"pycc_ext_exports.inc\"")
        .expect("the shim splices the generated companion in");
    for helper in [
        "static Py_ssize_t pycc_ext_bridge_mark(void)",
        "static void pycc_ext_bridge_release_to(Py_ssize_t mark)",
        "static PyObject *pycc_ext_activation_exit(PyObject *result)",
        "static int pycc_ext_activation_exit_status(int status)",
        "static int pycc_ext_raise_pending(void)",
    ] {
        let at = SHIM_C.find(helper).unwrap_or_else(|| panic!("{helper}"));
        assert!(at < include, "{helper} must precede the generated include");
    }
}

#[test]
fn module_exec_releases_to_its_own_mark_instead_of_clearing_the_table() {
    let shim = shim_c();
    assert!(!shim.contains("pycc_ext_bridge_table_clear"));
    // The mark is taken immediately before the body (#1501: after #1366's
    // exec-target save/set and every early return), with no bridge entry
    // made between them.
    let taken = "    mark = pycc_ext_bridge_mark();\n";
    let mark = shim.find(taken).expect("the mark is taken") + taken.len();
    let exec = shim
        .find("    exec_status = pycc_ext_module_exec();\n")
        .expect("the body runs");
    assert!(mark <= exec);
    assert!(!shim[mark..exec].contains("pycc_ext_bridge_"));
    assert!(shim[exec..].starts_with(
        "    exec_status = pycc_ext_module_exec();\n    \
         (void)PyThread_tss_set(pycc_ext_exec_target_key, saved_target);\n    \
         (void)PyThread_tss_set(pycc_ext_exec_classes_key, saved_classes);\n    \
         if (exec_status != 0) {\n        \
         Py_DECREF(classes);"
    ));
    assert_eq!(
        shim.matches("        pycc_ext_bridge_release_to(mark);\n        return pycc_ext_activation_exit_status(-1);\n    }\n    pycc_ext_bridge_release_to(mark);\n    (void)pycc_ext_activation_exit_status(0);\n    status = pycc_ext_publish_unbound_classes(module, classes);\n    Py_DECREF(classes);\n    if (status != 0) {").count(),
        1
    );
    // #1199's class safety net and #1467's PEP 562 hooks run after that
    // release, and their failure paths return without touching the bridge
    // table again.
    assert!(shim.contains(
        "    if (PyModule_AddFunctions(module, pycc_ext_module_hooks) != 0) {\n        \
         return -1;\n    }\n    return 0;\n}\n"
    ));
}

/// #1501: the watermark mark doubles as the live-activation count-in the
/// module-global rebind gate reads. The release only truncates the table,
/// so the finalizers it runs see the frame still counted; the frame is
/// counted out by the exit helpers alone, after its pack. Exec takes its
/// mark only once every early return is behind it, so each mark is counted
/// out exactly once; and the gate answers 1 only for the rebinding exec
/// alone.
#[test]
fn the_watermark_counts_live_activations_for_the_rebind_gate() {
    let shim = shim_c();
    assert!(shim.contains("static Py_ssize_t pycc_ext_live_activations = 0;\n"));
    assert!(shim.contains(
        "    pycc_ext_bridge_table *table = pycc_ext_bridge_current();\n\n    \
         pycc_ext_live_activations++;\n    return table == NULL ? 0 : table->len;\n}\n"
    ));
    assert!(
        shim.contains("    PyObject *saved;\n\n    if (table == NULL || mark >= table->len) {\n")
    );
    for helper in [
        "static PyObject *pycc_ext_activation_exit(PyObject *result)\n{\n    \
         pycc_ext_live_activations--;\n    return result;\n}\n",
        "static int pycc_ext_activation_exit_status(int status)\n{\n    \
         pycc_ext_live_activations--;\n    return status;\n}\n",
    ] {
        assert!(shim.contains(helper), "{helper}");
    }
    assert!(shim.contains(
        "int pycc_ext_obj_rebind_may_release(void)\n{\n    \
         return pycc_ext_live_activations == 1;\n}\n"
    ));
    assert_eq!(shim.matches("pycc_ext_live_activations++").count(), 1);
    assert_eq!(shim.matches("pycc_ext_live_activations--").count(), 2);
    assert!(shim.contains(
        "    mark = pycc_ext_bridge_mark();\n    exec_status = pycc_ext_module_exec();\n"
    ));
    assert_eq!(shim.matches("mark = pycc_ext_bridge_mark();").count(), 1);
}

#[test]
fn a_bridged_entry_is_removed_in_order() {
    // An inner re-entrant frame restoring its own entry must never move an
    // outer frame's entry across the outer frame's mark, which the old
    // swap-remove did.
    assert!(shim_c().contains("            memmove(&table->entries[i], &table->entries[i + 1],\n"));
}
