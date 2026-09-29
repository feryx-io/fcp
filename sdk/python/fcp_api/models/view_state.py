from enum import StrEnum

class ViewState(StrEnum):
    REMOTE_REGIONS_LOST = "remote_regions_lost"
    STORE_UNREACHABLE = "store_unreachable"
    WHOLE = "whole"

    def __str__(self) -> str:
        return str(self.value)
