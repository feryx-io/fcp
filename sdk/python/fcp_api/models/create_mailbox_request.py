from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset






T = TypeVar("T", bound="CreateMailboxRequest")



@_attrs_define
class CreateMailboxRequest:
    """ Create mailbox request.

        Attributes:
            tenant_id (str):
            user_id (str):
            line_binding_id (str):
            pin (str):
            enabled (bool | Unset):
            max_message_duration (int | Unset):
            max_messages (int | Unset):
     """

    tenant_id: str
    user_id: str
    line_binding_id: str
    pin: str
    enabled: bool | Unset = UNSET
    max_message_duration: int | Unset = UNSET
    max_messages: int | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        tenant_id = self.tenant_id

        user_id = self.user_id

        line_binding_id = self.line_binding_id

        pin = self.pin

        enabled = self.enabled

        max_message_duration = self.max_message_duration

        max_messages = self.max_messages


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "tenant_id": tenant_id,
            "user_id": user_id,
            "line_binding_id": line_binding_id,
            "pin": pin,
        })
        if enabled is not UNSET:
            field_dict["enabled"] = enabled
        if max_message_duration is not UNSET:
            field_dict["max_message_duration"] = max_message_duration
        if max_messages is not UNSET:
            field_dict["max_messages"] = max_messages

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        tenant_id = d.pop("tenant_id")

        user_id = d.pop("user_id")

        line_binding_id = d.pop("line_binding_id")

        pin = d.pop("pin")

        enabled = d.pop("enabled", UNSET)

        max_message_duration = d.pop("max_message_duration", UNSET)

        max_messages = d.pop("max_messages", UNSET)

        create_mailbox_request = cls(
            tenant_id=tenant_id,
            user_id=user_id,
            line_binding_id=line_binding_id,
            pin=pin,
            enabled=enabled,
            max_message_duration=max_message_duration,
            max_messages=max_messages,
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
