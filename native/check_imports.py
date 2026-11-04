"""Inspect the actual PE import tables; no extra Python packages are needed."""
import struct
from pathlib import Path


def imported_dlls(path):
    data=Path(path).read_bytes()
    pe=struct.unpack_from('<I',data,0x3c)[0]
    if data[:2]!=b'MZ' or data[pe:pe+4]!=b'PE\0\0':
        raise ValueError('Not a Windows PE executable')
    sections=struct.unpack_from('<H',data,pe+6)[0]
    optional_size=struct.unpack_from('<H',data,pe+20)[0]
    optional=pe+24
    if struct.unpack_from('<H',data,optional)[0]!=0x20b:
        raise ValueError('Expected a 64-bit executable')
    # Data directories 1 (imports) and 13 (delay-load imports).
    imports=struct.unpack_from('<I',data,optional+120)[0]
    delayed=struct.unpack_from('<I',data,optional+216)[0]
    table=optional+optional_size
    def offset(rva):
        for i in range(sections):
            size,address,raw_size,raw=struct.unpack_from('<IIII',data,table+i*40+8)
            if address<=rva<address+max(size,raw_size):
                result=raw+rva-address
                if result<len(data):
                    return result
        raise ValueError('Invalid PE address')
    names=[]
    # (directory RVA, descriptor size, offset of the DLL name RVA)
    for rva,size,field in ((imports,20,12),(delayed,32,4)):
        if not rva:
            continue
        cursor=offset(rva)
        while any(data[cursor:cursor+size]):
            name=offset(struct.unpack_from('<I',data,cursor+field)[0])
            names.append(data[name:data.index(b'\0',name)].decode('ascii'))
            cursor+=size
    return names


def verify(path):
    names=imported_dlls(path)
    # The static CRT build must not need the VC++ runtime or the Universal CRT,
    # whether imported directly or delay-loaded.
    external=[name for name in names if name.upper().startswith(('VCRUNTIME','MSVCP','CONCRT','API-MS-WIN-CRT-','UCRTBASE'))]
    if external:
        raise RuntimeError(f'{Path(path).name} still requires an external C/C++ runtime: {external}')
    print(f'{Path(path).name}: no external Visual C++ runtime imports')


if __name__=='__main__':
    import sys
    for path in sys.argv[1:]:
        verify(path)
