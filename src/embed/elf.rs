//! Pure ELF reading for an embedded executable's Linux dependency scan
//! (#1243, lifting the embedded-executable decision entry's NEG-002): the
//! dynamic section of an ELF64 little-endian image, `ldconfig -p` output,
//! and `$ORIGIN` expansion. Nothing here touches the file system or spawns
//! anything, so the macOS coverage host drives every line over synthetic
//! byte fixtures; `native_linux.rs` does the walking.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What the Linux scan needs from one ELF image's dynamic section.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ElfImage {
    /// `e_machine`: a candidate library must match its requester's.
    pub(crate) machine: u16,
    /// The image has a `PT_INTERP`, so it is a program rather than a
    /// library a process loads.
    pub(crate) program: bool,
    /// `DT_SONAME`.
    pub(crate) soname: Option<String>,
    /// Every `DT_NEEDED`, in order.
    pub(crate) needed: Vec<String>,
    /// `DT_RPATH`.
    pub(crate) rpath: Option<String>,
    /// `DT_RUNPATH`.
    pub(crate) runpath: Option<String>,
}

const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;
const DT_NULL: u64 = 0;
const DT_NEEDED: u64 = 1;
const DT_STRTAB: u64 = 5;
const DT_SONAME: u64 = 14;
const DT_RPATH: u64 = 15;
const DT_RUNPATH: u64 = 29;
const PHDR_SIZE: u64 = 56;
const DYN_SIZE: u64 = 16;

/// Whether `bytes` start with the ELF magic, whatever the class.
pub(crate) fn is_elf_magic(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x7fELF")
}

fn field<const N: usize>(bytes: &[u8], at: u64) -> Option<[u8; N]> {
    let start = usize::try_from(at).ok()?;
    let raw = bytes.get(start..start.checked_add(N)?)?;
    raw.try_into().ok()
}

fn u16_at(bytes: &[u8], at: u64) -> Option<u16> {
    field(bytes, at).map(u16::from_le_bytes)
}

fn u32_at(bytes: &[u8], at: u64) -> Option<u32> {
    field(bytes, at).map(u32::from_le_bytes)
}

fn u64_at(bytes: &[u8], at: u64) -> Option<u64> {
    field(bytes, at).map(u64::from_le_bytes)
}

/// One program header's type, file offset, virtual address and file size.
struct Segment {
    kind: u32,
    offset: u64,
    vaddr: u64,
    filesz: u64,
}

fn segment_at(bytes: &[u8], at: u64) -> Option<Segment> {
    Some(Segment {
        kind: u32_at(bytes, at)?,
        offset: u64_at(bytes, at.checked_add(8)?)?,
        vaddr: u64_at(bytes, at.checked_add(16)?)?,
        filesz: u64_at(bytes, at.checked_add(32)?)?,
    })
}

fn truncated(what: &str) -> String {
    format!("its ELF {what} runs past the end of the file")
}

/// Reads the dynamic section of an ELF64 little-endian image. `Ok(None)`:
/// not such an image (no ELF magic, or another class or byte order), which
/// the scan ignores as the loader would. `Err`: an ELF64 little-endian
/// image whose structures run past the file or point outside it.
pub(crate) fn parse_elf(bytes: &[u8]) -> Result<Option<ElfImage>, String> {
    if !is_elf_magic(bytes) || bytes.get(4) != Some(&2) || bytes.get(5) != Some(&1) {
        return Ok(None);
    }
    let header = || truncated("header");
    let machine = u16_at(bytes, 18).ok_or_else(header)?;
    let phoff = u64_at(bytes, 32).ok_or_else(header)?;
    let phnum = u16_at(bytes, 56).ok_or_else(header)?;
    let mut segments = Vec::new();
    for index in 0..u64::from(phnum) {
        let segment = phoff
            .checked_add(index * PHDR_SIZE)
            .and_then(|at| segment_at(bytes, at))
            .ok_or_else(|| truncated("program header table"))?;
        segments.push(segment);
    }
    let mut image = ElfImage {
        machine,
        program: segments.iter().any(|segment| segment.kind == PT_INTERP),
        ..ElfImage::default()
    };
    let Some(dynamic) = segments.iter().find(|segment| segment.kind == PT_DYNAMIC) else {
        return Ok(Some(image));
    };
    let mut entries = Vec::new();
    for index in 0..dynamic.filesz / DYN_SIZE {
        let entry = dynamic
            .offset
            .checked_add(index * DYN_SIZE)
            .and_then(|at| Some((u64_at(bytes, at)?, u64_at(bytes, at.checked_add(8)?)?)));
        let (tag, value) = entry.ok_or_else(|| truncated("dynamic section"))?;
        if tag == DT_NULL {
            break;
        }
        entries.push((tag, value));
    }
    let Some(&(_, strtab)) = entries.iter().find(|(tag, _)| *tag == DT_STRTAB) else {
        return Ok(Some(image));
    };
    let strtab = file_offset(&segments, strtab)
        .ok_or_else(|| "its ELF string table is outside every loaded segment".to_string())?;
    for (tag, value) in entries {
        let slot = match tag {
            DT_NEEDED => {
                image.needed.push(string_at(bytes, strtab, value)?);
                continue;
            }
            DT_SONAME => &mut image.soname,
            DT_RPATH => &mut image.rpath,
            DT_RUNPATH => &mut image.runpath,
            _ => continue,
        };
        *slot = Some(string_at(bytes, strtab, value)?);
    }
    Ok(Some(image))
}

/// Maps a virtual address to a file offset through the `PT_LOAD` segment
/// whose file image contains it.
fn file_offset(segments: &[Segment], vaddr: u64) -> Option<u64> {
    segments
        .iter()
        .filter(|segment| segment.kind == PT_LOAD)
        .find(|segment| vaddr >= segment.vaddr && vaddr - segment.vaddr < segment.filesz)
        .and_then(|segment| segment.offset.checked_add(vaddr - segment.vaddr))
}

/// The NUL-terminated string at `strtab + index`.
fn string_at(bytes: &[u8], strtab: u64, index: u64) -> Result<String, String> {
    let outside = || "an ELF dynamic string of it lies outside the file".to_string();
    let start = strtab
        .checked_add(index)
        .and_then(|start| usize::try_from(start).ok())
        .ok_or_else(outside)?;
    let rest = bytes.get(start..).ok_or_else(outside)?;
    let end = rest
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(outside)?;
    Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
}

/// Parses `ldconfig -p`: every `name (annotations) => path` line, in cache
/// order, keyed by the name. The header line and anything else without
/// ` => ` are skipped.
pub(crate) fn parse_ldconfig(stdout: &str) -> BTreeMap<String, Vec<PathBuf>> {
    let mut map: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for line in stdout.lines() {
        let Some((left, path)) = line.split_once(" => ") else {
            continue;
        };
        let name = left.split(" (").next().unwrap_or_default().trim();
        let path = path.trim();
        if name.is_empty() || path.is_empty() {
            continue;
        }
        map.entry(name.to_string())
            .or_default()
            .push(PathBuf::from(path));
    }
    map
}

/// The directories one `DT_RPATH`/`DT_RUNPATH` value names, each with
/// whether it came through `$ORIGIN`, which is replaced by `origin` (the
/// directory of the path the image was found at). An empty entry, and one
/// carrying any other `$` token (`$LIB`, `$PLATFORM`), is dropped: the scan
/// does not model them, so a library found only through one stays
/// unresolved and is left to the run-time loader.
pub(crate) fn search_dirs(value: &str, origin: &Path) -> Vec<(PathBuf, bool)> {
    let origin = origin.to_string_lossy();
    value
        .split(':')
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let via_origin = entry.contains("$ORIGIN") || entry.contains("${ORIGIN}");
            let expanded = entry
                .replace("${ORIGIN}", &origin)
                .replace("$ORIGIN", &origin);
            (!expanded.contains('$')).then(|| (PathBuf::from(expanded), via_origin))
        })
        .collect()
}

#[cfg(test)]
#[cfg_attr(windows, allow(dead_code))]
#[path = "elf_fixture.rs"]
pub(crate) mod fixture;

#[cfg(test)]
#[path = "elf_tests.rs"]
mod tests;
