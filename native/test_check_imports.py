"""Synthetic PE files for check_imports; run with: python -m unittest test_check_imports"""
import struct
import tempfile
import unittest
from pathlib import Path

import check_imports


def pe(imports=(), delayed=()):
    """A minimal 64-bit PE with one section holding the given import tables."""
    data = bytearray(0x800)
    data[:2] = b'MZ'
    struct.pack_into('<I', data, 0x3c, 0x40)
    data[0x40:0x44] = b'PE\0\0'
    struct.pack_into('<HH', data, 0x44, 0x8664, 1)
    struct.pack_into('<H', data, 0x40 + 20, 240)
    optional = 0x40 + 24
    struct.pack_into('<H', data, optional, 0x20b)
    struct.pack_into('<II', data, optional + 120, 0x1000 if imports else 0, 0)
    struct.pack_into('<II', data, optional + 216, 0x1100 if delayed else 0, 0)
    struct.pack_into('<IIII', data, optional + 240 + 8, 0x400, 0x1000, 0x400, 0x400)
    names = 0x1200
    def name(text):
        nonlocal names
        rva = names
        encoded = text.encode('ascii') + b'\0'
        data[0x400 + rva - 0x1000:0x400 + rva - 0x1000 + len(encoded)] = encoded
        names += len(encoded)
        return rva
    for i, dll in enumerate(imports):
        struct.pack_into('<I', data, 0x400 + i * 20 + 12, name(dll))
    for i, dll in enumerate(delayed):
        struct.pack_into('<I', data, 0x400 + 0x100 + i * 32 + 4, name(dll))
    return bytes(data)


class ImportCheck(unittest.TestCase):
    def check(self, image):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'sample.exe'
            path.write_bytes(image)
            check_imports.verify(path)

    def test_static_runtime_passes(self):
        self.check(pe(['KERNEL32.dll'], ['dwmapi.dll']))

    def test_dynamic_vc_runtime_fails(self):
        with self.assertRaises(RuntimeError):
            self.check(pe(['KERNEL32.dll', 'VCRUNTIME140.dll']))

    def test_dynamic_ucrt_fails(self):
        for dll in ('api-ms-win-crt-runtime-l1-1-0.dll', 'ucrtbase.dll'):
            with self.assertRaises(RuntimeError):
                self.check(pe(['KERNEL32.dll', dll]))

    def test_delay_loaded_runtime_fails(self):
        with self.assertRaises(RuntimeError):
            self.check(pe(['KERNEL32.dll'], ['MSVCP140.dll']))


if __name__ == '__main__':
    unittest.main()
