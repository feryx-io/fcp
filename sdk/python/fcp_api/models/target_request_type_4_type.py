from enum import StrEnum

class TargetRequestType4Type(StrEnum):
    RCLONE = "rclone"

    def __str__(self) -> str:
        return str(self.value)
