from enum import StrEnum

class RequireTotp(StrEnum):
    ADMINS = "admins"
    EVERYONE = "everyone"
    NONE = "none"

    def __str__(self) -> str:
        return str(self.value)
