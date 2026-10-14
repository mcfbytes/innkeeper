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
# LAND.CFG land types; place 1 is the one innkeeperd lists for each (yserbius.md section 3).
YSERBIUS_LAND_TYPE = 4
TWINION_LAND_TYPE = 8
RPG_PLACE = 1


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


def rpg_block(land_type: int) -> Callable[[str], SharedBlock]:
    """The words CGENN, DARKSTRT, TWGENN and FATES read: user SID, flags, land number and type."""
    def block(user_name: str) -> SharedBlock:
        return SharedBlock(HAND_MADE_USER_SID, 0, user_name, b"",
                           land_number=RPG_PLACE, land_type=land_type)
    return block


GAME_LAUNCHES: dict[str, GameLaunch] = {
    "GOLF": GameLaunch(directory="GOLF", program="Golf", shared_block=golf_block),
    "YSERBIUS": GameLaunch(directory="YSERBIUS", program="Yserbius",
                           shared_block=rpg_block(YSERBIUS_LAND_TYPE)),
    "TWINION": GameLaunch(directory="TWINION", program="Twinion",
                          shared_block=rpg_block(TWINION_LAND_TYPE)),
}
