from enum import StrEnum

class PasswordLogin(StrEnum):
    ADMINS_ONLY = "admins_only"
    ALLOWED = "allowed"
    DISABLED = "disabled"

    def __str__(self) -> str:
        return str(self.value)
