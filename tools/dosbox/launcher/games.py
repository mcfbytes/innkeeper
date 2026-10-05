"""DOS games the launcher can start: where they live and the hand-made shared block each one reads.
Block fields per game are in the INT 14h census (docs/protocol/int14h-census/)."""
from collections.abc import Callable
from dataclasses import dataclass

from .messages import SharedBlock

# Hand-made SIDs: innkeeperd's first allocated SID for the user, an unknown one for the group.
HAND_MADE_USER_SID = 0x0100
HAND_MADE_GROUP_SID = 0x0200
GOLF_PLAYER_COUNT_INDEX = 4
GOLF_PARAMETER_LENGTH = 14


@dataclass(frozen=True)
class GameLaunch:
    directory: str
    program: str
    shared_block: Callable[[str], SharedBlock]


def golf_block(user_name: str) -> SharedBlock:
    """One player, stroke play over 18 holes (golf.md section 5)."""
    parameters = bytearray(GOLF_PARAMETER_LENGTH)
    parameters[GOLF_PLAYER_COUNT_INDEX] = 1
    return SharedBlock(HAND_MADE_USER_SID, HAND_MADE_GROUP_SID, user_name, bytes(parameters))


GAME_LAUNCHES: dict[str, GameLaunch] = {
    "GOLF": GameLaunch(directory="GOLF", program="Golf", shared_block=golf_block),
}
