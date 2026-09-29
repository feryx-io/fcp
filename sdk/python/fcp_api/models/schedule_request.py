from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.holiday import Holiday
  from ..models.weekly_window import WeeklyWindow





T = TypeVar("T", bound="ScheduleRequest")



@_attrs_define
class ScheduleRequest:
    """ What a schedule is written as.

        Attributes:
            name (str):
            timezone (None | str | Unset): IANA; unset, the tenant's timezone (§3.7), else UTC — and on an
                update, the schedule's own.
            weekly (list[WeeklyWindow] | Unset):
            holidays (list[Holiday] | Unset):
     """

    name: str
    timezone: None | str | Unset = UNSET
    weekly: list[WeeklyWindow] | Unset = UNSET
    holidays: list[Holiday] | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.holiday import Holiday # noqa: PLC0415
        from ..models.weekly_window import WeeklyWindow # noqa: PLC0415
        name = self.name

        timezone: None | str | Unset
        if isinstance(self.timezone, Unset):
            timezone = UNSET
        else:
            timezone = self.timezone

        weekly: list[dict[str, Any]] | Unset = UNSET
        if not isinstance(self.weekly, Unset):
            weekly = []
            for weekly_item_data in self.weekly:
                weekly_item = weekly_item_data.to_dict()
                weekly.append(weekly_item)



        holidays: list[dict[str, Any]] | Unset = UNSET
        if not isinstance(self.holidays, Unset):
            holidays = []
            for holidays_item_data in self.holidays:
                holidays_item = holidays_item_data.to_dict()
                holidays.append(holidays_item)




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "name": name,
        })
        if timezone is not UNSET:
            field_dict["timezone"] = timezone
        if weekly is not UNSET:
            field_dict["weekly"] = weekly
        if holidays is not UNSET:
            field_dict["holidays"] = holidays

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.holiday import Holiday # noqa: PLC0415
        from ..models.weekly_window import WeeklyWindow # noqa: PLC0415
        d = dict(src_dict)
        name = d.pop("name")

        def _parse_timezone(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        timezone = _parse_timezone(d.pop("timezone", UNSET))


        _weekly = d.pop("weekly", UNSET)
        weekly: list[WeeklyWindow] | Unset = UNSET
        if _weekly is not UNSET:
            weekly = []
            for weekly_item_data in _weekly:
                weekly_item = WeeklyWindow.from_dict(weekly_item_data)



                weekly.append(weekly_item)


        _holidays = d.pop("holidays", UNSET)
        holidays: list[Holiday] | Unset = UNSET
        if _holidays is not UNSET:
            holidays = []
            for holidays_item_data in _holidays:
                holidays_item = Holiday.from_dict(holidays_item_data)



                holidays.append(holidays_item)


        schedule_request = cls(
            name=name,
            timezone=timezone,
            weekly=weekly,
            holidays=holidays,
        )


        schedule_request.additional_properties = d
        return schedule_request

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
