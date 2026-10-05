#!/usr/bin/env python3
"""Symbolic stack evaluation of one LSCI code item: what each send, call and store receives.
A linear walk that merges stack states at branch targets; opcode semantics: docs/lsci/interpreter.md."""
import re
from dataclasses import dataclass, field
from typing import Protocol, Union

from lsci_bytecode import Instruction, decode_code
from lsci_format import Item

SMALL_NUMBER = 255
VARIABLE_ACCESS = re.compile(r"([ls+-])([as])([gltp])(i?)")
VARIABLE_KIND_NAMES = {"g": "global", "l": "local", "t": "temp", "p": "param"}
BINARY_SYMBOLS = {
    "add": "+", "sub": "-", "mul": "*", "div": "/", "mod": "%", "shr": ">>", "shl": "<<",
    "xor": "^", "and": "&", "or": "|", "eq?": "==", "ne?": "!=", "gt?": ">", "ge?": ">=",
    "lt?": "<", "le?": "<=", "ugt?": ">", "uge?": ">=", "ult?": "<", "ule?": "<=",
}
UNARY_SYMBOLS = {"bnot": "~", "neg": "-", "not": "!"}
COMPARISONS = frozenset({"eq?", "ne?", "gt?", "ge?", "lt?", "le?", "ugt?", "uge?", "ult?", "ule?"})


@dataclass(frozen=True)
class Const:
    value: int

    def __str__(self) -> str:
        if -SMALL_NUMBER <= self.value <= SMALL_NUMBER:
            return str(self.value)
        return f"0x{self.value & 0xFFFF:x}"


@dataclass(frozen=True)
class Text:
    text: str

    def __str__(self) -> str:
        return '"' + self.text.encode("unicode_escape").decode("ascii").replace('"', '\\"') + '"'


@dataclass(frozen=True)
class Name:
    """Anything known only by name: a variable, property, object, class or code item."""

    name: str

    def __str__(self) -> str:
        return self.name


@dataclass(frozen=True)
class Message:
    selector: str
    args: tuple["Value", ...]

    def __str__(self) -> str:
        return f"{self.selector}:" + "".join(f" {arg}" for arg in self.args)


@dataclass(frozen=True)
class SendResult:
    receiver: "Value"
    message: Message

    def __str__(self) -> str:
        return f"({self.receiver} {self.message})"


@dataclass(frozen=True)
class CallResult:
    target: str
    args: tuple["Value", ...]

    def __str__(self) -> str:
        return f"{self.target}(" + ", ".join(str(arg) for arg in self.args) + ")"


@dataclass(frozen=True)
class Operation:
    symbol: str
    operands: tuple["Value", ...]

    def __str__(self) -> str:
        if len(self.operands) == 1:
            return f"{self.symbol}{self.operands[0]}"
        return f"({self.operands[0]} {self.symbol} {self.operands[1]})"


@dataclass(frozen=True)
class Either:
    """The value differs by the path that reached this point."""

    choices: tuple["Value", ...]

    def __str__(self) -> str:
        return "{" + " | ".join(str(choice) for choice in self.choices) + "}"


Value = Union[Const, Text, Name, SendResult, CallResult, Operation, Either]
UNKNOWN = Name("?")


class Context(Protocol):
    """What the evaluator needs to name things in one module."""

    def selector_name(self, selector: int) -> str: ...
    def class_name(self, species: int) -> str: ...
    def kernel_name(self, number: int) -> str: ...
    def property_name(self, offset: int) -> str: ...
    def item_value(self, index: int) -> Value: ...


@dataclass(frozen=True)
class KernelCall:
    offset: int
    name: str
    args: tuple[Value, ...]
    passes_rest: bool


@dataclass(frozen=True)
class Call:
    offset: int
    target: str
    args: tuple[Value, ...]


@dataclass(frozen=True)
class Send:
    offset: int
    receiver: Value
    messages: tuple[Message, ...]


@dataclass(frozen=True)
class Store:
    offset: int
    target: str
    value: Value


@dataclass(frozen=True)
class Comparison:
    offset: int
    symbol: str
    left: Value
    right: Value


Event = Union[KernelCall, Call, Send, Store, Comparison]


@dataclass
class Machine:
    stack: list[Value] = field(default_factory=list)
    acc: Value = UNKNOWN
    rest_pending: bool = False

    def pop(self) -> Value:
        return self.stack.pop() if self.stack else UNKNOWN

    def pop_frame(self, words: int) -> tuple[Value, ...]:
        frame = [self.pop() for _ in range(words)]
        return tuple(reversed(frame))

    def copy(self) -> "Machine":
        return Machine(list(self.stack), self.acc, self.rest_pending)


def merge(first: Machine, second: Machine) -> Machine:
    """Join two paths: keep the deeper stack's shape, mark values that differ as Either."""
    if len(first.stack) != len(second.stack):
        return first.copy() if len(first.stack) >= len(second.stack) else second.copy()
    stack = [a if a == b else Either((a, b)) for a, b in zip(first.stack, second.stack)]
    acc = first.acc if first.acc == second.acc else Either((first.acc, second.acc))
    return Machine(stack, acc, first.rest_pending or second.rest_pending)


def split_messages(frame: tuple[Value, ...], context: Context) -> tuple[Message, ...]:
    """A send frame is repeated {selector, argc, args[argc]}; anything else is one opaque message."""
    messages, position = [], 0
    while position + 1 < len(frame):
        selector, argc = frame[position], frame[position + 1]
        if not (isinstance(selector, Const) and isinstance(argc, Const)):
            break
        end = position + 2 + argc.value
        if end > len(frame):
            break
        name = context.selector_name(selector.value)
        messages.append(Message(name, frame[position + 2 : end]))
        position = end
    if position != len(frame):
        return (Message("?", frame),)
    return tuple(messages)


class Evaluator:
    """Walks one code item once and records events; see the module docstring for the model."""

    def __init__(self, item: Item, context: Context):
        self.item = item
        self.context = context
        self.events: list[Event] = []
        self.pending: dict[int, Machine] = {}

    def run(self) -> list[Event]:
        instructions, _error = decode_code(self.item.payload)
        machine: Machine | None = Machine()
        for ins in instructions:
            machine = self.join(ins.offset, machine)
            if machine is None:
                continue
            machine = self.step(ins, machine)
        return self.events

    def join(self, offset: int, machine: Machine | None) -> Machine | None:
        waiting = self.pending.pop(offset, None)
        if waiting is None:
            return machine
        return waiting if machine is None else merge(machine, waiting)

    def remember(self, target: int, machine: Machine) -> None:
        if target in self.pending:
            self.pending[target] = merge(self.pending[target], machine)
        else:
            self.pending[target] = machine.copy()

    def operand_value(self, ins: Instruction) -> Value:
        if self.item.is_reference(ins.operand_offsets[0]):
            return self.context.item_value(ins.operands[0])
        return Const(ins.operands[0])

    def step(self, ins: Instruction, machine: Machine) -> Machine | None:
        name = ins.row.name
        target = ins.branch_target()
        if target is not None:
            if target > ins.offset:
                self.remember(target, machine)
            return None if name == "jmp" else machine
        if name == "ret":
            return None
        handler = STEP_HANDLERS.get(name)
        if handler:
            handler(self, ins, machine)
        else:
            self.variable_access(ins, machine)
        return machine

    def binary(self, ins: Instruction, machine: Machine) -> None:
        name = ins.row.name
        left = machine.pop()
        if name in COMPARISONS:
            self.events.append(Comparison(ins.offset, BINARY_SYMBOLS[name], left, machine.acc))
        machine.acc = Operation(BINARY_SYMBOLS[name], (left, machine.acc))

    def unary(self, ins: Instruction, machine: Machine) -> None:
        machine.acc = Operation(UNARY_SYMBOLS[ins.row.name], (machine.acc,))

    def send(self, ins: Instruction, machine: Machine, receiver: Value) -> None:
        machine.rest_pending = False
        frame = machine.pop_frame(ins.operands[-1] // 2)
        messages = split_messages(frame, self.context)
        self.events.append(Send(ins.offset, receiver, messages))
        machine.acc = SendResult(receiver, messages[-1]) if messages else UNKNOWN

    def call(self, ins: Instruction, machine: Machine, target: str) -> tuple[Value, ...]:
        machine.rest_pending = False
        frame = machine.pop_frame(ins.operands[-1] // 2 + 1)
        args = frame[1:]
        machine.acc = CallResult(target, args)
        return args

    def kernel(self, ins: Instruction, machine: Machine) -> None:
        name = self.context.kernel_name(ins.operands[0])
        passes_rest = machine.rest_pending
        args = self.call(ins, machine, name)
        self.events.append(KernelCall(ins.offset, name, args, passes_rest))

    def procedure(self, ins: Instruction, machine: Machine, target: str) -> None:
        args = self.call(ins, machine, target)
        self.events.append(Call(ins.offset, target, args))

    def store(self, ins: Instruction, target: str, value: Value) -> None:
        self.events.append(Store(ins.offset, target, value))

    def variable_access(self, ins: Instruction, machine: Machine) -> None:
        match = VARIABLE_ACCESS.fullmatch(ins.row.name)
        if not match:
            return
        operation, form, kind, indexed = match.groups()
        name = f"{VARIABLE_KIND_NAMES[kind]}{ins.operands[0]}"
        variable: Value = Name(name + ("[acc]" if indexed else ""))
        if operation == "s":
            value = machine.pop() if form == "s" or indexed else machine.acc
            if indexed and form == "a":
                machine.acc = value
            self.store(ins, variable.name, value)
            return
        if operation in "+-":
            self.store(ins, variable.name, Operation(operation, (variable, Const(1))))
        if form == "a":
            machine.acc = variable
        else:
            machine.stack.append(variable)


def property_access(evaluator: Evaluator, ins: Instruction, machine: Machine) -> None:
    name = evaluator.context.property_name(ins.operands[0])
    kind = ins.row.name
    if kind in ("pToa", "ipToa", "dpToa"):
        machine.acc = Name(name)
    elif kind in ("pTos", "ipTos", "dpTos"):
        machine.stack.append(Name(name))
    elif kind == "aTop":
        evaluator.store(ins, name, machine.acc)
    elif kind == "sTop":
        evaluator.store(ins, name, machine.pop())


def push_value(value: Value):
    def handler(evaluator: Evaluator, ins: Instruction, machine: Machine) -> None:
        machine.stack.append(value)
    return handler


def set_acc(evaluator: Evaluator, ins: Instruction, machine: Machine) -> None:
    machine.acc = evaluator.operand_value(ins)


def push_operand(evaluator: Evaluator, ins: Instruction, machine: Machine) -> None:
    machine.stack.append(evaluator.operand_value(ins))


def push_acc(evaluator: Evaluator, ins: Instruction, machine: Machine) -> None:
    machine.stack.append(machine.acc)


def duplicate(evaluator: Evaluator, ins: Instruction, machine: Machine) -> None:
    machine.stack.append(machine.stack[-1] if machine.stack else UNKNOWN)


def toss(evaluator: Evaluator, ins: Instruction, machine: Machine) -> None:
    machine.pop()


def load_class(evaluator: Evaluator, ins: Instruction, machine: Machine) -> None:
    machine.acc = Name(evaluator.context.class_name(ins.operands[0]))


def local_call(evaluator: Evaluator, ins: Instruction, machine: Machine) -> None:
    evaluator.procedure(ins, machine, str(evaluator.operand_value(ins)))


def export_call(evaluator: Evaluator, ins: Instruction, machine: Machine) -> None:
    script = 0 if ins.row.name == "callb" else ins.operands[0]
    evaluator.procedure(ins, machine, f"script_{script}:export_{ins.operands[-2]}")


def super_send(evaluator: Evaluator, ins: Instruction, machine: Machine) -> None:
    evaluator.send(ins, machine, Name("super"))


def ignore(evaluator: Evaluator, ins: Instruction, machine: Machine) -> None:
    return


def pass_rest(evaluator: Evaluator, ins: Instruction, machine: Machine) -> None:
    machine.rest_pending = True


STEP_HANDLERS = {
    **{name: Evaluator.binary for name in BINARY_SYMBOLS},
    **{name: Evaluator.unary for name in UNARY_SYMBOLS},
    **{name: property_access for name in
       ("pToa", "aTop", "pTos", "sTop", "ipToa", "dpToa", "ipTos", "dpTos")},
    "ldi": set_acc, "loadID": set_acc,
    "pushi": push_operand, "pushID": push_operand,
    "push0": push_value(Const(0)), "push1": push_value(Const(1)), "push2": push_value(Const(2)),
    "pushSelf": push_value(Name("self")),
    "selfID": lambda evaluator, ins, machine: setattr(machine, "acc", Name("self")),
    "pprev": lambda evaluator, ins, machine: setattr(machine, "acc", Name("prev")),
    "push": push_acc, "dup": duplicate, "toss": toss,
    "class": load_class,
    "call": local_call, "callk": Evaluator.kernel, "callb": export_call, "calle": export_call,
    "send": lambda evaluator, ins, machine: evaluator.send(ins, machine, machine.acc),
    "self": lambda evaluator, ins, machine: evaluator.send(ins, machine, Name("self")),
    "super": super_send,
    "link": ignore, "&rest": pass_rest, "_line_": ignore, "_file_": ignore,
}


def evaluate(item: Item, context: Context) -> list[Event]:
    return Evaluator(item, context).run()
