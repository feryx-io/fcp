from enum import StrEnum

class TargetViewType3Type(StrEnum):
    REST = "rest"

    def __str__(self) -> str:
        return str(self.value)
