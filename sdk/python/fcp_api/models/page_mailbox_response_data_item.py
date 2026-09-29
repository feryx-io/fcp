from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="PageMailboxResponseDataItem")



@_attrs_define
class PageMailboxResponseDataItem:
    """ Voicemail mailbox response.

        Attributes:
            id (str):
            tenant_id (str):
            user_id (str):
            line_binding_id (str):
            extension (str):
            enabled (bool):
            max_message_duration (int):
            max_messages (int):
            email_notification (bool):
            attach_audio (bool):
            include_transcription (bool):
            transcription_enabled (bool):
            created_at (datetime.datetime):
            updated_at (datetime.datetime):
            email_address (None | str | Unset):
            retention_days (int | None | Unset):
     """

    id: str
    tenant_id: str
    user_id: str
    line_binding_id: str
    extension: str
    enabled: bool
    max_message_duration: int
    max_messages: int
    email_notification: bool
    attach_audio: bool
    include_transcription: bool
    transcription_enabled: bool
    created_at: datetime.datetime
    updated_at: datetime.datetime
    email_address: None | str | Unset = UNSET
    retention_days: int | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        tenant_id = self.tenant_id

        user_id = self.user_id

        line_binding_id = self.line_binding_id

        extension = self.extension

        enabled = self.enabled

        max_message_duration = self.max_message_duration

        max_messages = self.max_messages

        email_notification = self.email_notification

        attach_audio = self.attach_audio

        include_transcription = self.include_transcription

        transcription_enabled = self.transcription_enabled

        created_at = self.created_at.isoformat()

        updated_at = self.updated_at.isoformat()

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
            "tenant_id": tenant_id,
            "user_id": user_id,
            "line_binding_id": line_binding_id,
            "extension": extension,
            "enabled": enabled,
            "max_message_duration": max_message_duration,
            "max_messages": max_messages,
            "email_notification": email_notification,
            "attach_audio": attach_audio,
            "include_transcription": include_transcription,
            "transcription_enabled": transcription_enabled,
            "created_at": created_at,
            "updated_at": updated_at,
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

        tenant_id = d.pop("tenant_id")

        user_id = d.pop("user_id")

        line_binding_id = d.pop("line_binding_id")

        extension = d.pop("extension")

        enabled = d.pop("enabled")

        max_message_duration = d.pop("max_message_duration")

        max_messages = d.pop("max_messages")

        email_notification = d.pop("email_notification")

        attach_audio = d.pop("attach_audio")

        include_transcription = d.pop("include_transcription")

        transcription_enabled = d.pop("transcription_enabled")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




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


        page_mailbox_response_data_item = cls(
            id=id,
            tenant_id=tenant_id,
            user_id=user_id,
            line_binding_id=line_binding_id,
            extension=extension,
            enabled=enabled,
            max_message_duration=max_message_duration,
            max_messages=max_messages,
            email_notification=email_notification,
            attach_audio=attach_audio,
            include_transcription=include_transcription,
            transcription_enabled=transcription_enabled,
            created_at=created_at,
            updated_at=updated_at,
            email_address=email_address,
            retention_days=retention_days,
        )


        page_mailbox_response_data_item.additional_properties = d
        return page_mailbox_response_data_item

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
