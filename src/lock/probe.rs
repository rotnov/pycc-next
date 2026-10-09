//! The lock probe: asks the `PYCC_PYTHON` interpreter for its site
//! directories, its bytecode cache tag, its platform tag and its PEP 508
//! marker environment (the pycc.lock decision entry, rules 1 and 6).

use super::marker::{MARKER_VARIABLES, MarkerEnv};
use crate::ext_build;
use std::ffi::OsStr;
use std::path::PathBuf;

/// The probe's first line, so a truncated or foreign output never parses.
const PROBE_MARKER: &str = "pycc-lock-probe 1";

/// Prints [`PROBE_MARKER`], `purelib`, `platlib`, the cache tag,
/// `sysconfig.get_platform()` and the eleven marker variables in
/// [`MARKER_VARIABLES`] order, one per line, computed as PEP 508 defines
/// them.
pub(crate) const LOCK_PROBE_SCRIPT: &str = "import os,platform,sys,sysconfig\n\
     print('pycc-lock-probe 1')\n\
     print(sysconfig.get_path('purelib'))\n\
     print(sysconfig.get_path('platlib'))\n\
     print(sys.implementation.cache_tag)\n\
     print(sysconfig.get_platform())\n\
     i = sys.implementation.version\n\
     v = '%d.%d.%d' % (i.major, i.minor, i.micro)\n\
     if i.releaselevel != 'final':\n\
     \x20   v += i.releaselevel[0] + str(i.serial)\n\
     for x in (sys.implementation.name, v, os.name, platform.machine(),\n\
     \x20         platform.python_implementation(), platform.release(), platform.system(),\n\
     \x20         platform.version(), platform.python_version(),\n\
     \x20         '.'.join(platform.python_version_tuple()[:2]), sys.platform):\n\
     \x20   print(x)\n";

/// What the lock probe reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LockProbe {
    pub(crate) purelib: PathBuf,
    pub(crate) platlib: PathBuf,
    pub(crate) cache_tag: String,
    pub(crate) platform: String,
    pub(crate) markers: MarkerEnv,
}

/// Parses [`LOCK_PROBE_SCRIPT`]'s output: exactly the marker line and
/// fifteen values, none of the path or tag values empty.
pub(crate) fn parse_lock_probe(stdout: &str) -> Option<LockProbe> {
    let lines: Vec<&str> = stdout
        .lines()
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();
    let [marker, purelib, platlib, cache_tag, platform, rest @ ..] = lines.as_slice() else {
        return None;
    };
    if *marker != PROBE_MARKER
        || [purelib, platlib, cache_tag, platform]
            .iter()
            .any(|v| v.is_empty())
    {
        return None;
    }
    let values: [String; 11] = rest
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .try_into()
        .ok()?;
    debug_assert_eq!(values.len(), MARKER_VARIABLES.len());
    Some(LockProbe {
        purelib: PathBuf::from(purelib),
        platlib: PathBuf::from(platlib),
        cache_tag: cache_tag.to_string(),
        platform: platform.to_string(),
        markers: MarkerEnv::new(values),
    })
}

/// Runs the lock probe with `-I` and without the two variables that
/// redirect `sysconfig` to another platform's configuration.
pub(crate) fn run_lock_probe(interpreter: &OsStr) -> Result<LockProbe, String> {
    let name = interpreter.to_string_lossy();
    let output = ext_build::probe_command(interpreter, LOCK_PROBE_SCRIPT)
        .env_remove("_PYTHON_HOST_PLATFORM")
        .env_remove("_PYTHON_SYSCONFIGDATA_NAME")
        .output()
        .map_err(|e| format!("could not run the lock interpreter `{name}`: {e}; set PYCC_PYTHON to name one (default `python3.14`)"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed = output.status.success().then(|| parse_lock_probe(&stdout));
    parsed.flatten().ok_or_else(|| {
        format!(
            "the lock interpreter `{name}` did not report an environment this pycc could \
             parse (exit {})",
            output.status.code().unwrap_or(-1)
        )
    })
}

#[cfg(test)]
#[path = "probe_tests.rs"]
pub(crate) mod tests;
