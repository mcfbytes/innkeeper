#!/usr/bin/env python3
"""List/extract Sierra Network / ImagiNation 'defuse' PART.n install archives.

Each PART.n starts with a 31-byte directory record naming the current dir.
Records: name[13], u16 flags (0x4000 = dir), u32 total_size, u32 chunk_len,
u32 chunk_offset, u32 dos_datetime.  Files split across disks repeat their
header in the next part with chunk_offset > 0.  Files named _xxx are further
compressed with Sierra's 'puff' and are left as-is.
"""
import struct, sys, os
def records(data):
    off, cur = 0, ''
    while off + 31 <= len(data):
        name = data[off:off+13].split(b'\0')[0].decode('latin1')
        if not name: return
        flags, total, chunk, coff, dt = struct.unpack_from('<HIIII', data, off+13)
        off += 31
        if flags & 0x4000:
            cur = '' if name == '.' else name
            continue
        yield os.path.join(cur, name), total, coff, data[off:off+chunk], dt
        off += chunk
if __name__ == '__main__':
    d = sys.argv[1]; out = sys.argv[2] if len(sys.argv) > 2 else None
    parts = sorted([p for p in os.listdir(d) if p.upper().startswith('PART.')], key=lambda p: int(p.split('.')[1]))
    files = {}
    for pn in parts:
        for name, total, coff, blob, dt in records(open(os.path.join(d, pn), 'rb').read()):
            buf = files.setdefault(name, [bytearray(total), 0, dt])
            buf[0][coff:coff+len(blob)] = blob; buf[1] += len(blob)
    for name, (buf, got, dt) in files.items():
        print(f'{name:28s} {len(buf):9d} {"OK" if got == len(buf) else "INCOMPLETE %d" % got}')
        if out:
            p = os.path.join(out, name); os.makedirs(os.path.dirname(p) or '.', exist_ok=True); open(p, 'wb').write(buf)
