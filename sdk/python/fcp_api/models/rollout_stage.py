from enum import StrEnum

class RolloutStage(StrEnum):
    ALL = "all"
    PILOT = "pilot"

    def __str__(self) -> str:
        return str(self.value)
