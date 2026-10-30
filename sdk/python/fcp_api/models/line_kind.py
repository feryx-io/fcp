from enum import StrEnum

class LineKind(StrEnum):
    EXTENSION = "extension"
    ON_BEHALF = "on_behalf"
    OWN = "own"
    SHARED = "shared"

    def __str__(self) -> str:
        return str(self.value)
