from enum import StrEnum

class ExtensionTargetResponseType2Type(StrEnum):
    DEVICE = "device"

    def __str__(self) -> str:
        return str(self.value)
