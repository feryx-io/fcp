from enum import StrEnum

class QuestionStatus(StrEnum):
    ANSWERED = "answered"
    DISMISSED = "dismissed"
    PENDING = "pending"

    def __str__(self) -> str:
        return str(self.value)
