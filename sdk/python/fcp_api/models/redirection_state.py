from enum import StrEnum

class RedirectionState(StrEnum):
    FAILED = "failed"
    REGISTERED = "registered"

    def __str__(self) -> str:
        return str(self.value)
