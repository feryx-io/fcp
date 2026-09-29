from enum import StrEnum

class BalanceTransactionKind(StrEnum):
    ADJUSTMENT = "adjustment"
    CALL_CHARGE = "call_charge"
    TOP_UP = "top_up"

    def __str__(self) -> str:
        return str(self.value)
