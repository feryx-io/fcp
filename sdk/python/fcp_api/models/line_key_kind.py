from enum import StrEnum

class LineKeyKind(StrEnum):
    BLF = "blf"
    PARK = "park"
    SPEED_DIAL = "speed_dial"

    def __str__(self) -> str:
        return str(self.value)
