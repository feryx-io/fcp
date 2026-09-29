from enum import StrEnum

class ExtensionTargetResponseType0Type(StrEnum):
    USER = "user"

    def __str__(self) -> str:
        return str(self.value)
