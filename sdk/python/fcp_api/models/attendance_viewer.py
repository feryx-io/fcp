from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="AttendanceViewer")



@_attrs_define
class AttendanceViewer:
    """ A registrant who watched the sitting's stream by their own link (W8).

        Attributes:
            registrant_id (str):
            first_at (datetime.datetime):
            last_at (datetime.datetime):
            watched_secs (int): Seconds watched while it was live: the gaps under 30 s between the
                player's fetches of the playlist. A replay opened shows none.
            name (None | str | Unset):
     """

    registrant_id: str
    first_at: datetime.datetime
    last_at: datetime.datetime
    watched_secs: int
    name: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        registrant_id = self.registrant_id

        first_at = self.first_at.isoformat()

        last_at = self.last_at.isoformat()

        watched_secs = self.watched_secs

        name: None | str | Unset
        if isinstance(self.name, Unset):
            name = UNSET
        else:
            name = self.name


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "registrant_id": registrant_id,
            "first_at": first_at,
            "last_at": last_at,
            "watched_secs": watched_secs,
        })
        if name is not UNSET:
            field_dict["name"] = name

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        registrant_id = d.pop("registrant_id")

        first_at = datetime.datetime.fromisoformat(d.pop("first_at"))




        last_at = datetime.datetime.fromisoformat(d.pop("last_at"))




        watched_secs = d.pop("watched_secs")

        def _parse_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        name = _parse_name(d.pop("name", UNSET))


        attendance_viewer = cls(
            registrant_id=registrant_id,
            first_at=first_at,
            last_at=last_at,
            watched_secs=watched_secs,
            name=name,
        )


        attendance_viewer.additional_properties = d
        return attendance_viewer

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
