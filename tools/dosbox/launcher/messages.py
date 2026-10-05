"""The two byte layouts the launcher hands to TSNEXEC: the Login message and the shared block.
Login is docs/protocol/messages.md section 4.2; the block is section 5.3 and the census README."""
import struct
from dataclasses import dataclass

LOGIN_COMMAND = 53
LOGIN_HEADER = struct.Struct("<BBHBBBBHHB")
PASSWORD_FIELD_LENGTH = 11
NAME_FIELD_LENGTH = 11
PASSWORD_FROM_FILE = 1
HUB_LAND_TYPE = 1
SHARED_BLOCK_LIMIT = 256
USER_SID_OFFSET = 0x00
HANDED_SID_OFFSET = 0x04
USER_NAME_OFFSET = 0x10
PARAMETERS_OFFSET = 0x80


@dataclass(frozen=True)
class ClientVersion:
    major: int
    minor: int
    revision: int


# The interpreter version the stock CD client sent (docs/protocol/captures.md section 4).
STOCK_CLIENT_VERSION = ClientVersion(2, 3, 18)


@dataclass(frozen=True)
class Login:
    account: int
    encoded_password: bytes
    name: str
    land_type: int = HUB_LAND_TYPE
    version: ClientVersion = STOCK_CLIENT_VERSION


def encode_login(login: Login) -> bytes:
    """Command 53 exactly as LSCI sends it; the password field's eleventh byte is zero."""
    header = LOGIN_HEADER.pack(
        LOGIN_COMMAND, 0, 0, login.land_type,
        login.version.major, login.version.minor, login.version.revision,
        login.account & 0xFFFF, login.account >> 16, PASSWORD_FROM_FILE)
    password = login.encoded_password.ljust(PASSWORD_FIELD_LENGTH, b"\0")
    return header + password + login.name.encode("ascii") + b"\0"


@dataclass(frozen=True)
class SharedBlock:
    user_sid: int
    handed_sid: int
    user_name: str
    parameters: bytes


def encode_shared_block(block: SharedBlock) -> bytes:
    """The words and name the DOS games read, zeros elsewhere, then the game's parameters."""
    image = bytearray(PARAMETERS_OFFSET)
    struct.pack_into("<H", image, USER_SID_OFFSET, block.user_sid)
    struct.pack_into("<H", image, HANDED_SID_OFFSET, block.handed_sid)
    name = block.user_name.encode("ascii")[: NAME_FIELD_LENGTH - 1]
    image[USER_NAME_OFFSET : USER_NAME_OFFSET + len(name)] = name
    image += block.parameters
    if len(image) > SHARED_BLOCK_LIMIT:
        raise ValueError(f"shared block of {len(image)} bytes exceeds {SHARED_BLOCK_LIMIT}")
    return bytes(image)
