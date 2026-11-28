from enum import StrEnum

class TargetViewType2Type(StrEnum):
    DIRECTORY = "directory"

    def __str__(self) -> str:
        return str(self.value)
