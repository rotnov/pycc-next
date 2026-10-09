//! Pure PE reading for the Windows embedded build's import scan (#1305,
//! Part 1 of #1297): the machine, the optional-header kind, the DLL flag,
//! and the DLL names of the import directory (1) and the delay-import
//! directory (13); and, for the closure scan (#1306), the modules the
//! export directory (0) forwards to ([`parse_forwarders`]). Nothing here
//! touches the file system or spawns anything, so the macOS coverage host
//! drives every line over synthetic byte fixtures; `native_windows.rs` and
//! `native_windows_closure.rs` do the walking.
//!
//! Every read is bounds-checked, so a truncated or hostile image is an
//! `Err`, never a panic. The file is also compiled into
//! `tests/issue_1286_windows_embedded_executable.rs` (the `llvm-readobj`
//! oracle), so it and its nested fixture and tests use only `std` and
//! `super::` paths.

/// `IMAGE_FILE_MACHINE_AMD64`.
pub(crate) const MACHINE_AMD64: u16 = 0x8664;

/// What the scan needs from one PE image.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PeImage {
    /// The COFF header's `Machine`.
    pub(crate) machine: u16,
    /// The optional-header magic is `0x20b` (PE32+), not `0x10b` (PE32).
    pub(crate) pe32_plus: bool,
    /// `IMAGE_FILE_DLL` is set.
    pub(crate) dll: bool,
    /// The import directory's DLL names, in descriptor order.
    pub(crate) imports: Vec<String>,
    /// The delay-import directory's DLL names, in descriptor order.
    pub(crate) delay_imports: Vec<String>,
}

const IMAGE_FILE_DLL: u16 = 0x2000;
const EXPORT_DIRECTORY: usize = 0;
const EXPORT_DIRECTORY_SIZE: usize = 40;
const IMPORT_DIRECTORY: usize = 1;
const DELAY_IMPORT_DIRECTORY: usize = 13;
const IMPORT_DESCRIPTOR_SIZE: usize = 20;
const DELAY_DESCRIPTOR_SIZE: usize = 32;
const SECTION_HEADER_SIZE: usize = 40;
const MAX_NAME: usize = 260;

/// Whether `bytes` start with the DOS `MZ` magic.
pub(crate) fn is_mz(bytes: &[u8]) -> bool {
    bytes.starts_with(b"MZ")
}

/// Whether `name` is an API set (`api-ms-win-*`, `ext-ms-*`, ASCII
/// case-insensitively): a name the loader maps to a system DLL, never a
/// file.
pub(crate) fn is_api_set(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("api-ms-win-") || lower.starts_with("ext-ms-")
}

fn field<const N: usize>(bytes: &[u8], at: usize) -> Option<[u8; N]> {
    bytes.get(at..at.checked_add(N)?)?.try_into().ok()
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    field(bytes, at).map(u16::from_le_bytes)
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    field(bytes, at).map(u32::from_le_bytes)
}

fn past_end() -> String {
    "the headers or the section table run past the end of the file".to_string()
}

/// One section's mapping: `[va, va + len)` is backed by the file bytes at
/// `[raw, raw + len)`, where `len` is `min(VirtualSize, SizeOfRawData)`:
/// memory past `SizeOfRawData` is zero-fill, whose file offset belongs to
/// the next section.
struct Section {
    va: u64,
    len: u64,
    raw: u64,
}

/// The file offset `rva` maps to, and the end of its section's file data
/// (clamped to the file), or `None` when no section's file data holds it:
/// an RVA in no section, or in a section's zero-fill.
fn locate(sections: &[Section], rva: u32, file_len: usize) -> Option<(usize, usize)> {
    let rva = u64::from(rva);
    let section = sections
        .iter()
        .find(|s| rva >= s.va && rva - s.va < s.len)?;
    let at = usize::try_from(section.raw + (rva - section.va)).ok()?;
    let end = usize::try_from(section.raw + section.len).ok()?;
    Some((at, end.min(file_len)))
}

/// What [`headers`] reads: the COFF fields, the mapped sections, and the
/// data directory entries as `(rva, size)`.
struct Headers {
    machine: u16,
    pe32_plus: bool,
    characteristics: u16,
    sections: Vec<Section>,
    directories: Vec<(u32, u32)>,
}

impl Headers {
    /// Data directory `index`: `(0, 0)` past `NumberOfRvaAndSizes`.
    fn directory(&self, index: usize) -> (u32, u32) {
        self.directories.get(index).copied().unwrap_or_default()
    }
}

/// Reads the headers and the section table of the image `bytes`, which
/// start with `MZ` or are refused for having no `PE` signature.
fn headers(bytes: &[u8]) -> Result<Headers, String> {
    let pe = u32_at(bytes, 0x3C).ok_or_else(past_end)? as usize;
    if bytes.get(pe..pe.saturating_add(4)) != Some(&b"PE\0\0"[..]) {
        return Err("the image has no `PE` signature".to_string());
    }
    let coff = pe + 4;
    let machine = u16_at(bytes, coff).ok_or_else(past_end)?;
    let sections = u16_at(bytes, coff + 2).ok_or_else(past_end)?;
    let optional_size = usize::from(u16_at(bytes, coff + 16).ok_or_else(past_end)?);
    let characteristics = u16_at(bytes, coff + 18).ok_or_else(past_end)?;
    let optional = coff + 20;
    let (pe32_plus, dirs_off) = match u16_at(bytes, optional).ok_or_else(past_end)? {
        0x20b => (true, 112),
        0x10b => (false, 96),
        other => return Err(format!("unknown optional-header magic {other:#x}")),
    };
    if optional_size < dirs_off {
        return Err(format!(
            "the optional header is {optional_size} bytes, shorter than its {dirs_off} fixed bytes"
        ));
    }
    let declared = u32_at(bytes, optional + dirs_off - 4).ok_or_else(past_end)? as usize;
    let dir_count = declared.min((optional_size - dirs_off) / 8);
    let table = optional + optional_size;
    if bytes.len() < table + usize::from(sections) * SECTION_HEADER_SIZE {
        return Err(past_end());
    }
    let mut mapped = Vec::with_capacity(usize::from(sections));
    for index in 0..usize::from(sections) {
        let header = table + index * SECTION_HEADER_SIZE;
        let at = |off: usize| {
            u32_at(bytes, header + off)
                .map(u64::from)
                .ok_or_else(past_end)
        };
        let (virtual_size, va, raw_size, raw) = (at(8)?, at(12)?, at(16)?, at(20)?);
        let len = virtual_size.min(raw_size);
        mapped.push(Section { va, len, raw });
    }
    // Every directory entry lies inside the optional header, before the
    // section table just read, so it is in the file.
    let entry = |at: usize| u32_at(bytes, at).unwrap_or_default();
    let directories = (0..dir_count)
        .map(|index| optional + dirs_off + index * 8)
        .map(|at| (entry(at), entry(at + 4)))
        .collect();
    Ok(Headers {
        machine,
        pe32_plus,
        characteristics,
        sections: mapped,
        directories,
    })
}

/// Reads `bytes` as a PE image: `Ok(None)` when they do not start with
/// `MZ`, and `Err(reason)` when they do but the image is malformed or
/// truncated. A PE32 image is parsed, reporting `pe32_plus = false`, so the
/// caller can refuse it precisely.
pub(crate) fn parse_pe(bytes: &[u8]) -> Result<Option<PeImage>, String> {
    if !is_mz(bytes) {
        return Ok(None);
    }
    let headers = headers(bytes)?;
    let mapped = &headers.sections;
    let imports = names(bytes, mapped, headers.directory(IMPORT_DIRECTORY).0, false)?;
    let delay = headers.directory(DELAY_IMPORT_DIRECTORY).0;
    let delay_imports = names(bytes, mapped, delay, true)?;
    Ok(Some(PeImage {
        machine: headers.machine,
        pe32_plus: headers.pe32_plus,
        dll: headers.characteristics & IMAGE_FILE_DLL != 0,
        imports,
        delay_imports,
    }))
}

/// The DLL names the export directory (0) of the image `bytes` forwards
/// to (#1306): every export address inside the directory's own range
/// `[rva, rva + size)` is a forwarder string `module.symbol` (or
/// `module.#ordinal`), NUL-terminated within that range. The module is the
/// text before the last `.`, with `.dll` appended unless it already ends
/// in it, as the loader reads it. Deduplicated ASCII case-insensitively,
/// in first-seen order; no export directory is an empty list. Kept apart
/// from [`parse_pe`], whose readers need no export directory.
pub(crate) fn parse_forwarders(bytes: &[u8]) -> Result<Vec<String>, String> {
    let headers = headers(bytes)?;
    let (rva, size) = headers.directory(EXPORT_DIRECTORY);
    if rva == 0 {
        return Ok(Vec::new());
    }
    let sections = &headers.sections;
    let unmapped = |what: &str, at: u32| {
        format!("the export {what} RVA {at:#x} maps to no section's file data")
    };
    let (at, end) = locate(sections, rva, bytes.len()).ok_or_else(|| unmapped("directory", rva))?;
    let directory = bytes
        .get(at..at + EXPORT_DIRECTORY_SIZE)
        .filter(|_| at + EXPORT_DIRECTORY_SIZE <= end)
        .ok_or_else(|| "the export directory runs past its section".to_string())?;
    let count = u32_at(directory, 20).unwrap_or_default() as usize;
    if count == 0 {
        return Ok(Vec::new());
    }
    let table = u32_at(directory, 28).unwrap_or_default();
    let (table_at, table_end) =
        locate(sections, table, bytes.len()).ok_or_else(|| unmapped("address table", table))?;
    let table_len = count
        .checked_mul(4)
        .filter(|len| table_at + len <= table_end);
    let table_len =
        table_len.ok_or_else(|| "the export address table runs past its section".to_string())?;
    let range = u64::from(rva)..u64::from(rva) + u64::from(size);
    let mut modules: Vec<String> = Vec::new();
    for entry in bytes[table_at..table_at + table_len].chunks_exact(4) {
        let address = u32_at(entry, 0).unwrap_or_default();
        if !range.contains(&u64::from(address)) {
            continue;
        }
        let (text_at, text_end) =
            locate(sections, address, bytes.len()).ok_or_else(|| unmapped("forwarder", address))?;
        let bound = usize::try_from(range.end - u64::from(address)).unwrap_or(usize::MAX);
        let window = bytes.get(text_at..text_end.min(text_at.saturating_add(bound)));
        let window = window.ok_or_else(|| {
            format!("the forwarder at RVA {address:#x} lies past the end of the file")
        })?;
        let length = window.iter().position(|byte| *byte == 0);
        let length = length.ok_or_else(|| {
            format!("the forwarder at RVA {address:#x} has no NUL within the export directory")
        })?;
        let text = String::from_utf8_lossy(&window[..length]);
        let (module, _) = text
            .rsplit_once('.')
            .ok_or_else(|| format!("the forwarder `{text}` names no module"))?;
        let module = match module.to_ascii_lowercase().ends_with(".dll") {
            true => module.to_string(),
            false => format!("{module}.dll"),
        };
        if !modules
            .iter()
            .any(|seen| seen.eq_ignore_ascii_case(&module))
        {
            modules.push(module);
        }
    }
    Ok(modules)
}

/// The DLL names of the descriptor list at `rva`: the import directory's
/// 20-byte descriptors (name RVA at +12), or with `delay` the
/// delay-import directory's 32-byte descriptors (name RVA at +4). An RVA
/// of 0 is an empty list, as LLVM reads it; the list otherwise ends at an
/// all-zero descriptor, which must lie in the section's file data.
fn names(bytes: &[u8], sections: &[Section], rva: u32, delay: bool) -> Result<Vec<String>, String> {
    if rva == 0 {
        return Ok(Vec::new());
    }
    let (kind, size, name_at) = match delay {
        true => ("delay-import", DELAY_DESCRIPTOR_SIZE, 4),
        false => ("import", IMPORT_DESCRIPTOR_SIZE, 12),
    };
    let unmapped = || format!("the {kind} directory RVA {rva:#x} maps to no section's file data");
    let (mut at, end) = locate(sections, rva, bytes.len()).ok_or_else(unmapped)?;
    let mut out = Vec::new();
    loop {
        let unterminated = || format!("the {kind} directory has no terminating descriptor");
        let stop = Some(at + size).filter(|stop| *stop <= end);
        let descriptor = stop
            .and_then(|stop| bytes.get(at..stop))
            .ok_or_else(unterminated)?;
        if descriptor.iter().all(|byte| *byte == 0) {
            return Ok(out);
        }
        // After the terminator test: the terminator's attributes are 0 too.
        if delay && u32_at(descriptor, 0).unwrap_or_default() & 1 == 0 {
            return Err("delay-import descriptor is not RVA-based".to_string());
        }
        let name_rva = u32_at(descriptor, name_at).unwrap_or_default();
        out.push(name(bytes, sections, name_rva, kind)?);
        at += size;
    }
}

/// The NUL-terminated ASCII name at `rva`: at most [`MAX_NAME`] bytes, and
/// never past its section's file data.
fn name(bytes: &[u8], sections: &[Section], rva: u32, kind: &str) -> Result<String, String> {
    let unmapped = || format!("an {kind} name RVA {rva:#x} maps to no section's file data");
    let (at, end) = locate(sections, rva, bytes.len()).ok_or_else(unmapped)?;
    let window = bytes.get(at..end.min(at + MAX_NAME)).unwrap_or_default();
    let unterminated =
        || format!("an {kind} name has no NUL within {MAX_NAME} bytes or its section");
    let length = window
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(unterminated)?;
    let text = window.get(..length).unwrap_or_default();
    if text.is_empty() || !text.is_ascii() {
        return Err(format!(
            "an {kind} name at RVA {rva:#x} is empty or not ASCII"
        ));
    }
    Ok(String::from_utf8_lossy(text).into_owned())
}

#[cfg(test)]
#[path = "pe_fixture.rs"]
pub(crate) mod fixture;

#[cfg(test)]
#[path = "pe_tests.rs"]
mod tests;
