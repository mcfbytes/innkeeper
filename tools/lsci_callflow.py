#!/usr/bin/env python3
"""Kernel call sites of LSCI code with argument provenance and result use.
Method and limits (forward branches merged, backward ones ignored): docs/lsci/kernel-usage.md."""
import enum
from collections import defaultdict
from dataclasses import dataclass
from typing import Callable, Iterator

from lsci_bytecode import Instruction, Operand, decode_code
from lsci_format import ITEM_HEADER, Item, ItemTag, Module

CALL_FRAME_OPCODES = frozenset({"call", "callk", "callb", "calle"})
SEND_OPCODES = frozenset({"send", "self", "super"})
UNARY_OPCODES = frozenset({"bnot", "neg", "not"})
COMPARISONS = frozenset(
    {"eq?", "ne?", "gt?", "ge?", "lt?", "le?", "ugt?", "uge?", "ult?", "ule?"})
BINARY_OPCODES = frozenset({"add", "sub", "mul", "div", "mod", "shr", "shl", "xor", "and", "or"})
PROPERTY_LOADS = {"pToa": "acc", "ipToa": "acc", "dpToa": "acc", "pTos": "stack", "ipTos": "stack",
                  "dpTos": "stack"}
VARIABLE_OPCODE_BASE = 0x40

OPCODE_HANDLERS = {
    "ldi": "_op_ldi",
    "loadID": "_op_ldi",
    "pushi": "_op_pushi",
    "pushID": "_op_pushi",
    "push": "_op_push",
    "push0": "_op_push0",
    "push1": "_op_push1",
    "push2": "_op_push2",
    "pushSelf": "_op_push_self",
    "selfID": "_op_self_id",
    "class": "_op_class",
    "toss": "_op_toss",
    "dup": "_op_dup",
    "pprev": "_op_pprev",
    "&rest": "_op_rest",
    "bt": "_op_branch_if",
    "bnt": "_op_branch_if",
    "jmp": "_op_jump",
    "ret": "_op_return",
    "aTop": "_op_store_property",
    "sTop": "_op_store_property_from_stack",
}


class Source(enum.Enum):
    """Where the value of an argument or accumulator came from."""

    IMMEDIATE = "immediate"
    SELECTOR = "selector"
    STRING = "string"
    OBJECT = "object"
    CLASS = "class"
    CODE = "code"
    GLOBAL = "global"
    LOCAL = "local"
    TEMP = "temp"
    PARAM = "param"
    PROPERTY = "property"
    SELF = "self"
    KERNEL_RESULT = "kernel result"
    CALL_RESULT = "call result"
    SEND_RESULT = "send result"
    COMPUTED = "computed"
    REST = "rest"
    UNKNOWN = "unknown"


class UseKind(enum.Enum):
    """What the code does with a kernel's result after the call."""

    DISCARDED = "discarded"
    TESTED = "tested"
    COMPARED = "compared"
    STORED = "stored"
    PASSED = "passed"
    COMPUTED = "computed"
    RETURNED = "returned"
    RECEIVER = "receiver"


VARIABLE_SOURCES = {Operand.GLOBAL: Source.GLOBAL, Operand.LOCAL: Source.LOCAL,
                    Operand.TEMP: Source.TEMP, Operand.PARAM: Source.PARAM}


@dataclass(frozen=True)
class Origin:
    source: Source
    detail: int | str | None = None

    def describe(self) -> str:
        if self.source is Source.IMMEDIATE:
            return f"{self.detail:#x}"
        if self.source is Source.CLASS:
            return f"<class {self.detail:#x}>"
        if self.source in (Source.STRING, Source.OBJECT, Source.CODE):
            return str(self.detail)
        if self.detail is None:
            return f"<{self.source.value}>"
        return f"<{self.source.value} {self.detail}>"


@dataclass(frozen=True)
class Use:
    kind: UseKind
    detail: str | None = None


class Value:
    """One accumulator or stack word; producers are the kernel call sites whose result it holds."""

    __slots__ = ("origin", "producers", "open_producers", "stacked")

    def __init__(self, origin: Origin, producers: tuple[int, ...] = ()):
        self.origin, self.producers, self.stacked = origin, producers, False
        self.open_producers = frozenset(producers)


UNKNOWN_ORIGIN = Origin(Source.UNKNOWN)
COMPUTED_ORIGIN = Origin(Source.COMPUTED)


def merged(first: Value, second: Value) -> Value:
    if first is second:
        return first
    origin = first.origin if first.origin == second.origin else COMPUTED_ORIGIN
    value = Value(origin, tuple(dict.fromkeys(first.producers + second.producers)))
    value.open_producers = first.open_producers | second.open_producers
    value.stacked = first.stacked or second.stacked
    return value


@dataclass
class State:
    acc: Value
    stack: list[Value]

    def copy(self) -> "State":
        return State(self.acc, list(self.stack))


def merged_states(first: State, second: State) -> State:
    """Join two paths; if their stack depths disagree the first path's stack is kept."""
    if len(first.stack) != len(second.stack):
        return State(merged(first.acc, second.acc), first.stack)
    stack = [merged(a, b) for a, b in zip(first.stack, second.stack)]
    return State(merged(first.acc, second.acc), stack)


@dataclass(frozen=True)
class KernelCall:
    item: int
    offset: int
    file_offset: int
    kernel: int
    argc: int
    arguments: tuple[Origin, ...]
    block_verified: bool
    uses: tuple[Use, ...]

    @property
    def subop(self) -> int | None:
        first = self.arguments[0] if self.arguments else None
        return first.detail if first and first.source is Source.IMMEDIATE else None


@dataclass(frozen=True)
class SendSite:
    offset: int
    selector: int | None
    receiver: Origin


@dataclass
class _SiteBuilder:
    item: int
    offset: int
    file_offset: int
    kernel: int
    argc: int
    arguments: tuple[Origin, ...]
    block_verified: bool
    uses: list[Use]


class CallFlow:
    """Forward simulation of one Code item over a symbolic accumulator and stack."""

    def __init__(self, module: Module, item: Item, kernel_names: tuple[str, ...],
                 describe_item: Callable[[int], str]):
        self.module, self.item, self.kernel_names = module, item, kernel_names
        self.describe_item = describe_item
        self.sites: list[_SiteBuilder] = []
        self.sends: list[SendSite] = []
        self.uses: dict[int, list[Use]] = defaultdict(list)
        self.pending: dict[int, State] = {}
        self.state = self._fresh_state()

    def run(self) -> list[KernelCall]:
        instructions, _error = decode_code(self.item.payload)
        for instruction in instructions:
            self._enter(instruction.offset)
            self._execute(instruction)
        return [self._finish(site) for site in self.sites]

    def _finish(self, site: _SiteBuilder) -> KernelCall:
        return KernelCall(site.item, site.offset, site.file_offset, site.kernel, site.argc,
                          site.arguments, site.block_verified, tuple(site.uses))

    @staticmethod
    def _fresh_state() -> State:
        return State(Value(UNKNOWN_ORIGIN), [])

    def _enter(self, offset: int) -> None:
        """Join the fall-through state with any forward branch that targets this offset."""
        incoming = self.pending.pop(offset, None)
        if incoming is not None:
            self.state = incoming if self.state is None else merged_states(self.state, incoming)
        elif self.state is None:
            self.state = self._fresh_state()

    def _execute(self, ins: Instruction) -> None:
        name = ins.row.name
        if name in OPCODE_HANDLERS:
            getattr(self, OPCODE_HANDLERS[name])(ins)
        elif name in COMPARISONS:
            self._comparison(name)
        elif name in BINARY_OPCODES:
            self._binary()
        elif name in UNARY_OPCODES:
            self._consume(self.state.acc, UseKind.COMPUTED)
            self._set_acc(Value(COMPUTED_ORIGIN))
        elif name in PROPERTY_LOADS:
            self._property_load(ins)
        elif name in CALL_FRAME_OPCODES or name in SEND_OPCODES:
            self._call(ins)
        elif ins.opcode >= VARIABLE_OPCODE_BASE:
            self._variable_access(ins)

    # Value bookkeeping ------------------------------------------------------------------------

    def _consume(self, value: Value, kind: UseKind, detail: str | None = None) -> None:
        for producer in value.producers:
            self.uses[producer].append(Use(kind, detail))
        value.open_producers = frozenset()

    def _discard(self, value: Value) -> None:
        for producer in sorted(value.open_producers):
            self.uses[producer].append(Use(UseKind.DISCARDED))
        value.open_producers = frozenset()

    def _set_acc(self, value: Value) -> None:
        old = self.state.acc
        if not old.stacked:
            self._discard(old)
        self.state.acc = value

    def _push(self, value: Value) -> None:
        value.stacked = True
        self.state.stack.append(value)

    def _pop(self) -> Value:
        return self.state.stack.pop() if self.state.stack else Value(UNKNOWN_ORIGIN)

    def _reference_origin(self, ins: Instruction) -> Origin:
        if not self.item.is_reference(ins.operand_offsets[0]):
            return Origin(Source.IMMEDIATE, ins.operands[0] & 0xFFFF)
        index = ins.operands[0]
        target = self.module.items[index] if index < len(self.module.items) else None
        label = self.describe_item(index)
        if target is not None and target.tag == ItemTag.STRING:
            return Origin(Source.STRING, label)
        is_code = target is not None and target.tag == ItemTag.CODE
        return Origin(Source.CODE if is_code else Source.OBJECT, label)

    # Stack, load and branch opcodes -------------------------------------------------------------

    def _op_ldi(self, ins: Instruction) -> None:
        self._set_acc(Value(self._reference_origin(ins)))

    def _push_origin(self, origin: Origin) -> None:
        self._push(Value(origin))

    def _op_pushi(self, ins: Instruction) -> None:
        self._push_origin(self._reference_origin(ins))

    def _op_push(self, _ins: Instruction) -> None:
        self._push(self.state.acc)

    def _op_push0(self, _ins: Instruction) -> None:
        self._push_origin(Origin(Source.IMMEDIATE, 0))

    def _op_push1(self, _ins: Instruction) -> None:
        self._push_origin(Origin(Source.IMMEDIATE, 1))

    def _op_push2(self, _ins: Instruction) -> None:
        self._push_origin(Origin(Source.IMMEDIATE, 2))

    def _op_push_self(self, _ins: Instruction) -> None:
        self._push_origin(Origin(Source.SELF))

    def _op_self_id(self, _ins: Instruction) -> None:
        self._set_acc(Value(Origin(Source.SELF)))

    def _op_class(self, ins: Instruction) -> None:
        self._set_acc(Value(Origin(Source.CLASS, ins.operands[0])))

    def _op_toss(self, _ins: Instruction) -> None:
        self._discard(self._pop())

    def _op_dup(self, _ins: Instruction) -> None:
        top = self.state.stack[-1] if self.state.stack else Value(UNKNOWN_ORIGIN)
        self.state.stack.append(top)

    def _op_pprev(self, _ins: Instruction) -> None:
        self._push_origin(COMPUTED_ORIGIN)

    def _op_rest(self, _ins: Instruction) -> None:
        self._push_origin(Origin(Source.REST))

    def _op_branch_if(self, ins: Instruction) -> None:
        self._consume(self.state.acc, UseKind.TESTED)
        self._branch(ins)

    def _op_jump(self, ins: Instruction) -> None:
        self._branch(ins)
        self.state = None

    def _op_return(self, _ins: Instruction) -> None:
        self._consume(self.state.acc, UseKind.RETURNED)
        self.state = None

    def _branch(self, ins: Instruction) -> None:
        target = ins.branch_target()
        if target is None or target <= ins.offset:
            return
        known = self.pending.get(target)
        snapshot = self.state.copy()
        self.pending[target] = snapshot if known is None else merged_states(known, snapshot)

    # Arithmetic and variables ----------------------------------------------------------------

    def _binary(self) -> None:
        self._consume(self._pop(), UseKind.COMPUTED)
        self._consume(self.state.acc, UseKind.COMPUTED)
        self._set_acc(Value(COMPUTED_ORIGIN))

    def _comparison(self, name: str) -> None:
        left, right = self._pop(), self.state.acc
        self._consume(left, UseKind.COMPARED, f"{name} {right.origin.describe()}")
        self._consume(right, UseKind.COMPARED, f"{name} {left.origin.describe()}")
        self._set_acc(Value(COMPUTED_ORIGIN))

    def _property_load(self, ins: Instruction) -> None:
        value = Value(Origin(Source.PROPERTY, ins.operands[0]))
        if PROPERTY_LOADS[ins.row.name] == "acc":
            self._set_acc(value)
        else:
            self._push(value)

    def _op_store_property(self, ins: Instruction) -> None:
        self._consume(self.state.acc, UseKind.STORED, f"property {ins.operands[0]}")

    def _op_store_property_from_stack(self, ins: Instruction) -> None:
        self._consume(self._pop(), UseKind.STORED, f"property {ins.operands[0]}")

    def _variable_access(self, ins: Instruction) -> None:
        """Names are operation (l s + -), target (a = accumulator, s = stack), kind, optional i."""
        name, kind = ins.row.name, ins.row.operands[0]
        operation, to_stack, indexed = name[0], name[1] == "s", name.endswith("i")
        origin = Origin(VARIABLE_SOURCES[kind], ins.operands[0])
        if indexed:
            self._consume(self.state.acc, UseKind.COMPUTED)
            origin = Origin(origin.source, f"{origin.detail}+index")
        if operation == "s":
            self._store(ins, origin, to_stack)
        elif to_stack:
            self._push(Value(origin))
        else:
            self._set_acc(Value(origin))

    def _store(self, ins: Instruction, origin: Origin, from_stack: bool) -> None:
        stored = f"{origin.source.value} {origin.detail}"
        if from_stack or ins.row.name.endswith("i"):
            value = self._pop()
            self._consume(value, UseKind.STORED, stored)
            if not from_stack:
                self.state.acc = value
            return
        self._consume(self.state.acc, UseKind.STORED, stored)

    # Calls ------------------------------------------------------------------------------------

    def _call(self, ins: Instruction) -> None:
        name = ins.row.name
        frame_words = ins.operands[-1] // 2
        is_send = name in SEND_OPCODES
        block = self._pop_frame(frame_words if is_send else frame_words + 1)
        callee = self._kernel_name(ins.operands[0]) if name == "callk" else name
        if name == "send":
            self._consume(self.state.acc, UseKind.RECEIVER)
        if is_send:
            self._record_send(ins, block)
        for position, value in enumerate(block):
            self._consume(value, UseKind.PASSED, self._argument_label(name, callee, position))
        if name == "callk":
            self._kernel_call(ins, block)
        else:
            source = Source.SEND_RESULT if is_send else Source.CALL_RESULT
            self._set_acc(Value(Origin(source, callee)))

    def _record_send(self, ins: Instruction, block: list[Value]) -> None:
        first = block[0].origin if block else UNKNOWN_ORIGIN
        selector = first.detail if first.source is Source.IMMEDIATE else None
        self.sends.append(SendSite(ins.offset, selector, self._receiver_origin(ins)))

    def _receiver_origin(self, ins: Instruction) -> Origin:
        if ins.row.name == "send":
            return self.state.acc.origin
        if ins.row.name == "super":
            return Origin(Source.CLASS, ins.operands[0])
        return Origin(Source.SELF)

    @staticmethod
    def _argument_label(name: str, callee: str, position: int) -> str:
        """Kernel arguments are numbered from 0 after the argc word."""
        return f"{callee}[{position - 1}]" if name == "callk" and position else callee

    def _pop_frame(self, words: int) -> list[Value]:
        """Pop the argument block; a trailing &rest marker stands for an unknown number of words."""
        block = []
        if self.state.stack and self.state.stack[-1].origin.source is Source.REST:
            block.append(self.state.stack.pop())
        block.extend(self._pop() for _ in range(words))
        block.reverse()
        return block

    def _file_offset(self, ins: Instruction) -> int:
        return self.item.file_offset + ITEM_HEADER.size + ins.offset

    def _kernel_name(self, number: int) -> str:
        known = number < len(self.kernel_names)
        return self.kernel_names[number] if known else f"kernel_{number:x}"

    def _kernel_call(self, ins: Instruction, block: list[Value]) -> None:
        kernel, frame = ins.operands
        origins = [value.origin for value in block]
        verified = (origins[0] == Origin(Source.IMMEDIATE, frame // 2)
                    and all(origin.source is not Source.REST for origin in origins))
        site_id = len(self.sites)
        self.sites.append(_SiteBuilder(
            self.item.index, ins.offset, self._file_offset(ins), kernel, frame // 2,
            tuple(origins[1:]), verified, self.uses[site_id]))
        result = Origin(Source.KERNEL_RESULT, self._kernel_name(kernel))
        self._set_acc(Value(result, (site_id,)))


def item_sends(module: Module, item: Item) -> list[SendSite]:
    """Every send, self and super of a Code item with its receiver and constant selector."""
    flow = CallFlow(module, item, (), str)
    flow.run()
    return flow.sends


def find_kernel_calls(module: Module, kernel_names: tuple[str, ...],
                      describe_item: Callable[[int], str]) -> Iterator[KernelCall]:
    """Every callk of every Code item, in item order."""
    for item in module.items_of(ItemTag.CODE):
        yield from CallFlow(module, item, kernel_names, describe_item).run()
