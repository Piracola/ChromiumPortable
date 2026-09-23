import ctypes
import os
from ctypes import wintypes
from pathlib import Path


MACHINE_NAMES = {
    0x014C: "x86",
    0x8664: "x64",
    0xAA64: "arm64",
}


def _pe_layout(data):
    if len(data) < 0x40 or data[:2] != b"MZ":
        raise RuntimeError("Not a PE image (missing MZ signature).")

    pe_offset = int.from_bytes(data[0x3C:0x40], "little")
    if data[pe_offset:pe_offset + 4] != b"PE\0\0":
        raise RuntimeError("Not a PE image (missing PE signature).")

    coff = pe_offset + 4
    machine = int.from_bytes(data[coff:coff + 2], "little")
    section_count = int.from_bytes(data[coff + 2:coff + 4], "little")
    optional_size = int.from_bytes(data[coff + 16:coff + 18], "little")
    optional = coff + 20
    magic = int.from_bytes(data[optional:optional + 2], "little")
    if magic == 0x20B:
        data_directories = optional + 112
    elif magic == 0x10B:
        data_directories = optional + 96
    else:
        raise RuntimeError(f"Unsupported PE optional header magic 0x{magic:x}.")

    directories = []
    for index in range(16):
        entry = data_directories + index * 8
        directories.append((
            int.from_bytes(data[entry:entry + 4], "little"),
            int.from_bytes(data[entry + 4:entry + 8], "little"),
        ))

    sections = []
    section_table = optional + optional_size
    for index in range(section_count):
        entry = section_table + index * 40
        sections.append({
            "name": data[entry:entry + 8].rstrip(b"\0").decode("ascii", errors="replace"),
            "virtual_size": int.from_bytes(data[entry + 8:entry + 12], "little"),
            "virtual_address": int.from_bytes(data[entry + 12:entry + 16], "little"),
            "raw_size": int.from_bytes(data[entry + 16:entry + 20], "little"),
            "raw_pointer": int.from_bytes(data[entry + 20:entry + 24], "little"),
        })
    return machine, directories, sections


def _rva_to_offset(sections, rva):
    for section in sections:
        span = max(section["virtual_size"], section["raw_size"])
        start = section["virtual_address"]
        if span and start <= rva < start + span:
            return section["raw_pointer"] + (rva - start)
    return None


def read_pe_machine(path):
    data = Path(path).read_bytes()
    machine, _, _ = _pe_layout(data)
    return MACHINE_NAMES.get(machine, f"0x{machine:04x}")


def iter_pe_resources(path):
    """Yield ((type, name, language), bytes) without loading or executing the PE."""
    data = Path(path).read_bytes()
    _, directories, sections = _pe_layout(data)
    resource_rva, resource_size = directories[2]
    if not resource_rva or not resource_size:
        return

    resource_base = _rva_to_offset(sections, resource_rva)
    if resource_base is None:
        return

    def read_name(value):
        if not value & 0x80000000:
            return value
        offset = resource_base + (value & 0x7FFFFFFF)
        length = int.from_bytes(data[offset:offset + 2], "little")
        raw = data[offset + 2:offset + 2 + length * 2]
        return raw.decode("utf-16le", errors="replace")

    def walk(directory_offset, parts):
        absolute = resource_base + directory_offset
        if absolute < 0 or absolute + 16 > len(data):
            return
        named = int.from_bytes(data[absolute + 12:absolute + 14], "little")
        numbered = int.from_bytes(data[absolute + 14:absolute + 16], "little")
        entry_count = named + numbered
        for index in range(entry_count):
            entry = absolute + 16 + index * 8
            if entry + 8 > len(data):
                continue
            name_value = int.from_bytes(data[entry:entry + 4], "little")
            child_value = int.from_bytes(data[entry + 4:entry + 8], "little")
            name = read_name(name_value)
            child_offset = child_value & 0x7FFFFFFF
            if child_value & 0x80000000:
                yield from walk(child_offset, (*parts, name))
                continue

            data_entry = resource_base + child_offset
            if data_entry + 16 > len(data):
                continue
            payload_rva = int.from_bytes(data[data_entry:data_entry + 4], "little")
            payload_size = int.from_bytes(data[data_entry + 4:data_entry + 8], "little")
            payload_offset = _rva_to_offset(sections, payload_rva)
            if payload_offset is None or payload_offset + payload_size > len(data):
                continue
            identifiers = (*parts, name)
            while len(identifiers) < 3:
                identifiers = (*identifiers, 0)
            yield identifiers[:3], data[payload_offset:payload_offset + payload_size]

    yield from walk(0, ())


def read_version_info(path):
    """Read Windows VERSIONINFO as data. The executable is never started."""
    if os.name != "nt":
        return {}

    version = ctypes.WinDLL("version", use_last_error=True)
    version.GetFileVersionInfoSizeW.argtypes = [wintypes.LPCWSTR, ctypes.POINTER(wintypes.DWORD)]
    version.GetFileVersionInfoSizeW.restype = wintypes.DWORD
    version.GetFileVersionInfoW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, wintypes.LPVOID]
    version.GetFileVersionInfoW.restype = wintypes.BOOL
    version.VerQueryValueW.argtypes = [wintypes.LPCVOID, wintypes.LPCWSTR, ctypes.POINTER(wintypes.LPVOID), ctypes.POINTER(wintypes.UINT)]
    version.VerQueryValueW.restype = wintypes.BOOL

    file_path = str(Path(path).resolve())
    ignored = wintypes.DWORD()
    size = version.GetFileVersionInfoSizeW(file_path, ctypes.byref(ignored))
    if not size:
        return {}

    buffer = ctypes.create_string_buffer(size)
    if not version.GetFileVersionInfoW(file_path, 0, size, buffer):
        return {}

    translations = []
    pointer = wintypes.LPVOID()
    length = wintypes.UINT()
    if version.VerQueryValueW(buffer, r"\VarFileInfo\Translation", ctypes.byref(pointer), ctypes.byref(length)):
        raw = ctypes.string_at(pointer, length.value)
        translations = [
            (int.from_bytes(raw[index:index + 2], "little"), int.from_bytes(raw[index + 2:index + 4], "little"))
            for index in range(0, len(raw) - 3, 4)
        ]
    translations.extend([(0x0409, 0x04B0), (0x0409, 0x04E4), (0x0804, 0x04B0)])

    fields = (
        "CompanyName",
        "FileDescription",
        "FileVersion",
        "OriginalFilename",
        "ProductName",
        "ProductVersion",
    )
    result = {}
    seen = set()
    for language, codepage in translations:
        key = (language, codepage)
        if key in seen:
            continue
        seen.add(key)
        table = f"{language:04x}{codepage:04x}"
        for field in fields:
            if field in result:
                continue
            query = f"\\StringFileInfo\\{table}\\{field}"
            value_pointer = wintypes.LPVOID()
            value_length = wintypes.UINT()
            if version.VerQueryValueW(buffer, query, ctypes.byref(value_pointer), ctypes.byref(value_length)):
                value = ctypes.wstring_at(value_pointer, value_length.value).rstrip("\0").strip()
                if value:
                    result[field] = value
    return result
