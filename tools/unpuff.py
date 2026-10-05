#!/usr/bin/env python3
"""Decode Sierra Network 'puff' files (_XXXX): name[13], u32 crc, u16 dos time, u16 dos date, then a PKWARE DCL stream."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from dcl import decompress
def unpuff(b):
    name = b[:13].split(b'\0')[0].decode('latin1')
    crc, t, d = struct.unpack_from('<IHH', b, 13)
    return name, decompress(b[21:]), (1980 + (d >> 9), (d >> 5) & 15, d & 31)
if __name__ == '__main__':
    for p in sys.argv[1:]:
        try:
            name, data, date = unpuff(open(p, 'rb').read())
        except Exception as e:
            print(f'{p}: FAILED {e!r}'); continue
        out = os.path.join(os.path.dirname(p), name)
        open(out, 'wb').write(data)
        print(f'{p} -> {name} {len(data)} bytes {date[0]}-{date[1]:02d}-{date[2]:02d}')
