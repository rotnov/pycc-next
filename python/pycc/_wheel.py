"""Wheel and sdist writing: the platform tag of a compiled wheel, RECORD hashes,
and reproducible zip and tar archives."""

from __future__ import annotations

import base64
import gzip
import hashlib
import io
import os
import pathlib
import platform
import re
import struct
import sys
import tarfile
import time
import zipfile

# The compiled modules use the stable ABI of CPython 3.13 (`Py_LIMITED_API`
# 0x030d0000), so one wheel serves CPython 3.13 and newer.
ABI3_PYTHON = "cp313"

# Shared libraries a manylinux wheel may depend on without bundling them
# (auditwheel's manylinux policy whitelist; the dynamic loader itself too).
MANYLINUX_LIBS = frozenset({
    "libc.so.6", "libm.so.6", "libdl.so.2", "librt.so.1", "libpthread.so.0",
    "libutil.so.1", "libnsl.so.1", "libresolv.so.2", "libgcc_s.so.1", "libstdc++.so.6",
    "ld-linux-x86-64.so.2", "ld-linux-aarch64.so.1", "ld-linux.so.2",
    "libX11.so.6", "libXext.so.6", "libXrender.so.1", "libICE.so.6", "libSM.so.6",
    "libGL.so.1", "libgobject-2.0.so.0", "libgthread-2.0.so.0", "libglib-2.0.so.0",
})

_ARCH = {"amd64": "x86_64", "x86_64": "x86_64", "arm64": "aarch64", "aarch64": "aarch64"}


class TagError(Exception):
    """A platform pycc.build cannot tag a compiled wheel for."""


def elf_needed(path: pathlib.Path) -> list[str] | None:
    """The `DT_NEEDED` entries of the ELF shared object at `path`, or None when it is
    not an ELF file. Parses only the headers (program headers, dynamic section and
    its string table); nothing is loaded or run."""
    data = path.read_bytes()
    if data[:4] != b"\x7fELF":
        return None
    is64 = data[4] == 2
    end = {1: "<", 2: ">"}.get(data[5])
    if end is None or data[4] not in (1, 2):
        raise TagError(f"{path}: unsupported ELF class or byte order")
    if is64:
        phoff, = struct.unpack_from(end + "Q", data, 0x20)
        phentsize, phnum = struct.unpack_from(end + "HH", data, 0x36)
        ph = end + "IIQQQQQQ"  # type, flags, offset, vaddr, paddr, filesz, memsz, align
        dyn, dynsize = end + "qQ", 16
    else:
        phoff, = struct.unpack_from(end + "I", data, 0x1C)
        phentsize, phnum = struct.unpack_from(end + "HH", data, 0x2A)
        ph = end + "IIIIIIII"  # type, offset, vaddr, paddr, filesz, memsz, flags, align
        dyn, dynsize = end + "iI", 8
    loads, dynamic = [], None
    for i in range(phnum):
        fields = struct.unpack_from(ph, data, phoff + i * phentsize)
        if is64:
            ptype, _, offset, vaddr, _, filesz, _, _ = fields
        else:
            ptype, offset, vaddr, _, filesz, _, _, _ = fields
        if ptype == 1:  # PT_LOAD
            loads.append((vaddr, offset, filesz))
        elif ptype == 2:  # PT_DYNAMIC
            dynamic = (offset, filesz)
    if dynamic is None:
        return []
    needed, strtab = [], None
    offset, size = dynamic
    for i in range(size // dynsize):
        tag, val = struct.unpack_from(dyn, data, offset + i * dynsize)
        if tag == 0:  # DT_NULL
            break
        if tag == 1:  # DT_NEEDED
            needed.append(val)
        elif tag == 5:  # DT_STRTAB (an address)
            strtab = val
    if not needed:
        return []
    if strtab is None:
        raise TagError(f"{path}: DT_NEEDED without a string table")
    for vaddr, off, filesz in loads:
        if vaddr <= strtab < vaddr + filesz:
            base = strtab - vaddr + off
            break
    else:
        raise TagError(f"{path}: the dynamic string table is outside every loaded segment")
    names = []
    for n in needed:
        stop = data.index(b"\0", base + n)
        names.append(data[base + n:stop].decode("ascii", "replace"))
    return names


def macho_min_version(path: pathlib.Path) -> tuple[int, int] | None:
    """The minimum macOS version a 64-bit little-endian Mach-O file records
    (`LC_BUILD_VERSION`, or the older `LC_VERSION_MIN_MACOSX`), or None when it is
    not one or records none. Reads only the load commands."""
    data = path.read_bytes()
    if data[:4] != b"\xcf\xfa\xed\xfe":
        return None
    ncmds, sizeofcmds = struct.unpack_from("<II", data, 16)
    offset, end = 32, 32 + sizeofcmds
    for _ in range(ncmds):
        if offset + 8 > end:
            break
        cmd, cmdsize = struct.unpack_from("<II", data, offset)
        if cmd == 0x32:  # LC_BUILD_VERSION: platform, minos, sdk, ntools
            platform_id, minos = struct.unpack_from("<II", data, offset + 8)
            if platform_id == 1:  # PLATFORM_MACOS
                return minos >> 16, (minos >> 8) & 0xFF
        elif cmd == 0x24:  # LC_VERSION_MIN_MACOSX: version, sdk
            version, = struct.unpack_from("<I", data, offset + 8)
            return version >> 16, (version >> 8) & 0xFF
        if cmdsize < 8:
            break
        offset += cmdsize
    return None


def platform_tag(compiled: list[pathlib.Path], *, system: str | None = None,
                 machine: str | None = None, libc: tuple[str, str] | None = None,
                 mac_version: str | None = None, env: dict | None = None) -> str:
    """The platform tag of a wheel holding the extension modules `compiled`, built for
    this machine (the keyword arguments stand in for the host in tests).

    Linux with glibc: `manylinux_<major>_<minor>_<arch>` with the host's glibc version,
    an upper bound of the symbol versions anything built here requires, as long as the
    modules link only libraries the manylinux policy allows (else `linux_<arch>`, which
    pip installs on the machine that built it). macOS: `macosx_<major>_0_<arch>` for
    the newest minimum macOS version the modules record (`LC_BUILD_VERSION`; pycc
    targets `MACOSX_DEPLOYMENT_TARGET` when set, else the running macOS); pip only
    knows `_0` minor versions from macOS 11 on."""
    system = system or platform.system()
    machine = (machine or platform.machine()).lower()
    env = os.environ if env is None else env
    if system == "Linux":
        arch = _ARCH.get(machine, machine)
        lib, version = libc if libc is not None else platform.libc_ver()
        m = re.match(r"^(\d+)\.(\d+)", version or "")
        if lib != "glibc" or not m:
            return f"linux_{arch}"
        for so in compiled:
            needed = elf_needed(so)
            if needed is None:
                raise TagError(f"{so} is not an ELF shared object")
            foreign = [n for n in needed if n not in MANYLINUX_LIBS]
            if foreign:
                print(f"pycc.build: {so.name} links {', '.join(foreign)}, which manylinux does "
                      f"not allow; tagging the wheel linux_{arch}", file=sys.stderr)
                return f"linux_{arch}"
        return f"manylinux_{m[1]}_{m[2]}_{arch}"
    if system == "Darwin":
        arch = {"arm64": "arm64", "aarch64": "arm64", "x86_64": "x86_64"}.get(machine)
        if arch is None:
            raise TagError(f"unsupported macOS architecture {machine!r}")
        # The oldest macOS the modules run on is what they record (LC_BUILD_VERSION);
        # without that record, what pycc targeted: the deployment target, else the
        # running macOS.
        found = [v for v in (macho_min_version(so) for so in compiled) if v is not None]
        if found:
            major, minor = max(found)
        else:
            version = env.get("MACOSX_DEPLOYMENT_TARGET") or mac_version or platform.mac_ver()[0]
            m = re.match(r"^(\d+)(?:\.(\d+))?", version or "")
            if not m:
                raise TagError(f"cannot tell the macOS version from {version!r}")
            major, minor = int(m[1]), int(m[2] or 0)
        if major >= 11:
            minor = 0
        return f"macosx_{major}_{minor}_{arch}"
    raise TagError(f"pycc builds extension modules for Linux and macOS, not {system}")


def record_hash(data: bytes) -> str:
    """A RECORD hash: `sha256=` and the urlsafe base64 digest without padding."""
    digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=")
    return "sha256=" + digest.decode("ascii")


def _epoch() -> int:
    """`SOURCE_DATE_EPOCH` when set (reproducible builds), else 1980-01-01, the
    earliest date a zip entry holds."""
    floor = 315532800
    try:
        return max(int(os.environ["SOURCE_DATE_EPOCH"]), floor)
    except (KeyError, ValueError):
        return floor


class WheelWriter:
    """Collects `(path in wheel, bytes, executable)` entries and writes the wheel,
    `.dist-info` last with its RECORD."""

    def __init__(self, dist_info: str):
        self.dist_info = dist_info
        self.entries: dict[str, tuple[bytes, bool]] = {}

    def add(self, arcname: str, data: bytes, executable: bool = False) -> None:
        if arcname in self.entries:
            raise ValueError(f"{arcname} is added twice")
        if arcname.startswith("/") or ".." in arcname.split("/"):
            raise ValueError(f"{arcname} is not a relative path inside the wheel")
        self.entries[arcname] = (data, executable)

    def record(self) -> str:
        rows = [f"{name},{record_hash(data)},{len(data)}"
                for name, (data, _) in self._ordered()]
        rows.append(f"{self.dist_info}/RECORD,,")
        return "\n".join(rows) + "\n"

    def _ordered(self):
        prefix = self.dist_info + "/"
        return sorted(self.entries.items(), key=lambda kv: (kv[0].startswith(prefix), kv[0]))

    def write(self, path: pathlib.Path) -> None:
        stamp = time.gmtime(_epoch())[:6]
        record = self.record().encode()
        tmp = path.with_name(path.name + ".part")
        with zipfile.ZipFile(tmp, "w", zipfile.ZIP_DEFLATED) as z:
            for name, (data, executable) in self._ordered() + [
                    (f"{self.dist_info}/RECORD", (record, False))]:
                info = zipfile.ZipInfo(name, stamp)
                info.external_attr = (0o100755 if executable else 0o100644) << 16
                info.compress_type = zipfile.ZIP_DEFLATED
                z.writestr(info, data)
        os.replace(tmp, path)


def write_sdist(path: pathlib.Path, top: str, files: list[tuple[str, bytes]]) -> None:
    """A gzipped pax tarball of `files` (paths relative to `top/`), normalized:
    sorted, owner 0, mode 0644, the reproducible timestamp."""
    mtime = _epoch()
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w", format=tarfile.PAX_FORMAT) as tar:
        for rel, data in sorted(files):
            info = tarfile.TarInfo(f"{top}/{rel}")
            info.size = len(data)
            info.mtime = mtime
            info.mode = 0o644
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            tar.addfile(info, io.BytesIO(data))
    tmp = path.with_name(path.name + ".part")
    with open(tmp, "wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=mtime) as gz:
            gz.write(buf.getvalue())
    os.replace(tmp, path)
