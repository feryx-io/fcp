from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="UpdateVoicemailRequest")



@_attrs_define
class UpdateVoicemailRequest:
    """ 
        Attributes:
            pin (None | str | Unset):
            email_notification (bool | None | Unset):
            email_address (None | str | Unset):
            attach_audio (bool | None | Unset):
            greeting_choice (None | str | Unset): `standard`, `personalized` or `own`; empty returns to the default.
            greeting_name (None | str | Unset): `full_name`, `display_name` or `recorded`.
     """

    pin: None | str | Unset = UNSET
    email_notification: bool | None | Unset = UNSET
    email_address: None | str | Unset = UNSET
    attach_audio: bool | None | Unset = UNSET
    greeting_choice: None | str | Unset = UNSET
    greeting_name: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        pin: None | str | Unset
        if isinstance(self.pin, Unset):
            pin = UNSET
        else:
            pin = self.pin

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

        greeting_choice: None | str | Unset
        if isinstance(self.greeting_choice, Unset):
            greeting_choice = UNSET
        else:
            greeting_choice = self.greeting_choice

        greeting_name: None | str | Unset
        if isinstance(self.greeting_name, Unset):
            greeting_name = UNSET
        else:
            greeting_name = self.greeting_name


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if pin is not UNSET:
            field_dict["pin"] = pin
        if email_notification is not UNSET:
            field_dict["email_notification"] = email_notification
        if email_address is not UNSET:
            field_dict["email_address"] = email_address
        if attach_audio is not UNSET:
            field_dict["attach_audio"] = attach_audio
        if greeting_choice is not UNSET:
            field_dict["greeting_choice"] = greeting_choice
        if greeting_name is not UNSET:
            field_dict["greeting_name"] = greeting_name

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


        def _parse_greeting_choice(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        greeting_choice = _parse_greeting_choice(d.pop("greeting_choice", UNSET))


        def _parse_greeting_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        greeting_name = _parse_greeting_name(d.pop("greeting_name", UNSET))


        update_voicemail_request = cls(
            pin=pin,
            email_notification=email_notification,
            email_address=email_address,
            attach_audio=attach_audio,
            greeting_choice=greeting_choice,
            greeting_name=greeting_name,
        )


        update_voicemail_request.additional_properties = d
        return update_voicemail_request

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
