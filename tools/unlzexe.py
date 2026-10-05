#!/usr/bin/env python3
"""Decompress an LZEXE 0.91 packed DOS EXE into its load image (+ rebuilt MZ header)."""
import struct, sys
def unpack(d):
    hdr_paras = struct.unpack_from('<H', d, 8)[0]
    ip, cs = struct.unpack_from('<HH', d, 0x14)
    base = hdr_paras * 16
    lz = base + cs * 16                       # decompressor segment (file offset)
    rip, rcs, rsp, rss, csize = struct.unpack_from('<HHHHH', d, lz)
    src = base; out = bytearray()
    bits = struct.unpack_from('<H', d, src)[0]; src += 2; nb = 16
    def bit():
        nonlocal bits, nb, src
        b = bits & 1; bits >>= 1; nb -= 1
        if nb == 0:
            bits = struct.unpack_from('<H', d, src)[0]; src += 2; nb = 16
        return b
    def byte():
        nonlocal src
        src += 1; return d[src - 1]
    while True:
        if bit(): out.append(byte()); continue
        if not bit():
            ln = (bit() << 1 | bit()) + 2
            span = byte() - 0x100
        else:
            lo, hi = byte(), byte()
            span = (lo | ((hi & 0xF8) << 5) | 0xE000) - 0x10000
            ln = hi & 7
            if ln: ln += 2
            else:
                ln = byte()
                if ln == 0: break
                if ln == 1: continue
                ln += 1
        for _ in range(ln): out.append(out[span])
    # relocations (0.91: at decompressor+0x158)
    p = lz + 0x158; seg = 0; off = 0; rel = []
    while True:
        s = d[p]; p += 1
        if s == 0:
            s = struct.unpack_from('<H', d, p)[0]; p += 2
            if s == 0: seg += 0xFFF; continue
            if s == 1: break
        off += s
        while off > 0xF: seg += 1; off -= 0x10
        rel.append((off, seg))
    return out, rel, (rip, rcs, rsp, rss)
if __name__ == '__main__':
    d = open(sys.argv[1], 'rb').read()
    img, rel, (ip, cs, sp, ss) = unpack(d)
    hsize = (0x1C + 4 * len(rel) + 15) // 16
    total = hsize * 16 + len(img)
    h = struct.pack('<2sHHHHHHHHHHHHH', b'MZ', total % 512, (total + 511) // 512, len(rel), hsize, 0, 0xFFFF, ss, sp, 0, ip, cs, 0x1C, 0)
    h += b''.join(struct.pack('<HH', o, s) for o, s in rel)
    h += b'\0' * (hsize * 16 - len(h))
    open(sys.argv[2], 'wb').write(h + img)
    print(f'{sys.argv[1]}: image {len(img)} bytes, {len(rel)} relocs, entry {cs:04x}:{ip:04x}')
