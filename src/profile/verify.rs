//! Pass 3 of `pycc profile` (docs/design/compile-little.md, section 2.3):
//! only measured winners are selected.
//!
//! The selection rule picks candidates from the profile's self times, but a
//! compiled untyped body called from bytecode, or calling back into it, can be
//! slower than CPython's specializing interpreter running it. So the profile
//! builds the candidates and times the training workload with them compiled
//! against the unchanged sources, interleaved over `--verify-rounds` fresh
//! processes each after one discarded warm-up run of each (a check stops after [`EARLY_ROUNDS`] once the two
//! interquartile ranges are apart):
//!
//! 1. all the candidates together; if the workload is no slower (the compiled
//!    median under CPython's third quartile), every one is kept;
//! 2. else each region (the candidates connected by calls among them), most
//!    self time first, alone;
//! 3. then the regions that were no slower alone, together, once more.
//!
//! At most [`MAX_BUILDS`] builds. A function a check finds slower, or one no
//! check could reach within the budget, is recorded as measured slower; the
//! selection rule rejects it from then on, in `--check` and in builds alike, and
//! a later `pycc profile` keeps the verdict while the function's source hash
//! is unchanged (`carry_rejections`). The verdict is on wall time;
//! `--verify-instructions` adds each check's instruction delta under callgrind
//! as evidence.

use super::{Driver, FunctionKey, Measured, Profile, Rule};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// The most rounds per check.
pub(crate) const MAX_ROUNDS: u32 = 15;
/// Rounds after which a check whose interquartile ranges are apart stops.
pub(crate) const EARLY_ROUNDS: u32 = 5;
/// The most builds pass 3 makes.
pub(crate) const MAX_BUILDS: usize = 6;

/// Pass 3's settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Options {
    /// Interleaved rounds per check (each one run in CPython and one compiled).
    pub(crate) rounds: u32,
    /// Also count each check's instructions under callgrind.
    pub(crate) instructions: bool,
}

/// What pass 3 works with.
pub(super) struct Context<'a> {
    pub(super) driver: &'a Driver<'a>,
    pub(super) workload: &'a [OsString],
    pub(super) rule: Rule,
    pub(super) scratch: &'a Path,
    pub(super) options: Options,
}

/// Copies each rejection a previous profile `old` recorded onto the entry of
/// `profile` with the same module, qualified name and source hash: a function
/// measured slower stays rejected until its source changes.
pub(crate) fn carry_rejections(profile: &mut Profile, old: &Profile) {
    let rejected: BTreeMap<(&str, &str, &str), &Measured> = old
        .functions
        .iter()
        .filter_map(|e| {
            e.measured
                .as_ref()
                .filter(|m| !m.compiled)
                .map(|m| ((e.module.as_str(), e.qualname.as_str(), e.sha256.as_str()), m))
        })
        .collect();
    for e in &mut profile.functions {
        if let Some(m) = rejected.get(&(e.module.as_str(), e.qualname.as_str(), e.sha256.as_str())) {
            e.measured = Some((*m).clone());
        }
    }
}

/// The candidates connected by calls among them, each region in order of its
/// self time (most first).
pub(crate) fn regions(profile: &Profile, candidates: &[FunctionKey]) -> Vec<Vec<FunctionKey>> {
    let keys: BTreeMap<String, usize> = candidates
        .iter()
        .enumerate()
        .map(|(i, (m, q, l))| (format!("{m}:{q}:{l}"), i))
        .collect();
    let mut parent: Vec<usize> = (0..candidates.len()).collect();
    fn find(parent: &mut [usize], i: usize) -> usize {
        let mut i = i;
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut self_ns = vec![0u64; candidates.len()];
    for e in &profile.functions {
        let Some(&i) = keys.get(&e.callee_key()) else {
            continue;
        };
        self_ns[i] = e.self_ns;
        for callee in e.callees.keys() {
            if let Some(&j) = keys.get(callee) {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                parent[a.max(b)] = a.min(b);
            }
        }
    }
    let mut by_root: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..candidates.len() {
        let root = find(&mut parent, i);
        by_root.entry(root).or_default().push(i);
    }
    let mut out: Vec<(u64, Vec<FunctionKey>)> = by_root
        .into_values()
        .map(|members| {
            let ns = members.iter().map(|&i| self_ns[i]).sum();
            (ns, members.into_iter().map(|i| candidates[i].clone()).collect())
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    out.into_iter().map(|(_, r)| r).collect()
}

/// First quartile, median and third quartile (each the median of its half).
pub(crate) fn quartiles(samples: &[u64]) -> [u64; 3] {
    let mut v = samples.to_vec();
    v.sort_unstable();
    let median = |v: &[u64]| -> u64 {
        match v.len() {
            0 => 0,
            n if n % 2 == 1 => v[n / 2],
            n => (v[n / 2 - 1] + v[n / 2]) / 2,
        }
    };
    let n = v.len();
    if n < 2 {
        let m = median(&v);
        return [m, m, m];
    }
    [median(&v[..n / 2]), median(&v), median(&v[n.div_ceil(2)..])]
}

/// Whether the two samples' interquartile ranges do not overlap: the verdict
/// will not change with more rounds.
pub(crate) fn clear(plain: &[u64], built: &[u64]) -> bool {
    let (p, b) = (quartiles(plain), quartiles(built));
    b[0] > p[2] || b[2] < p[0]
}

/// The outcome of one check.
struct Check {
    cpython: [u64; 3],
    compiled: [u64; 3],
    instructions: Option<i64>,
}

impl Check {
    fn no_slower(&self) -> bool {
        self.compiled[1] <= self.cpython[2]
    }
}

/// Pass 3 over `candidates` (the selection rule's picks from `profile`):
/// records a [`Measured`] on each.
pub(super) fn verify(
    profile: &mut Profile,
    candidates: &[FunctionKey],
    cx: &Context<'_>,
) -> Result<(), String> {
    if candidates.is_empty() {
        return Ok(());
    }
    let started = Instant::now();
    let mut builds = 0usize;
    let all: BTreeSet<FunctionKey> = candidates.iter().cloned().collect();
    let whole = check(profile, &all, &all, cx, &mut builds)?;
    let mut verdicts: BTreeMap<FunctionKey, Measured> = BTreeMap::new();
    let record = |verdicts: &mut BTreeMap<FunctionKey, Measured>, set: &BTreeSet<FunctionKey>, c: &Check| {
        for key in set {
            verdicts.insert(
                key.clone(),
                Measured {
                    compiled: c.no_slower(),
                    with: set.len(),
                    cpython_ns: c.cpython,
                    compiled_ns: c.compiled,
                    instructions: c.instructions,
                },
            );
        }
    };
    record(&mut verdicts, &all, &whole);
    let regions = regions(profile, candidates);
    if !whole.no_slower() && regions.len() > 1 {
        let mut kept: BTreeSet<FunctionKey> = BTreeSet::new();
        let mut kept_regions = 0;
        for region in &regions {
            // One build stays for the kept regions together.
            if builds >= MAX_BUILDS - 1 {
                break;
            }
            let set: BTreeSet<FunctionKey> = region.iter().cloned().collect();
            let c = check(profile, &set, &all, cx, &mut builds)?;
            record(&mut verdicts, &set, &c);
            if c.no_slower() {
                kept.extend(set);
                kept_regions += 1;
            }
        }
        if kept_regions > 1 {
            let c = check(profile, &kept, &all, cx, &mut builds)?;
            record(&mut verdicts, &kept, &c);
        }
    }
    let kept = verdicts.values().filter(|m| m.compiled).count();
    for e in &mut profile.functions {
        if let Some(m) = verdicts.remove(&(e.module.clone(), e.qualname.clone(), e.firstlineno)) {
            e.measured = Some(m);
        }
    }
    eprintln!(
        "pycc profile: pass 3 kept {kept} of {} candidates, measured in {builds} builds in {:.1}s",
        candidates.len(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

/// Builds `set` (of the candidates `all`) into a copy of the package and times
/// the workload against it and against the sources.
fn check(
    profile: &Profile,
    set: &BTreeSet<FunctionKey>,
    all: &BTreeSet<FunctionKey>,
    cx: &Context<'_>,
    builds: &mut usize,
) -> Result<Check, String> {
    *builds += 1;
    let dir = cx.scratch.join(format!("verify{builds}"));
    let copy = copy_package(cx.driver.pkg, &dir.join("tree"))?;
    // The other candidates are left out, so the build's rule takes only `set`.
    let mut only = profile.clone();
    only.functions
        .retain(|e| set.contains(&key(e)) || !all.contains(&key(e)));
    let profile_path = dir.join("profile.json");
    std::fs::write(&profile_path, only.to_json()).map_err(|e| e.to_string())?;
    build(&copy, &profile_path, cx)?;
    let compiled = Driver {
        python: cx.driver.python,
        script: cx.driver.script,
        pkg: &copy,
    };
    let out = dir.join("time.json");
    // One discarded run of each first: the copy's bytecode cache is written
    // then, as an installed package's is at install.
    time(cx.driver, &out, cx.workload)?;
    time(&compiled, &out, cx.workload)?;
    let (mut plain, mut built) = (Vec::new(), Vec::new());
    for round in 0..cx.options.rounds.clamp(1, MAX_ROUNDS) {
        // Alternating which goes first, against drift.
        let order: [(&Driver, bool); 2] = if round % 2 == 0 {
            [(cx.driver, false), (&compiled, true)]
        } else {
            [(&compiled, true), (cx.driver, false)]
        };
        for (driver, is_compiled) in order {
            let ns = time(driver, &out, cx.workload)?;
            if is_compiled { &mut built } else { &mut plain }.push(ns);
        }
        if round + 1 >= EARLY_ROUNDS && clear(&plain, &built) {
            break;
        }
    }
    let instructions = if cx.options.instructions {
        Some(instructions(&compiled, &out, cx.workload)? - instructions(cx.driver, &out, cx.workload)?)
    } else {
        None
    };
    let c = Check {
        cpython: quartiles(&plain),
        compiled: quartiles(&built),
        instructions,
    };
    let ms = |ns: u64| ns as f64 / 1e6;
    eprintln!(
        "pycc profile: pass 3: {} functions compiled: {:.1} ms (IQR {:.1}-{:.1}) against CPython's {:.1} ms (IQR {:.1}-{:.1}): {}",
        set.len(),
        ms(c.compiled[1]),
        ms(c.compiled[0]),
        ms(c.compiled[2]),
        ms(c.cpython[1]),
        ms(c.cpython[0]),
        ms(c.cpython[2]),
        if c.no_slower() { "no slower" } else { "slower" }
    );
    // The copy is not needed past the check.
    let _ = std::fs::remove_dir_all(&dir);
    Ok(c)
}

fn key(e: &super::Entry) -> FunctionKey {
    (e.module.clone(), e.qualname.clone(), e.firstlineno)
}

/// One run of the workload, its wall time; its output is kept from the
/// terminal, and shown when it fails.
fn time(driver: &Driver<'_>, out: &Path, workload: &[OsString]) -> Result<u64, String> {
    let output = driver
        .command("time", out, &[], workload, &[])
        .env("PYCC_REQUIRE_PATCH", "1")
        .output()
        .map_err(|e| format!("cannot run {}: {e}", driver.python.to_string_lossy()))?;
    if !output.status.success() {
        unpatchable(&output.stderr)?;
        return Err(format!(
            "the profile's time pass failed ({}):\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    #[derive(serde::Deserialize)]
    struct Timed {
        ns: u64,
    }
    Ok(Driver::json::<Timed>(out)?.ns)
}

/// The error for a timed run whose compiled copy could not patch its
/// functions (`forward.c`, under `PYCC_REQUIRE_PATCH`): its verdicts would
/// compare bytecode with bytecode, so none is recorded.
fn unpatchable(stderr: &[u8]) -> Result<(), String> {
    let stderr = String::from_utf8_lossy(stderr);
    match stderr.lines().find(|line| line.contains("cannot be patched on this interpreter")) {
        Some(line) => Err(format!(
            "pass 3 cannot measure the compiled functions with this interpreter: {}
             (profile with CPython 3.13 or 3.15, or pass --no-verify)",
            line.trim()
        )),
        None => Ok(()),
    }
}

/// One run of the workload under callgrind: its instruction count.
fn instructions(driver: &Driver<'_>, out: &Path, workload: &[OsString]) -> Result<i64, String> {
    let file = format!("--callgrind-out-file={}", out.with_extension("cg").display());
    let wrapper: [&OsStr; 3] = ["valgrind".as_ref(), "--tool=callgrind".as_ref(), file.as_ref()];
    let output = driver
        .command("time", out, &[], workload, &wrapper)
        .env("PYCC_REQUIRE_PATCH", "1")
        .output()
        .map_err(|e| format!("--verify-instructions needs valgrind: {e}"))?;
    let _ = std::fs::remove_file(out.with_extension("cg"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        unpatchable(&output.stderr)?;
        return Err(format!("the profile's time pass under callgrind failed ({}):\n{stderr}", output.status));
    }
    stderr
        .lines()
        .find_map(|line| line.split_once("Collected :").and_then(|(_, n)| n.trim().parse().ok()))
        .ok_or_else(|| "callgrind reported no instruction count".to_string())
}

/// `pycc build COPY --ext --non-strict --release --profile PROFILE` with the
/// profile's own rule, by this pycc.
fn build(copy: &Path, profile: &Path, cx: &Context<'_>) -> Result<(), String> {
    let out = if copy.is_dir() {
        copy.join("__init__")
    } else {
        copy.with_extension("")
    };
    // A module of a package imports its siblings as CPython does.
    let member = copy.is_file() && copy.parent().is_some_and(|dir| dir.join("__init__.py").is_file());
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut command = std::process::Command::new(exe);
    command
        .arg("build")
        .arg(copy)
        .arg("-o")
        .arg(&out)
        .args(["--ext", "--non-strict", "--release", "--profile"])
        .arg(profile)
        .arg(format!("--select-share={}", cx.rule.share))
        .arg(format!("--select-calls={}", cx.rule.calls))
        .arg(format!("--select-max={}", cx.rule.max))
        .arg(format!("--select-outside={}", cx.rule.outside))
        .arg(format!("--select-handled={}", cx.rule.handled));
    if member {
        command.arg("--foreign-relative-imports");
    }
    if std::env::var_os("PYCC_PYTHON").is_none() {
        command.env("PYCC_PYTHON", cx.driver.python);
    }
    let output = command.output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "pass 3 could not build the candidates ({}):\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

/// Copies the package (or single module) `pkg` under `root` at the same path
/// below its import root; returns the copy's path. A module or subpackage
/// inside a package brings its whole top-level package, so its imports of
/// the package's other modules find them.
pub(crate) fn copy_package(pkg: &Path, root: &Path) -> Result<PathBuf, String> {
    let mut base = pkg.parent().unwrap_or(Path::new("")).to_path_buf();
    let mut top = None;
    while base.join("__init__.py").is_file() {
        top = Some(base.clone());
        match base.parent() {
            Some(parent) => base = parent.to_path_buf(),
            None => break,
        }
    }
    let rel = pkg.strip_prefix(&base).map_err(|e| e.to_string())?;
    match top {
        Some(top) => copy_dir(&top, &root.join(top.strip_prefix(&base).map_err(|e| e.to_string())?))?,
        None if pkg.is_file() => {
            std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
            std::fs::copy(pkg, root.join(rel)).map_err(|e| e.to_string())?;
        }
        None => copy_dir(pkg, &root.join(rel))?,
    }
    Ok(root.join(rel))
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(from).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let name = entry.file_name();
        if kind.is_dir() {
            if name != "__pycache__" {
                copy_dir(&entry.path(), &to.join(&name))?;
            }
        } else if kind.is_file() {
            std::fs::copy(entry.path(), to.join(&name)).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
