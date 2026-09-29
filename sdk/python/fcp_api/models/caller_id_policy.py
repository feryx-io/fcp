from enum import StrEnum

class CallerIdPolicy(StrEnum):
    LINE = "line"
    WITHHELD = "withheld"

    def __str__(self) -> str:
        return str(self.value)
