//! The Windows sidecar (D-253, Part 1 of #1226): the interpreter's DLL,
//! `python3.dll` and the VC runtime DLLs at the sidecar root, a filtered
//! `Lib\` and `DLLs\`, the locked closure in `closure\` (#1296), the
//! closure's natives in `natives\` (#1306), and the marker. There is no
//! `lib\` (it would be `Lib\` on a case-insensitive volume) and no
//! relocation: the stub loads the program DLL with the sidecar as its DLL
//! search root and the launcher adds the sidecar, and `natives\` when it
//! exists, as DLL directories. Every interpreter image copied here was
//! scanned before staging (#1305), and every closure image and native was
//! classified before staging (#1306).

use super::{EmbedProbe, NativePlan, bundle_library, copy_library, copy_stdlib, io_error};
use crate::embed::closure;
use crate::embed::layout::{self, LibpythonLink};
use crate::lock::build::LockedClosure;
use std::path::Path;

/// Fills `staging` for a Windows embedded executable. The interpreter
/// DLL's digest is recorded in the marker, as on every platform, and
/// checked against a consumed lock section's `libpython-sha256`, which
/// `pycc lock` took from the same DLL. Each native is copied into
/// `natives\`, created only when there is one, after its bytes are checked
/// against its lock entry.
pub(super) fn populate(
    probe: &EmbedProbe,
    staging: &Path,
    locked: Option<&LockedClosure>,
    natives: &NativePlan,
) -> Result<(), String> {
    // The interpreter's DLL comes first; the probe checked `python3.dll`,
    // and the list holds only the VC runtime DLLs that exist.
    let roots = layout::windows_root_dlls(probe);
    let dll = layout::windows_interpreter_dll(probe);
    let digest = bundle_library(&dll, &staging.join(&roots[0]), locked)?;
    for name in &roots[1..] {
        copy_file(&probe.base_prefix.join(name), &staging.join(name))?;
    }
    let skip = layout::skip_in_windows_stdlib_copy;
    copy_stdlib(&probe.stdlib, &staging.join("Lib"), skip)?;
    let extensions = probe.base_prefix.join("DLLs");
    copy_stdlib(&extensions, &staging.join("DLLs"), skip)?;
    if let Some(locked) = locked.filter(|locked| !locked.files.is_empty()) {
        // No Mach-O image is relocated on Windows: the closure's PE images
        // were classified before staging (#1306) and are copied as they
        // are, so the image list is not needed.
        closure::copy_closure(locked, staging)?;
    }
    if !natives.natives.is_empty() {
        let dir = staging.join(layout::WINDOWS_NATIVES_DIR);
        std::fs::create_dir(&dir).map_err(|e| io_error("create", &dir, &e))?;
        for native in &natives.natives {
            let name = &native.locked.name;
            copy_library(&native.source, &dir, name, Some(&native.locked), locked)?;
        }
    }
    let marker = staging.join(layout::MARKER_NAME);
    let text = layout::marker_text(probe, &digest, LibpythonLink::Shared);
    std::fs::write(&marker, text).map_err(|e| io_error("write", &marker, &e))
}

/// Copies one file's bytes from `from` to `to`.
fn copy_file(from: &Path, to: &Path) -> Result<(), String> {
    let bytes = std::fs::read(from).map_err(|e| io_error("read", from, &e))?;
    std::fs::write(to, bytes).map_err(|e| io_error("write", to, &e))
}
