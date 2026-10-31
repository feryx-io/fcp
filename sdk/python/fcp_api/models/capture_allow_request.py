from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="CaptureAllowRequest")



@_attrs_define
class CaptureAllowRequest:
    """ Allow a user (every app they sign in to) or one device to capture.

        Attributes:
            until (datetime.datetime): When the allow ends: in the future, at most 30 days away.
            username (None | str | Unset): The user; or name `device_id`.
            device_id (None | str | Unset):
            reason (None | str | Unset): Why, for the record.
     """

    until: datetime.datetime
    username: None | str | Unset = UNSET
    device_id: None | str | Unset = UNSET
    reason: None | str | Unset = UNSET





    def to_dict(self) -> dict[str, Any]:
        until = self.until.isoformat()

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

        reason: None | str | Unset
        if isinstance(self.reason, Unset):
            reason = UNSET
        else:
            reason = self.reason


        field_dict: dict[str, Any] = {}

        field_dict.update({
            "until": until,
        })
        if username is not UNSET:
            field_dict["username"] = username
        if device_id is not UNSET:
            field_dict["device_id"] = device_id
        if reason is not UNSET:
            field_dict["reason"] = reason

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        until = datetime.datetime.fromisoformat(d.pop("until"))




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


        def _parse_reason(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        reason = _parse_reason(d.pop("reason", UNSET))


        capture_allow_request = cls(
            until=until,
            username=username,
            device_id=device_id,
            reason=reason,
        )

        return capture_allow_request

