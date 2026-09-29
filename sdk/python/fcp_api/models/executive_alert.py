from enum import StrEnum

class ExecutiveAlert(StrEnum):
    ASSISTANTS = "assistants"
    ASSISTANTS_AND_EXECUTIVE = "assistants_and_executive"

    def __str__(self) -> str:
        return str(self.value)
