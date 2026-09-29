from enum import StrEnum

class SiteStatus(StrEnum):
    DISABLED = "disabled"
    ENROLLED = "enrolled"
    PENDING = "pending"

    def __str__(self) -> str:
        return str(self.value)
