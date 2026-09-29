from enum import StrEnum

class JoinRole(StrEnum):
    GUEST = "guest"
    HOST = "host"

    def __str__(self) -> str:
        return str(self.value)
