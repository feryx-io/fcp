from enum import StrEnum

class TargetViewType0Type(StrEnum):
    SFTP = "sftp"

    def __str__(self) -> str:
        return str(self.value)
