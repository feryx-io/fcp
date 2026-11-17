from enum import StrEnum

class JoinRole(StrEnum):
    ATTENDEE = "attendee"
    GUEST = "guest"
    HOST = "host"
    PANELIST = "panelist"

    def __str__(self) -> str:
        return str(self.value)
