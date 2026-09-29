from enum import StrEnum

class SubjectIdentifierType0Type(StrEnum):
    USER = "user"

    def __str__(self) -> str:
        return str(self.value)
