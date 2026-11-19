from enum import StrEnum

class ChatAudience(StrEnum):
    HOSTS = "hosts"
    ROOM = "room"

    def __str__(self) -> str:
        return str(self.value)
