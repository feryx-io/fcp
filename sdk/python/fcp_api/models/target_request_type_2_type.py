from enum import StrEnum

class TargetRequestType2Type(StrEnum):
    DIRECTORY = "directory"

    def __str__(self) -> str:
        return str(self.value)
