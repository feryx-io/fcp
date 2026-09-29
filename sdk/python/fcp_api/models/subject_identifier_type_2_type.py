from enum import StrEnum

class SubjectIdentifierType2Type(StrEnum):
    AOR = "aor"

    def __str__(self) -> str:
        return str(self.value)
