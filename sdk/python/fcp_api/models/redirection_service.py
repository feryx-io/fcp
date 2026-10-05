from enum import StrEnum

class RedirectionService(StrEnum):
    POLY_ZTP = "poly_ztp"
    YEALINK_RPS = "yealink_rps"

    def __str__(self) -> str:
        return str(self.value)
