from enum import StrEnum

class StatusState(StrEnum):
    AVAILABLE = "available"
    AWAY = "away"
    BUSY = "busy"
    DND = "dnd"

    def __str__(self) -> str:
        return str(self.value)
