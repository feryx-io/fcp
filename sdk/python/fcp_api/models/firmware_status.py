from enum import StrEnum

class FirmwareStatus(StrEnum):
    BEHIND = "behind"
    CURRENT = "current"
    UNKNOWN = "unknown"

    def __str__(self) -> str:
        return str(self.value)
