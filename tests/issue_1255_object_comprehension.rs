//! Part 1 of #1255: a list or set comprehension whose iterable is a CPython
//! object expression, producing a CPython `list` or `set`, in a module body
//! and in a function body.
//!
//! Every hosted test builds the source with `pycc build --ext`, imports the
//! artifact into the host CPython, and compares its output with CPython's
//! own run of the same source. The hosted tests are `#[ignore]`d and
//! contribute no line coverage; the Tier-1 `native-build-test` leg runs them
//! with `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered by the unit tests in `crates/pycc_hir`, in
//! `crates/pycc_types/src/comprehension/object_tests.rs`,
//! `crates/pycc_mir/src/tests/object_comprehension.rs` and
//! `crates/pycc_codegen/src/tests/object_comprehension.rs`.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn host_python() -> Command {
    let mut python =
        Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()));
    python.env("PYTHONIOENCODING", "utf-8");
    python
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

fn write(dir: &Path, file: &str, body: &str) -> PathBuf {
    let path = dir.join(file);
    std::fs::write(&path, body).expect("write the fixture source");
    path
}

fn check_with(dir: &Path, body: &str) -> Output {
    pycc()
        .arg("check")
        .arg(write(dir, "m.py", body))
        .output()
        .expect("pycc should spawn")
}

/// `pycc check` of `body` fails with exactly one `code` diagnostic whose
/// text contains `needle`.
fn assert_one_error(tag: &str, body: &str, code: &str, needle: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    let output = check_with(&dir, body);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert_eq!(rendered.matches("error[").count(), 1, "{rendered}");
    assert!(rendered.contains(&format!("error[{code}]")), "{rendered}");
    assert!(rendered.contains(needle), "{rendered}");
}

/// The module-body shapes, which `pycc check` (a native-mode check) accepts
/// because every object in them comes from a foreign import.
const MODULE: &str = "import builtins\n\
    \n\
    table = builtins.dict.fromkeys(builtins.str(\"Ab CD ef GH\").split())\n\
    upper = [s for s in table.keys() if s.isupper()]\n\
    print(upper)\n\
    lowered = {s.lower() for s in table.keys()}\n\
    print(builtins.sorted(lowered))\n\
    print([len(s) + 1 for s in table.keys() if len(s) + 0])\n\
    print(builtins.sorted({s[0] for s in builtins.iter(table)}))\n\
    print(len([1.5 for s in table.values()]))\n\
    print([True for s in builtins.range(0)])\n";

/// What `MODULE` prints under CPython.
const MODULE_OUT: &str = "['CD', 'GH']\n['ab', 'cd', 'ef', 'gh']\n[3, 3, 3, 3]\n\
    ['A', 'C', 'G', 'e']\n4\n[]\n";

/// `MODULE` plus the function-body shapes. `Parser.expected` is lark
/// `lalr_parser_state.py` line 79 in its `try`/`except KeyError` context:
/// `{s for s in states[state].keys() if s.isupper()}`.
///
/// `nested` builds an object comprehension whose element is another one,
/// and the module-body loop calls `count_upper` three times, so a function
/// body's comprehension runs again on each call.
///
/// `catch` covers the raising paths inside a function body: `iter()` of a
/// non-iterable, an `__iter__` that raises, a `__next__` that raises
/// mid-iteration (`int("x")` from `map`), a raising filter, a raising
/// element, and a `set.add` refusing an unhashable item.
fn hosted_source() -> String {
    format!(
        "{MODULE}\
        ns = builtins.dict()\n\
        builtins.exec(\"class BadIter:\\n    def __iter__(self):\\n        raise KeyError('iter')\\n\", ns)\n\
        \n\
        \n\
        class Parser:\n    \
        def expected(self, states: object, state: int, token: str) -> object:\n        \
        try:\n            pair = states[state][token]\n        \
        except KeyError:\n            \
        expected = {{s for s in states[state].keys() if s.isupper()}}\n            \
        return builtins.sorted(expected)\n        \
        return pair\n\
        \n\
        \n\
        def keys_of(t: object) -> object:\n    \
        return [k for k in t.keys()]\n\
        \n\
        \n\
        def count_upper(t: object) -> int:\n    \
        found = {{k for k in t.keys() if k.isupper()}}\n    \
        return len(found)\n\
        \n\
        \n\
        def nested(t: object) -> object:\n    \
        return [[c for c in k.lower()] for k in t.keys() if k.isupper()]\n\
        \n\
        \n\
        def catch() -> None:\n    \
        try:\n        print([k for k in builtins.len])\n    except TypeError:\n        print(\"TypeError\")\n    \
        try:\n        print([k for k in ns[\"BadIter\"]()])\n    except KeyError:\n        print(\"KeyError\")\n    \
        try:\n        print([k for k in builtins.map(builtins.int, builtins.list(\"1x\"))])\n    except ValueError:\n        print(\"ValueError\")\n    \
        try:\n        print([k for k in table.keys() if k.missing()])\n    except AttributeError:\n        print(\"AttributeError\")\n    \
        try:\n        print({{k.missing() for k in table.keys()}})\n    except AttributeError:\n        print(\"AttributeError\")\n    \
        try:\n        print({{k for k in builtins.eval(\"[[1], [2]]\")}})\n    except TypeError:\n        print(\"TypeError\")\n\
        \n\
        \n\
        states = builtins.dict.fromkeys(builtins.range(2), table)\n\
        print(Parser().expected(states, 1, \"zz\"))\n\
        print(Parser().expected(states, 1, \"Ab\"))\n\
        print(keys_of(table))\n\
        print(count_upper(table))\n\
        print(nested(table))\n\
        total = 0\n\
        for i in range(3):\n    \
        total = total + count_upper(table)\n\
        print(total)\n\
        catch()\n"
    )
}

#[test]
fn check_accepts_the_module_shapes() {
    let dir = ScratchDir::new("obj_comp_check").expect("scratch");
    let output = check_with(&dir, MODULE);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
}

/// The shapes Part 1 does not admit keep a diagnostic, never a panic.
#[test]
fn the_shapes_outside_part_1_are_refused() {
    const HEAD: &str = "import builtins\n\ntable = builtins.__dict__\n";
    for (tag, tail, code, needle) in [
        (
            "obj_comp_bare_name",
            "xs = [k for k in table]\n",
            "I0404",
            "a list or set comprehension over it unless it is a bare name",
        ),
        (
            "obj_comp_dict",
            "d = {k: 1 for k in table.keys()}\n",
            "I0404",
            "a dict comprehension over a CPython object",
        ),
        (
            "obj_comp_unpackable_list",
            "xs = [[1] for k in table.keys()]\n",
            "I0404",
            "collecting a `list[int]` element into a CPython list",
        ),
        (
            "obj_comp_unpackable_set",
            "xs = {None for k in table.keys()}\n",
            "I0404",
            "collecting a `None` element into a CPython set",
        ),
        (
            "obj_comp_native_expr",
            "xs = [k for k in [1, 2]]\n",
            "C0001",
            "a CPython object is supported so far as a comprehension's iterable, got an expression of type `list[int]`",
        ),
    ] {
        assert_one_error(tag, &format!("{HEAD}{tail}"), code, needle);
    }
    // Admitting an object iterable moved the native-expression refusal from
    // HIR lowering (which had the iterable's span) to the type stage, whose
    // diagnostics carry no span yet: it is reported at 1:1, the same trade
    // Part 2b of #1371 made, until Part 5 of #1371 gives type-stage
    // diagnostics their source location. Pinned so the restoration is a
    // visible test change.
    let dir = ScratchDir::new("obj_comp_native_expr_span").expect("scratch");
    let rendered = stdout_of(&check_with(
        &dir,
        &format!("{HEAD}xs = [k for k in [1, 2]]\n"),
    ));
    assert!(rendered.contains(".py:1:1"), "{rendered}");
    assert!(!rendered.contains(".py:4:"), "{rendered}");
}

fn build_ext(dir: &Path, module: &str, body: &str) {
    let build = pycc()
        .arg("build")
        .arg(write(dir, "m.py", body))
        .arg("-o")
        .arg(dir.join(module))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
}

fn python(dir: &Path, script: &str) -> Output {
    host_python()
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn")
}

fn assert_ok(run: &Output) {
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(run),
        stderr_of(run)
    );
}

/// Runs `target` and prints what it raised, by type only, after whatever
/// the module body printed.
fn raised_report(target: &str) -> String {
    format!(
        "import runpy\n\
         try:\n\
         \x20   {target}\n\
         except Exception as e:\n\
         \x20   print(type(e).__name__)\n\
         else:\n\
         \x20   print('no error')\n"
    )
}

/// Builds `body`, imports the extension and returns its report.
fn compiled_report(tag: &str, module: &str, body: &str) -> (ScratchDir, String) {
    let dir = ScratchDir::new(tag).expect("scratch");
    build_ext(&dir, module, body);
    let compiled = python(&dir, &raised_report(&format!("import {module}")));
    assert_ok(&compiled);
    let out = stdout_of(&compiled);
    (dir, out)
}

/// Builds `body`, then asserts the extension's import report matches
/// CPython's own run of the same source, and returns it.
fn assert_matches_cpython(tag: &str, module: &str, body: &str) -> String {
    let (dir, compiled) = compiled_report(tag, module, body);
    let oracle = python(&dir, &raised_report("runpy.run_path('m.py')"));
    assert_ok(&oracle);
    assert_eq!(compiled, stdout_of(&oracle));
    compiled
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn comprehensions_over_objects_behave_like_cpython_in_the_host() {
    let out = assert_matches_cpython("obj_comp_hosted", "pycc_obj_comp_mod", &hosted_source());
    assert_eq!(
        out,
        format!(
            "{MODULE_OUT}['CD', 'GH']\nNone\n['Ab', 'CD', 'ef', 'GH']\n2\n\
             [['c', 'd'], ['g', 'h']]\n6\nTypeError\nKeyError\n\
             ValueError\nAttributeError\nAttributeError\nTypeError\nno error\n"
        )
    );
}

/// An error raised while iterating, filtering, packing or collecting
/// propagates out of the module body like any other object-operation error.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_raising_comprehension_propagates_from_the_module_body() {
    for (tag, module, line, raised) in [
        (
            "obj_comp_raise_iter",
            "pycc_obj_comp_raise_iter",
            "print([k for k in builtins.len])\n",
            "TypeError",
        ),
        (
            "obj_comp_raise_next",
            "pycc_obj_comp_raise_next",
            "print([k for k in builtins.map(builtins.int, builtins.list(\"1x\"))])\n",
            "ValueError",
        ),
        (
            "obj_comp_raise_cond",
            "pycc_obj_comp_raise_cond",
            "print({k for k in builtins.list(\"ab\") if k.missing()})\n",
            "AttributeError",
        ),
        (
            "obj_comp_raise_elt",
            "pycc_obj_comp_raise_elt",
            "print([k.missing() for k in builtins.list(\"ab\")])\n",
            "AttributeError",
        ),
        (
            "obj_comp_raise_collect",
            "pycc_obj_comp_raise_collect",
            "print({k for k in builtins.eval(\"[[1], [2]]\")})\n",
            "TypeError",
        ),
    ] {
        let body = format!("import builtins\n\n{line}print(\"unreached\")\n");
        let out = assert_matches_cpython(tag, module, &body);
        assert_eq!(out, format!("{raised}\n"), "{tag}");
    }
}

/// Reference-count pins on a *mortal* item and on the iterated source, at
/// two trip counts. Under the #1092 leak-only rule (`docs/RUNTIME.md`) a
/// list comprehension of `n` items leaves each item `+2` (the leaked loop
/// item plus the leaked list's own reference), and a set comprehension of
/// `n` identical items `n + 1` (each leaked loop item plus the one the set
/// holds); the source list itself is unchanged, since the only thing that
/// referenced it is the list iterator, released since Part 3 of #1092 (and
/// CPython makes an exhausted one drop its sequence regardless). The deltas scale exactly with `n`, so a helper that
/// consumed a packed element twice, or not at all, moves them. CPython
/// frees everything (`n 1 0` per line), so this report is compiled-only.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_leak_only_reference_counts_are_pinned() {
    let body = "import builtins\n\
        import sys\n\
        \n\
        \n\
        def pin(n: int) -> None:\n    \
        probe = builtins.object()\n    \
        src = builtins.list(builtins.dict.fromkeys(builtins.range(n), probe).values())\n    \
        before_p = int(sys.getrefcount(probe))\n    \
        before_s = int(sys.getrefcount(src))\n    \
        xs = [x for x in src.__iter__() if x]\n    \
        after_list = int(sys.getrefcount(probe))\n    \
        ys = {x for x in src.__iter__()}\n    \
        after_set = int(sys.getrefcount(probe))\n    \
        print(after_list - before_p, after_set - after_list, int(sys.getrefcount(src)) - before_s)\n\
        \n\
        \n\
        pin(100)\n\
        pin(200)\n";
    let (_dir, out) = compiled_report("obj_comp_pin", "pycc_obj_comp_pin", body);
    assert_eq!(out, "200 101 0\n400 201 0\nno error\n");
}
