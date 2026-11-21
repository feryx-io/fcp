from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="AttendanceRegistrant")



@_attrs_define
class AttendanceRegistrant:
    """ A registration and whether they came.

        Attributes:
            id (str):
            meeting_id (str):
            name (str):
            created_at (datetime.datetime):
            attended (bool):
            attended_secs (int): Their time in the room across legs (the lobby aside).
            watched (bool): They watched the stream by their own link (W8).
            watched_secs (int): Seconds of it they watched live.
            email (None | str | Unset):
            revoked_at (datetime.datetime | None | Unset): When it was revoked: its token is refused.
     """

    id: str
    meeting_id: str
    name: str
    created_at: datetime.datetime
    attended: bool
    attended_secs: int
    watched: bool
    watched_secs: int
    email: None | str | Unset = UNSET
    revoked_at: datetime.datetime | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        meeting_id = self.meeting_id

        name = self.name

        created_at = self.created_at.isoformat()

        attended = self.attended

        attended_secs = self.attended_secs

        watched = self.watched

        watched_secs = self.watched_secs

        email: None | str | Unset
        if isinstance(self.email, Unset):
            email = UNSET
        else:
            email = self.email

        revoked_at: None | str | Unset
        if isinstance(self.revoked_at, Unset):
            revoked_at = UNSET
        elif isinstance(self.revoked_at, datetime.datetime):
            revoked_at = self.revoked_at.isoformat()
        else:
            revoked_at = self.revoked_at


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "meeting_id": meeting_id,
            "name": name,
            "created_at": created_at,
            "attended": attended,
            "attended_secs": attended_secs,
            "watched": watched,
            "watched_secs": watched_secs,
        })
        if email is not UNSET:
            field_dict["email"] = email
        if revoked_at is not UNSET:
            field_dict["revoked_at"] = revoked_at

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        meeting_id = d.pop("meeting_id")

        name = d.pop("name")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        attended = d.pop("attended")

        attended_secs = d.pop("attended_secs")

        watched = d.pop("watched")

        watched_secs = d.pop("watched_secs")

        def _parse_email(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        email = _parse_email(d.pop("email", UNSET))


        def _parse_revoked_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                revoked_at_type_0 = datetime.datetime.fromisoformat(data)



                return revoked_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        revoked_at = _parse_revoked_at(d.pop("revoked_at", UNSET))


        attendance_registrant = cls(
            id=id,
            meeting_id=meeting_id,
            name=name,
            created_at=created_at,
            attended=attended,
            attended_secs=attended_secs,
            watched=watched,
            watched_secs=watched_secs,
            email=email,
            revoked_at=revoked_at,
        )


        attendance_registrant.additional_properties = d
        return attendance_registrant

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
