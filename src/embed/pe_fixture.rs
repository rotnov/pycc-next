//! Test-only PE writer (the `elf_fixture.rs` pattern): a DOS header, the
//! `PE` signature, a COFF header, a PE32+ or PE32 optional header with 16
//! data directories, and one section at a virtual address other than its
//! file offset, so a reader that skips the RVA-to-offset translation reads
//! the wrong bytes. The section holds the import descriptors, then the
//! delay-import descriptors, then the names, then (with forwarders) the
//! export directory, its address table and the forwarder strings. Enough
//! for the Windows scan to read on any host.

/// The COFF header's file offset (`e_lfanew` is `0x40`).
pub(crate) const COFF: usize = 0x44;
/// The optional header's file offset.
pub(crate) const OPTIONAL: usize = COFF + 20;
/// The section header's file offset in a PE32+ image (240-byte optional
/// header).
pub(crate) const SECTION_HEADER: usize = OPTIONAL + 240;
/// The section's file offset.
pub(crate) const RAW: usize = 0x400;
/// The section's virtual address.
pub(crate) const VA: u32 = 0x2000;

/// What one synthetic image carries.
#[derive(Debug, Clone)]
pub(crate) struct PeSpec {
    pub(crate) machine: u16,
    pub(crate) pe32_plus: bool,
    pub(crate) dll: bool,
    pub(crate) imports: Vec<String>,
    pub(crate) delay_imports: Vec<String>,
    /// `NumberOfRvaAndSizes`.
    pub(crate) rva_count: u32,
    /// The export forwarder strings (`module.symbol`), in address-table
    /// order; none writes no export directory.
    pub(crate) forwarders: Vec<String>,
    /// Whether the address table starts with one ordinary export, whose
    /// RVA lies outside the export directory.
    pub(crate) plain_export: bool,
}

fn owned(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

impl PeSpec {
    /// An x86-64 PE32+ DLL importing `imports`.
    pub(crate) fn dll(imports: &[&str]) -> Self {
        Self {
            machine: super::MACHINE_AMD64,
            pe32_plus: true,
            dll: true,
            imports: owned(imports),
            delay_imports: Vec::new(),
            rva_count: 16,
            forwarders: Vec::new(),
            plain_export: false,
        }
    }

    /// Exports `forwarders` as forwarder strings.
    pub(crate) fn forwards(mut self, forwarders: &[&str]) -> Self {
        self.forwarders = owned(forwarders);
        self
    }

    /// Adds one ordinary export before the forwarders.
    pub(crate) fn plain_export(mut self) -> Self {
        self.plain_export = true;
        self
    }

    /// The export directory's file offset: after the descriptors and the
    /// import names.
    pub(crate) fn export_offset(&self) -> usize {
        let names = self.imports.iter().chain(&self.delay_imports);
        let names: usize = names.map(|name| name.len() + 1).sum();
        RAW + 20 * (self.imports.len() + 1) + 32 * (self.delay_imports.len() + 1) + names
    }

    pub(crate) fn delay(mut self, names: &[&str]) -> Self {
        self.delay_imports = owned(names);
        self
    }

    pub(crate) fn machine(mut self, machine: u16) -> Self {
        self.machine = machine;
        self
    }

    /// A PE32 image, with the `i386` machine.
    pub(crate) fn pe32(mut self) -> Self {
        self.pe32_plus = false;
        self.machine = 0x14c;
        self
    }

    pub(crate) fn rva_count(mut self, count: u32) -> Self {
        self.rva_count = count;
        self
    }

    /// Clears `IMAGE_FILE_DLL`, as in an executable.
    pub(crate) fn not_dll(mut self) -> Self {
        self.dll = false;
        self
    }

    /// The image's bytes.
    pub(crate) fn bytes(&self) -> Vec<u8> {
        let import_size = 20 * (self.imports.len() + 1);
        let delay_size = 32 * (self.delay_imports.len() + 1);
        let mut section = vec![0u8; import_size + delay_size];
        let mut name_rvas = Vec::new();
        for name in self.imports.iter().chain(&self.delay_imports) {
            name_rvas.push(VA + section.len() as u32);
            section.extend_from_slice(name.as_bytes());
            section.push(0);
        }
        let (imports, delays) = name_rvas.split_at(self.imports.len());
        for (index, rva) in imports.iter().enumerate() {
            put32(&mut section, 20 * index + 12, *rva);
        }
        for (index, rva) in delays.iter().enumerate() {
            let at = import_size + 32 * index;
            put32(&mut section, at, 1);
            put32(&mut section, at + 4, *rva);
        }
        let export = self.export_directory(&mut section);
        let optional_size: usize = if self.pe32_plus { 240 } else { 224 };
        let mut out = vec![0u8; RAW];
        out[..2].copy_from_slice(b"MZ");
        put32(&mut out, 0x3C, 0x40);
        out[0x40..0x44].copy_from_slice(b"PE\0\0");
        put16(&mut out, COFF, self.machine);
        put16(&mut out, COFF + 2, 1);
        put16(&mut out, COFF + 16, optional_size as u16);
        let characteristics = if self.dll { 0x2022 } else { 0x0022 };
        put16(&mut out, COFF + 18, characteristics);
        let (magic, dirs) = if self.pe32_plus {
            (0x20b, 112)
        } else {
            (0x10b, 96)
        };
        put16(&mut out, OPTIONAL, magic);
        put32(&mut out, OPTIONAL + dirs - 4, self.rva_count);
        let directory = |index: usize, len: usize, size: usize, at: usize| {
            (
                index,
                if len == 0 {
                    (0, 0)
                } else {
                    (VA + at as u32, size as u32)
                },
            )
        };
        for (index, (rva, size)) in [
            export,
            directory(1, self.imports.len(), import_size, 0),
            directory(13, self.delay_imports.len(), delay_size, import_size),
        ] {
            put32(&mut out, OPTIONAL + dirs + 8 * index, rva);
            put32(&mut out, OPTIONAL + dirs + 8 * index + 4, size);
        }
        let header = OPTIONAL + optional_size;
        out[header..header + 5].copy_from_slice(b".idat");
        put32(&mut out, header + 8, section.len() as u32);
        put32(&mut out, header + 12, VA);
        put32(&mut out, header + 16, section.len() as u32);
        put32(&mut out, header + 20, RAW as u32);
        out.extend_from_slice(&section);
        out
    }

    /// Appends the export directory, its address table and the forwarder
    /// strings to `section`, and returns data directory 0's entry.
    fn export_directory(&self, section: &mut Vec<u8>) -> (usize, (u32, u32)) {
        if self.forwarders.is_empty() {
            return (0, (0, 0));
        }
        let start = section.len();
        let count = self.forwarders.len() + usize::from(self.plain_export);
        let table = start + 40;
        section.resize(table + 4 * count, 0);
        put32(section, start + 20, count as u32);
        put32(section, start + 28, VA + table as u32);
        let mut slot = table;
        if self.plain_export {
            put32(section, slot, VA);
            slot += 4;
        }
        for forwarder in &self.forwarders {
            let rva = VA + section.len() as u32;
            put32(section, slot, rva);
            slot += 4;
            section.extend_from_slice(forwarder.as_bytes());
            section.push(0);
        }
        let size = (section.len() - start) as u32;
        (0, (VA + start as u32, size))
    }
}

pub(crate) fn put16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

pub(crate) fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
