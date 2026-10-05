# Coding conventions

Binding for every change in this project. A violation is a defect, not a style preference.
Gate: `tools/check_comments.py` (comment budget); `clang-format`, `rustfmt` and `clippy` where noted.

Three code bases, three rule sets, one set of shared principles (section 1):

| Area | Where | Rule set |
|---|---|---|
| ScummVM fork (C++) | `scummvm/`, branch `lsci` | section 2: ScummVM's own conventions win |
| Server (Rust) | `crates/` | section 3 |
| Tools (Python) | `tools/` | section 4 |

## 1. Shared principles

1. **Reuse before writing.** Look for an existing facility first, extend it second, add code third.
   A new helper needs a one-line justification in the commit message saying what was not reusable.
2. **Names carry the meaning.** If a comment is needed to explain what a name stands for, rename it.
3. **Comment budget: at most two consecutive lines per block, everywhere.** Longer explanations go in
   `docs/` and the code names the document. Comments say why, never what. `check_comments.py` enforces it.
4. **No plan references in code comments** (decision numbers, section numbers, task names). State the
   fact in plain words; the identifier belongs in the doc.
5. **Small and focused.** Function over ~40 lines or a file over ~500 lines (new files only) is a split
   candidate; split along a named concept, never into `part1`/`part2`/`util`.
6. **Parse at the boundary.** Wire bytes, resource bytes and config text become domain types
   (enum, struct, strong integer) at the edge. Inner code never touches raw offsets or magic numbers.
7. **Tables for facts, code for sequences.** Facts (opcode names, kernel signatures, command codes)
   live in a static table with one row per case. Behaviour with an order lives in functions.
8. **Interface only when something differs at run time.** See the decision rule in 1.2.
9. **Errors are values on recoverable paths.** Bad input from the network, a corrupt resource or a
   missing file returns an error; only a violated internal invariant aborts.
10. **Every claim about the original software is labelled** in docs as CONFIRMED (seen in code or data,
    with `seg:off` and file offset) or INFERRED. Code never encodes an INFERRED fact without a named
    constant that says so, for example `kTsnKernelIdAssumed`.
11. **Original game data is never committed**, never copied into `docs/` or `tools/` beyond short
    quoted strings or byte snippets used as evidence.

### 1.1 Naming vocabulary

Suffixes carry fixed meaning in all three languages; do not invent synonyms.

| Name ends in | Meaning |
|---|---|
| `Parser` / `parse_*` | bytes or text in, domain value out, no side effects |
| `Writer` / `encode_*` | domain value in, bytes out |
| `Table` | static rows, no behaviour |
| `Session` | owns the state of one connection or one login |
| `Transport` | moves bytes, knows nothing about their meaning |
| `Handler` | reacts to one decoded message kind |
| `Factory` / `make*` / `from_*` | constructs an object, never mutates existing state |

Receive and transmit are named from the client's point of view: receive came off the host.

### 1.2 Interface decision rule

Stop at the first row that fits.

| Behaviour differs by | Write |
|---|---|
| data only, no sequencing | a static table row |
| a choice fixed at build time | a template parameter (C++) or generic (Rust) |
| a collaborator injected at run time with a genuine second implementation (real socket vs loopback) | an interface one level deep, few verbs, protocol vocabulary |
| steps of one fixed sequence | a function that owns the sequence, calling small named steps |

No inheritance for code reuse. No interface with a single implementation and no test double.

## 2. ScummVM fork (C++)

**ScummVM wins.** Wherever this section and ScummVM's published rules disagree, ScummVM's rules apply
inside `scummvm/`. Sources: wiki "Code Formatting Conventions", "Coding Conventions", "Commit
Guidelines", `scummvm/.clang-format`, `scummvm/AI-GUIDELINES.md`. The shared principles in section 1
still apply on top; where the two meet, the stricter comment budget and naming rules in this file stay.

### 2.1 Reuse map (check here before writing anything)

| Need | Use | Not |
|---|---|---|
| containers, strings | `Common::Array`, `HashMap`, `List`, `String`, `Rect`, `Point` | `std::*` |
| reading game files | `Common::File`, `SeekableReadStream`, `readUint16LE()`, `READ_LE_UINT16` | `fopen`, `fread`, struct casts |
| decompression | the existing `Sci::DecompressorDCL` (method 8) and sibling decompressors | a second inflate/explode |
| resources | `ResourceManager`, `ResourceId`, resource sources in `engines/sci/resource/` | direct file walking |
| kernel calls | `kernel_tables.h` rows plus `k*` functions in the `engines/sci/engine/k*.cpp` files | ad-hoc dispatch |
| script bugs | `script_patches.cpp` signature and patch entries; `workarounds.cpp` | `if (gameId == ...)` in the VM |
| game identification | `detection_tables.h` entries, `ADGF_*` flags, MD5 entries | custom file sniffing |
| debug output | `debugC(kDebugLevel..., ...)` with a registered channel, `warning()`, `error()` | `printf`, `cout` |
| debugger commands | `registerCmd("name", WRAP_METHOD(Console, cmdName))`, `debugPrintf` | a private command loop |
| sockets, HTTP | `backends/networking/basic` and the existing `Networking` backends | raw BSD sockets |
| saves | the SCI savegame serialisation (`Common::Serializer`, `saveLoadWithSerializer`) | hand-rolled save files |
| graphics, palette, sound | `engines/sci/graphics`, `engines/sci/sound` managers | direct surface poking |
| tests | `scummvm/test/` (cxxtest) | ad-hoc `main()` programs |

**Extend, never fork.** LSCI differences are expressed as a version or feature predicate
(`getSciVersion()`, `engine/features.h`) or a new table selected by version. Do not copy an upstream
file to edit it. Keep the diff against `upstream/master` minimal and additive so rebases stay cheap;
new LSCI-only code goes in new files with their own `module.mk` rows.

### 2.2 Formatting (from the ScummVM wiki; `scummvm/.clang-format` approximates it)

- Tabs, tab stop 4. Braces attach (`if (x) {`, `} else {`). `else if` on the same line as `}`.
- A space after `if`, `for`, `while`, `switch`; none after a cast; `const char *p`, `int &r`, `a->b`.
- Namespace contents not indented; close with `} // End of namespace Sci`.
- `case` labels not indented; write exactly `// fall through` where intended.
- Empty loop body is `{}`, never `;`. No composite one-liners (`if (x) y();` goes on two lines).
- Preprocessor directives start at column 0.
- Run `git clang-format upstream/master` on touched lines only. Never reformat a whole upstream file.

### 2.3 Naming

| Thing | Form | Example |
|---|---|---|
| type, template parameter | `UpperCamel` | `TsnSession` |
| method, function, local | `lowerCamel` | `sendFrame()`, `frameSize` |
| data member | `_lowerCamel` | `_segMan`, `_pendingFrames` |
| constant, enumerator | `kUpperCamel` (preferred) or `UPPER_SNAKE` | `kTsnMaxFrame` |
| global (rare) | `g_lowerCamel`, with a comment saying why it is needed | `g_system` |
| header guard | `SCI_<DIR>_<FILE>_H` | `SCI_ENGINE_TSN_H` |
| file | existing SCI pattern: lowercase, no separators for kernel files (`ktsn.cpp`) | |

Match the surrounding file when it uses the second constant style. Prefer `enum`/`const` over `#define`.

### 2.4 Language subset

- C++11 subset. No exceptions, no RTTI, no global objects with constructors, no non-const static
  locals (an engine must be re-entrant after "return to launcher"). Use `nullptr`, `override`, `const`.
- Fixed-width ScummVM types (`byte`, `uint16`, `int32`), `reg_t` for VM values.
- Endianness: use the stream and `endian.h` helpers. Never read a struct by casting a buffer.
- Memory: ownership is explicit. An owner holds a member or `Common::ScopedPtr`; a borrower holds a
  reference or raw pointer that the code names as borrowed. Every `new` has one visible `delete` owner.
- Mark intentional hacks `FIXME`, incomplete work `TODO`, original-game bug fixes `WORKAROUND:` with the
  document that explains the bug. These keywords are ScummVM vocabulary and stay.

### 2.5 Comments in the fork

- The licence header on new files is the ScummVM one, verbatim, and is exempt from the budget.
- Everything else is two lines per block. Doxygen (`/** ... */`, `@param`) is required only for new
  methods in shared code outside `engines/`; inside `engines/` it counts against the budget.
- A protocol or format description longer than two lines becomes a file in `docs/`, and the header
  that implements it names the file in one line.

### 2.6 Structure rules for new code

- **One concept per header.** New headers define one class or struct plus its nested types. Enums,
  constants and static tables do not count.
- **Parse then act.** A message from the server is decoded by a parser into a typed struct; the kernel
  function receives the struct, not the byte buffer.
- **Transport is an interface with two implementations**: the socket one and an in-memory loopback for
  tests and the debugger. This is the single justified interface in the network path.
- **Kernel functions stay thin.** `kTsn` validates arguments, calls one session method, converts the
  result to `reg_t`. Session logic lives in a class that has no dependency on `EngineState`.
- **Tables use designated rows and static storage only** (`static const`, no constructors), like
  `s_kernelMap`. Function pointers inside a table row are fine in the fork, because ScummVM does it.
- **Unknown input is survivable.** An unknown sub-op or opcode logs through `debugC` or `warning`, returns
  a defined failure, and keeps the game running. `error()` is for "the original data is impossible".
- **Debug channels are added next to the existing ones** (`kDebugLevel*` plus the `debugFlagList`
  entry in `detection.cpp`), one per concern: network, modules, compression.
- **Detection entries ship `ADGF_UNSTABLE`** until a full session can be played.

### 2.7 Commits in the fork

- First line `SUBSYSTEM: Summary`, 50 characters or fewer, present tense (`SCI: Add LSCI kernel table`).
  `devtools/check-commit-msg.py` rejects anything not matching `[A-Z_]+:`. Subsystems in use: `SCI`,
  `COMMON`, `BUILD`, `DEVTOOLS`, `GUI`, `I18N`, `ALL`, `JANITORIAL`.
- Body wrapped at about 72 characters, explains why. No merge commits; linear history.
- Every commit builds. One concern per commit: formatting and behaviour never share a commit.
- Do not touch common code or other engines unless the change is required; if you must, keep other
  engines working and say so in the message.
- AI assistance: `scummvm/AI-GUIDELINES.md` applies. Disclose with an `Assisted-by: AGENT:MODEL` trailer
  and never as author or co-author; a human reviews and owns every commit.

## 3. Rust workspace (`innkeeper`)

Toolchain gates: `cargo fmt --check` (config `rustfmt.toml`), `cargo clippy --all-targets -D warnings`,
`cargo test`, `tools/check_comments.py`. Stable Rust only.

### 3.1 Layout and layering

Crates form a downward-only graph. A crate may depend on those below it, never above or sideways.

```
innkeeperd           bin: argument parsing, runtime, sockets, capture files, wiring only
innkeeper-world      accounts, logon, land tables; games, rooms, chat to come (domain behaviour)
innkeeper-session    connection state machines, no I/O
pad_thai, tsn-link,  modem and PAD dialogue, link framing, INT 14h API and transport:
int14h               wire types, parse and encode, no I/O
```

The crates as built are described in `docs/server/innkeeperd.md`.

- Only the bin crate and a thin transport module depend on an async runtime. Everything below is
  synchronous and deterministic: input bytes and a clock value in, output bytes and events out.
- Crates start with `#![forbid(unsafe_code)]`. Exceptions need a doc entry naming the reason.
- A module is one concept; `lib.rs` re-exports the crate's public surface and nothing else.
- Public items are `pub(crate)` until a second crate needs them.

### 3.2 Types and errors

- Wire integers become newtypes (`struct RoomId(u16)`), closed sets become enums, and a `match` over a
  closed enum has no `_` arm. A wire byte with unknown values is a `TryFrom<u8>` that returns an error.
- Fallible functions return `Result<T, E>` with an error enum per crate (`thiserror`). No `unwrap`,
  `expect`, `panic!` or indexing that can panic outside tests; `debug_assert!` and `unreachable!` are
  for proven invariants only.
- Mark results that must not be dropped with `#[must_use]`.
- Protocol facts the original client does not confirm sit in one `assumptions.rs` table of named
  constants, each with a doc reference, so they are easy to find and fix.

### 3.3 Traits, tables, state

- Same decision rule as 1.2. A trait is one level deep, has few methods and has two implementations
  (real and test). Inject `Clock`, `Transport`, `Store` as generics or `Box<dyn ...>` at the edge.
- Command and message tables are `const` arrays of rows. A `const` assertion or a test proves one row
  per enum variant.
- Session state is an enum of named states with transitions as methods returning the next state; no
  `Option` fields that mean "only valid in state X".
- Ownership is single and explicit; shared mutable state crosses tasks only through a channel.

### 3.4 Naming and style

- Standard Rust casing (`snake_case`, `UpperCamel`, `SCREAMING_SNAKE`) plus the suffix vocabulary in 1.1.
- Constructors: `new` (cannot fail), `try_new` or `from_*` (parses or validates), `with_*` (builder step).
- Doc comments (`///`) follow the two-line budget; the crate-level `//!` is one or two lines and names
  the relevant file in `docs/`.
- Logging uses `tracing` with structured fields; no `println!` in library crates.

### 3.5 Tests

- Unit tests sit beside the code in `#[cfg(test)]`. Protocol tests are golden: captured or hand-built
  byte sequences stored under `crates/<crate>/tests/golden/` and compared byte for byte.
- A parser is round-trip tested with its encoder. Original-client captures that cannot be committed are
  referenced by path under `work/` and skipped when absent.

### 3.6 Commits

`crate: imperative summary` (`wire: parse login frame`), body explains why, one concern per commit,
every commit builds and passes tests.

## 4. Python tools (`tools/`)

- Python 3.10+, standard library plus `capstone`; run with `.venv/bin/python`.
- One tool per file with `main()` and an `if __name__ == "__main__"` guard. Parse arguments with
  `argparse`; return an exit code from `main()`; print errors to stderr.
- Four-space indent, lines up to 100, type hints on function signatures, `pathlib.Path` for paths.
- No semicolon-joined statements and no multi-statement one-liners. Binary layouts are named `struct`
  formats or `dataclass` fields, never unnamed `unpack_from` offsets scattered through a function.
- Return dataclasses or named tuples instead of positional tuples that callers must index.
- Share code through imports (`dcl.py`, `sci_res.py`); do not copy a routine into a second tool.
- Older tools predate this rule; bring a file in line when you next change it, never in a bulk pass.

## 5. Docs

- Live under `docs/`, one topic per file, named for the topic (`docs/tsn-protocol.md`).
- Every statement about original behaviour is labelled CONFIRMED or INFERRED, with `seg:off` in the
  unpacked image and the file offset as evidence.
- Code points to a doc by file name; a doc points to code by path and symbol, never by line number.

## 6. Checklist before handing work over

1. Did I look in the reuse map (2.1) and the existing code before writing a new helper?
2. Do names make the code readable without comments, and is every comment block two lines or fewer?
3. Is raw data converted to a domain type at the edge?
4. Does an unknown input fail safely instead of aborting?
5. `python3 tools/check_comments.py` is clean, the formatter and linter for the language are clean, and
   the commit message follows the area's format.
