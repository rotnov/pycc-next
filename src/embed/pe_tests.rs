//! The PE import reader over synthetic images: the round trip, every
//! refusal arm by byte surgery, and a truncation sweep.

use super::fixture::{COFF, OPTIONAL, PeSpec, RAW, SECTION_HEADER, VA, put16, put32};
use super::*;

fn parsed(bytes: &[u8]) -> PeImage {
    parse_pe(bytes).expect("parses").expect("an MZ image")
}

fn refused(bytes: &[u8]) -> String {
    parse_pe(bytes).expect_err("refused")
}

#[test]
fn imports_and_delay_imports_round_trip_in_order_with_their_case() {
    let spec = PeSpec::dll(&["python314.dll", "KERNEL32.dll", "VCRUNTIME140.dll"])
        .delay(&["api-ms-win-crt-runtime-l1-1-0.dll", "Ws2_32.DLL"]);
    let image = parsed(&spec.bytes());
    assert_eq!(
        image,
        PeImage {
            machine: MACHINE_AMD64,
            pe32_plus: true,
            dll: true,
            imports: vec![
                "python314.dll".to_string(),
                "KERNEL32.dll".to_string(),
                "VCRUNTIME140.dll".to_string()
            ],
            delay_imports: vec![
                "api-ms-win-crt-runtime-l1-1-0.dll".to_string(),
                "Ws2_32.DLL".to_string()
            ],
        }
    );
}

#[test]
fn bytes_without_mz_are_not_an_image() {
    assert_eq!(parse_pe(b""), Ok(None));
    assert_eq!(parse_pe(b"\x7fELF"), Ok(None));
    assert_eq!(parse_pe(b"M"), Ok(None));
    assert!(!is_mz(b"ssl"));
    assert!(is_mz(b"MZ"));
}

#[test]
fn a_pe32_image_and_a_non_dll_are_parsed_and_reported() {
    let image = parsed(&PeSpec::dll(&["KERNEL32.dll"]).pe32().bytes());
    assert_eq!((image.machine, image.pe32_plus), (0x14c, false));
    assert_eq!(image.imports, ["KERNEL32.dll"]);
    let image = parsed(&PeSpec::dll(&[]).machine(0xaa64).not_dll().bytes());
    assert_eq!((image.machine, image.dll), (0xaa64, false));
}

/// A zero directory RVA is an empty list whatever its size, as LLVM reads
/// it; so is a directory past `NumberOfRvaAndSizes`.
#[test]
fn absent_directories_are_empty_lists() {
    let image = parsed(&PeSpec::dll(&[]).bytes());
    assert!(image.imports.is_empty() && image.delay_imports.is_empty());
    let mut bytes = PeSpec::dll(&[]).bytes();
    put32(&mut bytes, OPTIONAL + 112 + 8 + 4, 40);
    assert!(parsed(&bytes).imports.is_empty());
    let spec = PeSpec::dll(&["KERNEL32.dll"])
        .delay(&["x.dll"])
        .rva_count(2);
    let image = parsed(&spec.bytes());
    assert_eq!(image.imports, ["KERNEL32.dll"]);
    assert!(image.delay_imports.is_empty());
}

/// The delay list's terminator has `Attributes = 0`: it is recognized
/// before the RVA-based check, so a valid list parses.
#[test]
fn a_delay_list_ends_at_its_terminator_before_the_attribute_check() {
    let image = parsed(&PeSpec::dll(&[]).delay(&["a.dll", "b.dll"]).bytes());
    assert_eq!(image.delay_imports, ["a.dll", "b.dll"]);
}

#[test]
fn a_va_based_delay_descriptor_is_refused() {
    let mut bytes = PeSpec::dll(&[]).delay(&["a.dll"]).bytes();
    put32(&mut bytes, RAW + 20, 0);
    assert_eq!(refused(&bytes), "delay-import descriptor is not RVA-based");
}

#[test]
fn a_missing_pe_signature_is_refused() {
    let mut bytes = PeSpec::dll(&[]).bytes();
    bytes[0x40] = b'X';
    assert!(refused(&bytes).contains("no `PE` signature"));
    let mut bytes = PeSpec::dll(&[]).bytes();
    put32(&mut bytes, 0x3C, u32::MAX);
    assert!(refused(&bytes).contains("no `PE` signature"));
}

#[test]
fn an_unknown_optional_header_magic_is_refused() {
    let mut bytes = PeSpec::dll(&[]).bytes();
    put16(&mut bytes, OPTIONAL, 0x107);
    assert!(refused(&bytes).contains("magic 0x107"));
}

#[test]
fn a_short_optional_header_is_refused() {
    let mut bytes = PeSpec::dll(&[]).bytes();
    put16(&mut bytes, COFF + 16, 111);
    assert!(refused(&bytes).contains("111 bytes"));
    let mut bytes = PeSpec::dll(&[]).pe32().bytes();
    put16(&mut bytes, COFF + 16, 95);
    assert!(refused(&bytes).contains("its 96 fixed bytes"));
}

#[test]
fn a_section_table_past_the_end_is_refused() {
    let mut bytes = PeSpec::dll(&[]).bytes();
    put16(&mut bytes, COFF + 2, 60);
    assert_eq!(refused(&bytes), past_end());
}

#[test]
fn a_directory_rva_outside_every_section_is_refused() {
    let mut bytes = PeSpec::dll(&["a.dll"]).bytes();
    put32(&mut bytes, OPTIONAL + 112 + 8, 0x10);
    assert!(refused(&bytes).contains("import directory RVA 0x10 maps to no section"));
}

/// Memory past `SizeOfRawData` is zero-fill: a descriptor list there maps
/// to no file data, even though the RVA is inside the section's
/// `VirtualSize`.
#[test]
fn a_directory_in_a_sections_zero_fill_is_refused() {
    let mut bytes = PeSpec::dll(&[]).delay(&["b.dll"]).bytes();
    put32(&mut bytes, SECTION_HEADER + 8, 0x1000);
    put32(&mut bytes, SECTION_HEADER + 16, 20);
    let err = refused(&bytes);
    assert!(err.contains("delay-import directory RVA 0x2014"), "{err}");
}

#[test]
fn a_descriptor_list_without_a_terminator_is_refused() {
    // The name moves into the terminator's place, and the section ends
    // before a second descriptor would.
    let mut bytes = PeSpec::dll(&["a.dll"]).bytes();
    bytes[RAW + 20..RAW + 26].copy_from_slice(b"a.dll\0");
    put32(&mut bytes, RAW + 12, VA + 20);
    put32(&mut bytes, SECTION_HEADER + 8, 30);
    put32(&mut bytes, SECTION_HEADER + 16, 30);
    let err = refused(&bytes);
    assert!(
        err.contains("import directory has no terminating descriptor"),
        "{err}"
    );
}

#[test]
fn a_name_rva_outside_every_section_is_refused() {
    let mut bytes = PeSpec::dll(&["a.dll"]).bytes();
    put32(&mut bytes, RAW + 12, VA + 0x5000);
    assert!(refused(&bytes).contains("import name RVA 0x7000"));
}

#[test]
fn a_name_without_a_nul_is_refused() {
    let long = "x".repeat(300);
    let bytes = PeSpec::dll(&[&long]).bytes();
    assert!(refused(&bytes).contains("no NUL within 260 bytes"));
    let mut bytes = PeSpec::dll(&["a.dll"]).bytes();
    let end = bytes.len();
    bytes[end - 1] = b'x';
    assert!(refused(&bytes).contains("no NUL"));
}

#[test]
fn an_empty_or_non_ascii_name_is_refused() {
    let bytes = PeSpec::dll(&[""]).bytes();
    assert!(refused(&bytes).contains("empty or not ASCII"));
    let bytes = PeSpec::dll(&["caf\u{e9}.dll"]).bytes();
    assert!(refused(&bytes).contains("empty or not ASCII"));
}

/// Every prefix of a valid image parses or is refused; none panics.
#[test]
fn every_truncation_is_refused_without_a_panic() {
    let bytes = PeSpec::dll(&["python314.dll", "KERNEL32.dll"])
        .delay(&["x.dll"])
        .bytes();
    for len in 0..bytes.len() {
        let result = parse_pe(&bytes[..len]);
        assert!(!matches!(result, Ok(Some(_))), "{len}");
    }
    assert!(parse_pe(&bytes).is_ok_and(|image| image.is_some()));
}

#[test]
fn api_sets_are_recognized_case_insensitively() {
    for name in [
        "api-ms-win-crt-runtime-l1-1-0.dll",
        "API-MS-WIN-core-synch-l1-2-0.dll",
        "ext-ms-win-ntuser-window-l1-1-0.dll",
        "EXT-MS-onecore.dll",
    ] {
        assert!(is_api_set(name), "{name}");
    }
    for name in [
        "api-ms-winx.dll",
        "kernel32.dll",
        "ext-msx.dll",
        "api-ms.dll",
    ] {
        assert!(!is_api_set(name), "{name}");
    }
}

fn forwarders(bytes: &[u8]) -> Vec<String> {
    parse_forwarders(bytes).expect("the forwarders parse")
}

fn forwarder_refused(bytes: &[u8]) -> String {
    parse_forwarders(bytes).expect_err("refused")
}

/// No export directory, a zero `NumberOfRvaAndSizes` and an empty address
/// table all forward nowhere.
#[test]
fn an_image_without_forwarders_forwards_nowhere() {
    assert!(forwarders(&PeSpec::dll(&["KERNEL32.dll"]).bytes()).is_empty());
    let spec = PeSpec::dll(&[]).forwards(&["NTDLL.RtlAllocateHeap"]);
    assert!(forwarders(&spec.clone().rva_count(0).bytes()).is_empty());
    let mut bytes = spec.bytes();
    put32(&mut bytes, spec.export_offset() + 20, 0);
    assert!(forwarders(&bytes).is_empty());
}

/// Each forwarder names its module before the last `.`, with `.dll`
/// appended unless present; duplicates fold case-insensitively; an
/// ordinary export beside them is skipped; a forwarder longer than a
/// file name is read to its NUL.
#[test]
fn forwarders_name_their_modules_once_in_first_seen_order() {
    let long = format!("python314.{}", "x".repeat(300));
    let spec = PeSpec::dll(&["KERNEL32.dll"]).plain_export().forwards(&[
        "NTDLL.RtlAllocateHeap",
        &long,
        "ntdll.RtlFreeHeap",
        "api-ms-win-core-heap-l1-1-0.HeapAlloc",
        "helper.dll.#12",
        "HELPER.DLL.Other",
    ]);
    assert_eq!(
        forwarders(&spec.bytes()),
        [
            "NTDLL.dll",
            "python314.dll",
            "api-ms-win-core-heap-l1-1-0.dll",
            "helper.dll"
        ]
    );
    assert!(parse_pe(&spec.bytes()).is_ok_and(|image| image.is_some()));
}

#[test]
fn a_forwarder_without_a_module_is_refused() {
    let bytes = PeSpec::dll(&[]).forwards(&["nodot"]).bytes();
    assert_eq!(
        forwarder_refused(&bytes),
        "the forwarder `nodot` names no module"
    );
}

#[test]
fn a_forwarder_without_a_nul_in_the_export_directory_is_refused() {
    let mut bytes = PeSpec::dll(&[]).forwards(&["a.b"]).bytes();
    let end = bytes.len();
    bytes[end - 1] = b'x';
    let err = forwarder_refused(&bytes);
    assert!(
        err.contains("has no NUL within the export directory"),
        "{err}"
    );
}

#[test]
fn an_address_table_past_its_section_is_refused() {
    let spec = PeSpec::dll(&[]).forwards(&["a.b"]);
    let mut bytes = spec.bytes();
    put32(&mut bytes, spec.export_offset() + 20, 1000);
    let err = forwarder_refused(&bytes);
    assert_eq!(err, "the export address table runs past its section");
    let mut bytes = spec.bytes();
    put32(&mut bytes, spec.export_offset() + 28, 0x10);
    let err = forwarder_refused(&bytes);
    assert!(
        err.contains("export address table RVA 0x10 maps to no section"),
        "{err}"
    );
}

#[test]
fn an_export_directory_outside_its_section_is_refused() {
    let spec = PeSpec::dll(&[]).forwards(&["a.b"]);
    let mut bytes = spec.bytes();
    put32(&mut bytes, OPTIONAL + 112, 0x10);
    let err = forwarder_refused(&bytes);
    assert!(
        err.contains("export directory RVA 0x10 maps to no section"),
        "{err}"
    );
    // The directory starts in the section but its 40 bytes run past it.
    let mut bytes = spec.bytes();
    let section_len = (bytes.len() - RAW) as u32;
    put32(&mut bytes, OPTIONAL + 112, VA + section_len - 8);
    let err = forwarder_refused(&bytes);
    assert_eq!(err, "the export directory runs past its section");
}

/// A declared export range wider than the section admits an address past
/// the section's file data, which is refused, not read.
#[test]
fn a_forwarder_outside_every_section_is_refused() {
    let spec = PeSpec::dll(&[]).forwards(&["a.b"]);
    let mut bytes = spec.bytes();
    put32(&mut bytes, OPTIONAL + 112 + 4, 0x10000);
    put32(&mut bytes, spec.export_offset() + 40, VA + 0x8000);
    let err = forwarder_refused(&bytes);
    assert!(
        err.contains("export forwarder RVA 0xa000 maps to no section"),
        "{err}"
    );
}

/// A truncated file whose section headers still declare the full section
/// places a forwarder past the end of the file: refused, never a panic.
#[test]
fn every_truncation_of_a_forwarding_image_is_refused_without_a_panic() {
    let bytes = PeSpec::dll(&["KERNEL32.dll"])
        .forwards(&["NTDLL.RtlAllocateHeap", "python314.Py_Initialize"])
        .bytes();
    for len in 0..bytes.len() {
        forwarder_refused(&bytes[..len]);
    }
    assert_eq!(forwarders(&bytes), ["NTDLL.dll", "python314.dll"]);
}

/// A section declared larger than the file places a forwarder past the
/// file's end: refused, never read.
#[test]
fn a_forwarder_past_the_end_of_the_file_is_refused() {
    let spec = PeSpec::dll(&[]).forwards(&["a.b"]);
    let mut bytes = spec.bytes();
    put32(&mut bytes, SECTION_HEADER + 8, 0x10000);
    put32(&mut bytes, SECTION_HEADER + 16, 0x10000);
    put32(&mut bytes, OPTIONAL + 112 + 4, 0x10000);
    put32(&mut bytes, spec.export_offset() + 40, VA + 0x8000);
    let err = forwarder_refused(&bytes);
    assert_eq!(
        err,
        "the forwarder at RVA 0xa000 lies past the end of the file"
    );
}

#[test]
fn forwarders_of_a_malformed_image_are_refused() {
    let mut bytes = PeSpec::dll(&[]).forwards(&["a.b"]).bytes();
    bytes[0x40] = b'X';
    assert!(forwarder_refused(&bytes).contains("no `PE` signature"));
}
