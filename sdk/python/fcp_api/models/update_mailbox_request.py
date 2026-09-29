from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="UpdateMailboxRequest")



@_attrs_define
class UpdateMailboxRequest:
    """ Update mailbox request.

        Attributes:
            pin (None | str | Unset):
            enabled (bool | None | Unset):
            max_message_duration (int | None | Unset):
            max_messages (int | None | Unset):
            email_notification (bool | None | Unset):
            email_address (None | str | Unset):
            attach_audio (bool | None | Unset):
            include_transcription (bool | None | Unset):
            transcription_enabled (bool | None | Unset):
            retention_days (int | None | Unset): Double Option: outer = field not provided, inner = null to clear.
     """

    pin: None | str | Unset = UNSET
    enabled: bool | None | Unset = UNSET
    max_message_duration: int | None | Unset = UNSET
    max_messages: int | None | Unset = UNSET
    email_notification: bool | None | Unset = UNSET
    email_address: None | str | Unset = UNSET
    attach_audio: bool | None | Unset = UNSET
    include_transcription: bool | None | Unset = UNSET
    transcription_enabled: bool | None | Unset = UNSET
    retention_days: int | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        pin: None | str | Unset
        if isinstance(self.pin, Unset):
            pin = UNSET
        else:
            pin = self.pin

        enabled: bool | None | Unset
        if isinstance(self.enabled, Unset):
            enabled = UNSET
        else:
            enabled = self.enabled

        max_message_duration: int | None | Unset
        if isinstance(self.max_message_duration, Unset):
            max_message_duration = UNSET
        else:
            max_message_duration = self.max_message_duration

        max_messages: int | None | Unset
        if isinstance(self.max_messages, Unset):
            max_messages = UNSET
        else:
            max_messages = self.max_messages

        email_notification: bool | None | Unset
        if isinstance(self.email_notification, Unset):
            email_notification = UNSET
        else:
            email_notification = self.email_notification

        email_address: None | str | Unset
        if isinstance(self.email_address, Unset):
            email_address = UNSET
        else:
            email_address = self.email_address

        attach_audio: bool | None | Unset
        if isinstance(self.attach_audio, Unset):
            attach_audio = UNSET
        else:
            attach_audio = self.attach_audio

        include_transcription: bool | None | Unset
        if isinstance(self.include_transcription, Unset):
            include_transcription = UNSET
        else:
            include_transcription = self.include_transcription

        transcription_enabled: bool | None | Unset
        if isinstance(self.transcription_enabled, Unset):
            transcription_enabled = UNSET
        else:
            transcription_enabled = self.transcription_enabled

        retention_days: int | None | Unset
        if isinstance(self.retention_days, Unset):
            retention_days = UNSET
        else:
            retention_days = self.retention_days


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if pin is not UNSET:
            field_dict["pin"] = pin
        if enabled is not UNSET:
            field_dict["enabled"] = enabled
        if max_message_duration is not UNSET:
            field_dict["max_message_duration"] = max_message_duration
        if max_messages is not UNSET:
            field_dict["max_messages"] = max_messages
        if email_notification is not UNSET:
            field_dict["email_notification"] = email_notification
        if email_address is not UNSET:
            field_dict["email_address"] = email_address
        if attach_audio is not UNSET:
            field_dict["attach_audio"] = attach_audio
        if include_transcription is not UNSET:
            field_dict["include_transcription"] = include_transcription
        if transcription_enabled is not UNSET:
            field_dict["transcription_enabled"] = transcription_enabled
        if retention_days is not UNSET:
            field_dict["retention_days"] = retention_days

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        def _parse_pin(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        pin = _parse_pin(d.pop("pin", UNSET))


        def _parse_enabled(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        enabled = _parse_enabled(d.pop("enabled", UNSET))


        def _parse_max_message_duration(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        max_message_duration = _parse_max_message_duration(d.pop("max_message_duration", UNSET))


        def _parse_max_messages(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        max_messages = _parse_max_messages(d.pop("max_messages", UNSET))


        def _parse_email_notification(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        email_notification = _parse_email_notification(d.pop("email_notification", UNSET))


        def _parse_email_address(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        email_address = _parse_email_address(d.pop("email_address", UNSET))


        def _parse_attach_audio(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        attach_audio = _parse_attach_audio(d.pop("attach_audio", UNSET))


        def _parse_include_transcription(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        include_transcription = _parse_include_transcription(d.pop("include_transcription", UNSET))


        def _parse_transcription_enabled(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        transcription_enabled = _parse_transcription_enabled(d.pop("transcription_enabled", UNSET))


        def _parse_retention_days(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        retention_days = _parse_retention_days(d.pop("retention_days", UNSET))


        update_mailbox_request = cls(
            pin=pin,
            enabled=enabled,
            max_message_duration=max_message_duration,
            max_messages=max_messages,
            email_notification=email_notification,
            email_address=email_address,
            attach_audio=attach_audio,
            include_transcription=include_transcription,
            transcription_enabled=transcription_enabled,
            retention_days=retention_days,
        )


        update_mailbox_request.additional_properties = d
        return update_mailbox_request

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
