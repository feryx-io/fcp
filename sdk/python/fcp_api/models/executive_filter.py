from enum import StrEnum

class ExecutiveFilter(StrEnum):
    ALL = "all"
    EXTERNAL = "external"
    NONE = "none"

    def __str__(self) -> str:
        return str(self.value)
