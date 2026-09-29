from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="TimeWindow")



@_attrs_define
class TimeWindow:
    """ A time window during which subscriptions are allowed.

        Attributes:
            start_hour (int): Start hour (0-23, UTC).
            end_hour (int): End hour (0-23, UTC). If end < start, wraps past midnight.
            days (list[int] | Unset): Days of week (0=Sun, 1=Mon, ..., 6=Sat). Empty = all days.
     """

    start_hour: int
    end_hour: int
    days: list[int] | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        start_hour = self.start_hour

        end_hour = self.end_hour

        days: list[int] | Unset = UNSET
        if not isinstance(self.days, Unset):
            days = self.days




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "start_hour": start_hour,
            "end_hour": end_hour,
        })
        if days is not UNSET:
            field_dict["days"] = days

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        start_hour = d.pop("start_hour")

        end_hour = d.pop("end_hour")

        days = cast(list[int], d.pop("days", UNSET))


        time_window = cls(
            start_hour=start_hour,
            end_hour=end_hour,
            days=days,
        )


        time_window.additional_properties = d
        return time_window

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
