from enum import StrEnum

class ExtensionTargetResponseType1Type(StrEnum):
    PHONE_NUMBER = "phone_number"

    def __str__(self) -> str:
        return str(self.value)
