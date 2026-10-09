//! The generated `METH_FASTCALL` wrapper and the slot machinery it and
//! `Py_tp_init` share (extracted while Part 1 of #1142 taught the slot
//! vector per-parameter writability).
//!
//! One cohesion: everything here turns a carried signature into the C text
//! that moves arguments across the boundary -- the per-parameter carrier
//! vector, the wrapper body, its argument locals, its unpack calls, its
//! buffer releases, its C parameter list and its return packer. Nothing
//! here decides *which* functions are exported or *whether* a type is
//! carriable; those stay in the parent module with the collectors.

use pycc_types::Ty;

use super::carrier::{
    BUFFER_VIEW_C_TYPE, BoundaryCarrier, SlotCleanup, boundary_carrier, return_c_type,
};
use super::{ExtExport, ExtReceiver, defaults, keywords, source_level_name};

/// One export's or constructor's slot vector, with Part 1 of #1142's
/// writability applied.
///
/// The single place a [`BoundaryCarrier::Buffer`]'s `writable` flag is ever
/// set, shared by [`wrapper_for`] and `method_types`' `tp_init_c` -- the
/// only two places a slot vector is built -- so a `METH_FASTCALL` wrapper
/// and a `Py_tp_init` can never request different buffer flags for the same
/// declared parameter of the same body.
pub(crate) fn slot_carriers(params: &[Ty], param_writable: &[bool]) -> Vec<BoundaryCarrier> {
    // `zip` truncates to the shorter side rather than failing, and a slot
    // vector shorter than `params` would emit a wrapper that unpacks fewer
    // arguments than it declares. The two vectors are built together in
    // `collect_exports` and `ctor_descriptor`, so a mismatch is a bug in a
    // fixture or in a future producer, and it is made loud here rather than
    // silently miscompiled.
    assert_eq!(
        params.len(),
        param_writable.len(),
        "pycc: internal error: a slot vector's writability flags must be parallel to its parameters"
    );
    params
        .iter()
        .zip(param_writable)
        .map(|(ty, writable)| {
            match boundary_carrier(ty).expect("only carriable parameters are collected") {
                BoundaryCarrier::Buffer { .. } => BoundaryCarrier::Buffer {
                    writable: *writable,
                },
                other => other,
            }
        })
        .collect()
}

/// One export's `METH_FASTCALL` wrapper.
///
/// `METH_FASTCALL` rather than `METH_VARARGS` for two reasons. It is the
/// only limited-API calling convention that receives the argument count
/// directly, so arity and type checking cost no tuple; and without
/// `METH_KEYWORDS` CPython itself raises the keyword `TypeError` before the
/// wrapper is entered, which is D-244 rule 7's closed boundary for free --
/// a `METH_VARARGS` wrapper would silently accept `f(x=1)` at the C level.
/// An export whose keywords the driver can bind as CPython does (#1461,
/// [`ExtExport::keyword_names`]) adds `METH_KEYWORDS` and takes `kwnames`,
/// binding through `keywords::fastcall_prologue` before the arity check.
///
/// A scalar-only signature's call goes through `fnptr_<name>`, the
/// module-level function-pointer global codegen emits for each `def`'s
/// binding, and *not* through the compiled function's own symbol. That is
/// what makes a rebound name (`f = g` at module level) call what Python
/// says it calls.
///
/// A signature carrying a `tuple` calls `pycc_ext_thunk_<name>` instead
/// (#1050), which dispatches through that same global one LLVM frame
/// further in and so keeps the rebinding property. The indirection exists
/// because C cannot spell a pycc aggregate: pycc's own convention for
/// passing and returning one is not the platform C struct ABI, so the thunk
/// presents the signature as scalars and out-pointers and every aggregate
/// stays on the LLVM side. That declaration is a real `extern` *function*
/// declaration and the call is direct -- deliberately not the scalar path's
/// `void *` global cast to a function-pointer type. The cast form was
/// measured to fault (SIGBUS) on aarch64-apple-darwin for an out-pointer
/// signature, and it is also the only one of the two that no C compiler can
/// type-check at all.
pub(crate) fn wrapper_for(export: &ExtExport) -> String {
    let name = &export.name;
    let arity = export.params.len();
    // Every local, unpack helper and call slot below is a function of the
    // declared type alone and never of the object that arrives: `def
    // f(x: int)` gets `long long a0` and `pycc_ext_unpack_int` even though
    // that helper also accepts `True`, because widening the accepted object
    // set is the helper's business and never narrows the local.
    // `collect_exports` refused every type these two lookups cannot name, so
    // an export in hand always has both.
    let slots: Vec<BoundaryCarrier> = slot_carriers(&export.params, &export.param_writable);
    let return_c = return_c_type(&export.return_ty).expect("a carriable return type");
    let returns_none = export.return_ty == Ty::None;
    // One entry per element of a returned `tuple`, and empty for every
    // other return type: these become trailing out-pointer arguments, not
    // a return value.
    let out_slots: Vec<(&'static str, &'static str)> = match boundary_carrier(&export.return_ty) {
        Some(BoundaryCarrier::Tuple(elements)) => elements,
        _ => Vec::new(),
    };
    // #1050 keeps two index spaces apart on purpose. `arity` and every
    // `a{index}` below are per *Python argument* -- what `nargs` counts,
    // what the arity message names, and what an unpack failure has to clean
    // up after -- while the C call's own argument list is the flattened
    // one, built only at the call expression further down. Renumbering
    // `a{index}` to follow the flattened list would make
    // `def f(t: tuple[int, int])` report "takes exactly 2 arguments" for a
    // one-argument function.
    // Part 2 of #1175 (#1179): the three trailing `long long *` out-slots a
    // buffer sub-range egress carries -- `has_slice`, `start`, `stop`.
    // Deliberately *not* folded into `out_slots` above, which stays
    // tuple-only: `out_slots` also drives the `r{index}`/`e{index}` locals,
    // the `packed` local, the suppressed `result` declaration and the
    // suppressed `result = ` assignment, every one of which is a `tuple`
    // property. A buffer return needs `result` bound, because the
    // `result == &a{index}` identity chain is what discriminates its
    // provenance.
    let slice_out = export.returns_buffer_slice;
    // `ext_thunk_required` asks whether a *declared* type is a tuple -- and
    // a receiver is never one, so the receiver-free tail gives the same
    // verdict the codegen side reaches from the MIR function's full
    // parameter list -- and, since #1179, whether the body carries a buffer
    // sub-range egress. That second half is no longer a function of the
    // declared signature at all: the driver computes it here from HIR
    // (`ExtExport::returns_buffer_slice`) and codegen computes it from MIR
    // (`pycc_codegen::buffer_slice_out_names`), two independent walks over
    // two IRs -- each resolving the fact per export *name* over every
    // definition of it, because two `def`s share one signature. Their agreement is what keeps this wrapper's call form
    // matching the compiled function's real arity, and it is pinned by a
    // parity test rather than by this comment.
    let use_thunk =
        pycc_codegen::ext_thunk_required(name, &export.params, &export.return_ty, slice_out);
    let thunk = pycc_codegen::ext_thunk_symbol(name);
    // A method's `name` is dotted, and these four sites paste it into C
    // identifiers: the thunk `extern` (through `ext_thunk_symbol`, which
    // mangles for itself), the `fnptr_` `extern`, the `fnptr_` cast at the
    // call, and this wrapper's own definition -- plus the `PyMethodDef` row
    // `generate_exports_inc` emits for it. The mangling is the identity for
    // a dot-free name, so every module-level function's wrapper is
    // byte-identical to what it was.
    let symbol = pycc_codegen::mangle_ext_name(name);
    // The `PyErr_Format` arity message is the one interpolation of `name`
    // that is *not* a C identifier: the host reads it, so it renders the
    // source-level spelling. Left alone it would say
    // `Grid.scale.static() takes exactly 1 argument` next to CPython's own
    // `Grid.scale() takes no keyword arguments` on the same object (or the
    // keyword binder's messages, #1461). The unpack helpers below take the
    // same spelling for the same reason.
    let source_name = source_level_name(name);
    // A `@classmethod`'s compiled signature leads with `cls` and an
    // instance method's with `self` (`ExtExport::receiver`); neither is a
    // carried argument. The leading `void *` is reinstated textually here,
    // so this declaration and `pycc_codegen`'s thunk -- which builds its own
    // list from the MIR function's parameters -- declare the same arity for
    // the same symbol. Only the *call* argument differs between the two:
    // `NULL` for `cls`, the unwrapped instance pointer for `self`.
    let params = {
        let carried = c_param_list(&slots, &out_slots, slice_out);
        if export.receiver == ExtReceiver::None {
            carried
        } else if carried == "void" {
            // `c_param_list` answers `"void"` for an empty list, because an
            // empty C parameter list means "unspecified". With a receiver
            // the list is not empty.
            "void *".to_string()
        } else {
            format!("void *, {carried}")
        }
    };
    let mut out = String::new();
    // The method part of #1140: the cached default objects an omitted
    // argument is read from, emitted ahead of the wrapper that uses them --
    // and nothing at all for an export without defaults, which keeps every
    // such wrapper byte-identical.
    let default_prefix = format!("wrap_{symbol}");
    out.push_str(&defaults::default_object_helpers(
        &default_prefix,
        &export.defaults,
    ));
    // #1199: the thunk arm reads `fnptr_` too, for the null guard below.
    if use_thunk {
        out.push_str(&format!("extern {return_c} {thunk}({params});\n"));
    }
    out.push_str(&format!("extern void *fnptr_{symbol};\n"));
    // #1461: a keyword-enabled export takes `METH_KEYWORDS`' fourth
    // argument and binds through its name table; every other wrapper keeps
    // the three-argument signature byte for byte. The flag side of the same
    // predicate is `keywords::method_flags`.
    let prologue = export.keyword_names.as_deref().map(|names| {
        keywords::fastcall_prologue(&default_prefix, source_name, names, &export.defaults)
    });
    let kwnames_param = match &prologue {
        Some(prologue) => {
            out.push_str(&prologue.before);
            ", PyObject *kwnames"
        }
        None => "",
    };
    out.push_str(&format!(
        "static PyObject *pycc_ext_wrap_{symbol}(PyObject *self, PyObject *const *args, \
         Py_ssize_t nargs{kwnames_param})\n{{\n"
    ));
    out.push_str(&fnptr_null_guard(&symbol, source_name, "NULL"));
    // A `METH_STATIC` wrapper is handed `NULL` in `self` and a `METH_CLASS`
    // one the *type object*; both discard it, exactly as
    // `MirExpr::NullInstance` discards `cls` at a native call site -- the
    // method was compiled for one class, so nothing in its body reads the
    // receiver, and forwarding a CPython type pointer into a slot typed
    // `Ty::Instance` would be type confusion even though nothing
    // dereferences it.
    //
    // A plain `METH_FASTCALL` instance-method wrapper (#1145) is handed the
    // carrier object itself and *does* read it. The NULL guard is not
    // defence in depth: `mod.Grid.__new__(mod.Grid)` runs
    // `PyType_GenericNew`, which zeroes the carrier and never runs
    // `tp_init`, so `inst` really is NULL at the wrapper's entry and the
    // guard is what makes that a `TypeError` instead of a segfault.
    // CPython's own method-descriptor machinery has already refused a
    // `self` of the wrong type before this point, so no type check is
    // needed here.
    if export.receiver == ExtReceiver::SelfInstance {
        out.push_str(&format!(
            "    void *self_inst = ((PyccExtInstance *)self)->inst;\n    \
             if (self_inst == NULL) {{\n        PyErr_SetString(PyExc_TypeError, \
             \"{source_name}() called on an uninitialized instance\");\n        \
             return NULL;\n    }}\n"
        ));
    } else {
        out.push_str("    (void)self;\n");
    }
    out.push_str("    (void)args;\n");
    // A `-> None` export has no result to hold: codegen emits its return as
    // LLVM `void`, so a result local would be a C type error, not a waste.
    // A `tuple` return has no single result either -- its elements arrive
    // in the `r{index}` locals below, through the thunk's out-pointers.
    if !returns_none && out_slots.is_empty() {
        out.push_str(&format!("    {return_c} result;\n"));
    }
    for (index, (c_type, _)) in out_slots.iter().enumerate() {
        out.push_str(&format!("    {c_type} r{index};\n"));
        out.push_str(&format!("    PyObject *e{index};\n"));
    }
    if !out_slots.is_empty() {
        out.push_str("    PyObject *packed;\n");
    }
    // Part 1 of #1175's two locals, declared with the rest rather than at
    // the acquire point so the generated function keeps one declaration
    // block, and emitted only under the same conjunction the acquire uses --
    // so every wrapper that cannot have a caller-owned buffer return stays
    // byte-identical to what it was.
    //
    // `caller_owned` is a separate flag rather than a `borrowed != NULL`
    // test, and that is load-bearing: `PyMemoryView_FromObject` answers NULL
    // with the exception set on failure, so `borrowed == NULL` cannot tell
    // "no parameter matched" (fall through to the owning packer, which is
    // correct) from "the match failed" (return NULL, which is the only
    // correct answer -- handing a parameter pointer to the owning packer
    // would free the host's storage).
    if !caller_owned_buffer_slots(&export.return_ty, &slots).is_empty() {
        out.push_str("    int caller_owned = 0;\n    PyObject *borrowed = NULL;\n");
    }
    // Part 2 of #1175's three out-slot locals, on their own declaration
    // path beside Part 1's `caller_owned`/`borrowed` rather than through
    // `out_slots`, and emitted only for an export that actually carries a
    // sub-range egress -- so every wrapper generated before #1179 stays
    // byte-identical.
    //
    // `slice_present` carries `1` only for a sub-range `return`, because no
    // pair of `long long` bounds is available as an in-band whole-view
    // sentinel: `b[0:-1]` is a legal slice, and `LLONG_MIN`/`LLONG_MAX` are
    // legal bounds that clamp correctly.
    //
    // Its `0` initializer is a belt-and-braces default and **not** the
    // protocol: every `return` in a widened body writes the slot itself,
    // `1` with its bounds or `0` for a whole-window return, so a frame that
    // executes more than one `return` describes the one that actually
    // reached here. Relying on the initializer instead made a sub-range
    // abandoned by a raising finalizer, and swallowed by an enclosing
    // handler, colour the later bare `return b` with its bounds. The
    // two bound locals are initialized too: a call that raises leaves them
    // exactly as it found them, and an indeterminate read is undefined
    // behaviour even on a path that discards the value.
    if slice_out {
        out.push_str(
            "    long long slice_present = 0;\n    long long slice_start = 0;\n    \
             long long slice_stop = 0;\n",
        );
    }
    out.push_str(&arg_slot_locals(&slots));
    if let Some(prologue) = &prologue {
        out.push_str(&prologue.body);
    }
    if export.defaults.is_empty() {
        out.push_str(&format!(
            "    if (nargs != {arity}) {{\n        PyErr_Format(PyExc_TypeError, \
             \"{source_name}() takes exactly {arity} argument{plural} (%zd given)\", nargs);\n        \
             return NULL;\n    }}\n",
            plural = if arity == 1 { "" } else { "s" },
        ));
    } else {
        out.push_str(&defaults::range_arity_check(
            source_name,
            &default_prefix,
            &export.defaults,
            &defaults::ArgSource {
                count: "nargs",
                item: &|index| format!("args[{index}]"),
                fail: "        return NULL;\n",
            },
        ));
    }
    out.push_str(&unpack_args(
        &slots,
        source_name,
        &|index| defaults::arg_expr(&export.defaults, index, format!("args[{index}]")),
        "        return NULL;\n",
    ));
    let mut call_args: Vec<String> = Vec::new();
    match export.receiver {
        ExtReceiver::None => {}
        ExtReceiver::NullCls => call_args.push("NULL".to_string()),
        ExtReceiver::SelfInstance => call_args.push("self_inst".to_string()),
    }
    for (index, slot) in slots.iter().enumerate() {
        match slot {
            BoundaryCarrier::Scalar(..) | BoundaryCarrier::Instance(_) => {
                call_args.push(format!("a{index}"))
            }
            BoundaryCarrier::Tuple(elements) => {
                call_args.extend((0..elements.len()).map(|element| format!("a{index}_{element}")))
            }
            BoundaryCarrier::Buffer { .. } => call_args.push(format!("&a{index}")),
        }
    }
    call_args.extend((0..out_slots.len()).map(|index| format!("&r{index}")));
    if slice_out {
        call_args.extend(
            ["&slice_present", "&slice_start", "&slice_stop"]
                .into_iter()
                .map(str::to_string),
        );
    }
    let call_args = call_args.join(", ");
    // Nothing is assigned on the `-> None` arm (a `void` call has no value)
    // nor on the `tuple` arm (its elements arrive through the out-pointers).
    let assign = if returns_none || !out_slots.is_empty() {
        ""
    } else {
        "result = "
    };
    // #1316: the bridge-table watermark, taken immediately before compiled
    // code runs. Anything the call bridges above it is released on both
    // exits below, so a host calling this export in a loop never grows the
    // table. See the C shim's `pycc_ext_bridge_release_to`.
    //
    // The mark also counts this wrapper as a live activation (Part 1 of
    // #1499), and every exit below returns through
    // `pycc_ext_activation_exit`, which counts it out only once its
    // argument -- the pack that gives the return value its own host
    // reference -- has been evaluated, and after the watermark release and
    // every `PyBuffer_Release` above it. Each of those can run a finalizer
    // that switches the GIL; a wrapper counted out before them would let a
    // suspended exec's rebind release the object this wrapper still has to
    // return.
    out.push_str("    Py_ssize_t bridge_mark = pycc_ext_bridge_mark();\n");
    if use_thunk {
        out.push_str(&format!("    {assign}{thunk}({call_args});\n"));
    } else {
        out.push_str(&format!(
            "    {assign}(({return_c} (*)({params}))fnptr_{symbol})({call_args});\n"
        ));
    }
    // A compiled function that raised returns a neutral carrier and leaves
    // the runtime's thread-local flag set (see `pycc_codegen`'s
    // `exception_exit` block), so the carrier must never be packed: the
    // pending exception is checked first and translated into a CPython one.
    // This reads no part of the call's return value, so it stands unchanged
    // on the `-> None` arm -- where it is the only thing between a raised
    // exception and a fabricated `None`.
    //
    // #1050: its *position*, before the pack below, is load-bearing for a
    // `tuple` return in a way it is not for a scalar one. A raising call
    // leaves every `r{index}` out-pointer local exactly as uninitialized as
    // it found it, so a pack that ran first would read indeterminate
    // storage -- undefined behaviour, not merely a wrong value.
    // The success-path half of the cleanup discipline, and the half nothing
    // else in the tree would notice was missing: a `Py_buffer` acquired
    // before the call is still held after it returns. Emitted once, at the
    // single point every remaining exit passes through -- the pending-
    // exception bail and the pack below both sit after it -- rather than
    // duplicated at each `return`. Releasing before the pack is safe and
    // deliberate *for every packer that existed before Part 1 of #1175*:
    // those packs read only `result`/`r{index}`, machine words the compiled
    // function already produced, never the buffer's storage.
    //
    // Part 1 of #1175 adds the one pack that breaks that premise, and
    // `caller_owned_buffer_acquire` below is emitted *before* these releases
    // for exactly that reason -- see its own comment.
    //
    // Empty for an export with no `memoryview` parameter, so every wrapper
    // generated before Part 1 of #1027 is byte-identical to what it was.
    let release: String = buffer_releases(&slots, "    ");
    // #1316: the watermark release runs *after* `pycc_ext_raise_pending`,
    // which must find the escaping exception's bridge entry first; the
    // release sets CPython's indicator aside around its own `Py_DECREF`s.
    out.push_str(&format!(
        "    if (pycc_rt_ext_pending_type() >= 0) {{\n{}        pycc_ext_raise_pending();\n        \
         pycc_ext_bridge_release_to(bridge_mark);\n        \
         return pycc_ext_activation_exit(NULL);\n    }}\n    \
         pycc_ext_bridge_release_to(bridge_mark);\n",
        buffer_releases(&slots, "        ")
    ));
    out.push_str(&caller_owned_buffer_acquire(
        &export.return_ty,
        &slots,
        source_name,
        slice_out,
    ));
    out.push_str(&release);
    match &export.return_ty {
        Ty::None => out.push_str("    return pycc_ext_activation_exit(Py_NewRef(Py_None));\n}\n\n"),
        Ty::Tuple(_) => out.push_str(&pack_tuple_return(source_name, &out_slots)),
        // `pack_int` is the one packer whose failure is a property of the
        // *value*, and the only one whose message therefore names the
        // function: D-141's bigint egress (#1040). `PyFloat_FromDouble` and
        // `PyBool_FromLong` cannot fail at all, and `pack_str` can only fail
        // the way any allocation can -- it refuses no `str` -- so none of
        // the three take a name. Arity is uniform across them, so every
        // packer but `int` shares the generic arm below.
        Ty::Int => out.push_str(&format!(
            "    return pycc_ext_activation_exit(pycc_ext_pack_int(\"{source_name}\", result));\n}}\n\n"
        )),
        // Part 2b of #1142 (#1164): its own arm rather than the generic one
        // below, because that arm resolves its packer through
        // `BoundaryCarrier::into_scalar`, which answers `None` for a buffer
        // and would panic here. Loosening `into_scalar` instead is the trap:
        // its other caller is the `tuple`-element lookup, so a `Buffer` arm
        // there admits `tuple[memoryview]`, for which no element shim exists.
        // `return_c_type` states the egress at exactly the top-level return
        // position, and this arm is its other half.
        //
        // `result` is the `PyccExtBufferView *` the compiled function
        // produced -- artifact-owned storage -- and the packer takes
        // ownership of it on every path, including its own failures. It
        // therefore takes no `source_name`: nothing it can refuse is a
        // property of the function, only of the allocation.
        //
        // Position: after `buffer_releases`, which the shared emission above
        // already ran. A released *parameter* view and a returned
        // artifact-owned one are disjoint allocations -- the parameter's
        // belongs to the host's exporter and the return's to this artifact --
        // so an export that both takes a `memoryview` and returns one
        // releases the first and hands back the second with no interaction
        // between them.
        //
        // Part 1 of #1175 puts a second provenance in front of it. The
        // `caller_owned` branch is the one case where `result` is *not*
        // artifact-owned storage, and taking this packer on it would free
        // the host's own block. The acquire that sets the flag ran before
        // the releases; only the hand-back is here, because it is a
        // `return` and every release owes its position before one.
        Ty::MemoryView => {
            if !caller_owned_buffer_slots(&export.return_ty, &slots).is_empty() {
                out.push_str(
                    "    if (caller_owned) {\n        return pycc_ext_activation_exit(borrowed);\n    }\n",
                );
            }
            out.push_str("    return pycc_ext_activation_exit(pycc_ext_pack_memoryview(result));\n}\n\n");
        }
        // Part 1 of #1447 (#1449): its own arm for the reason the buffer
        // arm above gives -- `into_scalar` answers `None` for an instance so
        // that `tuple[C]` stays refused, and the generic arm below would
        // panic. The packer reuses #1435's carrier egress, so a returned
        // instance with a live carrier (`self`, or an argument the host
        // still holds) is that very object.
        Ty::Instance(_) => {
            out.push_str("    return pycc_ext_activation_exit(pycc_ext_pack_instance(result));\n}\n\n");
        }
        ty => {
            let (_, helper) = boundary_carrier(ty)
                .and_then(BoundaryCarrier::into_scalar)
                .expect("a carriable scalar return type");
            out.push_str(&format!(
                "    return pycc_ext_activation_exit(pycc_ext_pack_{helper}(result));\n}}\n\n"
            ));
        }
    }
    out
}

/// The C local declarations one generated function needs for its argument
/// slots: `a{index}` per scalar, one `a{index}_{element}` per `tuple`
/// element, and a `Py_buffer b{index}` beside the view pair for a
/// `memoryview`.
///
/// Shared by [`wrapper_for`] and the generated `Py_tp_init`
/// (`method_types_c`) so a constructor and an ordinary export declare the
/// same locals for the same declared type.
pub(crate) fn arg_slot_locals(slots: &[BoundaryCarrier]) -> String {
    let mut out = String::new();
    for (index, slot) in slots.iter().enumerate() {
        match slot {
            BoundaryCarrier::Scalar(c_type, _) => {
                out.push_str(&format!("    {c_type} a{index};\n"));
            }
            // Part 1 of #1447: the compiled instance pointer the carrier
            // holds, which is what `ty_to_basic_type` gives `Ty::Instance`.
            BoundaryCarrier::Instance(_) => out.push_str(&format!("    void *a{index};\n")),
            BoundaryCarrier::Tuple(elements) => {
                for (element, (c_type, _)) in elements.iter().enumerate() {
                    out.push_str(&format!("    {c_type} a{index}_{element};\n"));
                }
            }
            // Two locals, both owned by the generated function for the
            // whole call: the `Py_buffer` the shim acquires (and the
            // function releases on every exit past that point), and the
            // `{ptr, len}` pair that is all the compiled body ever sees of
            // it.
            BoundaryCarrier::Buffer { .. } => {
                out.push_str(&format!("    Py_buffer b{index};\n"));
                out.push_str(&format!("    {BUFFER_VIEW_C_TYPE} a{index};\n"));
            }
        }
    }
    out
}

/// The per-argument ingress: one `pycc_ext_unpack_*` call per slot, each
/// bailing with `fail` after releasing whatever the earlier slots hold.
///
/// `arg_expr` renders the `PyObject *` for argument `index`, because the two
/// callers receive their arguments differently: a `METH_FASTCALL` wrapper
/// gets a `PyObject *const *` vector and indexes it, while the generated
/// `Py_tp_init` gets a real tuple and has to bridge through
/// `PyTuple_GetItem`. Everything else -- which helper, which local, what the
/// bail path owes -- is shared, which is the point: the unpack helper is a
/// function of the declared type alone, so `mod.Grid(True, 4)` and
/// `mod.Grid(3, 4).scale(True)` must admit exactly the same object set for
/// the same declared `int`. `docs/RUNTIME.md` claims one admissibility
/// matrix, not two.
///
/// Emitted inline rather than behind a shared `goto` label: the cleanup
/// differs per argument index, and neither caller has another exit that owes
/// anything at this point. A `tuple` argument owes nothing -- its elements
/// are copied out by value.
pub(crate) fn unpack_args(
    slots: &[BoundaryCarrier],
    source_name: &str,
    arg_expr: &dyn Fn(usize) -> String,
    fail: &str,
) -> String {
    let mut out = String::new();
    for (index, slot) in slots.iter().enumerate() {
        // Each `str` argument already unpacked holds a fresh reference that
        // only the compiled function's own parameter slot ever consumes, and
        // this branch bails before the call -- so release them here, or a
        // `TypeError` on argument 2 would leak argument 1's `PyStrObj` on
        // every raising call.
        let cleanup: String = slots[..index]
            .iter()
            .enumerate()
            .filter_map(|(earlier, carrier)| Some((earlier, carrier.cleanup()?)))
            .map(|(earlier, owed)| match owed {
                SlotCleanup::StrDecref => format!("        pycc_rt_str_decref(a{earlier});\n"),
                SlotCleanup::BufferRelease => format!("        PyBuffer_Release(&b{earlier});\n"),
            })
            .collect();
        let arg = arg_expr(index);
        match slot {
            BoundaryCarrier::Scalar(_, helper) => out.push_str(&format!(
                "    if (pycc_ext_unpack_{helper}({arg}, \"{source_name}\", {index}, &a{index}) \
                 != 0) {{\n{cleanup}{fail}    }}\n"
            )),
            // Part 1 of #1447 (#1449): the scalar call shape plus the class
            // the argument must be an instance of, which the shim checks
            // against the instance's run-time MRO. A refusal leaves nothing
            // owed, so only the earlier slots' cleanup runs.
            BoundaryCarrier::Instance(class) => out.push_str(&format!(
                "    if (pycc_ext_unpack_instance({arg}, \"{source_name}\", {index}, \"{class}\", \
                 &a{index}) != 0) {{\n{cleanup}{fail}    }}\n"
            )),
            BoundaryCarrier::Tuple(elements) => {
                let elements_len = elements.len();
                out.push_str(&format!(
                    "    if (pycc_ext_unpack_tuple({arg}, \"{source_name}\", {index}, \
                     {elements_len}) != 0) {{\n{cleanup}{fail}    }}\n"
                ));
                for (element, (_, helper)) in elements.iter().enumerate() {
                    // `PyTuple_GetItem` cannot fail at this call: the check
                    // just emitted refused every non-tuple and every length
                    // but this one, so the index is always in range.
                    out.push_str(&format!(
                        "    if (pycc_ext_unpack_{helper}_at(PyTuple_GetItem({arg}, \
                         {element}), \"{source_name}\", {index}, {element}, &a{index}_{element}) != 0) \
                         {{\n{cleanup}{fail}    }}\n"
                    ));
                }
            }
            // The shim refuses everything that is not an exact,
            // C-contiguous, one-dimensional `float` buffer and leaves
            // nothing acquired when it does, so this arm owes no cleanup of
            // its own -- only the earlier slots'. On success the caller
            // takes the two words it is allowed to keep: the data pointer,
            // and a *copy* of `shape[0]`. `b{index}.shape` itself is
            // exporter-owned storage that dies at `PyBuffer_Release`, so it
            // is never carried across the boundary.
            BoundaryCarrier::Buffer { writable } => {
                // The sole consumer of Part 1 of #1142's writability bit.
                // `1` asks the shim for `PyBUF_WRITABLE`, which is the only
                // thing that makes the element store this parameter's body
                // performs a write to storage the exporter agreed to share
                // mutably; a read-only exporter is refused here, by
                // CPython's own `BufferError`, rather than scribbled over.
                let writable = i32::from(*writable);
                out.push_str(&format!(
                    "    if (pycc_ext_unpack_memoryview({arg}, \"{source_name}\", {index}, \
                     {writable}, &b{index}) != 0) {{\n{cleanup}{fail}    }}\n"
                ));
                out.push_str(&format!("    a{index}.ptr = b{index}.buf;\n"));
                out.push_str(&format!(
                    "    a{index}.len = (long long)b{index}.shape[0];\n"
                ));
            }
        }
    }
    out
}

/// One `PyBuffer_Release` line per `memoryview` slot, indented with
/// `indent`, or the empty string when the export has none.
///
/// Shared by the two success-path emission points (inside the pending-
/// exception block, and just before the egress) so the two can never
/// release different sets.
/// The declared-parameter indices a **caller-owned** buffer return could
/// name, or empty when this export cannot have one.
///
/// Part 1 of #1175. Empty unless the export both returns the buffer type and
/// takes at least one `memoryview` parameter, which is the conjunction that
/// keeps every wrapper generated before this change byte-identical: an
/// export with no buffer parameter has no `&a{index}` for `result` to equal,
/// and one that does not return a buffer never reaches the egress arm.
///
/// The index is the *declared* parameter index and therefore also the
/// `args[index]` index: a receiver is pushed into `call_args` separately
/// (see [`wrapper_for`]) and never occupies a slot, so an instance method's
/// first declared parameter is `args[0]` here exactly as a module-level
/// function's is.
fn caller_owned_buffer_slots(return_ty: &Ty, slots: &[BoundaryCarrier]) -> Vec<usize> {
    if !matches!(return_ty, Ty::MemoryView) {
        return Vec::new();
    }
    slots
        .iter()
        .enumerate()
        .filter(|(_, carrier)| matches!(carrier, BoundaryCarrier::Buffer { .. }))
        .map(|(index, _)| index)
        .collect()
}

/// Part 1 of #1175: the provenance test that decides whether a returned
/// `PyccExtBufferView *` is the host's own buffer rather than artifact-owned
/// storage, and, when it is, acquires the view the host will receive.
///
/// **Why a runtime pointer identity test.** `&a{index}` are distinct locals
/// in this generated function's own live stack frame, while artifact-owned
/// storage is a heap block from `pycc_rt_buffer_f64_alloc`, which cannot
/// alias a live stack frame. The test is therefore exact rather than
/// heuristic: no match means artifact-owned and the existing packer runs.
/// Two parameters bound to the *same* host object still have distinct
/// `&a{index}`, so the chain picks the index the body actually returned; and
/// a function that branches -- `return b0` on one path, `return b1` on
/// another -- is admitted for free, where a checker-computed "returns
/// parameter i" tag would have had to refuse it with no memory-safety
/// ground.
///
/// **Why it is emitted here, before the releases.** This is the only pack on
/// the buffer path that re-enters the argument *object*:
/// `PyMemoryView_FromObject` acquires a second, independent buffer export on
/// it. `pycc_ext_unpack_memoryview` admits any `PyObject_CheckBuffer` object
/// with no type allowlist, including a PEP 688 Python-level exporter, which
/// the shim's `Py_LIMITED_API 0x030D0000` floor permits. For such an
/// argument `PyBuffer_Release(&b{index})` runs `__release_buffer__`, and at
/// that instant the host object has **zero** outstanding exports and may
/// legally reallocate its storage -- so a release-then-acquire ordering can
/// hand the host a view over storage the compiled body never touched.
/// Acquiring first keeps at least one export outstanding across the whole
/// boundary, which is the configuration the mechanism was measured in.
///
/// Nothing here frees or releases anything: `args[index]` is a borrowed
/// reference the caller holds for the call's duration, and the view carries
/// its own export.
fn caller_owned_buffer_acquire(
    return_ty: &Ty,
    slots: &[BoundaryCarrier],
    source_name: &str,
    buffer_slice_out: bool,
) -> String {
    let indices = caller_owned_buffer_slots(return_ty, slots);
    if indices.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "    /*\n     * Part 1 of #1175: `result` is the host's own buffer when it is one of\n     \
         * this frame's own `a{i}` locals -- artifact-owned storage is a heap block\n     * and \
         cannot alias a live stack frame. Acquired before the releases below so\n     * the host \
         object is never at zero outstanding exports across the boundary.\n     */\n",
    );
    for (position, index) in indices.iter().enumerate() {
        let lead = if position == 0 { "if" } else { "} else if" };
        // The packer re-reads the exporter, so it is handed the window this
        // call actually operated on and the writability the unpack demanded:
        // a PEP 688 exporter may legally answer the second `__buffer__` with
        // a different window, and only `b{index}` says which one is right.
        let writable = i32::from(matches!(
            slots[*index],
            BoundaryCarrier::Buffer { writable: true }
        ));
        // Part 2 of #1175 (#1179): the sub-range is derived host-side,
        // *after* Part 1's whole-window PEP 688 `held`-vs-probe check,
        // which therefore still runs against the window this call actually
        // operated on. `slice_present` is `0` unless the `return` that
        // reached here was a sub-range one, so an export that also contains
        // a bare `return b` takes Part 1's path on that branch unchanged.
        let borrowed = if buffer_slice_out {
            format!(
                "slice_present\n            ? pycc_ext_pack_memoryview_borrowed_slice(\
                 args[{index}], &b{index}, \"{source_name}\", {index}, {writable}, \
                 slice_start, slice_stop)\n            : \
                 pycc_ext_pack_memoryview_borrowed(args[{index}], &b{index}, \
                 \"{source_name}\", {index}, {writable})"
            )
        } else {
            format!(
                "pycc_ext_pack_memoryview_borrowed(args[{index}], &b{index}, \
                 \"{source_name}\", {index}, {writable})"
            )
        };
        out.push_str(&format!(
            "    {lead} (result == &a{index}) {{\n        caller_owned = 1;\n        \
             borrowed = {borrowed};\n"
        ));
    }
    out.push_str("    }\n");
    out
}

pub(crate) fn buffer_releases(slots: &[BoundaryCarrier], indent: &str) -> String {
    slots
        .iter()
        .enumerate()
        .filter(|(_, carrier)| carrier.cleanup() == Some(SlotCleanup::BufferRelease))
        .map(|(index, _)| format!("{indent}PyBuffer_Release(&b{index});\n"))
        .collect()
}

/// The C parameter-type list of the call a wrapper makes into the compiled
/// program: every declared parameter flattened to the slots it occupies,
/// then one out-pointer per element of a returned `tuple`.
///
/// `"void"` and not `""` for the empty list, because an empty C parameter
/// list means "unspecified", not "none". The rule applies to the *combined*
/// list: only a nullary export with no `tuple` return has one.
pub(crate) fn c_param_list(
    slots: &[BoundaryCarrier],
    out_slots: &[(&'static str, &'static str)],
    buffer_slice_out: bool,
) -> String {
    let mut types: Vec<String> = Vec::new();
    for slot in slots {
        match slot {
            BoundaryCarrier::Scalar(c_type, _) => types.push((*c_type).to_string()),
            BoundaryCarrier::Instance(_) => types.push("void *".to_string()),
            BoundaryCarrier::Tuple(elements) => {
                types.extend(elements.iter().map(|(c_type, _)| (*c_type).to_string()));
            }
            BoundaryCarrier::Buffer { .. } => types.push(format!("{BUFFER_VIEW_C_TYPE} *")),
        }
    }
    types.extend(out_slots.iter().map(|(c_type, _)| format!("{c_type} *")));
    // Part 2 of #1175 (#1179): extended, never narrowed. A sub-range
    // egress's compiled function really does take these three trailing
    // pointers, so omitting them from the `extern` declaration and the cast
    // would be exactly the silent ABI mismatch `pycc_codegen::ext`'s own
    // SIGBUS note records -- undiagnosable by either compiler.
    if buffer_slice_out {
        types.extend(std::iter::repeat_n("long long *".to_string(), 3));
    }
    if types.is_empty() {
        "void".to_string()
    } else {
        types.join(", ")
    }
}

/// The egress of a `tuple`-returning wrapper (#1050): pack every element,
/// then build the tuple.
///
/// Each `r{index}` arrives **borrowed**, not retained. D-180 rule 6 retains
/// at a `return` only where the returned value is a scalar: codegen's
/// `MirStmt::Return` routes the value through `retain_if_int_duplicate`,
/// which acts on a `Scalar::Int` and does nothing for an aggregate, and the
/// `pycc_ext_thunk_` emitter then `extractvalue`s each field straight into
/// its out-pointer. So returning a *stored* tuple (`saved = (2**62,)`;
/// `return saved`) hands this function a word the module global still owns.
/// `pycc_ext_pack_int` discharges one reference on its `OverflowError` path,
/// which without a matching retain here decrements a count this wrapper
/// never took -- a refcount underflow, and on the next call a use-after-free
/// in the host interpreter.
///
/// So each `int` element takes its own reference with `pycc_rt_bigint_retain`
/// immediately before the packer that discharges it. Retain and release share
/// one predicate (`classify_encoded_int(word) == BigInt`), so the pairing is
/// exactly balanced on a smallint, a bool marker, the word `0`, and an
/// unclassifiable word alike -- and the retain is emitted from the same
/// `helper == "int"` arm as the packer, so the two can never drift apart.
///
/// Every element is packed *unconditionally*, before any failure is acted
/// on, and that ordering is the refcount discipline rather than a style
/// choice. Bailing out at the first failing element would leave every later
/// element's word undischarged, leaking one `BigIntObj` per call on exactly
/// the path that already raises.
///
/// The cost is that when two `int` elements both overflow, the second
/// `PyErr_Format` replaces the first. Both carry the same message text and
/// the same exception type, so the observable difference is nil, and
/// replacing a pending exception is well-defined in CPython -- unlike
/// dropping an owned word.
///
/// `PyTuple_New` is reached only once every element is packed, so its own
/// failure path has a fixed, fully-owned set to release.
///
/// Every exit returns through `pycc_ext_activation_exit` (Part 1 of #1499):
/// emitted only into `wrapper_for`, after its watermark mark, so the
/// wrapper stays a live activation until the tuple -- or the failure's
/// element releases, which can run finalizers -- is done.
pub(crate) fn pack_tuple_return(name: &str, out_slots: &[(&'static str, &'static str)]) -> String {
    let mut out = String::new();
    for (index, (_, helper)) in out_slots.iter().enumerate() {
        let argument = if *helper == "int" {
            out.push_str(&format!("    pycc_rt_bigint_retain(r{index});\n"));
            format!("\"{name}\", r{index}")
        } else {
            format!("r{index}")
        };
        out.push_str(&format!(
            "    e{index} = pycc_ext_pack_{helper}({argument});\n"
        ));
    }
    let arity = out_slots.len();
    let any_null = (0..arity)
        .map(|index| format!("e{index} == NULL"))
        .collect::<Vec<_>>()
        .join(" || ");
    let x_release: String = (0..arity)
        .map(|index| format!("        Py_XDECREF(e{index});\n"))
        .collect();
    out.push_str(&format!(
        "    if ({any_null}) {{\n{x_release}        return pycc_ext_activation_exit(NULL);\n    }}\n"
    ));
    let release: String = (0..arity)
        .map(|index| format!("        Py_DECREF(e{index});\n"))
        .collect();
    out.push_str(&format!(
        "    packed = PyTuple_New({arity});\n    if (packed == NULL) {{\n{release}        \
         return pycc_ext_activation_exit(NULL);\n    }}\n"
    ));
    out.push_str(
        "    /* Every index is in range and `packed` is a fresh tuple, so each\n       \
         PyTuple_SetItem succeeds and steals its element reference. */\n",
    );
    for index in 0..arity {
        out.push_str(&format!(
            "    PyTuple_SetItem(packed, {index}, e{index});\n"
        ));
    }
    out.push_str("    return pycc_ext_activation_exit(packed);\n}\n\n");
    out
}

/// The first statement of every generated C function that calls compiled
/// code through `fnptr_<symbol>` (#1199): a `NameError` instead of a call
/// through a null pointer while the slot is unbound.
///
/// Per-definition publication (`pycc_ext_publish`) never exposes a wrapper
/// before its slot is stored, so this is defence in depth against a path
/// that reaches one anyway -- a module whose body failed part-way and was
/// kept alive by a reference the host took earlier, or a class published
/// with its bases' methods before its own statement ran. It precedes
/// argument unpacking, instance allocation and the error-bridge mark, so a
/// refused call has nothing to release. `fail` is the function's own
/// error return (`NULL` for a method wrapper, `-1` for `tp_init`).
pub(crate) fn fnptr_null_guard(symbol: &str, source_name: &str, fail: &str) -> String {
    format!(
        "    if (fnptr_{symbol} == NULL) {{\n        PyErr_SetString(PyExc_NameError, \
         \"name '{source_name}' is not defined\");\n        return {fail};\n    }}\n"
    )
}
