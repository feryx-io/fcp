from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="MeMailboxResponse")



@_attrs_define
class MeMailboxResponse:
    """ Mailbox response (reuse same shape as admin).

        Attributes:
            id (str):
            extension (str):
            enabled (bool):
            max_message_duration (int):
            max_messages (int):
            email_notification (bool):
            attach_audio (bool):
            email_address (None | str | Unset):
            retention_days (int | None | Unset):
     """

    id: str
    extension: str
    enabled: bool
    max_message_duration: int
    max_messages: int
    email_notification: bool
    attach_audio: bool
    email_address: None | str | Unset = UNSET
    retention_days: int | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        extension = self.extension

        enabled = self.enabled

        max_message_duration = self.max_message_duration

        max_messages = self.max_messages

        email_notification = self.email_notification

        attach_audio = self.attach_audio

        email_address: None | str | Unset
        if isinstance(self.email_address, Unset):
            email_address = UNSET
        else:
            email_address = self.email_address

        retention_days: int | None | Unset
        if isinstance(self.retention_days, Unset):
            retention_days = UNSET
        else:
            retention_days = self.retention_days


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "extension": extension,
            "enabled": enabled,
            "max_message_duration": max_message_duration,
            "max_messages": max_messages,
            "email_notification": email_notification,
            "attach_audio": attach_audio,
        })
        if email_address is not UNSET:
            field_dict["email_address"] = email_address
        if retention_days is not UNSET:
            field_dict["retention_days"] = retention_days

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        extension = d.pop("extension")

        enabled = d.pop("enabled")

        max_message_duration = d.pop("max_message_duration")

        max_messages = d.pop("max_messages")

        email_notification = d.pop("email_notification")

        attach_audio = d.pop("attach_audio")

        def _parse_email_address(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        email_address = _parse_email_address(d.pop("email_address", UNSET))


        def _parse_retention_days(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        retention_days = _parse_retention_days(d.pop("retention_days", UNSET))


        me_mailbox_response = cls(
            id=id,
            extension=extension,
            enabled=enabled,
            max_message_duration=max_message_duration,
            max_messages=max_messages,
            email_notification=email_notification,
            attach_audio=attach_audio,
            email_address=email_address,
            retention_days=retention_days,
        )


        me_mailbox_response.additional_properties = d
        return me_mailbox_response

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
