from enum import StrEnum

class ExtensionTargetResponseType3Type(StrEnum):
    SERVICE = "service"

    def __str__(self) -> str:
        return str(self.value)
