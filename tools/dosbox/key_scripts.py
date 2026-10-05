#!/usr/bin/env python3
"""Named keyboard scripts that drive the stock client's menus.
The key syntax is in docs/dosbox.md."""
from dataclasses import dataclass

PERSONA_NAME = "guybrush"
PERSONA_AGE = "30"
PERSONA_PLACE = "maine"


@dataclass(frozen=True)
class KeyScript:
    keys: str
    wait: float
    pace: float
    seconds: int
    creates_persona: bool = False


# Waits are DOSBox-X seconds measured from the autotype command; screens take 10 to 20 s to appear.
KEY_SCRIPTS = {
    "create-persona": KeyScript(
        keys=(
            "enter 3s right enter 20s enter 6s "
            f"={PERSONA_NAME} tab ={PERSONA_AGE} tab ={PERSONA_PLACE} 1s enter 2s "
            "enter 20s enter 12s enter 15s enter 15s tab 3s tab 3s tab 3s enter 30s"
        ),
        wait=22,
        pace=0.5,
        seconds=260,
        creates_persona=True,
    ),
    "play": KeyScript(keys="enter 40s", wait=30, pace=0.5, seconds=120),
    # Thirteen tabs walk the map's places back round to the Clubhouse, where the cursor starts;
    # then enter it, accept the place list and the "Want To Play" dialog.
    "clubhouse": KeyScript(
        keys="enter 40s " + "tab 5s " * 13 + "enter 12s enter 30s enter 150s",
        wait=30,
        pace=0.5,
        seconds=520,
    ),
}
