from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="PageCaptureResponseDataItem")



@_attrs_define
class PageCaptureResponseDataItem:
    """ An uploaded capture.

        Attributes:
            id (str):
            device_id (str): The app that took it, which may since have been removed.
            user_id (str):
            size (int):
            sha256 (str):
            uploaded_at (datetime.datetime):
            expires_at (datetime.datetime): When retention removes it, unless a hold keeps it.
            held (bool): A legal hold keeps it past its expiry.
            device_name (None | str | Unset):
            username (None | str | Unset):
            call_id (None | str | Unset): The call it holds, when the app named one.
     """

    id: str
    device_id: str
    user_id: str
    size: int
    sha256: str
    uploaded_at: datetime.datetime
    expires_at: datetime.datetime
    held: bool
    device_name: None | str | Unset = UNSET
    username: None | str | Unset = UNSET
    call_id: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        device_id = self.device_id

        user_id = self.user_id

        size = self.size

        sha256 = self.sha256

        uploaded_at = self.uploaded_at.isoformat()

        expires_at = self.expires_at.isoformat()

        held = self.held

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

        call_id: None | str | Unset
        if isinstance(self.call_id, Unset):
            call_id = UNSET
        else:
            call_id = self.call_id


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "device_id": device_id,
            "user_id": user_id,
            "size": size,
            "sha256": sha256,
            "uploaded_at": uploaded_at,
            "expires_at": expires_at,
            "held": held,
        })
        if device_name is not UNSET:
            field_dict["device_name"] = device_name
        if username is not UNSET:
            field_dict["username"] = username
        if call_id is not UNSET:
            field_dict["call_id"] = call_id

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        device_id = d.pop("device_id")

        user_id = d.pop("user_id")

        size = d.pop("size")

        sha256 = d.pop("sha256")

        uploaded_at = datetime.datetime.fromisoformat(d.pop("uploaded_at"))




        expires_at = datetime.datetime.fromisoformat(d.pop("expires_at"))




        held = d.pop("held")

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


        def _parse_call_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        call_id = _parse_call_id(d.pop("call_id", UNSET))


        page_capture_response_data_item = cls(
            id=id,
            device_id=device_id,
            user_id=user_id,
            size=size,
            sha256=sha256,
            uploaded_at=uploaded_at,
            expires_at=expires_at,
            held=held,
            device_name=device_name,
            username=username,
            call_id=call_id,
        )


        page_capture_response_data_item.additional_properties = d
        return page_capture_response_data_item

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
