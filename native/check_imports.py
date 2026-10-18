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
    imports=struct.unpack_from('<I',data,optional+120)[0]
    table=optional+optional_size
    def offset(rva):
        for i in range(sections):
            size,address,raw_size,raw=struct.unpack_from('<IIII',data,table+i*40+8)
            if address<=rva<address+max(size,raw_size):
                result=raw+rva-address
                if result<len(data):
                    return result
        raise ValueError('Invalid PE address')
    cursor=offset(imports)
    names=[]
    while any(data[cursor:cursor+20]):
        name_rva=struct.unpack_from('<I',data,cursor+12)[0]
        name=offset(name_rva)
        names.append(data[name:data.index(b'\0',name)].decode('ascii'))
        cursor+=20
    return names


def verify(path):
    names=imported_dlls(path)
    external=[name for name in names if name.upper().startswith(('VCRUNTIME','MSVCP','CONCRT'))]
    if external:
        raise RuntimeError(f'{Path(path).name} still requires an external Visual C++ runtime: {external}')
    print(f'{Path(path).name}: no external Visual C++ runtime imports')


if __name__=='__main__':
    import sys
    for path in sys.argv[1:]:
        verify(path)
