from enum import StrEnum

class TargetRequestType0Type(StrEnum):
    SFTP = "sftp"

    def __str__(self) -> str:
        return str(self.value)
