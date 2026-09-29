from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast






T = TypeVar("T", bound="WeeklyWindow")



@_attrs_define
class WeeklyWindow:
    """ A span of hours on some days: `start` to `end` local time; an `end`
    before its `start` runs past midnight into the next day, and an `end`
    equal to its `start` is the whole day.

        Attributes:
            days (list[str]): `mon` … `sun`.
                `Mon` … `Sun`.
            start (str): `HH:MM`.
            end (str):
     """

    days: list[str]
    start: str
    end: str
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        days = self.days



        start = self.start

        end = self.end


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "days": days,
            "start": start,
            "end": end,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        days = cast(list[str], d.pop("days"))


        start = d.pop("start")

        end = d.pop("end")

        weekly_window = cls(
            days=days,
            start=start,
            end=end,
        )


        weekly_window.additional_properties = d
        return weekly_window

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
