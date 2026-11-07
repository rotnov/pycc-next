//! A guessed specialization of an unannotated function
//! (`pycc_ast::specialize_untyped`) keeps the argument objects CPython
//! would hand back: a `str` or `int` parameter returned, stored or passed
//! on unchanged (or through an operation CPython may answer with its
//! operand, `s[0:]`) is the argument itself, not a re-boxed copy of its
//! native value. A parameter the body binds again and whose value may
//! leave is not specialized; one whose every use is consumed stays native.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn python() -> std::ffi::OsString {
    std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into())
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).replace('\r', "")
}

const MODULE: &str = "\
def keep_text(s):
    n = 0
    for i in range(len(s)):
        n += 1
    return s


def tail_text(s):
    n = 0
    for i in range(len(s)):
        n += 1
    return s[0:]


def stored(s):
    out = []
    for i in range(len(s)):
        out.append(s[i])
    out.append(s)
    return out


def keep_int(k):
    t = 0
    for i in range(k % 3):
        t += i
    return k


def plus_int(k):
    t = 0
    for i in range(k % 3):
        t += i
    return +k


def rebound_tail(text, x):
    for i in range(x):
        pass
    text = text[x:]
    return text


def rebound_consumed(text, x):
    n = 0
    while len(text) > x % 7:
        text = text[1:]
        n += 1
    return n


def rebound_other(text, x):
    for i in range(len(text) + x % 2):
        pass
    for text in [x, x]:
        pass
    return text


class Box:
    def __init__(self):
        self.base = 0

    def keep(self, s):
        n = self.base
        for i in range(len(s)):
            n += 1
        return s
";

const CALLS: &str = "\
import sys
sys.path.insert(0, sys.argv[1])
import m

def show(f, *args):
    try:
        print(f.__name__, repr(f(*args)))
    except Exception as e:
        print(f.__name__, type(e).__name__, e)

text = ''.join(['spe', 'cial'])
big = 2 ** 40 + 1
print('text', m.keep_text(text) is text, m.tail_text(text) is text, m.stored(text)[-1] is text)
print('int', m.keep_int(big) is big, m.plus_int(big) is big, m.keep_int(5))
print('rebound', m.rebound_tail(text, 0) is text, m.rebound_tail(text, 3), m.rebound_consumed(text, 2))
show(m.rebound_other, 'abc', 7)
show(m.rebound_other, 'abc', 8)
print('method', m.Box().keep(text) is text, m.Box().keep('ab'))
show(m.keep_text, 5)
show(m.keep_int, 'a')
";

fn run(dir: &Path) -> String {
    let o = Command::new(python()).args(["-I", "-c", CALLS]).arg(dir).output().unwrap();
    assert!(o.status.success(), "{}", text(&o));
    text(&o)
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle and Python.h (PYCC_PYTHON, default python3)"]
fn a_guessed_parameter_that_leaves_is_the_argument_itself() {
    let scratch = ScratchDir::new("untyped_identity").unwrap();
    let (ext, src) = (scratch.join("ext"), scratch.join("src"));
    std::fs::create_dir_all(&ext).unwrap();
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("m.py"), MODULE).unwrap();
    let source = scratch.join("m.py");
    std::fs::write(&source, MODULE).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["build".as_ref(), source.as_os_str(), "-o".as_ref(), ext.join("m").as_os_str()])
        .args(["--ext", "--non-strict", "--select", "module", "--report-fallbacks"])
        .output()
        .unwrap();
    let report = text(&o);
    assert!(o.status.success(), "{report}");
    for spec in [
        "`keep_text` runs natively when called with s: str",
        "`tail_text` runs natively when called with s: str",
        "`keep_int` runs natively when called with k: int",
        "`plus_int` runs natively when called with k: int",
        // `text` is bound again and its value leaves: only `x` is native.
        "`rebound_tail` runs natively when called with x: int",
        // Every use of `text` is consumed: it stays native.
        "`rebound_consumed` runs natively when called with text: str, x: int",
        // A loop target binds `text` to values of any type.
        "`rebound_other` runs natively when called with x: int",
        "`Box.keep` runs natively when called with s: str",
    ] {
        assert!(report.contains(&format!("specialized: {spec}\n")), "{spec}\n{report}");
    }
    let out = run(&ext);
    assert_eq!(out, run(&src));
    assert!(out.contains("text True True True\nint True True 5\nrebound True cial 5\n"), "{out}");
    assert!(out.contains("method True ab\n"), "{out}");
}
