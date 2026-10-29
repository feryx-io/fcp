from enum import StrEnum

class Shown(StrEnum):
    AVAILABLE = "available"
    AWAY = "away"
    BUSY = "busy"
    DND = "dnd"
    OFFLINE = "offline"
    ON_CALL = "on_call"
    RINGING = "ringing"

    def __str__(self) -> str:
        return str(self.value)
