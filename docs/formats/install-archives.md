# Install media archive formats

## `PART.n` ("defuse")
- Each `PART.n` starts with a 31-byte directory record naming the current directory.
- Records are `name[13]`, `u16 flags` (0x4000 = directory), `u32 total_size`, `u32 chunk_len`,
  `u32 chunk_offset`, `u32 dos_datetime`.
- A file split across disks repeats its header in the next part with `chunk_offset > 0`.
- Extracted by `tools/defuse_list.py`.

## `_XXXX` files ("puff")
- `name[13]`, `u32 crc`, `u16 dos_time`, `u16 dos_date`, then a PKWARE DCL stream.
- The leading `_` replaces the first letter of the original name, which is stored in the header.
- Decoded by `tools/unpuff.py`.

## Executables
- `LSCITV.EXE`, `TSNEXEC.EXE`, `RB.EXE` and `SHOPADV.EXE` are LZEXE 0.91 packed.
- `tools/unlzexe.py` rebuilds an MZ header with relocations around the unpacked image.
