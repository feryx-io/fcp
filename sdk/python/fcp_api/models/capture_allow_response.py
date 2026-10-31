from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="CaptureAllowResponse")



@_attrs_define
class CaptureAllowResponse:
    """ An administrator's allow.

        Attributes:
            id (str):
            kind (str): `user` or `device`.
            until (datetime.datetime):
            active (bool): Not yet ended.
            created_by (str):
            created_at (datetime.datetime):
            user_id (None | str | Unset):
            username (None | str | Unset): The user's name, while they exist.
            device_id (None | str | Unset):
            device_name (None | str | Unset): The device's name, while it exists.
            reason (None | str | Unset):
     """

    id: str
    kind: str
    until: datetime.datetime
    active: bool
    created_by: str
    created_at: datetime.datetime
    user_id: None | str | Unset = UNSET
    username: None | str | Unset = UNSET
    device_id: None | str | Unset = UNSET
    device_name: None | str | Unset = UNSET
    reason: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        kind = self.kind

        until = self.until.isoformat()

        active = self.active

        created_by = self.created_by

        created_at = self.created_at.isoformat()

        user_id: None | str | Unset
        if isinstance(self.user_id, Unset):
            user_id = UNSET
        else:
            user_id = self.user_id

        username: None | str | Unset
        if isinstance(self.username, Unset):
            username = UNSET
        else:
            username = self.username

        device_id: None | str | Unset
        if isinstance(self.device_id, Unset):
            device_id = UNSET
        else:
            device_id = self.device_id

        device_name: None | str | Unset
        if isinstance(self.device_name, Unset):
            device_name = UNSET
        else:
            device_name = self.device_name

        reason: None | str | Unset
        if isinstance(self.reason, Unset):
            reason = UNSET
        else:
            reason = self.reason


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "kind": kind,
            "until": until,
            "active": active,
            "created_by": created_by,
            "created_at": created_at,
        })
        if user_id is not UNSET:
            field_dict["user_id"] = user_id
        if username is not UNSET:
            field_dict["username"] = username
        if device_id is not UNSET:
            field_dict["device_id"] = device_id
        if device_name is not UNSET:
            field_dict["device_name"] = device_name
        if reason is not UNSET:
            field_dict["reason"] = reason

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        kind = d.pop("kind")

        until = datetime.datetime.fromisoformat(d.pop("until"))




        active = d.pop("active")

        created_by = d.pop("created_by")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        def _parse_user_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        user_id = _parse_user_id(d.pop("user_id", UNSET))


        def _parse_username(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        username = _parse_username(d.pop("username", UNSET))


        def _parse_device_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        device_id = _parse_device_id(d.pop("device_id", UNSET))


        def _parse_device_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        device_name = _parse_device_name(d.pop("device_name", UNSET))


        def _parse_reason(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        reason = _parse_reason(d.pop("reason", UNSET))


        capture_allow_response = cls(
            id=id,
            kind=kind,
            until=until,
            active=active,
            created_by=created_by,
            created_at=created_at,
            user_id=user_id,
            username=username,
            device_id=device_id,
            device_name=device_name,
            reason=reason,
        )


        capture_allow_response.additional_properties = d
        return capture_allow_response

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
