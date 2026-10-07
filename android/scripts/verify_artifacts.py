#!/usr/bin/env python3
"""Verify Miaominal Android artifacts.

Two repeatable checks:

* ``apk``   - every ``.so`` inside an APK (including JNA's libjnidispatch.so)
  must have all PT_LOAD segments aligned to at least 0x4000 (16 KB pages), the
  expected ABI directories and native library names must be present, and stored
  (uncompressed) entries must be 16 KB aligned inside the zip.
* ``abi``   - the UniFFI interface embedded in two native libraries must be
  identical: same ``UNIFFI_META_*`` symbol set with identical metadata bytes,
  and the same exported ``uniffi_*``/``ffi_*`` function names.

Pure Python 3 standard library so it runs on Windows and CI alike.
"""

from __future__ import annotations

import argparse
import hashlib
import struct
import sys
import zipfile
from pathlib import Path

PAGE_16K = 0x4000
PT_LOAD = 1
SHT_SYMTAB = 2
SHT_DYNSYM = 11
STT_OBJECT = 1
STT_FUNC = 2
ELFCLASS64 = 2
ELFDATA2LSB = 1

META_PREFIX = "UNIFFI_META_"
FUNC_PREFIXES = ("uniffi_miaominal_android_", "ffi_miaominal_android_")


class VerificationError(Exception):
    pass


class ElfImage:
    """Minimal ELF64 little-endian reader for program headers, symbols and data."""

    def __init__(self, data: bytes, label: str) -> None:
        self.data = data
        self.label = label
        if data[:4] != b"\x7fELF":
            raise VerificationError(f"{label}: not an ELF file")
        if data[4] != ELFCLASS64 or data[5] != ELFDATA2LSB:
            raise VerificationError(f"{label}: only 64-bit little-endian ELF is supported")
        (
            self.e_phoff,
            self.e_shoff,
        ) = struct.unpack_from("<QQ", data, 0x20)
        (
            self.e_phentsize,
            self.e_phnum,
            self.e_shentsize,
            self.e_shnum,
            self.e_shstrndx,
        ) = struct.unpack_from("<HHHHH", data, 0x36)
        self.load_segments: list[tuple[int, int, int, int, int]] = []
        for index in range(self.e_phnum):
            offset = self.e_phoff + index * self.e_phentsize
            p_type, _flags, p_offset, p_vaddr, _p_paddr, p_filesz, _p_memsz, p_align = (
                struct.unpack_from("<IIQQQQQQ", data, offset)
            )
            if p_type == PT_LOAD:
                self.load_segments.append((p_offset, p_vaddr, p_filesz, p_align, index))
        if not self.load_segments:
            raise VerificationError(f"{label}: no PT_LOAD segments")

    def load_alignments(self) -> list[int]:
        return [segment[3] for segment in self.load_segments]

    def vaddr_to_offset(self, vaddr: int) -> int:
        for p_offset, p_vaddr, p_filesz, _p_align, _index in self.load_segments:
            if p_vaddr <= vaddr < p_vaddr + p_filesz:
                return p_offset + (vaddr - p_vaddr)
        raise VerificationError(f"{self.label}: address 0x{vaddr:x} is not file-backed")

    def section_headers(self) -> list[tuple[int, int, int, int, int, int]]:
        headers = []
        for index in range(self.e_shnum):
            offset = self.e_shoff + index * self.e_shentsize
            sh_name, sh_type, sh_flags, sh_addr, sh_offset, sh_size, sh_link, _info, _align, _entsize = (
                struct.unpack_from("<IIQQQQIIQQ", self.data, offset)
            )
            headers.append((sh_name, sh_type, sh_addr, sh_offset, sh_size, sh_link))
        return headers

    def _section_name(self, name_offset: int) -> str:
        headers = self.section_headers()
        if self.e_shstrndx >= len(headers):
            return ""
        _name, _type, _addr, str_offset, str_size, _link = headers[self.e_shstrndx]
        end = self.data.index(b"\x00", str_offset + name_offset)
        return self.data[str_offset + name_offset : end].decode("utf-8", "replace")

    def symbols(self) -> dict[str, tuple[int, int, int]]:
        """Returns name -> (symbol type, value, size) for every symbol table entry."""
        found: dict[str, tuple[int, int, int]] = {}
        headers = self.section_headers()
        for sh_name, sh_type, _sh_addr, sh_offset, sh_size, sh_link in headers:
            if sh_type not in (SHT_SYMTAB, SHT_DYNSYM):
                continue
            if sh_link >= len(headers):
                continue
            str_offset = headers[sh_link][3]
            entries = sh_size // 24
            for index in range(entries):
                offset = sh_offset + index * 24
                st_name, st_info, _st_other, _st_shndx, st_value, st_size = struct.unpack_from(
                    "<IBBHQQ", self.data, offset
                )
                if st_name == 0:
                    continue
                end = self.data.index(b"\x00", str_offset + st_name)
                name = self.data[str_offset + st_name : end].decode("utf-8", "replace")
                found.setdefault(name, (st_info & 0xF, st_value, st_size))
        return found

    def object_bytes(self, name: str, symbol: tuple[int, int, int]) -> bytes:
        _kind, value, size = symbol
        offset = self.vaddr_to_offset(value)
        return self.data[offset : offset + size]


def read_apk_native_libs(apk: Path) -> dict[str, dict[str, bytes]]:
    """abi -> {library file name: content} for every .so in the APK."""
    result: dict[str, dict[str, bytes]] = {}
    with zipfile.ZipFile(apk) as archive:
        for info in archive.infolist():
            parts = info.filename.split("/")
            if len(parts) != 3 or parts[0] != "lib" or not parts[2].endswith(".so"):
                continue
            result.setdefault(parts[1], {})[parts[2]] = archive.read(info)
    if not result:
        raise VerificationError(f"{apk}: no lib/<abi>/*.so entries found")
    return result


def stored_so_alignment(apk: Path) -> list[tuple[str, int, int]]:
    """Returns (entry name, data offset, alignment remainder) for STORED .so entries."""
    rows = []
    with zipfile.ZipFile(apk) as archive:
        for info in archive.infolist():
            if not info.filename.startswith("lib/") or not info.filename.endswith(".so"):
                continue
            if info.compress_type != zipfile.ZIP_STORED:
                continue
            with open(apk, "rb") as handle:
                handle.seek(info.header_offset)
                header = handle.read(30)
            if header[:4] != b"PK\x03\x04":
                raise VerificationError(f"{apk}: bad local header at {info.header_offset}")
            name_len, extra_len = struct.unpack_from("<HH", header, 26)
            data_offset = info.header_offset + 30 + name_len + extra_len
            rows.append((info.filename, data_offset, data_offset % PAGE_16K))
    return rows


def verify_apk(apk: Path, expected_abis: list[str] | None, uniffi_consistency: bool) -> bool:
    print(f"== APK {apk}")
    native = read_apk_native_libs(apk)
    ok = True

    if expected_abis is not None:
        present = sorted(native)
        if present != sorted(expected_abis):
            print(f"FAIL abi directories {present} != expected {sorted(expected_abis)}")
            ok = False
        else:
            print(f"OK   abi directories: {', '.join(present)}")

    for abi in sorted(native):
        libraries = native[abi]
        if "libmiaominal_android.so" not in libraries:
            print(f"FAIL {abi}: libmiaominal_android.so missing")
            ok = False
        if "libjnidispatch.so" not in libraries:
            print(f"FAIL {abi}: libjnidispatch.so (JNA) missing")
            ok = False
        for name, content in sorted(libraries.items()):
            image = ElfImage(content, f"{apk.name}!lib/{abi}/{name}")
            aligns = image.load_alignments()
            bad = [align for align in aligns if align < PAGE_16K]
            digest = hashlib.sha256(content).hexdigest()[:16]
            if bad:
                print(
                    f"FAIL {abi}/{name}: PT_LOAD align "
                    f"{', '.join(hex(align) for align in bad)} < 0x4000  sha256:{digest}"
                )
                ok = False
            else:
                print(
                    f"OK   {abi}/{name}: {len(aligns)} PT_LOAD, min align "
                    f"{hex(min(aligns))}  sha256:{digest}"
                )

    stored = stored_so_alignment(apk)
    misaligned = [row for row in stored if row[2] != 0]
    if misaligned:
        print("FAIL stored .so entries are not 16 KB aligned inside the zip:")
        for name, offset, remainder in misaligned:
            print(f"     {name}: data offset {offset} (% 0x4000 = {remainder})")
        ok = False
    elif stored:
        print(f"OK   {len(stored)} stored .so entries are 16 KB aligned inside the zip")
    else:
        print("WARN no stored (uncompressed) .so entries; apk uses compressed native libraries")

    if uniffi_consistency:
        images = {}
        for abi, libraries in native.items():
            if "libmiaominal_android.so" in libraries:
                images[abi] = ElfImage(
                    libraries["libmiaominal_android.so"], f"{apk.name}!lib/{abi}/libmiaominal_android.so"
                )
        if len(images) < 2:
            print("FAIL UniFFI consistency needs at least two ABIs in the APK")
            ok = False
        else:
            ok = compare_uniffi(images) and ok

    return ok


def compare_uniffi(images: dict[str, ElfImage]) -> bool:
    ok = True
    abis = sorted(images)
    reference_abi = abis[0]
    reference = uniffi_surface(images[reference_abi])
    for abi in abis[1:]:
        other = uniffi_surface(images[abi])
        if reference[0] != other[0]:
            missing = sorted(reference[0] - other[0])
            extra = sorted(other[0] - reference[0])
            print(f"FAIL {reference_abi} vs {abi}: metadata symbols differ")
            if missing:
                print(f"     only in {reference_abi}: {', '.join(missing[:10])}")
            if extra:
                print(f"     only in {abi}: {', '.join(extra[:10])}")
            ok = False
        else:
            print(f"OK   {reference_abi} vs {abi}: {len(reference[0])} UNIFFI_META_* symbols match")
        if reference[1] != other[1]:
            missing = sorted(reference[1] - other[1])
            extra = sorted(other[1] - reference[1])
            print(f"FAIL {reference_abi} vs {abi}: exported function symbols differ")
            if missing:
                print(f"     only in {reference_abi}: {', '.join(missing[:10])}")
            if extra:
                print(f"     only in {abi}: {', '.join(extra[:10])}")
            ok = False
        else:
            print(f"OK   {reference_abi} vs {abi}: {len(reference[1])} exported function symbols match")

        differing = []
        for name in sorted(reference[0]):
            if name in other[2] and reference[2].get(name) != other[2].get(name):
                differing.append(name)
        if differing:
            print(f"FAIL {reference_abi} vs {abi}: metadata bytes differ for {len(differing)} symbols")
            for name in differing[:10]:
                print(f"     {name}")
            ok = False
        else:
            print(f"OK   {reference_abi} vs {abi}: metadata bytes are identical")
    return ok


def uniffi_surface(image: ElfImage) -> tuple[set[str], set[str], dict[str, bytes]]:
    symbols = image.symbols()
    meta = {
        name: symbol
        for name, symbol in symbols.items()
        if name.startswith(META_PREFIX) and symbol[0] == STT_OBJECT
    }
    functions = {
        name
        for name, symbol in symbols.items()
        if name.startswith(FUNC_PREFIXES) and symbol[0] == STT_FUNC
    }
    meta_bytes = {
        name: hashlib.sha256(image.object_bytes(name, symbol)).digest()
        for name, symbol in meta.items()
    }
    return set(meta), functions, meta_bytes


def verify_abi_pair(path_a: Path, path_b: Path) -> bool:
    print(f"== UniFFI interface {path_a.name} vs {path_b.name}")
    images = {
        f"{path_a.parent.name}/{path_a.name}": ElfImage(path_a.read_bytes(), str(path_a)),
        f"{path_b.parent.name}/{path_b.name}": ElfImage(path_b.read_bytes(), str(path_b)),
    }
    return compare_uniffi(images)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    apk_parser = subparsers.add_parser("apk", help="verify 16 KB alignment inside APKs")
    apk_parser.add_argument("apks", nargs="+", type=Path)
    apk_parser.add_argument(
        "--expect-abis",
        default=None,
        help="comma separated ABI list every APK must contain (default: no check)",
    )
    apk_parser.add_argument(
        "--uniffi-consistency",
        action="store_true",
        help="also compare the UniFFI metadata of every ABI inside each APK",
    )

    abi_parser = subparsers.add_parser("abi", help="compare the UniFFI interface of two .so files")
    abi_parser.add_argument("libraries", nargs=2, type=Path)

    args = parser.parse_args(argv)
    try:
        if args.command == "apk":
            expected = (
                [abi.strip() for abi in args.expect_abis.split(",") if abi.strip()]
                if args.expect_abis
                else None
            )
            results = [
                verify_apk(apk, expected, args.uniffi_consistency) for apk in args.apks
            ]
            ok = all(results)
        else:
            ok = verify_abi_pair(args.libraries[0], args.libraries[1])
    except VerificationError as error:
        print(f"FAIL {error}")
        return 1

    print("RESULT:", "PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
