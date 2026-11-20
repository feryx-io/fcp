from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="PageSittingResponseDataItem")



@_attrs_define
class PageSittingResponseDataItem:
    """ One sitting of a meeting's room as the API shows it.

        Attributes:
            id (str):
            room_number (str):
            kind (str): The meeting's kind when it began.
            started_at (datetime.datetime):
            live_at (datetime.datetime | None | Unset): When it went live; a meeting's is its start, a webinar that never
                went live has none.
            ended_at (datetime.datetime | None | Unset): When it ended; none while it runs (or when its node stopped
                before it could say).
            ended_by (None | str | Unset): Who ended a webinar: a host's participant id, `host`, `api` or
                `stage_left`; none when its last caller left.
     """

    id: str
    room_number: str
    kind: str
    started_at: datetime.datetime
    live_at: datetime.datetime | None | Unset = UNSET
    ended_at: datetime.datetime | None | Unset = UNSET
    ended_by: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        room_number = self.room_number

        kind = self.kind

        started_at = self.started_at.isoformat()

        live_at: None | str | Unset
        if isinstance(self.live_at, Unset):
            live_at = UNSET
        elif isinstance(self.live_at, datetime.datetime):
            live_at = self.live_at.isoformat()
        else:
            live_at = self.live_at

        ended_at: None | str | Unset
        if isinstance(self.ended_at, Unset):
            ended_at = UNSET
        elif isinstance(self.ended_at, datetime.datetime):
            ended_at = self.ended_at.isoformat()
        else:
            ended_at = self.ended_at

        ended_by: None | str | Unset
        if isinstance(self.ended_by, Unset):
            ended_by = UNSET
        else:
            ended_by = self.ended_by


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "room_number": room_number,
            "kind": kind,
            "started_at": started_at,
        })
        if live_at is not UNSET:
            field_dict["live_at"] = live_at
        if ended_at is not UNSET:
            field_dict["ended_at"] = ended_at
        if ended_by is not UNSET:
            field_dict["ended_by"] = ended_by

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        room_number = d.pop("room_number")

        kind = d.pop("kind")

        started_at = datetime.datetime.fromisoformat(d.pop("started_at"))




        def _parse_live_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                live_at_type_0 = datetime.datetime.fromisoformat(data)



                return live_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        live_at = _parse_live_at(d.pop("live_at", UNSET))


        def _parse_ended_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                ended_at_type_0 = datetime.datetime.fromisoformat(data)



                return ended_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        ended_at = _parse_ended_at(d.pop("ended_at", UNSET))


        def _parse_ended_by(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        ended_by = _parse_ended_by(d.pop("ended_by", UNSET))


        page_sitting_response_data_item = cls(
            id=id,
            room_number=room_number,
            kind=kind,
            started_at=started_at,
            live_at=live_at,
            ended_at=ended_at,
            ended_by=ended_by,
        )


        page_sitting_response_data_item.additional_properties = d
        return page_sitting_response_data_item

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
