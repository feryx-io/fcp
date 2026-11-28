from enum import StrEnum

class BackupRunState(StrEnum):
    FAILED = "failed"
    REQUESTED = "requested"
    RUNNING = "running"
    SUCCEEDED = "succeeded"

    def __str__(self) -> str:
        return str(self.value)
