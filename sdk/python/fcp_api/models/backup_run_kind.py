from enum import StrEnum

class BackupRunKind(StrEnum):
    BACKUP = "backup"
    CHECK = "check"
    TEST = "test"

    def __str__(self) -> str:
        return str(self.value)
