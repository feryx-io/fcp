from enum import StrEnum

class SubjectIdentifierType1Type(StrEnum):
    NUMBER = "number"

    def __str__(self) -> str:
        return str(self.value)
