"""Checks the opcode table against capstone and the launcher's bytes against the documented layouts.
Run: .venv/bin/python -m unittest discover -s tools/dosbox"""
import unittest

import capstone

from install_client import ClientChoices, dial_string, encode_password
from launcher.assembler import (
    FORM_SLOT, OPCODES, Label, assemble, branch, label_addresses, op)
from launcher.games import GAME_LAUNCHES
from launcher.messages import Login, encode_login, encode_shared_block
from launcher.program import COM_ORIGIN, Export, LaunchPlan, build_launcher, launcher_items

SAMPLE_VALUES = {"ib": (0x12, 0x34), "iw": (0x1234, 0x5678), "mw": (0x1234, 0x5678),
                 "rb": (0x130, 0x130), "rw": (0x1337, 0x1337)}
# The Login the stock client sent (docs/protocol/captures.md section 4).
CAPTURED_LOGIN = bytes.fromhex(
    "35000000010203 12 a1860100 01 1d7a0166166618730300 00 6775796272757368 00".replace(" ", ""))
# Export calls in address order: the main path, then the shared poll routine.
EXPORTS_IN_CODE_ORDER = [
    Export.SET_CALLBACKS, Export.SET_ACK_TIMEOUT, Export.GET_PREVIOUS_PROGRAM, Export.CONNECT,
    Export.SEND, Export.FLUSH, Export.RECEIVE, Export.SET_SHARED_DATA, Export.SET_NEXT_PROGRAM,
    Export.FLUSH, Export.POLL,
]
DISASSEMBLER = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_16)


def disassemble(image: bytes) -> list[str]:
    return [f"{insn.mnemonic} {insn.op_str}".strip()
            for insn in DISASSEMBLER.disasm(image, COM_ORIGIN)]


def capstone_number(value: int) -> str:
    return str(value) if value < 10 else hex(value)


def fill_form(form: str, operands: list[int]) -> str:
    values = iter(operands)
    return FORM_SLOT.sub(lambda _: capstone_number(next(values)), form)


def sample_operands(form: str) -> list[int]:
    seen: dict[str, int] = {}
    operands = []
    for slot in FORM_SLOT.findall(form):
        operands.append(SAMPLE_VALUES[slot][seen.get(slot, 0)])
        seen[slot] = seen.get(slot, 0) + 1
    return operands


def golf_plan() -> LaunchPlan:
    choices = ClientChoices()
    game = GAME_LAUNCHES["GOLF"]
    login = Login(choices.member_id, encode_password(choices.password), "guybrush")
    return LaunchPlan(dial_string(choices), encode_login(login),
                      encode_shared_block(game.shared_block("guybrush")), game.program)


class OpcodeTableTest(unittest.TestCase):
    def test_every_row_disassembles_to_its_form(self):
        for form in OPCODES:
            with self.subTest(form=form):
                operands = sample_operands(form)
                image = assemble([op(form, *operands)], COM_ORIGIN)
                self.assertEqual(disassemble(image), [fill_form(form, operands)])

    def test_branch_skips_a_near_jump_with_the_opposite_condition(self):
        image = assemble([branch("jne", "far"), Label("far")], COM_ORIGIN)
        after = COM_ORIGIN + 5
        self.assertEqual(disassemble(image), [f"je {after:#x}", f"jmp {after:#x}"])

    def test_short_jump_out_of_range_is_an_error(self):
        with self.assertRaises(ValueError):
            assemble([op("jmp {rb}", COM_ORIGIN + 0x200)], COM_ORIGIN)


class MessageLayoutTest(unittest.TestCase):
    def test_login_matches_the_stock_client(self):
        login = Login(100001, encode_password("SWORDFISH"), "guybrush")
        self.assertEqual(encode_login(login), CAPTURED_LOGIN)

    def test_golf_block_carries_sids_name_and_one_player(self):
        block = encode_shared_block(GAME_LAUNCHES["GOLF"].shared_block("guybrush"))
        self.assertEqual(len(block), 0x80 + 14)
        self.assertEqual(block[0:2], b"\x00\x01")
        self.assertEqual(block[4:6], b"\x00\x02")
        self.assertEqual(block[0x10:0x1B], b"guybrush\0\0\0")
        self.assertEqual(block[0x84], 1)


class LauncherTest(unittest.TestCase):
    def test_code_disassembles_without_gaps(self):
        plan = golf_plan()
        data_start = label_addresses(launcher_items(plan), COM_ORIGIN)["table_offset"]
        code = build_launcher(plan)[: data_start - COM_ORIGIN]
        sizes = sum(insn.size for insn in DISASSEMBLER.disasm(code, COM_ORIGIN))
        self.assertEqual(sizes, len(code))

    def test_exports_are_called_in_order(self):
        listing = disassemble(build_launcher(golf_plan()))
        calls = [line for line in listing if line.startswith("lcall")]
        expected = [fill_form("lcall es:[bx + {ib}]", [export * 4])
                    for export in EXPORTS_IN_CODE_ORDER]
        self.assertEqual(calls[: len(expected)], expected)

    def test_image_carries_dial_login_block_and_program(self):
        plan = golf_plan()
        image = build_launcher(plan)
        for part in (plan.dial.encode() + b"\0", plan.login, plan.shared_block, b"Golf\0"):
            self.assertIn(part, image)


if __name__ == "__main__":
    unittest.main()
