#!/usr/bin/env python3
"""List/extract the install media's PART.n archives; format in docs/formats/install-archives.md."""
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
