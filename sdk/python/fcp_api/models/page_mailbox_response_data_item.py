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
            extension (str):
            enabled (bool):
            max_message_duration (int):
            max_messages (int):
            email_notification (bool):
            attach_audio (bool):
            include_transcription (bool):
            transcription_enabled (bool):
            greeting_name (str): The name a personalized greeting says: `full_name`, `display_name`
                or `recorded` (the mailbox's name greeting).
            greeting_heard (str): What callers hear now: `standard`, `personalized` or `own`. A
                personalized greeting not made (no name, no TTS) is heard as
                standard, as is an own greeting never recorded.
            created_at (datetime.datetime):
            updated_at (datetime.datetime):
            line_binding_id (None | str | Unset):
            email_address (None | str | Unset):
            retention_days (int | None | Unset):
            flow_id (None | str | Unset): The flow that handles a call this mailbox would take, if not the
                mailbox itself.
            greeting_choice (None | str | Unset): The greeting chosen: `standard`, `personalized` or `own`; `null` is
                the default — personalized when the owner has a first and last
                name, standard otherwise (#343).
     """

    id: str
    tenant_id: str
    user_id: str
    extension: str
    enabled: bool
    max_message_duration: int
    max_messages: int
    email_notification: bool
    attach_audio: bool
    include_transcription: bool
    transcription_enabled: bool
    greeting_name: str
    greeting_heard: str
    created_at: datetime.datetime
    updated_at: datetime.datetime
    line_binding_id: None | str | Unset = UNSET
    email_address: None | str | Unset = UNSET
    retention_days: int | None | Unset = UNSET
    flow_id: None | str | Unset = UNSET
    greeting_choice: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        tenant_id = self.tenant_id

        user_id = self.user_id

        extension = self.extension

        enabled = self.enabled

        max_message_duration = self.max_message_duration

        max_messages = self.max_messages

        email_notification = self.email_notification

        attach_audio = self.attach_audio

        include_transcription = self.include_transcription

        transcription_enabled = self.transcription_enabled

        greeting_name = self.greeting_name

        greeting_heard = self.greeting_heard

        created_at = self.created_at.isoformat()

        updated_at = self.updated_at.isoformat()

        line_binding_id: None | str | Unset
        if isinstance(self.line_binding_id, Unset):
            line_binding_id = UNSET
        else:
            line_binding_id = self.line_binding_id

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

        flow_id: None | str | Unset
        if isinstance(self.flow_id, Unset):
            flow_id = UNSET
        else:
            flow_id = self.flow_id

        greeting_choice: None | str | Unset
        if isinstance(self.greeting_choice, Unset):
            greeting_choice = UNSET
        else:
            greeting_choice = self.greeting_choice


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "tenant_id": tenant_id,
            "user_id": user_id,
            "extension": extension,
            "enabled": enabled,
            "max_message_duration": max_message_duration,
            "max_messages": max_messages,
            "email_notification": email_notification,
            "attach_audio": attach_audio,
            "include_transcription": include_transcription,
            "transcription_enabled": transcription_enabled,
            "greeting_name": greeting_name,
            "greeting_heard": greeting_heard,
            "created_at": created_at,
            "updated_at": updated_at,
        })
        if line_binding_id is not UNSET:
            field_dict["line_binding_id"] = line_binding_id
        if email_address is not UNSET:
            field_dict["email_address"] = email_address
        if retention_days is not UNSET:
            field_dict["retention_days"] = retention_days
        if flow_id is not UNSET:
            field_dict["flow_id"] = flow_id
        if greeting_choice is not UNSET:
            field_dict["greeting_choice"] = greeting_choice

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        tenant_id = d.pop("tenant_id")

        user_id = d.pop("user_id")

        extension = d.pop("extension")

        enabled = d.pop("enabled")

        max_message_duration = d.pop("max_message_duration")

        max_messages = d.pop("max_messages")

        email_notification = d.pop("email_notification")

        attach_audio = d.pop("attach_audio")

        include_transcription = d.pop("include_transcription")

        transcription_enabled = d.pop("transcription_enabled")

        greeting_name = d.pop("greeting_name")

        greeting_heard = d.pop("greeting_heard")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        def _parse_line_binding_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        line_binding_id = _parse_line_binding_id(d.pop("line_binding_id", UNSET))


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


        def _parse_flow_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        flow_id = _parse_flow_id(d.pop("flow_id", UNSET))


        def _parse_greeting_choice(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        greeting_choice = _parse_greeting_choice(d.pop("greeting_choice", UNSET))


        page_mailbox_response_data_item = cls(
            id=id,
            tenant_id=tenant_id,
            user_id=user_id,
            extension=extension,
            enabled=enabled,
            max_message_duration=max_message_duration,
            max_messages=max_messages,
            email_notification=email_notification,
            attach_audio=attach_audio,
            include_transcription=include_transcription,
            transcription_enabled=transcription_enabled,
            greeting_name=greeting_name,
            greeting_heard=greeting_heard,
            created_at=created_at,
            updated_at=updated_at,
            line_binding_id=line_binding_id,
            email_address=email_address,
            retention_days=retention_days,
            flow_id=flow_id,
            greeting_choice=greeting_choice,
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
