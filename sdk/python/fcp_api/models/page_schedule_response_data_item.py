from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast
import datetime

if TYPE_CHECKING:
  from ..models.holiday import Holiday
  from ..models.weekly_window import WeeklyWindow





T = TypeVar("T", bound="PageScheduleResponseDataItem")



@_attrs_define
class PageScheduleResponseDataItem:
    """ 
        Attributes:
            id (str):
            tenant_id (str):
            name (str):
            timezone (str):
            weekly (list[WeeklyWindow]):
            holidays (list[Holiday]):
            state (str): `open`, `closed` or `holiday`, now.
            updated_at (datetime.datetime):
     """

    id: str
    tenant_id: str
    name: str
    timezone: str
    weekly: list[WeeklyWindow]
    holidays: list[Holiday]
    state: str
    updated_at: datetime.datetime
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.holiday import Holiday # noqa: PLC0415
        from ..models.weekly_window import WeeklyWindow # noqa: PLC0415
        id = self.id

        tenant_id = self.tenant_id

        name = self.name

        timezone = self.timezone

        weekly = []
        for weekly_item_data in self.weekly:
            weekly_item = weekly_item_data.to_dict()
            weekly.append(weekly_item)



        holidays = []
        for holidays_item_data in self.holidays:
            holidays_item = holidays_item_data.to_dict()
            holidays.append(holidays_item)



        state = self.state

        updated_at = self.updated_at.isoformat()


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "tenant_id": tenant_id,
            "name": name,
            "timezone": timezone,
            "weekly": weekly,
            "holidays": holidays,
            "state": state,
            "updated_at": updated_at,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.holiday import Holiday # noqa: PLC0415
        from ..models.weekly_window import WeeklyWindow # noqa: PLC0415
        d = dict(src_dict)
        id = d.pop("id")

        tenant_id = d.pop("tenant_id")

        name = d.pop("name")

        timezone = d.pop("timezone")

        weekly = []
        _weekly = d.pop("weekly")
        for weekly_item_data in (_weekly):
            weekly_item = WeeklyWindow.from_dict(weekly_item_data)



            weekly.append(weekly_item)


        holidays = []
        _holidays = d.pop("holidays")
        for holidays_item_data in (_holidays):
            holidays_item = Holiday.from_dict(holidays_item_data)



            holidays.append(holidays_item)


        state = d.pop("state")

        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        page_schedule_response_data_item = cls(
            id=id,
            tenant_id=tenant_id,
            name=name,
            timezone=timezone,
            weekly=weekly,
            holidays=holidays,
            state=state,
            updated_at=updated_at,
        )


        page_schedule_response_data_item.additional_properties = d
        return page_schedule_response_data_item

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
