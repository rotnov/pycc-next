//! A selected function's compiled body is specialized for the exact types a
//! profile observed its parameters holding (docs/design/compile-little.md,
//! section 2.5): a guard tests them at each call and runs the native copy,
//! and any other argument -- another type, or a subclass of the observed one
//! -- runs the generic compiled body. Both paths must behave as CPython:
//! the same output and the same exceptions.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn python() -> std::ffi::OsString {
    std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into())
}

fn pycc(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pycc")).args(args).output().expect("pycc runs")
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).replace('\r', "")
}

const MODULE: &str = "\
def count(s, c):
    n = 0
    for i in range(len(s)):
        if s[i] == c:
            n += 1
    return n


def scale(xs, k):
    total = 0.0
    for x in xs:
        total += x * k
    return total


def keep_f(x):
    t = 0.0
    for _ in range(3):
        if x == x:
            t += 1.0
    return x


def keep_i(x):
    t = 0
    for _ in range(3):
        if x == x:
            t += 1
    return x


def keep_s(s):
    n = 0
    for _ in s:
        if s == s:
            n += 1
    return s


def seek(x, xs, d):
    n = 0
    for _ in range(2):
        if x == x:
            n += 1
    return x in xs, d.get(x), d[x] if x in d else None, [x], (x, x)


def hashed(x):
    t = 0.0
    for _ in range(2):
        if x == x:
            t += 1.0
    return hash(x)


def added(x, other):
    t = 0.0
    for _ in range(2):
        if x == x:
            t += 1.0
    return x + other


def measured(x, len):
    t = 0.0
    for _ in range(2):
        if x == x:
            t += 1.0
    return len(x)


def matched(x, v):
    t = 0.0
    for _ in range(2):
        if x == x:
            t += 1.0
    match v:
        case len:
            return len(x)


class Echo(Exception):
    def __call__(self, v):
        return v


def caught(x, e):
    t = 0.0
    for _ in range(2):
        if x == x:
            t += 1.0
    try:
        raise e
    except Echo as len:
        return len(x)


def plus(x):
    t = 0
    for _ in range(2):
        if x == x:
            t += 1
    return +x


def same_i(n):
    t = 0
    for _ in range(2):
        if n == n:
            t += 1
    return +n


def stripped(s):
    t = 0
    for _ in range(2):
        t += len(s)
    return s.strip()


def whole(s):
    t = 0
    for _ in range(2):
        t += len(s)
    return s[:]


def tail(s):
    t = 0
    for _ in range(2):
        t += len(s)
    return s + ''


def first(s):
    t = 0
    for _ in range(2):
        t += len(s)
    return s[0]


def parts(s):
    t = 0
    for _ in range(2):
        t += len(s)
    return s.split()


# tomli's scanner: `src` meets only the object `pos` and the object
# `chars`, so a native copy would convert at every step.
def skip(src, pos, chars):
    try:
        while src[pos] in chars:
            pos += 1
    except IndexError:
        pass
    return pos


def last(n, xs):
    for item in xs:
        n = item
    return n
";

/// The training workload: `count` always with two `str`s, `scale` with a
/// `list` and a `float`.
const WORK: &str = "\
from pkg import m
class Keep:
    def __radd__(self, other):
        return other
for i in range(1200):
    m.count('abcab' * 3, 'a')
    m.scale([1.0, 2.0, 3.0], 0.5)
    m.keep_f(1.5)
    m.keep_i(7)
    m.keep_s('abc')
    m.seek(2.5, [2.5], {2.5: 1})
    m.last(0, [1, 2])
    m.hashed(2.5)
    m.added(2.5, Keep())
    m.measured(2.5, lambda v: v)
    m.matched(2.5, lambda v: v)
    m.caught(2.5, m.Echo())
    m.plus(2.5)
    m.same_i(7)
    m.stripped('abc')
    m.whole('abc')
    m.tail('abc')
    m.first('abc')
    m.parts('abc')
    m.skip('  x', 0, frozenset(' '))
";

/// Calls on both paths: the observed exact types, other types, a `str`
/// subclass that changes indexing, and arguments that raise.
const CALLS: &str = "\
import sys
sys.path.insert(0, sys.argv[1])
from pkg import m

class Loud(str):
    def __getitem__(self, i):
        return str.__getitem__(self, i).upper()

class Weird(str):
    def __len__(self):
        return 2

def show(f, *args):
    try:
        print(f.__name__, repr(f(*args)))
    except Exception as e:
        print(f.__name__, type(e).__name__, e)

show(m.count, 'banana', 'a')
show(m.count, '', 'a')
show(m.count, ['a', 'b', 'a'], 'a')
show(m.count, ('a', 1, 'a'), 1)
show(m.count, Loud('banana'), 'A')
show(m.count, Loud('banana'), 'a')
show(m.count, Weird('aaaa'), 'a')
show(m.count, 'banana', Loud('a'))
show(m.count, 5, 'a')
show(m.count, None, 'a')
show(m.count, 'abc', None)
show(m.scale, [1.0, 2.0], 2.0)
show(m.scale, [1, 2], 3)
show(m.scale, 'ab', 2)
show(m.scale, [1.0], 'x')
show(m.scale, None, 1.0)

# A specialized parameter that leaves unchanged is the argument itself.
nan = float('nan')
print('nan', [m.keep_f(nan)] == [nan], m.keep_f(nan) is nan, m.keep_f(2.5))
big = 2 ** 40 + 1
print('big', m.keep_i(big) is big, m.keep_i(5))
huge = 10 ** 30
show(m.keep_i, huge)
text = ''.join(['spe', 'cial'])
print('str', m.keep_s(text) is text, m.keep_s('x'))
found, got, item, listed, pair = m.seek(nan, [nan], {nan: 'n'})
print('seek', found, got, item, listed == [nan], pair[0] is nan, pair[1] is nan)
show(m.seek, 2.5, [2.5], {2.5: 1})
# A NaN hashes by identity; a reflected method and a parameter named
# `len` receive the argument itself.
class Keep:
    def __radd__(self, other):
        return other
print('hash', m.hashed(nan) == hash(nan), m.hashed(2.5) == hash(2.5))
print('radd', m.added(nan, Keep()) is nan, m.added(big + 0.5, Keep()) == big + 0.5, m.added(1.5, 1.0))
print('len', m.measured(nan, lambda v: v) is nan, m.measured(text, len))
# So does a `len` bound by a `case` capture or an `except ... as`.
print('case', m.matched(nan, lambda v: v) is nan, m.matched(text, len))
print('except', m.caught(nan, m.Echo()) is nan, m.caught(2.5, m.Echo()))
# An operation CPython may answer with its operand itself: the argument.
print('plus', [m.plus(nan)] == [nan], m.plus(nan) is nan, m.same_i(big) is big, m.plus(-2.5))
print('self', m.stripped(text) is text, m.whole(text) is text, m.tail(text) is text, m.parts(text)[0] is text)
print('item', [m.first(c) is c for c in [''.join(['a', '']), chr(233), chr(8364), 'xy']])
show(m.skip, '  \tx', 0, frozenset(' \t'))
show(m.skip, '   ', 1, frozenset(' '))
show(m.skip, Loud('ab'), 0, frozenset('A'))
show(m.skip, 'ab', 'x', frozenset('a'))
# A parameter bound again in the body is not held to its observed type.
show(m.last, 0, ['ok'])
show(m.last, 0, [1, 2])
show(m.last, 0, [])
";

/// A package `pkg` with `m`, profiled (no pass 3) and built with `count`
/// and `scale` selected and specialized for the observed types; returns the
/// built directory, the plain one, and the build's report.
fn built(scratch: &Path) -> (std::path::PathBuf, std::path::PathBuf, String) {
    let mut dirs = Vec::new();
    for name in ["built", "plain"] {
        let pkg = scratch.join(name).join("pkg");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("__init__.py"), "").unwrap();
        std::fs::write(pkg.join("m.py"), MODULE).unwrap();
        dirs.push(scratch.join(name));
    }
    let work = scratch.join("work.py");
    std::fs::write(&work, WORK).unwrap();
    let profile = scratch.join("p.json");
    let pkg = dirs[0].join("pkg");
    let o = pycc(&[
        "profile".as_ref(),
        pkg.as_os_str(),
        "-o".as_ref(),
        profile.as_os_str(),
        "--no-verify".as_ref(),
        "--".as_ref(),
        &python(),
        work.as_os_str(),
    ]);
    assert!(o.status.success(), "{}", text(&o));
    let out = pkg.join("__init__");
    let o = pycc(&[
        "build".as_ref(),
        pkg.as_os_str(),
        "-o".as_ref(),
        out.as_os_str(),
        "--ext".as_ref(),
        "--non-strict".as_ref(),
        "--report-fallbacks".as_ref(),
        "--select".as_ref(),
        "none".as_ref(),
        "--compile".as_ref(),
        "pkg.m:count".as_ref(),
        "--compile".as_ref(),
        "pkg.m:scale".as_ref(),
        "--compile".as_ref(),
        "pkg.m:keep_f".as_ref(),
        "--compile".as_ref(),
        "pkg.m:keep_i".as_ref(),
        "--compile".as_ref(),
        "pkg.m:keep_s".as_ref(),
        "--compile".as_ref(),
        "pkg.m:seek".as_ref(),
        "--compile".as_ref(),
        "pkg.m:last".as_ref(),
        "--compile".as_ref(),
        "pkg.m:hashed".as_ref(),
        "--compile".as_ref(),
        "pkg.m:added".as_ref(),
        "--compile".as_ref(),
        "pkg.m:measured".as_ref(),
        "--compile".as_ref(),
        "pkg.m:matched".as_ref(),
        "--compile".as_ref(),
        "pkg.m:caught".as_ref(),
        "--compile".as_ref(),
        "pkg.m:plus".as_ref(),
        "--compile".as_ref(),
        "pkg.m:same_i".as_ref(),
        "--compile".as_ref(),
        "pkg.m:stripped".as_ref(),
        "--compile".as_ref(),
        "pkg.m:whole".as_ref(),
        "--compile".as_ref(),
        "pkg.m:tail".as_ref(),
        "--compile".as_ref(),
        "pkg.m:first".as_ref(),
        "--compile".as_ref(),
        "pkg.m:parts".as_ref(),
        "--compile".as_ref(),
        "pkg.m:skip".as_ref(),
        "--types-profile".as_ref(),
        profile.as_os_str(),
    ]);
    assert!(o.status.success(), "{}", text(&o));
    (dirs.remove(0), dirs.remove(0), text(&o))
}

fn run(dir: &Path) -> String {
    let o = Command::new(python()).args(["-I", "-c", CALLS]).arg(dir).output().unwrap();
    assert!(o.status.success(), "{}", text(&o));
    text(&o)
}

#[test]
fn a_specialized_body_and_its_guard_failures_behave_as_cpython() {
    let scratch = ScratchDir::new("select_types").unwrap();
    let (built, plain, report) = built(&scratch);
    // `count` is specialized for its two `str`s. `scale`'s list is no type
    // a native body takes, and its float only ever meets the list's object
    // items, so it is not specialized either; nor is `skip`'s `str`, which
    // meets only the object `pos` and `chars`.
    assert!(report.contains("compiled: pkg.m:count"), "{report}");
    assert!(report.contains("specialized: `count` runs natively when called with s: str, c: str"), "{report}");
    for (f, p, ty) in [("scale", "k", "float"), ("skip", "src", "str")] {
        assert!(
            report.contains(&format!(
                "not specialized: `{f}`: parameter `{p}` (observed `{ty}`) meets no native operand in a loop"
            )),
            "{report}"
        );
        assert!(!report.contains(&format!("specialized: `{f}` runs natively")), "{report}");
    }
    assert!(report.contains("specialized: `keep_f` runs natively when called with x: float"), "{report}");
    assert!(report.contains("specialized: `keep_s` runs natively when called with s: str"), "{report}");
    // `matched` is not compiled (the checker does not yet take a `case`
    // capture as a binding: T0021), so it runs as CPython runs it; the
    // pycc_ast tests check its specialization would read `x`'s object.
    for (f, p) in [("plus", "x: float"), ("same_i", "n: int"), ("stripped", "s: str"), ("whole", "s: str")] {
        assert!(report.contains(&format!("specialized: `{f}` runs natively when called with {p}")), "{report}");
    }
    for f in ["hashed", "added", "measured", "caught"] {
        assert!(report.contains(&format!("specialized: `{f}` runs natively when called with x: float")), "{report}");
    }
    assert!(
        report.contains("not specialized: `last`: parameter `n` (observed `int`) may be bound again in the body"),
        "{report}"
    );
    let out = run(&built);
    assert_eq!(out, run(&plain));
    // Both paths ran: the guard's (the observed `str`s), and the generic
    // body's (a subclass, another type, an exception).
    assert!(out.contains("count 3\ncount 0\ncount 2\ncount 1\ncount 3\ncount 0\ncount 2\ncount 3\n"), "{out}");
    assert!(out.contains("count TypeError object of type 'int' has no len()"), "{out}");
    assert!(out.contains("nan True True 2.5\nbig True 5\n"), "{out}");
    assert!(out.contains("str True x\nseek True n n True True True\n"), "{out}");
    assert!(out.contains("last 'ok'\nlast 2\nlast 0\n"), "{out}");
    assert!(out.contains("skip 3\nskip 3\nskip 1\nskip TypeError string indices must be integers, not 'str'\n"), "{out}");
    assert!(out.contains("hash True True\nradd True True 2.5\nlen True 7\ncase True 7\nexcept True 2.5\nplus True True True -2.5\nself True True True True\n"), "{out}");
    // The module is the one the package's extension serves.
    let o = Command::new(python())
        .args(["-I", "-c", "import sys; sys.path.insert(0, sys.argv[1]); from pkg import m; print(type(m.__spec__.loader).__name__)"])
        .arg(&built)
        .output()
        .unwrap();
    assert_eq!(text(&o), "_PyccFinder\n");
}
