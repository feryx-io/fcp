from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="CreateMailboxRequest")



@_attrs_define
class CreateMailboxRequest:
    """ Create mailbox request.

        Attributes:
            tenant_id (str):
            user_id (str):
            pin (str):
            line_binding_id (None | str | Unset): The line the mailbox answers for, when the user has one. A mailbox
                is the user's: calls find it by user, and it answers the user's
                extension, so a user with no line has one too.
            enabled (bool | Unset):
            max_message_duration (int | Unset):
            max_messages (int | Unset):
            flow_id (None | str | Unset): A flow of the tenant's to handle a call this mailbox would take —
                say something, ring someone, a queue, or a message after all. Unset:
                the mailbox takes it.
            greeting_choice (None | str | Unset): `standard`, `personalized` or `own`; unset is the default —
                personalized when the user has a first and last name.
            greeting_name (None | str | Unset): `full_name` (the default), `display_name` or `recorded`.
     """

    tenant_id: str
    user_id: str
    pin: str
    line_binding_id: None | str | Unset = UNSET
    enabled: bool | Unset = UNSET
    max_message_duration: int | Unset = UNSET
    max_messages: int | Unset = UNSET
    flow_id: None | str | Unset = UNSET
    greeting_choice: None | str | Unset = UNSET
    greeting_name: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        tenant_id = self.tenant_id

        user_id = self.user_id

        pin = self.pin

        line_binding_id: None | str | Unset
        if isinstance(self.line_binding_id, Unset):
            line_binding_id = UNSET
        else:
            line_binding_id = self.line_binding_id

        enabled = self.enabled

        max_message_duration = self.max_message_duration

        max_messages = self.max_messages

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

        greeting_name: None | str | Unset
        if isinstance(self.greeting_name, Unset):
            greeting_name = UNSET
        else:
            greeting_name = self.greeting_name


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "tenant_id": tenant_id,
            "user_id": user_id,
            "pin": pin,
        })
        if line_binding_id is not UNSET:
            field_dict["line_binding_id"] = line_binding_id
        if enabled is not UNSET:
            field_dict["enabled"] = enabled
        if max_message_duration is not UNSET:
            field_dict["max_message_duration"] = max_message_duration
        if max_messages is not UNSET:
            field_dict["max_messages"] = max_messages
        if flow_id is not UNSET:
            field_dict["flow_id"] = flow_id
        if greeting_choice is not UNSET:
            field_dict["greeting_choice"] = greeting_choice
        if greeting_name is not UNSET:
            field_dict["greeting_name"] = greeting_name

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        tenant_id = d.pop("tenant_id")

        user_id = d.pop("user_id")

        pin = d.pop("pin")

        def _parse_line_binding_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        line_binding_id = _parse_line_binding_id(d.pop("line_binding_id", UNSET))


        enabled = d.pop("enabled", UNSET)

        max_message_duration = d.pop("max_message_duration", UNSET)

        max_messages = d.pop("max_messages", UNSET)

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


        def _parse_greeting_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        greeting_name = _parse_greeting_name(d.pop("greeting_name", UNSET))


        create_mailbox_request = cls(
            tenant_id=tenant_id,
            user_id=user_id,
            pin=pin,
            line_binding_id=line_binding_id,
            enabled=enabled,
            max_message_duration=max_message_duration,
            max_messages=max_messages,
            flow_id=flow_id,
            greeting_choice=greeting_choice,
            greeting_name=greeting_name,
        )


        create_mailbox_request.additional_properties = d
        return create_mailbox_request

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
