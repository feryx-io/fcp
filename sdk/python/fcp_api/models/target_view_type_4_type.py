from enum import StrEnum

class TargetViewType4Type(StrEnum):
    RCLONE = "rclone"

    def __str__(self) -> str:
        return str(self.value)
