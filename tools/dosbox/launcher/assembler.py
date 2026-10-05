"""The 8086 opcode table and two-pass assembler behind the DOS launcher.
Forms are spelled as capstone prints them; docs/dosbox.md section 6 describes the slots."""
import re
from dataclasses import dataclass

SLOT_SIZES = {"ib": 1, "iw": 2, "mw": 2, "rb": 1, "rw": 2}
FORM_SLOT = re.compile(r"\{(" + "|".join(SLOT_SIZES) + r")\}")
RELATIVE_SLOTS = ("rb", "rw")
# Short conditional jumps; flipping the low opcode bit gives the opposite condition.
CONDITIONS = {"je": 0x74, "jne": 0x75, "jb": 0x72, "ja": 0x77}
NEAR_JUMP = 0xE9
BRANCH_SIZE = 5

OPCODES: dict[str, str] = {
    "push ax": "50",
    "pop ax": "58",
    "push bp": "55",
    "pop bp": "5D",
    "push es": "06",
    "pop es": "07",
    "push cs": "0E",
    "push ds": "1E",
    "pop ds": "1F",
    "mov ax, {iw}": "B8 iw",
    "mov bx, {iw}": "BB iw",
    "mov dx, {iw}": "BA iw",
    "mov al, {ib}": "B0 ib",
    "mov ah, {ib}": "B4 ib",
    "mov ax, bx": "89 D8",
    "mov bp, sp": "8B EC",
    "mov es, ax": "8E C0",
    "mov dx, cs": "8C CA",
    "mov ax, word ptr [{mw}]": "A1 mw",
    "mov bx, word ptr [{mw}]": "8B 1E mw",
    "mov word ptr [{mw}], ax": "A3 mw",
    "mov word ptr [{mw}], dx": "89 16 mw",
    "mov ax, word ptr [bp + {ib}]": "8B 46 ib",
    "mov dx, word ptr [bp + {ib}]": "8B 56 ib",
    "mov ax, word ptr es:[{mw}]": "26 A1 mw",
    "mov dx, word ptr es:[{mw}]": "26 8B 16 mw",
    "mov word ptr es:[bx], ax": "26 89 07",
    "mov word ptr es:[bx + {ib}], dx": "26 89 57 ib",
    "les bx, ptr [{mw}]": "C4 1E mw",
    "xor dx, dx": "31 D2",
    "or ax, ax": "0B C0",
    "or ax, dx": "0B C2",
    "or dx, dx": "0B D2",
    "add ax, bx": "01 D8",
    "add sp, {ib}": "83 C4 ib",
    "sub ax, word ptr [{mw}]": "2B 06 mw",
    "cmp ax, {iw}": "3D iw",
    "cmp byte ptr es:[bx], {ib}": "26 80 3F ib",
    "cmp byte ptr es:[bx + {ib}], {ib}": "26 80 7F ib ib",
    "je {rb}": "74 rb",
    "jne {rb}": "75 rb",
    "jb {rb}": "72 rb",
    "ja {rb}": "77 rb",
    "jmp {rb}": "EB rb",
    "jmp {rw}": "E9 rw",
    "call {rw}": "E8 rw",
    "ret": "C3",
    "retf": "CB",
    "lcall es:[bx + {ib}]": "26 FF 5F ib",
    "int {ib}": "CD ib",
}

Operand = int | str


@dataclass(frozen=True)
class Op:
    form: str
    operands: tuple[Operand, ...]


@dataclass(frozen=True)
class Label:
    name: str


@dataclass(frozen=True)
class Data:
    content: bytes


@dataclass(frozen=True)
class Branch:
    """A conditional jump of any distance: the opposite short jump skips a near jmp."""

    condition: str
    target: str


Item = Op | Label | Data | Branch


def op(form: str, *operands: Operand) -> Op:
    """One instruction; a str operand names a label, whose address or distance fills the slot."""
    slots = FORM_SLOT.findall(form)
    if form not in OPCODES or len(slots) != len(operands):
        raise ValueError(f"no opcode for {form!r} with {len(operands)} operands")
    return Op(form, operands)


def branch(condition: str, target: str) -> Branch:
    if condition not in CONDITIONS:
        raise ValueError(f"no conditional jump {condition!r}")
    return Branch(condition, target)


def encoding_tokens(form: str) -> list[str]:
    return OPCODES[form].split()


def item_size(item: Item) -> int:
    if isinstance(item, Label):
        return 0
    if isinstance(item, Data):
        return len(item.content)
    if isinstance(item, Branch):
        return BRANCH_SIZE
    return sum(SLOT_SIZES.get(token, 1) for token in encoding_tokens(item.form))


def label_addresses(items: list[Item], origin: int) -> dict[str, int]:
    addresses = {}
    address = origin
    for item in items:
        if isinstance(item, Label):
            if item.name in addresses:
                raise ValueError(f"label {item.name!r} defined twice")
            addresses[item.name] = address
        address += item_size(item)
    return addresses


def slot_value(slot: str, operand: Operand, addresses: dict[str, int], next_address: int) -> int:
    if isinstance(operand, str) and operand not in addresses:
        raise ValueError(f"undefined label {operand!r}")
    value = addresses[operand] if isinstance(operand, str) else operand
    if slot in RELATIVE_SLOTS:
        value -= next_address
    bits = 8 * SLOT_SIZES[slot]
    if slot == "rb" and not -0x80 <= value < 0x80:
        raise ValueError(f"short jump to {operand!r} is {value} bytes away")
    if not -(1 << bits) < value < (1 << bits):
        raise ValueError(f"operand {operand!r} does not fit in {bits} bits")
    return value & ((1 << bits) - 1)


def encode_op(item: Op, addresses: dict[str, int], address: int) -> bytes:
    next_address = address + item_size(item)
    operands = iter(item.operands)
    encoded = bytearray()
    for token in encoding_tokens(item.form):
        if token in SLOT_SIZES:
            value = slot_value(token, next(operands), addresses, next_address)
            encoded += value.to_bytes(SLOT_SIZES[token], "little")
        else:
            encoded.append(int(token, 16))
    return bytes(encoded)


def encode_branch(item: Branch, addresses: dict[str, int], address: int) -> bytes:
    skip_near_jump = bytes([CONDITIONS[item.condition] ^ 1, BRANCH_SIZE - 2, NEAR_JUMP])
    distance = slot_value("rw", item.target, addresses, address + BRANCH_SIZE)
    return skip_near_jump + distance.to_bytes(2, "little")


def assemble(items: list[Item], origin: int) -> bytes:
    """Lay out every item from `origin`, then encode with all label addresses known."""
    addresses = label_addresses(items, origin)
    image = bytearray()
    for item in items:
        if isinstance(item, Op):
            image += encode_op(item, addresses, origin + len(image))
        elif isinstance(item, Branch):
            image += encode_branch(item, addresses, origin + len(image))
        elif isinstance(item, Data):
            image += item.content
    return bytes(image)
