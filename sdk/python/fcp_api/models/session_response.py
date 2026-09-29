from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="SessionResponse")



@_attrs_define
class SessionResponse:
    """ A session, with the names of the host and the guest.

        Attributes:
            id (str):
            device_id (str):
            user_id (str):
            started_at (datetime.datetime):
            expires_at (datetime.datetime):
            active (bool):
            device_name (None | str | Unset):
            username (None | str | Unset):
            ended_at (datetime.datetime | None | Unset):
            ended_by (None | str | Unset): `logout`, `admin`, `expired` or `replaced`.
     """

    id: str
    device_id: str
    user_id: str
    started_at: datetime.datetime
    expires_at: datetime.datetime
    active: bool
    device_name: None | str | Unset = UNSET
    username: None | str | Unset = UNSET
    ended_at: datetime.datetime | None | Unset = UNSET
    ended_by: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        device_id = self.device_id

        user_id = self.user_id

        started_at = self.started_at.isoformat()

        expires_at = self.expires_at.isoformat()

        active = self.active

        device_name: None | str | Unset
        if isinstance(self.device_name, Unset):
            device_name = UNSET
        else:
            device_name = self.device_name

        username: None | str | Unset
        if isinstance(self.username, Unset):
            username = UNSET
        else:
            username = self.username

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
            "device_id": device_id,
            "user_id": user_id,
            "started_at": started_at,
            "expires_at": expires_at,
            "active": active,
        })
        if device_name is not UNSET:
            field_dict["device_name"] = device_name
        if username is not UNSET:
            field_dict["username"] = username
        if ended_at is not UNSET:
            field_dict["ended_at"] = ended_at
        if ended_by is not UNSET:
            field_dict["ended_by"] = ended_by

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        device_id = d.pop("device_id")

        user_id = d.pop("user_id")

        started_at = datetime.datetime.fromisoformat(d.pop("started_at"))




        expires_at = datetime.datetime.fromisoformat(d.pop("expires_at"))




        active = d.pop("active")

        def _parse_device_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        device_name = _parse_device_name(d.pop("device_name", UNSET))


        def _parse_username(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        username = _parse_username(d.pop("username", UNSET))


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


        session_response = cls(
            id=id,
            device_id=device_id,
            user_id=user_id,
            started_at=started_at,
            expires_at=expires_at,
            active=active,
            device_name=device_name,
            username=username,
            ended_at=ended_at,
            ended_by=ended_by,
        )


        session_response.additional_properties = d
        return session_response

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
