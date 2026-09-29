from enum import StrEnum

class DestinationClass(StrEnum):
    EMERGENCY = "emergency"
    INTERNAL = "internal"
    INTERNATIONAL = "international"
    NATIONAL = "national"
    PREMIUM = "premium"
    TOLL_FREE = "toll_free"
    UNKNOWN = "unknown"

    def __str__(self) -> str:
        return str(self.value)
