#!/usr/bin/env python3
"""Minimal SCI0/SCI01 resource.map + resource.00N reader (stored resources only for now)."""
import struct, sys, os, collections
from dcl import decompress as dcl_decompress
TYPES = ['view','pic','script','text','sound','memory','vocab','font','cursor','patch','bitmap','palette','cdaudio','audio','sync','message','map','heap']
def read_map(d):
    m = open(os.path.join(d, 'RESOURCE.MAP'), 'rb').read()
    out = {}
    for i in range(0, len(m) - 5, 6):
        rid, loc = struct.unpack_from('<HI', m, i)
        if rid == 0xffff and loc == 0xffffffff: break
        out[(rid >> 11, rid & 0x7ff)] = (loc >> 26, loc & 0x3ffffff)
    return out
def header(d, vol, off):
    with open(os.path.join(d, 'RESOURCE.%03d' % vol), 'rb') as f:
        f.seek(off); h = f.read(8)
        rid, csz, dsz, meth = struct.unpack('<HHHH', h)
        return rid, csz - 4, dsz, meth, f.read(csz - 4)
def load(d, mp, key):
    rid, csz, dsz, meth, data = header(d, *mp[key])
    if meth == 0: return data[:dsz]
    if meth in (8, 18, 19, 20): return dcl_decompress(data, dsz)
    raise ValueError('method %d' % meth)
if __name__ == '__main__':
    d = sys.argv[1]
    mp = read_map(d)
    meths = collections.Counter(); types = collections.Counter()
    for (t, n), (v, o) in sorted(mp.items()):
        try:
            rid, csz, dsz, meth, _ = header(d, v, o)
        except FileNotFoundError:
            meth = 'missing-vol%d' % v
        meths[meth] += 1; types[TYPES[t] if t < len(TYPES) else t] += 1
    print(d, len(mp), 'resources; types', dict(types), 'methods', dict(meths))
    if (6, 999) in mp: print('vocab.999 at', mp[(6, 999)], header(d, *mp[(6, 999)])[1:4])
