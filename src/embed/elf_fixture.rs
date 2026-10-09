//! Test-only ELF64 little-endian writer: a header, a `PT_LOAD` covering
//! the whole file at a virtual address other than its offset, an optional
//! `PT_INTERP`, a `PT_DYNAMIC`, and a string table. Enough for the Linux
//! scan to read on any host.

/// What one synthetic image carries.
#[derive(Debug, Clone, Default)]
pub(crate) struct ElfSpec {
    pub(crate) machine: u16,
    pub(crate) program: bool,
    pub(crate) soname: Option<String>,
    pub(crate) needed: Vec<String>,
    pub(crate) rpath: Option<String>,
    pub(crate) runpath: Option<String>,
}

impl ElfSpec {
    /// An x86-64 shared library with `soname` needing `needed`.
    pub(crate) fn library(soname: &str, needed: &[&str]) -> Self {
        Self {
            machine: X86_64,
            soname: Some(soname.to_string()),
            needed: needed.iter().map(|name| (*name).to_string()).collect(),
            ..Self::default()
        }
    }

    /// An x86-64 extension module (no soname) needing `needed`.
    pub(crate) fn module(needed: &[&str]) -> Self {
        Self {
            machine: X86_64,
            needed: needed.iter().map(|name| (*name).to_string()).collect(),
            ..Self::default()
        }
    }

    pub(crate) fn runpath(mut self, value: &str) -> Self {
        self.runpath = Some(value.to_string());
        self
    }

    pub(crate) fn rpath(mut self, value: &str) -> Self {
        self.rpath = Some(value.to_string());
        self
    }
}

pub(crate) const X86_64: u16 = 62;
pub(crate) const AARCH64: u16 = 183;
pub(crate) const BASE: u64 = 0x40_0000;

/// The bytes of the image `spec` describes.
pub(crate) fn elf_bytes(spec: &ElfSpec) -> Vec<u8> {
    let mut strings = vec![0u8];
    let mut add = |text: &str| {
        let at = strings.len() as u64;
        strings.extend_from_slice(text.as_bytes());
        strings.push(0);
        at
    };
    let mut dynamic: Vec<(u64, u64)> = Vec::new();
    for name in &spec.needed {
        dynamic.push((1, add(name)));
    }
    for (tag, value) in [(14, &spec.soname), (15, &spec.rpath), (29, &spec.runpath)] {
        if let Some(value) = value {
            dynamic.push((tag, add(value)));
        }
    }
    let interp = add("/lib/ld-linux.so.2");
    let phnum: u64 = if spec.program { 3 } else { 2 };
    let strtab = 64 + 56 * phnum;
    let dyn_off = strtab + strings.len() as u64;
    dynamic.push((5, BASE + strtab));
    dynamic.push((0, 0));
    let total = dyn_off + 16 * dynamic.len() as u64;
    let mut out = Vec::new();
    out.extend_from_slice(b"\x7fELF\x02\x01\x01\0\0\0\0\0\0\0\0\0");
    out.extend_from_slice(&3u16.to_le_bytes());
    out.extend_from_slice(&spec.machine.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes());
    out.extend_from_slice(&64u64.to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&64u16.to_le_bytes());
    out.extend_from_slice(&56u16.to_le_bytes());
    out.extend_from_slice(&(phnum as u16).to_le_bytes());
    out.extend_from_slice(&[0u8; 6]);
    let mut phdr = |kind: u32, offset: u64, size: u64| {
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&(BASE + offset).to_le_bytes());
        out.extend_from_slice(&(BASE + offset).to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&8u64.to_le_bytes());
    };
    phdr(1, 0, total);
    if spec.program {
        phdr(3, strtab + interp, 19);
    }
    phdr(2, dyn_off, 16 * dynamic.len() as u64);
    out.extend_from_slice(&strings);
    for (tag, value) in dynamic {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}
