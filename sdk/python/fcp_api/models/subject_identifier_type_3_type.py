from enum import StrEnum

class SubjectIdentifierType3Type(StrEnum):
    EMAIL = "email"

    def __str__(self) -> str:
        return str(self.value)
