from enum import StrEnum

class CapUnit(StrEnum):
    CHARACTERS = "characters"
    MINUTES = "minutes"

    def __str__(self) -> str:
        return str(self.value)
