from enum import StrEnum

class BackupRunKind(StrEnum):
    BACKUP = "backup"
    TEST = "test"

    def __str__(self) -> str:
        return str(self.value)
