"""Unit tests for the census scanners on hand-built code (run: python -m unittest tools.test_int14h_census)."""
import unittest

from tools import int14h_census as census

POINTER = 0x2328
LOAD_POINTER = bytes([0xC4, 0x1E]) + POINTER.to_bytes(2, "little")
DIRECT_CALL = bytes([0xFF, 0x5F, 0x24])
VIRTUAL_LOAD = bytes([0x26, 0x8B, 0x1F])
VIRTUAL_CALL = bytes([0xFF, 0x5F, 0x44])
RETURN_FAR = bytes([0xCB])


def calls_in(code: bytes) -> list[census.TableCall]:
    image = census.Image(0, code, frozenset())
    return census.find_table_calls(image, POINTER, 0)


class TableCallTests(unittest.TestCase):
    def test_direct_call_is_found(self) -> None:
        calls = calls_in(LOAD_POINTER + DIRECT_CALL + RETURN_FAR)
        self.assertEqual([call.export_offset for call in calls], [0x24])

    def test_last_slot_is_plus_44(self) -> None:
        calls = calls_in(LOAD_POINTER + VIRTUAL_CALL + RETURN_FAR)
        self.assertEqual([call.export_offset for call in calls], [0x44])

    def test_virtual_call_is_skipped(self) -> None:
        calls = calls_in(LOAD_POINTER + VIRTUAL_LOAD + DIRECT_CALL + RETURN_FAR)
        self.assertEqual(calls, [])


class FarCallTests(unittest.TestCase):
    def test_call_after_segment_ending_in_9a(self) -> None:
        first = bytes([0x9A, 0x10, 0x00, 0x9A, 0x01])
        second = bytes([0x9A, 0x20, 0x00, 0x30, 0x00])
        data = first + second
        image = census.Image(0, data, frozenset({3, 8}))
        self.assertEqual(census.far_call_targets(image), {0: (0x019A, 0x10), 5: (0x30, 0x20)})


if __name__ == "__main__":
    unittest.main()
